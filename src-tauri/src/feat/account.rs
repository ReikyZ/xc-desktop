use crate::{
    config::{
        Config, IProfiles, PrfItem, PrfOption,
        profiles::{
            profiles_append_item_safe, profiles_patch_item_safe, profiles_save_file_safe,
            PROFILE_WRITE_LOCK,
        },
    },
    core::{handle, timer::Timer, CoreManager},
    utils::dirs,
};
use anyhow::{anyhow, bail, Context as _, Result};
use clash_verge_logging::{Type, logging, logging_error};
use once_cell::sync::OnceCell;
use reqwest::header::{HeaderMap, COOKIE, SET_COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;
use smartstring::alias::String as SString;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;

/// Production API host. Domain in the URL string is required; keep it out of module/file names.
/// Override with runtime env `XC_API_BASE` or compile-time `XC_API_BASE`.
pub fn api_base() -> &'static str {
    static BASE: OnceCell<std::string::String> = OnceCell::new();
    BASE.get_or_init(|| {
        std::env::var("XC_API_BASE")
            .ok()
            .or_else(|| option_env!("XC_API_BASE").map(ToString::to_string))
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "https://xrozzz.pro".to_string())
    })
}

pub const CLASH_UA: &str = "clash-verge/xc";
const PROFILE_NAME: &str = "XC";
const PROFILE_MARKER: &str = "xc-account-subscription";
const CLIENT_UA: &str = "XC-Desktop/0.1.0";
/// P0: hide generic add-remote-URL. Set true for power-user manual profiles.
pub const ALLOW_MANUAL_PROFILES: bool = false;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionStats {
    pub data_remain: f64,
    pub data_total: f64,
    pub days_remain: i64,
    pub days_total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountState {
    pub logged_in: bool,
    pub email: Option<std::string::String>,
    pub has_subscription: bool,
    pub subscription_updated_at: Option<std::string::String>,
    pub stats: Option<SubscriptionStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedMeta {
    email: Option<std::string::String>,
    subscription_updated_at: Option<std::string::String>,
    has_subscription: bool,
    profile_uid: Option<std::string::String>,
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    #[serde(default)]
    email: Option<std::string::String>,
    #[serde(default)]
    data: Option<UserInfoData>,
}

#[derive(Debug, Deserialize)]
struct UserInfoData {
    #[serde(default)]
    email: Option<std::string::String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    token: Option<std::string::String>,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

#[derive(Default)]
struct CookieJar {
    pairs: HashMap<std::string::String, std::string::String>,
}

impl CookieJar {
    fn load(path: &PathBuf) -> Self {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(pairs) = serde_json::from_str::<HashMap<std::string::String, std::string::String>>(&text) {
                return Self { pairs };
            }
        }
        Self::default()
    }

    fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.pairs)?;
        fs::write(path, text)?;
        Ok(())
    }

    fn header_value(&self) -> Option<std::string::String> {
        if self.pairs.is_empty() {
            return None;
        }
        Some(
            self.pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    fn clear(&mut self) {
        self.pairs.clear();
    }

    fn absorb_set_cookie(&mut self, headers: &HeaderMap) {
        for value in headers.get_all(SET_COOKIE) {
            let Ok(raw) = value.to_str() else { continue };
            let Some(pair) = raw.split(';').next() else { continue };
            let Some((name, val)) = pair.split_once('=') else { continue };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            // Deletion / expired cookies often come as empty value with Max-Age=0.
            let lower = raw.to_ascii_lowercase();
            if lower.contains("max-age=0") || lower.contains("expires=thu, 01 jan 1970") {
                self.pairs.remove(name);
                continue;
            }
            self.pairs
                .insert(name.to_string(), val.trim().to_string());
        }
    }
}

pub struct AccountManager {
    http: reqwest::Client,
    cookies: Arc<Mutex<CookieJar>>,
    cookie_path: PathBuf,
    meta_path: PathBuf,
    state: AsyncMutex<AccountState>,
}

impl AccountManager {
    fn paths() -> Result<(PathBuf, PathBuf)> {
        let dir = dirs::app_home_dir()?.join("account");
        fs::create_dir_all(&dir)?;
        Ok((dir.join("cookies.json"), dir.join("meta.json")))
    }

    pub fn global() -> &'static Self {
        static INSTANCE: OnceCell<AccountManager> = OnceCell::new();
        INSTANCE.get_or_init(|| {
            Self::new().unwrap_or_else(|e| {
                logging!(error, Type::Cmd, "account manager init failed: {e:#}");
                // Fall back to temp paths so the process can still boot.
                let dir = std::env::temp_dir().join("xc-account");
                let _ = fs::create_dir_all(&dir);
                let cookies = Arc::new(Mutex::new(CookieJar::default()));
                let http = reqwest::Client::builder()
                    .user_agent(CLIENT_UA)
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .expect("http client");
                Self {
                    http,
                    cookies,
                    cookie_path: dir.join("cookies.json"),
                    meta_path: dir.join("meta.json"),
                    state: AsyncMutex::new(AccountState::default()),
                }
            })
        })
    }

    fn new() -> Result<Self> {
        let (cookie_path, meta_path) = Self::paths()?;
        let jar = CookieJar::load(&cookie_path);
        let meta = load_meta(&meta_path);
        let state = AccountState {
            logged_in: false,
            email: meta.email.clone(),
            has_subscription: meta.has_subscription,
            subscription_updated_at: meta.subscription_updated_at.clone(),
            stats: None,
        };
        let http = reqwest::Client::builder()
            .user_agent(CLIENT_UA)
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            cookies: Arc::new(Mutex::new(jar)),
            cookie_path,
            meta_path,
            state: AsyncMutex::new(state),
        })
    }

    fn persist_cookies(&self) -> Result<()> {
        let guard = self
            .cookies
            .lock()
            .map_err(|_| anyhow!("cookie lock poisoned"))?;
        guard.save(&self.cookie_path)
    }

    fn apply_cookie_header(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        let guard = self
            .cookies
            .lock()
            .map_err(|_| anyhow!("cookie lock poisoned"))?;
        if let Some(header) = guard.header_value() {
            Ok(req.header(COOKIE, header))
        } else {
            Ok(req)
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let req = self.apply_cookie_header(req)?;
        let res = req.send().await?;
        {
            let mut guard = self
                .cookies
                .lock()
                .map_err(|_| anyhow!("cookie lock poisoned"))?;
            guard.absorb_set_cookie(res.headers());
        }
        let _ = self.persist_cookies();
        Ok(res)
    }

    pub async fn snapshot(&self) -> AccountState {
        self.state.lock().await.clone()
    }

    pub async fn send_code(&self, email: std::string::String) -> Result<()> {
        let email = email.trim().to_string();
        if email.is_empty() || !email.contains('@') {
            bail!("Invalid email");
        }
        let url = format!("{}/action/code", api_base());
        let res = self
            .send(
                self.http
                    .post(&url)
                    .header("Content-Type", "text/plain")
                    .body(email),
            )
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Send code failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        Ok(())
    }

    pub async fn login(
        &self,
        email: std::string::String,
        code: std::string::String,
    ) -> Result<AccountState> {
        let email = email.trim().to_string();
        let code = code.trim().to_string();
        if email.is_empty() || code.is_empty() {
            bail!("Email and code are required");
        }
        let url = format!("{}/action/verify", api_base());
        let res = self
            .send(
                self.http
                    .post(&url)
                    .json(&json!({ "email": email, "password": code })),
            )
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Login failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }

        let info = self.fetch_user_info().await?;
        let resolved = extract_email(info).unwrap_or(email);
        {
            let mut st = self.state.lock().await;
            st.logged_in = true;
            st.email = Some(resolved.clone());
        }
        self.save_meta_from_state().await?;
        if let Err(e) = self.refresh_subscription().await {
            logging!(warn, Type::Cmd, "subscription import after login: {e:#}");
        }
        Ok(self.snapshot().await)
    }

    pub async fn logout(&self) -> Result<AccountState> {
        let url = format!("{}/action/signout", api_base());
        let _ = self.send(self.http.get(&url)).await;

        // Remove local subscription profile artifact.
        if let Err(e) = remove_subscription_profile().await {
            logging!(warn, Type::Cmd, "remove subscription profile: {e:#}");
        }

        {
            let mut guard = self
                .cookies
                .lock()
                .map_err(|_| anyhow!("cookie lock poisoned"))?;
            guard.clear();
        }
        self.persist_cookies()?;
        let _ = save_meta(&self.meta_path, &PersistedMeta::default());
        {
            let mut st = self.state.lock().await;
            *st = AccountState::default();
        }
        Ok(self.snapshot().await)
    }

    async fn fetch_user_info(&self) -> Result<UserInfoResponse> {
        let url = format!("{}/api/user/info", api_base());
        let res = self.send(self.http.get(&url)).await?;
        let status = res.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("Session expired");
        }
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!(
                "User info failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        let text = res.text().await?;
        serde_json::from_str::<UserInfoResponse>(&text).with_context(|| {
            format!(
                "User info decode failed; body={}",
                truncate(&text, 200)
            )
        })
    }

    pub async fn restore_session(&self) -> Result<AccountState> {
        match self.fetch_user_info().await {
            Ok(info) => {
                let email = extract_email(info);
                let meta = load_meta(&self.meta_path);
                {
                    let mut st = self.state.lock().await;
                    st.logged_in = email.is_some();
                    st.email = email.or(meta.email);
                    st.has_subscription = meta.has_subscription;
                    st.subscription_updated_at = meta.subscription_updated_at;
                }
                if self.state.lock().await.logged_in {
                    if let Err(e) = self.refresh_subscription().await {
                        logging!(warn, Type::Cmd, "subscription refresh on restore: {e:#}");
                    }
                    if let Ok(stats) = self.fetch_statistics().await {
                        self.state.lock().await.stats = Some(stats);
                    }
                }
                Ok(self.snapshot().await)
            }
            Err(_) => {
                let mut st = self.state.lock().await;
                st.logged_in = false;
                Ok(st.clone())
            }
        }
    }

    async fn fetch_subscription_token(&self) -> Result<std::string::String> {
        let url = format!("{}/api/user/subscription/token", api_base());
        let res = self.send(self.http.get(&url)).await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Token fetch failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        let text = res.text().await?;
        let body = serde_json::from_str::<TokenResponse>(&text).with_context(|| {
            format!("Token decode failed; body={}", truncate(&text, 200))
        })?;
        Ok(extract_token(body).unwrap_or_default())
    }

    async fn activate_subscription(&self) -> Result<()> {
        let url = format!("{}/api/activate", api_base());
        let res = self.send(self.http.post(&url)).await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Activate failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        Ok(())
    }

    async fn ensure_token(&self) -> Result<std::string::String> {
        let mut token = self.fetch_subscription_token().await?;
        if token.trim().is_empty() {
            self.activate_subscription().await?;
            token = self.fetch_subscription_token().await?;
        }
        if token.trim().is_empty() {
            bail!("Subscription token missing after activate");
        }
        Ok(token)
    }

    async fn download_clash_config(&self, token: &str) -> Result<std::string::String> {
        let url = format!("{}/external/user/subscription/config/clash/{token}", api_base());
        let res = self
            .send(self.http.get(&url).header(USER_AGENT, CLASH_UA))
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Config download failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        let text = res.text().await?;
        if text.trim().is_empty() {
            bail!("Empty subscription config");
        }
        Ok(text)
    }

    pub async fn refresh_subscription(&self) -> Result<AccountState> {
        let mut token = self.ensure_token().await?;
        let config_url = format!(
            "{}/external/user/subscription/config/clash/{token}",
            api_base()
        );

        // Prefer downloading with our clash UA; fall back to activate+retry once.
        let yaml = match self.download_clash_config(&token).await {
            Ok(c) => c,
            Err(first) => {
                logging!(warn, Type::Cmd, "config download failed, activating: {first:#}");
                self.activate_subscription().await?;
                token = self.ensure_token().await?;
                self.download_clash_config(&token)
                    .await
                    .map_err(|e| anyhow!("{first:#}; retry: {e:#}"))?
            }
        };

        let uid = import_or_refresh_profile(&config_url, &yaml).await?;

        // Make it current and refresh runtime.
        set_current_profile(&uid).await?;
        if let Err(e) = CoreManager::global().update_config_forced().await {
            logging!(warn, Type::Cmd, "enhance after subscription import: {e:#}");
        }
        handle::Handle::refresh_clash();

        let stats = self.fetch_statistics().await.ok();
        let updated_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        {
            let mut st = self.state.lock().await;
            st.has_subscription = true;
            st.subscription_updated_at = Some(updated_at.clone());
            st.stats = stats;
        }
        let email = self.state.lock().await.email.clone();
        save_meta(
            &self.meta_path,
            &PersistedMeta {
                email,
                subscription_updated_at: Some(updated_at),
                has_subscription: true,
                profile_uid: Some(uid),
            },
        )?;
        Ok(self.snapshot().await)
    }

    async fn fetch_statistics(&self) -> Result<SubscriptionStats> {
        let url = format!("{}/api/user/subscription/statistics", api_base());
        let res = self.send(self.http.get(&url)).await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            bail!(
                "Statistics failed ({status}{}){}",
                status_hint(status),
                format_body(&text)
            );
        }
        let text = res.text().await?;
        let body: StatsResponse = serde_json::from_str(&text).with_context(|| {
            format!("Statistics decode failed; body={}", truncate(&text, 200))
        })?;
        let data = body.data.unwrap_or_default();
        Ok(SubscriptionStats {
            data_remain: data.data.as_ref().map(|d| d.remain).unwrap_or(0.0),
            data_total: data.data.as_ref().map(|d| d.total).unwrap_or(0.0),
            days_remain: data.days.as_ref().map(|d| d.remain).unwrap_or(0),
            days_total: data.days.as_ref().map(|d| d.total).unwrap_or(0),
        })
    }

    async fn save_meta_from_state(&self) -> Result<()> {
        let st = self.state.lock().await.clone();
        let prev = load_meta(&self.meta_path);
        save_meta(
            &self.meta_path,
            &PersistedMeta {
                email: st.email,
                subscription_updated_at: st.subscription_updated_at.or(prev.subscription_updated_at),
                has_subscription: st.has_subscription || prev.has_subscription,
                profile_uid: prev.profile_uid,
            },
        )
    }
}

