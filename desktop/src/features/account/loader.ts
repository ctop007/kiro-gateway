/** Own one request per gateway process; late replies cannot cross account sessions. */
export function createAccountLoader<T>() {
  let current: string | null = null
  let generation = 0
  let busy = false
  return {
    session(key: string | null) {
      if (key !== current) {
        current = key
        generation++
        busy = false
      }
    },
    async run(call: () => Promise<T>): Promise<{ ok: true; value: T } | { ok: false; error: unknown } | null> {
      if (!current || busy) return null
      busy = true
      const ticket = generation
      try {
        const value = await call()
        return ticket === generation ? { ok: true, value } : null
      } catch (error) {
        return ticket === generation ? { ok: false, error } : null
      } finally {
        if (ticket === generation) busy = false
      }
    }
  }
}
