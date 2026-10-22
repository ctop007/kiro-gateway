import { call } from './tauri-core'
import type { AppConfig, ConfigPatch, ConfigUpdateResult } from './ipc-types'

export const configGet = (): Promise<AppConfig> => call('config_get')
export const configUpdate = (patch: ConfigPatch): Promise<ConfigUpdateResult> => call('config_update', { patch })
