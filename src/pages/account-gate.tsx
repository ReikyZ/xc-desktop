import { Box, Typography } from '@mui/material'
import { Outlet } from 'react-router'

import { useAccount } from '@/providers/account-provider'

import LoginPage from './login'

/** Login gate before the main Verge shell mounts. */
export default function AccountGate() {
  const { ready, loggedIn } = useAccount()

  if (!ready) {
    return (
      <Box
        sx={{
          minHeight: '100vh',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          bgcolor: 'background.default',
        }}
      >
        <Typography color="text.secondary">Restoring session…</Typography>
      </Box>
    )
  }

  if (!loggedIn) {
    return <LoginPage />
  }

  return <Outlet />
}
