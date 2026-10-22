/** Environment report → checklist rows and switch availability (pure). */
import type { CheckId, CheckItem, EnvReport, FixAction } from '@/lib/ipc-types'

export type CheckTone = 'ok' | 'bad' | 'skip'

export interface CheckRow {
  id: CheckId
  tone: CheckTone
  title: string
  detail: string
  fix: FixAction | null
}

export interface SwitchGate {
  allowed: boolean
  /** One sentence shown next to a disabled switch; null when allowed. */
  reason: string | null
}

const ORDER: readonly CheckId[] = ['kiro_cli_installed', 'kiro_cli_logged_in', 'login_valid', 'claude_installed']

/** The three checks that gate the switch. Claude Code is deliberately absent. */
const GATING: readonly CheckId[] = ['kiro_cli_installed', 'kiro_cli_logged_in', 'login_valid']

const GATE_REASONS: Partial<Record<CheckId, string>> = {
  kiro_cli_installed: '先安装 kiro-cli 才能打开',
  kiro_cli_logged_in: '先登录 kiro-cli 才能打开',
  login_valid: '先重新登录 kiro-cli 才能打开'
}

/**
 * Whether the gateway switch may be turned on. Any of the three kiro-cli
 * checks being `fail`, `skipped` or absent blocks it (the Rust side re-checks
 * with the same rule in `gateway_start`).
 */
export function switchGate(report: EnvReport | null): SwitchGate {
  if (!report) return { allowed: false, reason: '正在检测运行环境' }
  for (const id of GATING) {
    const found = report.items.find((i) => i.id === id)
    if (!found || found.status !== 'ok') return { allowed: false, reason: GATE_REASONS[id] ?? '先完成运行环境检测' }
  }
  return { allowed: true, reason: null }
}

/**
 * True when the command's executable is an absolute path (POSIX, drive letter
 * or UNC), optionally quoted with double quotes (Windows) or single quotes
 * (POSIX paths with spaces or apostrophes). Copy buttons only ever offer such commands so
 * they work when kiro-cli is not on the terminal's PATH.
 */
export function isAbsoluteCommand(command: string): boolean {
  const c = command.trim().replace(/^['"]/, '')
  return c.startsWith('/') || /^[A-Za-z]:[\\/]/.test(c) || c.startsWith('\\\\')
}

function isHttpsUrl(url: string): boolean {
  try {
    return new URL(url).protocol === 'https:'
  } catch {
    return false
  }
}

/** Keep only fixes the UI is willing to act on. */
function safeFix(fix: FixAction | null): FixAction | null {
  if (!fix) return null
  if (fix.kind === 'copy_command') return isAbsoluteCommand(fix.command) ? fix : null
  return isHttpsUrl(fix.url) ? fix : null
}

function toRow(i: CheckItem): CheckRow {
  const tone: CheckTone = i.status === 'ok' ? 'ok' : i.status === 'fail' ? 'bad' : 'skip'
  return { id: i.id, tone, title: i.title, detail: i.detail, fix: i.status === 'fail' ? safeFix(i.fix) : null }
}

/** Checklist rows in the contract's fixed order. */
export function checkRows(report: EnvReport | null): CheckRow[] {
  if (!report) return []
  return ORDER.map((id) => report.items.find((i) => i.id === id))
    .filter((i): i is CheckItem => i !== undefined)
    .map(toRow)
}

/**
 * The `<absolute kiro-cli path> login` command, taken from the report, or from
 * a failing item's copy action. Null when none is known or it is not absolute.
 */
export function loginCommandFor(report: EnvReport | null): string | null {
  if (!report) return null
  if (report.loginCommand && isAbsoluteCommand(report.loginCommand)) return report.loginCommand
  for (const i of report.items) {
    if (i.status === 'fail' && i.fix?.kind === 'copy_command' && isAbsoluteCommand(i.fix.command)) {
      return i.fix.command
    }
  }
  return null
}
