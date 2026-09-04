use super::{CmdResult, StringifyErr as _};
use crate::feat::account::{self, AccountState};
use clash_verge_logging::{Type, logging};

#[tauri::command]
pub async fn account_send_code(email: String) -> CmdResult {
    account::send_code(email).await.stringify_err()
}

#[tauri::command]
pub async fn account_login(email: String, code: String) -> CmdResult<AccountState> {
    account::login(email, code).await.stringify_err()
}

#[tauri::command]
pub async fn account_logout() -> CmdResult<AccountState> {
    let uid = account::find_subscription_profile_uid().await;
    if let Some(uid) = uid {
        if let Err(e) = crate::cmd::delete_profile(uid.clone().into()).await {
            logging!(warn, Type::Cmd, "logout profile delete failed: {e}");
        }
    }
    if let Err(e) = crate::core::CoreManager::global().stop_core().await {
        logging!(warn, Type::Cmd, "logout stop core failed: {e:#}");
    }
    account::logout().await.stringify_err()
}

#[tauri::command]
pub async fn account_restore_session() -> CmdResult<AccountState> {
    account::restore_session().await.stringify_err()
}

#[tauri::command]
pub async fn account_refresh_subscription() -> CmdResult<AccountState> {
    account::refresh_subscription().await.stringify_err()
}

#[tauri::command]
pub async fn account_get_state() -> CmdResult<AccountState> {
    Ok(account::get_account_state().await)
}