fn load_meta(path: &PathBuf) -> PersistedMeta {
    if let Ok(text) = fs::read_to_string(path) {
        if let Ok(meta) = serde_json::from_str(&text) {
            return meta;
        }
    }
    PersistedMeta::default()
}

fn save_meta(path: &PathBuf, meta: &PersistedMeta) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(meta)?)?;
    Ok(())
}

fn extract_email(info: UserInfoResponse) -> Option<std::string::String> {
    info.email
        .or_else(|| info.data.and_then(|d| d.email))
        .filter(|e| !e.is_empty())
}

fn extract_token(body: TokenResponse) -> Option<std::string::String> {
    if let Some(t) = body.token.filter(|t| !t.is_empty()) {
        return Some(t);
    }
    match body.data {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s),
        Some(serde_json::Value::Object(map)) => map
            .get("token")
            .and_then(|v| v.as_str())
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string()),
        _ => None,
    }
}

#[derive(Debug, Deserialize, Default)]
struct StatsResponse {
    #[serde(default)]
    data: Option<StatsPayload>,
}

#[derive(Debug, Deserialize, Default)]
struct StatsPayload {
    #[serde(default)]
    data: Option<RemainTotalF64>,
    #[serde(default)]
    days: Option<RemainTotalI64>,
}

#[derive(Debug, Deserialize, Default)]
struct RemainTotalF64 {
    #[serde(default)]
    remain: f64,
    #[serde(default)]
    total: f64,
}

