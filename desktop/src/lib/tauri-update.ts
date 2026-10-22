import { call, onEvent } from './tauri-core'
import type { UpdateStatus } from './ipc-types'

export const updateStatus = (): Promise<UpdateStatus> => call('update_status')
export const updateCheckNow = (): Promise<UpdateStatus> => call('update_check_now')
export const updateRestartNow = (): Promise<null> => call('update_restart_now')
export const updateRetry = (): Promise<UpdateStatus> => call('update_retry')
export const onUpdateStatus = (handler: (s: UpdateStatus) => void): Promise<() => void> =>
  onEvent('update-status', handler)
