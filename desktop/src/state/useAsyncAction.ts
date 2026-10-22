import { useCallback, useRef, useState } from 'react'
import { errorMessage } from '@/lib/ipc-error'

/**
 * Runs an async IPC action, exposing `pending` and the last error sentence.
 * Re-entrant calls while pending are ignored, so a double click cannot start
 * the same command twice.
 */
export function useAsyncAction() {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const busy = useRef(false)

  const run = useCallback(async <T,>(fn: () => Promise<T>): Promise<{ ok: true; value: T } | { ok: false; error: unknown }> => {
    if (busy.current) return { ok: false, error: null }
    busy.current = true
    setPending(true)
    setError(null)
    try {
      const value = await fn()
      return { ok: true, value }
    } catch (err) {
      setError(errorMessage(err))
      return { ok: false, error: err }
    } finally {
      busy.current = false
      setPending(false)
    }
  }, [])

  const clear = useCallback(() => setError(null), [])
  return { pending, error, run, clear }
}
