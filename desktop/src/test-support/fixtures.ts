/** Builders for contract-shaped test data. Test-only: never imported by app code. */
import type {
  CheckItem,
  EnvReport,
  GatewayFailure,
  GatewayStatus,
  KclaudeStatus,
  RequestRecord,
  UpdateStatus
} from '@/lib/ipc-types'

export const KIRO_CLI = '/Users/x/.local/bin/kiro-cli'
export const LOGIN_CMD = `${KIRO_CLI} login`

export function item(over: Partial<CheckItem> & Pick<CheckItem, 'id'>): CheckItem {
  return { status: 'ok', title: '标题', detail: '说明', fix: null, ...over }
}

export function okEnv(over: Partial<EnvReport> = {}): EnvReport {
  return {
    checkedAt: '2026-10-22T14:00:00+08:00',
    items: [
      item({ id: 'kiro_cli_installed', title: 'kiro-cli 已安装', detail: '~/.local/bin/kiro-cli' }),
      item({ id: 'kiro_cli_logged_in', title: 'kiro-cli 已登录', detail: 'AWS SSO 登录' }),
      item({ id: 'login_valid', title: '登录有效', detail: '到期后会自动续期' }),
      item({ id: 'claude_installed', title: 'Claude Code 已安装', detail: '~/.local/bin/claude' })
    ],
    kiroCliPath: '~/.local/bin/kiro-cli',
    claudePath: '~/.local/bin/claude',
    loginExpiresAt: null,
    loginCommand: LOGIN_CMD,
    ...over
  }
}

/** Replace one item of an env report by id. */
export function withItem(report: EnvReport, next: CheckItem): EnvReport {
  return { ...report, items: report.items.map((i) => (i.id === next.id ? next : i)) }
}

export function failure(over: Partial<GatewayFailure> = {}): GatewayFailure {
  return {
    kind: 'other',
    message: '网关进程立即退出了',
    occupant: null,
    logExcerpt: [],
    at: '2026-10-22T14:32:10+08:00',
    ...over
  }
}

export function gw(over: Partial<GatewayStatus> = {}): GatewayStatus {
  return { state: 'stopped', port: 6868, pid: null, since: null, failure: null, ...over }
}

export function kc(over: Partial<KclaudeStatus> = {}): KclaudeStatus {
  return {
    commandName: 'cclaude',
    state: 'installed',
    shell: 'zsh',
    rcFiles: ['~/.zshrc'],
    claudeInstalled: true,
    verify: { status: 'ok', detail: '', fix: null },
    ...over
  }
}

export function rec(over: Partial<RequestRecord> = {}): RequestRecord {
  return {
    ts: '2026-10-22T14:41:08+08:00',
    id: 'req-1',
    api: 'anthropic',
    model: 'claude-opus-5.5',
    stream: true,
    status: 200,
    outcome: 'ok',
    durationMs: 12_400,
    credits: 0.3112,
    error: null,
    ...over
  }
}

export function up(over: Record<string, unknown> = {}): UpdateStatus {
  return { state: 'none', currentVersion: '0.0.1', lastCheckedAt: null, ...over } as UpdateStatus
}
