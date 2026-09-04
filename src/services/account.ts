import { invoke } from '@tauri-apps/api/core'

export type AccountState = {
  logged_in: boolean
  email: string | null
  has_subscription: boolean
  subscription_updated_at: string | null
  stats?: {
    data_remain: number
    data_total: number
    days_remain: number
    days_total: number
  } | null
}

export async function accountSendCode(email: string) {
  return invoke<void>('account_send_code', { email })
}

export async function accountLogin(email: string, code: string) {
  return invoke<AccountState>('account_login', { email, code })
}

export async function accountLogout() {
  return invoke<AccountState>('account_logout')
}

export async function accountRestoreSession() {
  return invoke<AccountState>('account_restore_session')
}

export async function accountRefreshSubscription() {
  return invoke<AccountState>('account_refresh_subscription')
}

export async function accountGetState() {
  return invoke<AccountState>('account_get_state')
}
