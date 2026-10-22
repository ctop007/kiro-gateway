import { describe, expect, it } from 'vitest'
import { TABS, nextTab, versionLabel } from './tabs'

describe('shell tabs', () => {
  it('lists the three tabs in mockup order with Chinese labels', () => {
    expect(TABS.map((t) => [t.id, t.label])).toEqual([
      ['home', '主页'],
      ['logs', '请求日志'],
      ['settings', '设置']
    ])
  })

  it('moves focus right and wraps around at the end', () => {
    expect(nextTab('home', 1)).toBe('logs')
    expect(nextTab('settings', 1)).toBe('home')
  })

  it('moves focus left and wraps around at the start', () => {
    expect(nextTab('logs', -1)).toBe('home')
    expect(nextTab('home', -1)).toBe('settings')
  })
})

describe('versionLabel', () => {
  it('prefixes a plain semver with v', () => {
    expect(versionLabel('0.0.1')).toBe('v0.0.1')
  })

  it('does not double the prefix', () => {
    expect(versionLabel('v0.0.1')).toBe('v0.0.1')
  })

  it('trims whitespace and falls back for empty input', () => {
    expect(versionLabel('  0.2.0 ')).toBe('v0.2.0')
    expect(versionLabel('')).toBe('版本未知')
  })
})
