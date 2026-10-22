import { call } from './tauri-core'
import type { EnvReport } from './ipc-types'

export const envCheck = (): Promise<EnvReport> => call('env_check')
