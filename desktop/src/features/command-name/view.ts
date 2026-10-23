/**
 * Command name helpers (pure): the name the app installs into the shell
 * (default `cclaude`), its validation rules and every sentence that mentions it.
 *
 * The rules mirror the Rust side (`docs/ipc-contract.md` section 4). The
 * "already taken" check needs PATH and the shell startup files, so only Rust
 * can do it; its message is shown as returned.
 */
import { toIpcError } from '@/lib/ipc-error'

export const DEFAULT_COMMAND_NAME = 'cclaude'
export const COMMAND_NAME_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{1,31}$/

/** Names that must never be taken: Claude Code itself and common system commands. */
export const RESERVED_COMMAND_NAMES: readonly string[] = [
  'claude',
  'kiro-cli',
  'ls',
  'cd',
  'cat',
  'cp',
  'mv',
  'rm',
  'git',
  'node',
  'npm',
  'python',
  'python3',
  'sh',
  'bash',
  'zsh',
  'sudo',
  'open',
  'kill',
  'test',
  'which',
  'type',
  'echo'
]

export const COMMAND_NAME_EMPTY = '请输入命令名'
export const COMMAND_NAME_FORMAT = '命令名只能含字母、数字、- 和 _，以字母开头，长度 2 到 32'

export type CommandNameParse = { ok: true; name: string } | { ok: false; message: string }

export function reservedMessage(name: string): string {
  return `${name} 是系统或 Claude Code 已在用的命令，换一个名字`
}

/** Case-insensitive, so `Claude` and `LS` are refused too. */
export function isReservedCommandName(name: string): boolean {
  return RESERVED_COMMAND_NAMES.includes(name.toLowerCase())
}

/**
 * Validate the command name input. Surrounding whitespace is ignored (a pasted
 * name often has some); whitespace inside the name is a format error.
 */
export function parseCommandName(raw: string): CommandNameParse {
  const name = raw.trim()
  if (name === '') return { ok: false, message: COMMAND_NAME_EMPTY }
  if (!COMMAND_NAME_PATTERN.test(name)) return { ok: false, message: COMMAND_NAME_FORMAT }
  if (isReservedCommandName(name)) return { ok: false, message: reservedMessage(name) }
  return { ok: true, name }
}

/** The command name to show: config is the source of truth, status is the fallback. */
export function resolveCommandName(
  config: { commandName?: string } | null,
  status: { commandName?: string } | null
): string {
  const pick = (v: string | undefined) => (typeof v === 'string' && v.trim() !== '' ? v : null)
  return pick(config?.commandName) ?? pick(status?.commandName) ?? DEFAULT_COMMAND_NAME
}

// ----- sentences that mention the name -----

export function removeButtonLabel(name: string): string {
  return `移除 ${name}`
}

export function stoppedHint(name: string): string {
  return `打开开关后，就能在终端用 ${name}`
}

export function panelTitle(name: string): string {
  return `${name} 命令`
}

export interface LogsEmptyHint {
  before: string
  code: string
  after: string
}

/** Empty request log: "在终端运行 <code>X</code> 试试，…". */
export function logsEmptyHint(name: string): LogsEmptyHint {
  return { before: '还没有请求。在终端运行 ', code: name, after: ' 试试，请求结束后会出现在这里。' }
}

export const COMMAND_NAME_DESCRIPTION = '终端里敲的命令名，改完新开一个终端就能用'

// ----- saving -----

export interface CommandNameOutcome {
  tone: 'ok' | 'bad'
  text: string
}

/**
 * Map the result of `config_update({ patch: { commandName } })` to the line shown
 * under the input. Failure shows Rust's own sentence; a rejection that is not the
 * contract shape falls back to the generic sentence (see `toIpcError`).
 *
 * @param result  `ok` with the name Rust actually saved, or the rejection value;
 *                `null` error means the call never ran (a save was already pending)
 */
export function commandNameOutcome(
  result: { ok: true; name: string } | { ok: false; error: unknown }
): CommandNameOutcome | null {
  if (result.ok) return { tone: 'ok', text: `已改为 ${result.name}，新开一个终端即可使用` }
  if (result.error === null) return null
  return { tone: 'bad', text: toIpcError(result.error).message }
}

/** Whether the Save button can be pressed for the current input. */
export function canSaveCommandName(raw: string, current: string, pending: boolean): boolean {
  if (pending) return false
  const parsed = parseCommandName(raw)
  return parsed.ok && parsed.name !== current
}
