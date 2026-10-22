import { call } from './tauri-core'
import type { KclaudeStatus } from './ipc-types'

export const kclaudeStatus = (): Promise<KclaudeStatus> => call('kclaude_status')
export const kclaudeInstall = (): Promise<KclaudeStatus> => call('kclaude_install')
export const kclaudeAdopt = (): Promise<KclaudeStatus> => call('kclaude_adopt')
export const kclaudeRemove = (): Promise<KclaudeStatus> => call('kclaude_remove')
