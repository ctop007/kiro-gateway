import { call, onEvent } from './tauri-core'
import type { RequestLogAppended, RequestLogSnapshot } from './ipc-types'

export const requestLogSnapshot = (limit?: number): Promise<RequestLogSnapshot> =>
  call('request_log_snapshot', { limit })
export const requestLogClear = (): Promise<RequestLogSnapshot> => call('request_log_clear')
export const onRequestLogAppended = (handler: (p: RequestLogAppended) => void): Promise<() => void> =>
  onEvent('request-log-appended', handler)
