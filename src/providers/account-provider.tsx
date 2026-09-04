import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react'

import {
  accountGetState,
  accountRestoreSession,
  type AccountState,
} from '@/services/account'

type AccountContextValue = {
  ready: boolean
  loggedIn: boolean
  account: AccountState | null
  error: string | null
  refreshAccount: () => Promise<void>
  setAccount: (state: AccountState | null) => void
  setError: (msg: string | null) => void
}

const defaultAccount: AccountState = {
  logged_in: false,
  email: null,
  has_subscription: false,
  subscription_updated_at: null,
  stats: null,
}

const AccountContext = createContext<AccountContextValue | null>(null)

export function AccountProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(false)
  const [account, setAccount] = useState<AccountState | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refreshAccount = useCallback(async () => {
    const state = await accountGetState()
    setAccount(state)
  }, [])

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      try {
        const state = await accountRestoreSession()
        if (!cancelled) setAccount(state)
      } catch {
        if (!cancelled) setAccount(defaultAccount)
      } finally {
        if (!cancelled) setReady(true)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  const value = useMemo<AccountContextValue>(
    () => ({
      ready,
      loggedIn: Boolean(account?.logged_in),
      account,
      error,
      refreshAccount,
      setAccount,
      setError,
    }),
    [ready, account, error, refreshAccount],
  )

  return (
    <AccountContext.Provider value={value}>{children}</AccountContext.Provider>
  )
}

export function useAccount(): AccountContextValue {
  const ctx = useContext(AccountContext)
  if (!ctx) {
    throw new Error('useAccount must be used within AccountProvider')
  }
  return ctx
}