#[derive(Debug, Deserialize, Default)]
struct RemainTotalI64 {
    #[serde(default)]
    remain: i64,
    #[serde(default)]
    total: i64,
}

fn status_hint(status: reqwest::StatusCode) -> &'static str {
    match status.as_u16() {
        400 => " bad request",
        403 => " forbidden",
        503 => " service unavailable",
        _ => "",
    }
}

fn format_body(text: &str) -> std::string::String {
    let clipped = truncate(text, 200);
    if clipped.is_empty() {
        std::string::String::new()
    } else {
        format!(": {clipped}")
    }
}

fn truncate(s: &str, max: usize) -> std::string::String {
    let t = s.trim();
    if t.chars().count() <= max {
        t.to_string()
    } else {
        let clipped: std::string::String = t.chars().take(max).collect();
        format!("{clipped}…")
    }
}

fn find_existing_uid(items: &[PrfItem], url: &str, meta_uid: Option<&str>) -> Option<SString> {
    if let Some(uid) = meta_uid {
        if items.iter().any(|i| i.uid.as_deref() == Some(uid)) {
            return Some(uid.into());
        }
    }
    for item in items {
        if item.desc.as_deref() == Some(PROFILE_MARKER) {
            return item.uid.clone();
        }
        if item.name.as_deref() == Some(PROFILE_NAME)
            && item
                .url
                .as_deref()
                .is_some_and(|u| u.contains("/external/user/subscription/config/clash/"))
        {
            return item.uid.clone();
        }
        if item
            .url
            .as_deref()
            .is_some_and(|u| u == url || u.contains("/external/user/subscription/config/clash/"))
            && item.name.as_deref() == Some(PROFILE_NAME)
        {
            return item.uid.clone();
        }
    }
    None
}

