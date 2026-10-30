import { describe, expect, it } from 'vitest'
import { accountView, maskEmail } from './view'
import type { AccountSnapshot } from '@/lib/ipc-types'

describe('account presentation', () => {
  it('masks the account without exposing the full email', () => {
    expect(maskEmail('sample@example.com')).not.toContain('sample@example.com')
    expect(maskEmail(null)).toBe('未获取账号')
    expect(maskEmail('opaque-identifier')).toBe('已登录账号')
  })
  it('does not confuse unknown quota with an empty account', () => {
    expect(accountView(null).remaining).toBe('—')
    expect(accountView({ remaining: 0, used: 0, total: 0 } as AccountSnapshot).remaining).toBe('0')
  })
  it('clamps progress while preserving the actual usage percentage', () => {
    const view = accountView({ used: 2400, total: 2000, remaining: 0, usagePercent: 120 } as AccountSnapshot)
    expect(view.progress).toBe(100)
    expect(view.percentage).toBe('120%')
  })
})
