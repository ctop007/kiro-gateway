import { describe, expect, it } from 'vitest'
import { gw } from '@/test-support/fixtures'
import { DEFAULT_PORT, AUTOSTART_DESCRIPTION, autostartLabel, parsePort, portHint, restartNotice } from './view'

describe('parsePort', () => {
  it.each([
    ['6868', 6868],
    [' 8100 ', 8100],
    ['1024', 1024],
    ['65535', 65535]
  ])('accepts %s', (raw, port) => {
    expect(parsePort(raw)).toEqual({ ok: true, port })
  })

  it.each([
    ['', '请输入端口'],
    ['   ', '请输入端口'],
    ['abc', '端口只能是数字'],
    ['80.5', '端口只能是数字'],
    ['-1', '端口只能是数字'],
    ['1e3', '端口只能是数字'],
    ['0x1f90', '端口只能是数字'],
    ['1023', '端口要在 1024 到 65535 之间'],
    ['0', '端口要在 1024 到 65535 之间'],
    ['65536', '端口要在 1024 到 65535 之间'],
    ['99999999999999999999', '端口要在 1024 到 65535 之间']
  ])('rejects %j', (raw, message) => {
    expect(parsePort(raw)).toEqual({ ok: false, message })
  })
})

describe('DEFAULT_PORT', () => {
  it('is 6868 and passes its own validation', () => {
    expect(DEFAULT_PORT).toBe(6868)
    expect(parsePort(String(DEFAULT_PORT))).toEqual({ ok: true, port: 6868 })
  })
})

describe('portHint', () => {
  it('says restart is needed only when the gateway is running or starting', () => {
    expect(portHint(gw({ state: 'running' }))).toBe('改完后重启网关生效，cclaude 会自动跟上')
    expect(portHint(gw({ state: 'stopped' }))).toBe('改完后下次打开开关时生效，cclaude 会自动跟上')
    expect(portHint(null)).toBe('改完后下次打开开关时生效，cclaude 会自动跟上')
  })

  it('follows the current command name', () => {
    expect(portHint(gw({ state: 'running' }), 'kclaude-test')).toBe('改完后重启网关生效，kclaude-test 会自动跟上')
    expect(portHint(gw({ state: 'starting' }), 'kclaude-test')).toBe('改完后重启网关生效，kclaude-test 会自动跟上')
    expect(portHint(null, 'kclaude-test')).toBe('改完后下次打开开关时生效，kclaude-test 会自动跟上')
  })
})

describe('restartNotice', () => {
  it('appears only when the backend asked for a restart', () => {
    expect(restartNotice(true)).toBe('端口已保存。关掉再打开开关，网关才会用新端口。')
    expect(restartNotice(false)).toBeNull()
  })
})

describe('autostartLabel', () => {
  it('describes both states', () => {
    expect(autostartLabel(true)).toBe('已开启')
    expect(autostartLabel(false)).toBe('未开启')
  })
})

describe('AUTOSTART_DESCRIPTION', () => {
  it('says the gateway only reopens when it was on at the last exit', () => {
    expect(AUTOSTART_DESCRIPTION).toBe('登录后在托盘里启动；如果上次退出时网关是开着的，会自动打开')
    expect(AUTOSTART_DESCRIPTION).not.toContain('并自动打开网关')
  })
})
