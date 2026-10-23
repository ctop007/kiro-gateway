/** Gateway status → home switch panel and status bar content (pure). */
import type { GatewayStatus, IpcError } from '@/lib/ipc-types'
import { formatClockShort, formatUptime } from '@/features/shared/format'
import { isAbsoluteCommand, type SwitchGate } from '@/features/env/view'
import { DEFAULT_COMMAND_NAME, stoppedHint } from '@/features/command-name/view'

export type Pulse = '' | 'on' | 'bad'
export type NoticeTone = 'bad' | 'warn'
export type GatewayActionId = 'retry' | 'restart' | 'open_log' | 'change_port' | 'copy_login'

export interface TextSegment {
  text: string
  strong?: boolean
}

export interface NoticeAction {
  id: GatewayActionId
  label: string
  primary: boolean
  disabled: boolean
}

export interface GatewayNotice {
  tone: NoticeTone
  segments: TextSegment[]
  logLines: string[]
  /** Login command shown/copied by the copy_login action; null if unknown. */
  command: string | null
  actions: NoticeAction[]
}

export interface GatewayView {
  title: string
  /** `code` is rendered in monospace in front of `text`. */
  sub: { code: string | null; text: string }
  switchOn: boolean
  busy: boolean
  disabled: boolean
  /** What clicking the switch does. */
  toggle: 'start' | 'stop'
  pulse: Pulse
  notice: GatewayNotice | null
  statusBar: { text: string; pulse: Pulse }
}

export interface GatewayViewInput {
  status: GatewayStatus | null
  gate: SwitchGate
  upgradeRequired: boolean
  loginCommand: string | null
  now: Date
  timeZone?: string
  /** Current command name for the stopped-state hint; defaults to `cclaude`. */
  commandName?: string
}

const UPGRADE_REASON = '需要先升级到新版本才能使用'

function action(id: GatewayActionId, label: string, primary: boolean, disabled: boolean): NoticeAction {
  return { id, label, primary, disabled }
}

function joinText(segments: TextSegment[]): string {
  return segments.map((s) => s.text).join('')
}

function portInUseSegments(occupant: { name: string | null; pid: number | null } | null): TextSegment[] {
  if (occupant?.name) {
    const pid = occupant.pid !== null ? `（pid ${occupant.pid}）` : ''
    return [
      { text: '占用端口的是 ' },
      { text: occupant.name, strong: true },
      { text: `${pid}，可能是你手动起的网关。可以先关掉它，或者换一个端口。` }
    ]
  }
  if (occupant && occupant.pid !== null) {
    return [{ text: `占用端口的程序（pid ${occupant.pid}）没能识别，可以先关掉它，或者换一个端口。` }]
  }
  return [{ text: '有别的程序占用了这个端口。可以先关掉它，或者换一个端口。' }]
}

function buildNotice(status: GatewayStatus, blocked: boolean, loginCommand: string | null): GatewayNotice | null {
  const f = status.failure
  if (!f || (status.state !== 'start_failed' && status.state !== 'crashed')) return null
  const retryId: GatewayActionId = status.state === 'crashed' ? 'restart' : 'retry'
  const retryLabel = status.state === 'crashed' ? '重新启动' : '重试'

  if (f.kind === 'login_expired') {
    const command = loginCommand && isAbsoluteCommand(loginCommand) ? loginCommand : null
    return {
      tone: 'bad',
      segments: [{ text: '刷新登录失败。在终端运行下面的命令重新登录，然后再打开开关。' }],
      logLines: [],
      command,
      actions: command ? [action('copy_login', '复制登录命令', true, false)] : []
    }
  }

  if (f.kind === 'port_in_use') {
    return {
      tone: 'warn',
      segments: portInUseSegments(f.occupant),
      logLines: [],
      command: null,
      actions: [action('change_port', '换个端口', true, false), action('retry', '重试', false, blocked)]
    }
  }

  let segments: TextSegment[]
  if (status.state === 'crashed') {
    segments = [{ text: f.logExcerpt.length > 0 ? '网关进程意外退出。最后几行日志：' : '网关进程意外退出。' }]
  } else if (f.kind === 'timeout') {
    segments = [{ text: '等了 30 秒网关还没有就绪，已经结束了这次启动。最后几行日志：' }]
    if (f.logExcerpt.length === 0) segments = [{ text: '等了 30 秒网关还没有就绪，已经结束了这次启动。' }]
  } else {
    segments = [{ text: f.logExcerpt.length > 0 ? '网关没能启动。最后几行日志：' : '网关没能启动。' }]
  }
  return {
    tone: 'bad',
    segments,
    logLines: f.logExcerpt,
    command: null,
    actions: [action(retryId, retryLabel, true, blocked), action('open_log', '打开完整日志', false, false)]
  }
}

