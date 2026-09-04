import { useCallback } from 'react'

import { useProfiles } from '@/hooks/use-profiles'
import { useAccount } from '@/providers/account-provider'
import {
  accountLogout,
  accountRefreshSubscription,
} from '@/services/account'
import { errorDetail, showNotice } from '@/services/notice-service'

import { SettingItem, SettingList } from './mods/setting-comp'

function formatStats(account: {
  stats?: {
    data_remain: number
    data_total: number
    days_remain: number
    days_total: number
  } | null
  subscription_updated_at: string | null
}): string {
  const parts: string[] = []
  const stats = account.stats
  if (stats) {
    parts.push(
      `${stats.data_remain.toFixed(2)} / ${stats.data_total.toFixed(2)} GB`,
    )
    parts.push(`${stats.days_remain} / ${stats.days_total} days`)
  }
  if (account.subscription_updated_at) {
    parts.push(`updated ${account.subscription_updated_at}`)
  }
  return parts.join(' · ')
}

const SettingAccount = () => {
  const { account, setAccount } = useAccount()
  const { mutateProfiles } = useProfiles()

  const onRefresh = useCallback(async () => {
    try {
      const state = await accountRefreshSubscription()
      setAccount(state)
      await mutateProfiles()
      showNotice.success('Subscription refreshed')
    } catch (e) {
      showNotice.error(errorDetail(e) || String(e))
    }
  }, [mutateProfiles, setAccount])

  const onLogout = useCallback(async () => {
    try {
      const state = await accountLogout()
      setAccount(state)
      showNotice.success('Signed out')
    } catch (e) {
      showNotice.error(errorDetail(e) || String(e))
    }
  }, [setAccount])

  return (
    <SettingList title="Account">
      <SettingItem
        label={account?.email || 'Signed in'}
        secondary={account ? formatStats(account) : undefined}
      />
      <SettingItem label="Refresh subscription" onClick={onRefresh} />
      <SettingItem label="Sign out" onClick={onLogout} />
    </SettingList>
  )
}

export default SettingAccount
