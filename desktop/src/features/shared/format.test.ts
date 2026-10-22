import { describe, expect, it } from 'vitest'
import {
  formatBytesProgress,
  formatClockShort,
  formatCreditsRow,
  formatCreditsSummary,
  formatDuration,
  formatLogTime,
  formatMonthLabel,
  formatUptime
} from './format'

const SH = 'Asia/Shanghai'
const LA = 'America/Los_Angeles'

describe('formatLogTime', () => {
  it('shows only the clock for a record from the same local day', () => {
    expect(formatLogTime('2026-10-22T14:41:08+08:00', new Date('2026-10-22T15:00:00+08:00'), SH)).toBe(
      '14:41:08'
    )
  })

  it('converts to the given local time zone, not the offset in the string', () => {
    // 14:41:08+08:00 is 23:41:08 on the previous day in Los Angeles (PDT, UTC-7).
    expect(formatLogTime('2026-10-22T14:41:08+08:00', new Date('2026-10-22T07:00:00Z'), LA)).toBe(
      '10-21 23:41:08'
    )
  })

  it('treats 23:59 and 00:00 as different local days', () => {
    const now = new Date('2026-11-01T00:00:30+08:00')
    expect(formatLogTime('2026-10-31T23:59:59+08:00', now, SH)).toBe('10-31 23:59:59')
    expect(formatLogTime('2026-11-01T00:00:00+08:00', now, SH)).toBe('00:00:00')
  })

  it('adds the year when the record is from another year', () => {
    expect(formatLogTime('2025-12-31T23:59:59+08:00', new Date('2026-01-01T08:00:00+08:00'), SH)).toBe(
      '2025-12-31 23:59:59'
    )
  })

  it('accepts UTC timestamps', () => {
    expect(formatLogTime('2026-10-22T06:41:08Z', new Date('2026-10-22T15:00:00+08:00'), SH)).toBe('14:41:08')
  })

  it('falls back for unparseable timestamps', () => {
    expect(formatLogTime('not a time', new Date(), SH)).toBe('时间未知')
    expect(formatLogTime('', new Date(), SH)).toBe('时间未知')
  })
})

describe('formatClockShort', () => {
  it('formats hours and minutes in the given zone', () => {
    expect(formatClockShort('2026-10-22T14:32:59+08:00', SH)).toBe('14:32')
    expect(formatClockShort('2026-10-22T14:32:59+08:00', LA)).toBe('23:32')
  })

  it('returns null for invalid input', () => {
    expect(formatClockShort('garbage', SH)).toBeNull()
  })
})

describe('formatDuration', () => {
  it('uses one decimal second below a minute', () => {
    expect(formatDuration(12_400)).toBe('12.4 s')
    expect(formatDuration(900)).toBe('0.9 s')
    expect(formatDuration(41_000)).toBe('41.0 s')
    expect(formatDuration(0)).toBe('0.0 s')
  })

  it('switches to minutes and seconds from 60 s', () => {
    expect(formatDuration(60_000)).toBe('1 分 0 秒')
    expect(formatDuration(65_400)).toBe('1 分 5 秒')
  })

  it('shows a dash for negative or non-finite values', () => {
    expect(formatDuration(-1)).toBe('—')
    expect(formatDuration(Number.NaN)).toBe('—')
    expect(formatDuration(Number.POSITIVE_INFINITY)).toBe('—')
  })
})

describe('credits formatting', () => {
  it('summary uses two decimals and a bare 0', () => {
    expect(formatCreditsSummary(1.84)).toBe('1.84')
    expect(formatCreditsSummary(46.2)).toBe('46.20')
    expect(formatCreditsSummary(0)).toBe('0')
  })

  it('summary does not round a tiny positive amount down to zero', () => {
    expect(formatCreditsSummary(0.004)).toBe('<0.01')
  })

  it('row uses four decimals and a bare 0', () => {
    expect(formatCreditsRow(0.3112)).toBe('0.3112')
    expect(formatCreditsRow(0.012)).toBe('0.0120')
    expect(formatCreditsRow(0)).toBe('0')
  })

  it('treats non-finite or negative values as 0', () => {
    expect(formatCreditsSummary(Number.NaN)).toBe('0')
    expect(formatCreditsRow(-3)).toBe('0')
  })
})

describe('formatUptime', () => {
  const since = '2026-10-22T12:00:00+08:00'

  it('shows hours and minutes', () => {
    expect(formatUptime(since, new Date('2026-10-22T14:14:30+08:00'))).toBe('已运行 2 小时 14 分')
  })

  it('shows minutes only below an hour', () => {
    expect(formatUptime(since, new Date('2026-10-22T12:05:00+08:00'))).toBe('已运行 5 分钟')
  })

  it('shows days and hours beyond a day', () => {
    expect(formatUptime(since, new Date('2026-10-23T14:03:00+08:00'))).toBe('已运行 1 天 2 小时')
  })

  it('says under a minute right after start and on clock skew', () => {
    expect(formatUptime(since, new Date('2026-10-22T12:00:45+08:00'))).toBe('已运行不到 1 分钟')
    expect(formatUptime(since, new Date('2026-10-22T11:59:00+08:00'))).toBe('已运行不到 1 分钟')
  })

  it('returns null when since is missing or invalid', () => {
    expect(formatUptime(null, new Date())).toBeNull()
    expect(formatUptime('nope', new Date())).toBeNull()
  })
})

describe('formatMonthLabel', () => {
  it('drops the leading zero', () => {
    expect(formatMonthLabel('2026-10')).toBe('10 月')
    expect(formatMonthLabel('2026-01')).toBe('1 月')
  })

  it('falls back for malformed input', () => {
    expect(formatMonthLabel('')).toBe('本月')
    expect(formatMonthLabel('2026-13')).toBe('本月')
    expect(formatMonthLabel('october')).toBe('本月')
  })
})

describe('formatBytesProgress', () => {
  const MB = 1024 * 1024

  it('shows downloaded and total in MB', () => {
    expect(formatBytesProgress(17.4 * MB, 28.1 * MB)).toBe('已下载 17.4 / 28.1 MB')
  })

  it('omits the total when unknown', () => {
    expect(formatBytesProgress(3 * MB, null)).toBe('已下载 3.0 MB')
  })

  it('clamps negative byte counts to zero', () => {
    expect(formatBytesProgress(-5, 2 * MB)).toBe('已下载 0.0 / 2.0 MB')
  })
})