async fn import_or_refresh_profile(url: &str, yaml: &str) -> Result<std::string::String> {
    let _guard = PROFILE_WRITE_LOCK.lock().await;
    let meta = AccountManager::paths()
        .ok()
        .map(|(_, meta_path)| load_meta(&meta_path));
    let meta_uid = meta.as_ref().and_then(|m| m.profile_uid.as_deref());

    let profiles = Config::profiles().await;
    let data = profiles.data_arc();
    let items = data.items.clone().unwrap_or_default();

    if let Some(uid) = find_existing_uid(&items, url, meta_uid) {
        // Update remote URL + UA option and rewrite file contents.
        let option = PrfOption {
            user_agent: Some(CLASH_UA.into()),
            with_proxy: Some(false),
            self_proxy: Some(false),
            allow_auto_update: Some(true),
            update_interval: Some(60),
            ..PrfOption::default()
        };
        let patch = PrfItem {
            name: Some(PROFILE_NAME.into()),
            desc: Some(PROFILE_MARKER.into()),
            url: Some(url.into()),
            itype: Some("remote".into()),
            updated: Some(chrono::Local::now().timestamp() as usize),
            option: Some(option),
            ..PrfItem::default()
        };
        profiles_patch_item_safe(&uid, &patch).await?;

        // Write YAML into the existing profile file.
        let latest = Config::profiles().await.data_arc();
        if let Ok(item) = latest.get_item(&uid) {
            if let Some(file) = item.file.as_ref() {
                let path = dirs::app_profiles_dir()?.join(file.as_str());
                fs::write(&path, yaml.as_bytes())?;
            }
        }
        profiles_save_file_safe().await?;
        logging_error!(Type::Timer, Timer::global().refresh().await);
        handle::Handle::notify_profile_changed(&uid);
        return Ok(uid.to_string());
    }

    // Fresh remote profile via from_url (downloads again with UA) — but we already have YAML,
    // so create as remote metadata and write file via append with file_data by crafting item.
    let option = PrfOption {
        user_agent: Some(CLASH_UA.into()),
        with_proxy: Some(false),
        self_proxy: Some(false),
        allow_auto_update: Some(true),
        update_interval: Some(60),
        ..PrfOption::default()
    };

    // Use from_url so merge/script/rules chains are created correctly.
    let name: SString = PROFILE_NAME.into();
    let desc: SString = PROFILE_MARKER.into();
    let mut item = PrfItem::from_url(url, Some(&name), Some(&desc), Some(&option))
        .await
        .context("import subscription profile")?;

    // Prefer the YAML fetched with the account client (clash UA + token URL).
    if let Some(file) = item.file.as_ref() {
        let path = dirs::app_profiles_dir()?.join(file.as_str());
        fs::write(&path, yaml.as_bytes())?;
    }
    profiles_append_item_safe(&mut item).await?;
    profiles_save_file_safe().await?;
    logging_error!(Type::Timer, Timer::global().refresh().await);
    let uid = item
        .uid
        .clone()
        .ok_or_else(|| anyhow!("profile uid missing after import"))?;
    handle::Handle::notify_profile_changed(&uid);
    Ok(uid.to_string())
}