/**
 * Map the gateway status plus environment/upgrade gates to everything the
 * switch panel and status bar render. The switch is never disabled while the
 * gateway is running or starting, so a blocked environment cannot trap it on.
 */
export function gatewayView(input: GatewayViewInput): GatewayView {
  const { status, gate, upgradeRequired, loginCommand, now, timeZone, commandName = DEFAULT_COMMAND_NAME } = input
  if (!status) {
    return {
      title: '正在读取状态',
      sub: { code: null, text: '' },
      switchOn: false,
      busy: false,
      disabled: true,
      toggle: 'start',
      pulse: '',
      notice: null,
      statusBar: { text: '正在读取状态', pulse: '' }
    }
  }

  const blocked = upgradeRequired || !gate.allowed
  const blockReason = upgradeRequired ? UPGRADE_REASON : gate.reason ?? ''

  switch (status.state) {
    case 'running': {
      const address = `127.0.0.1:${status.port}`
      const uptime = formatUptime(status.since, now)
      return {
        title: '运行中',
        sub: { code: address, text: uptime ? `，${uptime}` : '' },
        switchOn: true,
        busy: false,
        disabled: false,
        toggle: 'stop',
        pulse: 'on',
        notice: null,
        statusBar: { text: address, pulse: 'on' }
      }
    }
    case 'starting':
      return {
        title: '启动中',
        sub: { code: null, text: '正在等待网关就绪' },
        switchOn: true,
        busy: true,
        disabled: false,
        toggle: 'stop',
        pulse: '',
        notice: null,
        statusBar: { text: '启动中', pulse: '' }
      }
    case 'start_failed':
    case 'crashed': {
      const crashed = status.state === 'crashed'
      const title = crashed ? '意外退出' : '没能启动'
      const f = status.failure
      let subText: string
      if (crashed && f?.kind !== 'login_expired') {
        const clock = f ? formatClockShort(f.at, timeZone) : null
        subText = clock ? `网关进程在 ${clock} 退出了` : '网关进程退出了'
      } else {
        subText = f?.message ?? title
      }
      return {
        title,
        sub: { code: null, text: subText },
        switchOn: false,
        busy: false,
        disabled: blocked,
        toggle: 'start',
        pulse: 'bad',
        notice: buildNotice(status, blocked, loginCommand),
        statusBar: { text: title, pulse: 'bad' }
      }
    }
    case 'stopped':
    default:
      return {
        title: '已停止',
        sub: { code: null, text: blocked ? blockReason : stoppedHint(commandName) },
        switchOn: false,
        busy: false,
        disabled: blocked,
        toggle: 'start',
        pulse: '',
        notice: null,
        statusBar: { text: '已停止', pulse: '' }
      }
  }
}

/** Plain-text form of a notice, used for the live region and tests. */
export function noticeText(n: GatewayNotice): string {
  return joinText(n.segments)
}

/** How a rejected `gateway_start` / `gateway_stop` is shown. */
export function startErrorView(err: IpcError): { message: string; recheckEnv: boolean } {
  if (err.code === 'upgrade_required') return { message: UPGRADE_REASON, recheckEnv: false }
  return { message: err.message, recheckEnv: err.code === 'env_not_ready' }
}
