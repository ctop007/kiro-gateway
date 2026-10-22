/** Settings page helpers (pure). */
import type { GatewayStatus } from '@/lib/ipc-types'

export const PORT_MIN = 1024
export const PORT_MAX = 65535

export type PortParse = { ok: true; port: number } | { ok: false; message: string }

/** Parse the port input. Digits only, within the range the backend accepts. */
export function parsePort(raw: string): PortParse {
  const s = raw.trim()
  if (s === '') return { ok: false, message: '请输入端口' }
  if (!/^\d+$/.test(s)) return { ok: false, message: '端口只能是数字' }
  const n = Number(s)
  if (!Number.isSafeInteger(n) || n < PORT_MIN || n > PORT_MAX) {
    return { ok: false, message: `端口要在 ${PORT_MIN} 到 ${PORT_MAX} 之间` }
  }
  return { ok: true, port: n }
}

export function portHint(status: GatewayStatus | null): string {
  const live = status?.state === 'running' || status?.state === 'starting'
  return live ? '改完后重启网关生效，kclaude 会自动跟上' : '改完后下次打开开关时生效，kclaude 会自动跟上'
}

/** Shown after `config_update` when the gateway is running and the port changed. */
export function restartNotice(restartRequired: boolean): string | null {
  return restartRequired ? '端口已保存。关掉再打开开关，网关才会用新端口。' : null
}

export function autostartLabel(on: boolean): string {
  return on ? '已开启' : '未开启'
}

/** What the autostart row says. The gateway only reopens if it was on when the app last quit. */
export const AUTOSTART_DESCRIPTION = '登录后在托盘里启动；如果上次退出时网关是开着的，会自动打开'