async fn set_current_profile(uid: &str) -> Result<()> {
    let profiles = Config::profiles().await;
    let uid_owned: SString = uid.into();
    profiles
        .with_data_modify(|mut committed| async move {
            committed.patch_config(&IProfiles {
                current: Some(uid_owned),
                items: None,
            });
            Ok((committed, ()))
        })
        .await?;
    profiles_save_file_safe().await?;
    let uid_s: SString = uid.into();
    handle::Handle::notify_profile_changed(&uid_s);
    Ok(())
}

/// Returns the subscription profile uid if one exists (for logout cleanup).
pub async fn find_subscription_profile_uid() -> Option<std::string::String> {
    let meta = AccountManager::paths()
        .map(|(_, p)| load_meta(&p))
        .unwrap_or_default();
    let profiles = Config::profiles().await;
    let items = profiles.data_arc().items.clone().unwrap_or_default();
    find_existing_uid(&items, "", meta.profile_uid.as_deref()).map(|u| u.to_string())
}

async fn remove_subscription_profile() -> Result<()> {
    // Best-effort: clear marker meta; actual profile deletion is done in cmd layer
    // to reuse the battle-tested delete_profile command.
    let Some(uid) = find_subscription_profile_uid().await else {
        return Ok(());
    };
    logging!(info, Type::Cmd, "subscription profile pending delete: {uid}");
    let _ = uid;
    Ok(())
}

/// Thin wrappers used by cmd module.
pub async fn send_code(email: std::string::String) -> Result<()> {
    AccountManager::global().send_code(email).await
}

pub async fn login(email: std::string::String, code: std::string::String) -> Result<AccountState> {
    AccountManager::global().login(email, code).await
}

pub async fn logout() -> Result<AccountState> {
    AccountManager::global().logout().await
}

pub async fn restore_session() -> Result<AccountState> {
    AccountManager::global().restore_session().await
}

pub async fn refresh_subscription() -> Result<AccountState> {
    AccountManager::global().refresh_subscription().await
}

pub async fn get_account_state() -> AccountState {
    AccountManager::global().snapshot().await
}
