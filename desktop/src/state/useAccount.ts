import { useCallback, useEffect, useRef, useState } from 'react'
import type { AccountSnapshot, GatewayStatus } from '@/lib/ipc-types'
import { accountGet, accountRefreshToken } from '@/lib/tauri-account'
import { errorMessage } from '@/lib/ipc-error'
import { createAccountLoader } from '@/features/account/loader'

/** Keep account data scoped to the running process and serialize both refresh actions. */
export function useAccount(gateway: GatewayStatus | null) {
  const key = gateway?.state === 'running' ? `${gateway.pid}:${gateway.since}:${gateway.port}` : null
  const [state, setState] = useState<{ key: string | null; data: AccountSnapshot | null; pending: 'usage' | 'token' | null; error: string | null; refreshed: boolean }>({ key: null, data: null, pending: null, error: null, refreshed: false })
  const loader = useRef(createAccountLoader<AccountSnapshot>()).current
  const active = useRef(key)
  active.current = key

  const refresh = useCallback(async (kind: 'usage' | 'token' = 'usage') => {
    const session = active.current
    if (!session) return
    setState((s) => ({ ...s, key: session, pending: s.pending ?? kind, error: null, refreshed: false }))
    const result = await loader.run(kind === 'token' ? accountRefreshToken : accountGet)
    if (!result || active.current !== session) return
    setState((s) => result.ok
      ? { key: session, data: result.value, pending: null, error: null, refreshed: kind === 'token' && !!result.value.tokenRefreshedAt }
      : { ...s, pending: null, error: errorMessage(result.error), refreshed: false })
  }, [loader])

  useEffect(() => {
    loader.session(key)
    setState({ key, data: null, pending: null, error: null, refreshed: false })
    if (key) void refresh()
    return () => loader.session(null)
  }, [key, loader, refresh])

  useEffect(() => {
    const visible = () => { if (document.visibilityState === 'visible') void refresh() }
    document.addEventListener('visibilitychange', visible)
    return () => document.removeEventListener('visibilitychange', visible)
  }, [refresh])

  return { ...(state.key === key ? state : { key, data: null, pending: null, error: null, refreshed: false }), refresh }
}
