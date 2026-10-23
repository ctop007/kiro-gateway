/**
 * TypeScript mirror of docs/ipc-contract.md. One section per contract module.
 * This file must not import @tauri-apps/*; it is shared by wrappers, pure
 * view functions and the in-memory mock backend.
 */

// ---------- Common ----------

export type IpcErrorCode =
  | 'invalid_argument'
  | 'upgrade_required'
  | 'env_not_ready'
  | 'gateway_not_running'
  | 'gateway_unreachable'
  | 'foreign_present'
  | 'backup_failed'
  | 'write_failed'
  | 'no_pending_update'
  | 'internal'

export const IPC_ERROR_CODES: readonly IpcErrorCode[] = [
  'invalid_argument',
  'upgrade_required',
  'env_not_ready',
  'gateway_not_running',
  'gateway_unreachable',
  'foreign_present',
  'backup_failed',
  'write_failed',
  'no_pending_update',
  'internal'
]

/** Rejection value of every command: `{ code, message }`. */
export interface IpcError {
  code: IpcErrorCode
  message: string
}

// ---------- 1. config ----------

export type ModelTier = 'opus' | 'sonnet' | 'haiku'

export const MODEL_TIERS: readonly ModelTier[] = ['opus', 'sonnet', 'haiku']

export interface AppConfig {
  port: number
  commandName: string
  models: Record<ModelTier, string>
  autostart: boolean
  gatewayWanted: boolean
  upgradeRequired: boolean
}

export interface ConfigPatch {
  port?: number
  commandName?: string
  models?: Partial<Record<ModelTier, string>>
  autostart?: boolean
}

export interface ConfigUpdateResult {
  config: AppConfig
  restartRequired: boolean
}

// ---------- 2. gateway ----------

export type GatewayState = 'stopped' | 'starting' | 'running' | 'start_failed' | 'crashed'

export type FailureKind = 'port_in_use' | 'login_expired' | 'timeout' | 'other'

export interface PortOccupant {
  name: string | null
  pid: number | null
}

export interface GatewayFailure {
  kind: FailureKind
  message: string
  occupant: PortOccupant | null
  logExcerpt: string[]
  at: string
}

export interface GatewayStatus {
  state: GatewayState
  port: number
  pid: number | null
  since: string | null
  failure: GatewayFailure | null
}

export interface GatewayModel {
  id: string
}

// ---------- 3. env ----------

export type CheckId = 'kiro_cli_installed' | 'kiro_cli_logged_in' | 'login_valid' | 'claude_installed'
export type CheckStatus = 'ok' | 'fail' | 'skipped'

export type FixAction =
  | { kind: 'open_url'; label: string; url: string }
  | { kind: 'copy_command'; label: string; command: string }

export interface CheckItem {
  id: CheckId
  status: CheckStatus
  title: string
  detail: string
  fix: FixAction | null
}

export interface EnvReport {
  checkedAt: string
  items: CheckItem[]
  kiroCliPath: string | null
  claudePath: string | null
  loginExpiresAt: string | null
  /** `<absolute kiro-cli path> login`; non-null whenever kiro-cli was found. */
  loginCommand: string | null
}

// ---------- 4. kclaude ----------

export type KclaudeState = 'installed' | 'not_installed' | 'foreign' | 'outdated'
export type ShellKind = 'zsh' | 'bash' | 'powershell'

export interface KclaudeVerify {
  status: 'ok' | 'fail' | 'skipped'
  detail: string
  fix: FixAction | null
}

export interface KclaudeStatus {
  commandName: string
  state: KclaudeState
  shell: ShellKind
  rcFiles: string[]
  claudeInstalled: boolean
  verify: KclaudeVerify
}

// ---------- 5. request_log ----------

export type ApiKind = 'openai' | 'anthropic'
export type Outcome = 'ok' | 'error' | 'refused' | 'client_disconnected'

/** Group and filter label for records whose `model` is null or empty (contract section 5). */
export const UNKNOWN_MODEL_LABEL = '未知模型'

export interface RequestRecord {
  ts: string
  id: string | null
  api: ApiKind
  model: string | null
  stream: boolean
  status: number
  outcome: Outcome
  durationMs: number
  credits: number
  error: string | null
}

export interface UsageTotal {
  credits: number
  requests: number
}

export interface ModelUsage extends UsageTotal {
  model: string
}

export interface UsageSummary {
  today: UsageTotal
  month: UsageTotal & { month: string }
  byModel: ModelUsage[]
}

export interface RequestLogSnapshot extends UsageSummary {
  records: RequestRecord[]
  generatedAt: string
}

export interface RequestLogAppended extends UsageSummary {
  records: RequestRecord[]
  /**
   * True when a month file was deleted, truncated or replaced behind the app's back.
   * `records` is then empty and the list the UI holds may contain records that no longer
   * exist: reload with `request_log_snapshot` instead of merging.
   */
  reset: boolean
}

// ---------- 6. update ----------

export interface DownloadProgress {
  downloadedBytes: number
  totalBytes: number | null
}

export type UpdateState =
  | { state: 'none' }
  | { state: 'downloading'; version: string; progress: DownloadProgress }
  | { state: 'ready'; version: string }
  | { state: 'installing'; version: string }
  | {
      state: 'forced'
      version: string
      progress: DownloadProgress | null
      error: string | null
      manualUrl: string
    }
  | { state: 'install_failed'; version: string; error: string; manualUrl: string }

export type UpdateStatus = UpdateState & {
  currentVersion: string
  lastCheckedAt: string | null
}

// ---------- Command / event registry ----------

/** Every command name in the contract, with its argument and result types. */
export interface CommandMap {
  config_get: { args: undefined; result: AppConfig }
  config_update: { args: { patch: ConfigPatch }; result: ConfigUpdateResult }
  gateway_status: { args: undefined; result: GatewayStatus }
  gateway_start: { args: undefined; result: GatewayStatus }
  gateway_stop: { args: undefined; result: GatewayStatus }
  gateway_models: { args: undefined; result: GatewayModel[] }
  gateway_reveal_log: { args: undefined; result: null }
  env_check: { args: undefined; result: EnvReport }
  kclaude_status: { args: undefined; result: KclaudeStatus }
  kclaude_install: { args: undefined; result: KclaudeStatus }
  kclaude_adopt: { args: undefined; result: KclaudeStatus }
  kclaude_remove: { args: undefined; result: KclaudeStatus }
  request_log_snapshot: { args: { limit?: number }; result: RequestLogSnapshot }
  request_log_clear: { args: undefined; result: RequestLogSnapshot }
  update_status: { args: undefined; result: UpdateStatus }
  update_check_now: { args: undefined; result: UpdateStatus }
  update_restart_now: { args: undefined; result: null }
  update_retry: { args: undefined; result: UpdateStatus }
}

export type CommandName = keyof CommandMap

/** Every event name in the contract, with its payload type. */
export interface EventMap {
  'gateway-status': GatewayStatus
  'request-log-appended': RequestLogAppended
  'update-status': UpdateStatus
}

export type EventName = keyof EventMap
