import { Box, Button, Paper, Stack, TextField, Typography } from '@mui/material'
import { useEffect, useState, type FormEvent } from 'react'

import { useAccount } from '@/providers/account-provider'
import {
  accountLogin,
  accountRefreshSubscription,
  accountSendCode,
} from '@/services/account'
import { errorDetail } from '@/services/notice-service'

const COOLDOWN_SEC = 60

export default function LoginPage() {
  const { setAccount, setError, error } = useAccount()
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [codeSent, setCodeSent] = useState(false)
  const [cooldown, setCooldown] = useState(0)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<string | null>(null)

  useEffect(() => {
    if (cooldown <= 0) return
    const id = window.setInterval(() => {
      setCooldown((c) => (c > 0 ? c - 1 : 0))
    }, 1000)
    return () => window.clearInterval(id)
  }, [cooldown])

  async function onSendCode() {
    setError(null)
    setStatus(null)
    const trimmed = email.trim()
    if (!trimmed || !trimmed.includes('@')) {
      setError('Enter a valid email address.')
      return
    }
    setBusy(true)
    try {
      await accountSendCode(trimmed)
      setCodeSent(true)
      setCooldown(COOLDOWN_SEC)
      setStatus('Verification code sent. Check your inbox.')
    } catch (e) {
      setError(errorDetail(e) || String(e))
    } finally {
      setBusy(false)
    }
  }

  async function onVerify(e: FormEvent) {
    e.preventDefault()
    setError(null)
    setStatus(null)
    const trimmed = email.trim()
    const otp = code.trim()
    if (!trimmed || !otp) {
      setError('Email and verification code are required.')
      return
    }
    setBusy(true)
    try {
      let state = await accountLogin(trimmed, otp)
      try {
        state = await accountRefreshSubscription()
      } catch {
        // Non-fatal: shell can retry later.
      }
      setAccount(state)
    } catch (err) {
      setError(errorDetail(err) || String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Box
      sx={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        bgcolor: 'background.default',
        p: 2,
      }}
    >
      <Paper sx={{ p: 4, width: '100%', maxWidth: 420 }} elevation={3}>
        <Typography variant="h4" component="h1" gutterBottom fontWeight={700}>
          XC
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 3 }}>
          Sign in with email verification
        </Typography>
        <Stack component="form" spacing={2} onSubmit={onVerify}>
          <TextField
            label="Email"
            type="email"
            autoComplete="email"
            value={email}
            onChange={(ev) => setEmail(ev.target.value)}
            placeholder="you@example.com"
            disabled={busy}
            required
            fullWidth
          />
          <Button
            type="button"
            variant="outlined"
            onClick={onSendCode}
            disabled={busy || cooldown > 0}
          >
            {cooldown > 0 ? `Resend (${cooldown}s)` : 'Send code'}
          </Button>
          <TextField
            label="Verification code"
            value={code}
            onChange={(ev) => setCode(ev.target.value)}
            placeholder={codeSent ? 'Enter code' : 'Send a code first'}
            disabled={busy}
            required
            fullWidth
            autoComplete="one-time-code"
          />
          {error ? (
            <Typography color="error" variant="body2">
              {error}
            </Typography>
          ) : null}
          {status ? (
            <Typography color="success.main" variant="body2">
              {status}
            </Typography>
          ) : null}
          <Button type="submit" variant="contained" disabled={busy} size="large">
            {busy ? 'Signing in…' : 'Sign in'}
          </Button>
        </Stack>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ display: 'block', mt: 3 }}
        >
          Based on Clash Verge Rev (GPL-3.0)
        </Typography>
      </Paper>
    </Box>
  )
}
