/**
 * Orders results of requests against pushed events (pure).
 *
 * Some state arrives both as a command result and as an event (gateway status). The
 * event is always newer than any request that was issued before it, even if that
 * request's answer shows up later. `begin()` stamps a request when it is issued; its
 * finisher applies the value only when nothing newer (an event, or a request issued
 * later that already answered) has been applied since.
 */
export interface RaceGuard<T> {
  /** Apply a pushed value. Always wins over requests issued earlier. */
  onEvent(value: T): void
  /** Mark a request as issued now; call the returned function with its result. */
  begin(): (value: T) => void
}

export function createRaceGuard<T>(apply: (value: T) => void): RaceGuard<T> {
  let clock = 0
  let applied = 0
  return {
    onEvent(value) {
      applied = ++clock
      apply(value)
    },
    begin() {
      const stamp = ++clock
      return (value) => {
        if (stamp < applied) return
        applied = stamp
        apply(value)
      }
    }
  }
}
