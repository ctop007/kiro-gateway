/**
 * Display formatting shared by several panels. All time functions take an
 * optional IANA `timeZone`; omitted means the machine's local zone (what the
 * app uses at runtime). Tests pass a fixed zone.
 */

interface LocalParts {
  year: string
  month: string
  day: string
  hour: string
  minute: string
  second: string
}

function parseTime(iso: string | null | undefined): Date | null {
  if (!iso) return null
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? null : d
}

function localParts(d: Date, timeZone?: string): LocalParts {
  const fmt = new Intl.DateTimeFormat('en-GB', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hourCycle: 'h23'
  })
  const out: Record<string, string> = {}
  for (const p of fmt.formatToParts(d)) out[p.type] = p.value
  return {
    year: out.year,
    month: out.month,
    day: out.day,
    hour: out.hour,
    minute: out.minute,
    second: out.second
  }
}

/**
 * Request-log time column. Same local day → `HH:MM:SS`; same year →
 * `MM-DD HH:MM:SS`; otherwise the full date.
 */
export function formatLogTime(iso: string, now: Date, timeZone?: string): string {
  const d = parseTime(iso)
  if (!d) return '时间未知'
  const p = localParts(d, timeZone)
  const n = localParts(now, timeZone)
  const clock = `${p.hour}:${p.minute}:${p.second}`
  if (p.year === n.year && p.month === n.month && p.day === n.day) return clock
  if (p.year === n.year) return `${p.month}-${p.day} ${clock}`
  return `${p.year}-${p.month}-${p.day} ${clock}`
}

/** `HH:MM` in the local zone, used in「网关进程在 14:32 退出了」. */
export function formatClockShort(iso: string, timeZone?: string): string | null {
  const d = parseTime(iso)
  if (!d) return null
  const p = localParts(d, timeZone)
  return `${p.hour}:${p.minute}`
}

/** Request duration: `12.4 s` below a minute, `1 分 5 秒` above. */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—'
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const total = Math.floor(ms / 1000)
  return `${Math.floor(total / 60)} 分 ${total % 60} 秒`
}

function cleanCredits(v: number): number {
  return Number.isFinite(v) && v > 0 ? v : 0
}

/** Summary cards: two decimals; never shows a non-zero amount as 0. */
export function formatCreditsSummary(v: number): string {
  const c = cleanCredits(v)
  if (c === 0) return '0'
  if (c < 0.005) return '<0.01'
  return c.toFixed(2)
}

/** Table rows: four decimals, matching the precision Kiro reports. */
export function formatCreditsRow(v: number): string {
  const c = cleanCredits(v)
  return c === 0 ? '0' : c.toFixed(4)
}

/** 「已运行 2 小时 14 分」 from GatewayStatus.since. */
export function formatUptime(since: string | null, now: Date): string | null {
  const d = parseTime(since)
  if (!d) return null
  const minutes = Math.floor((now.getTime() - d.getTime()) / 60_000)
  if (minutes < 1) return '已运行不到 1 分钟'
  if (minutes < 60) return `已运行 ${minutes} 分钟`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `已运行 ${hours} 小时 ${minutes % 60} 分`
  return `已运行 ${Math.floor(hours / 24)} 天 ${hours % 24} 小时`
}

/** `2026-10` → `10 月`. */
export function formatMonthLabel(month: string): string {
  const m = /^\d{4}-(\d{2})$/.exec(month)
  if (!m) return '本月'
  const n = Number(m[1])
  return n >= 1 && n <= 12 ? `${n} 月` : '本月'
}

const MB = 1024 * 1024

/** 「已下载 17.4 / 28.1 MB」. */
export function formatBytesProgress(downloaded: number, total: number | null): string {
  const done = (Math.max(0, downloaded) / MB).toFixed(1)
  if (total === null || !Number.isFinite(total) || total <= 0) return `已下载 ${done} MB`
  return `已下载 ${done} / ${(total / MB).toFixed(1)} MB`
}
