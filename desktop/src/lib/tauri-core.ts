/**
 * The only place that talks to Tauri's `invoke` / `listen`. Every module
 * wrapper (`tauri-config.ts`, `tauri-gateway.ts`, ...) is built on `call` and
 * `onEvent`, so command names and payload types come from `ipc-types.ts` only.
 *
 * With `VITE_MOCK_IPC=1` both go to the in-memory backend in `src/mock/`. The
 * mock is loaded with a dynamic import guarded by `import.meta.env.DEV` (a literal
 * `false` in production builds, so the branch and the mock chunk are dropped); the
 * guard must stay inline at every call site for that to work. A normal build
 * contains none of it (checked by `mock-isolation.test.ts`).
 */
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { toIpcError } from './ipc-error'
import type { CommandMap, CommandName, EventMap, EventName } from './ipc-types'

type ArgsOf<C extends CommandName> = CommandMap[C]['args'] extends undefined ? [] : [CommandMap[C]['args']]

/** Call a contract command. Rejections are normalized to `{ code, message }`. */
export async function call<C extends CommandName>(
  command: C,
  ...args: ArgsOf<C>
): Promise<CommandMap[C]['result']> {
  try {
    if (import.meta.env.DEV && import.meta.env.VITE_MOCK_IPC === '1') {
      const mock = await import('@/mock/backend')
      return (await mock.mockInvoke(command, args[0])) as CommandMap[C]['result']
    }
    return await invoke<CommandMap[C]['result']>(command, args[0] as Record<string, unknown> | undefined)
  } catch (err) {
    throw toIpcError(err)
  }
}

/** Subscribe to a contract event; resolves to the unsubscribe function. */
export async function onEvent<E extends EventName>(
  name: E,
  handler: (payload: EventMap[E]) => void
): Promise<() => void> {
  if (import.meta.env.DEV && import.meta.env.VITE_MOCK_IPC === '1') {
    const mock = await import('@/mock/backend')
    return mock.mockListen(name, handler)
  }
  return listen<EventMap[E]>(name, (e) => handler(e.payload))
}
