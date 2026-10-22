import { describe, expect, it } from 'vitest'
import { up } from '@/test-support/fixtures'
import { forcedOverlay, isUpgradeForced, retryCommandFor, progressPercent, settingsVersion, statusBarUpdate } from './view'

const MB = 1024 * 1024
const manualUrl = 'https://cos.example.com/dl/Kiro-Gateway_0.0.3.dmg'

describe('progressPercent', () => {
  it('floors a fraction', () => {
    expect(progressPercent({ downloadedBytes: 17.4 * MB, totalBytes: 28.1 * MB })).toBe(61)
  })
  it('is null for an unknown or zero total', () => {
    expect(progressPercent({ downloadedBytes: 5, totalBytes: null })).toBeNull()
    expect(progressPercent({ downloadedBytes: 5, totalBytes: 0 })).toBeNull()
  })
  it('clamps to 0..100', () => {
    expect(progressPercent({ downloadedBytes: 99, totalBytes: 10 })).toBe(100)
    expect(progressPercent({ downloadedBytes: -5, totalBytes: 10 })).toBe(0)
  })
})

describe('isUpgradeForced', () => {
  it('true when the update state is forced', () => {
    expect(isUpgradeForced(up({ state: 'forced', version: '0.0.3', progress: null, error: null, manualUrl }), false)).toBe(true)
  })
  it('true when config says upgrade required even before the first update status', () => {
    expect(isUpgradeForced(null, true)).toBe(true)
  })
  it('false for every other state', () => {
    for (const u of [
      up(),
      up({ state: 'ready', version: '0.0.2' }),
      up({ state: 'downloading', version: '0.0.2', progress: { downloadedBytes: 1, totalBytes: 2 } }),
      up({ state: 'install_failed', version: '0.0.2', error: 'x', manualUrl })
    ]) {
      expect(isUpgradeForced(u, false)).toBe(false)
    }
    expect(isUpgradeForced(null, false)).toBe(false)
  })
})

describe('statusBarUpdate', () => {
  it('up to date: just the version', () => {
    expect(statusBarUpdate(up())).toEqual({ text: 'v0.0.1', restart: false })
  })
  it('ready: version, downloaded version and a restart button', () => {
    expect(statusBarUpdate(up({ state: 'ready', version: '0.0.2' }))).toEqual({
      text: 'v0.0.1，0.0.2 已下载',
      restart: true
    })
  })
  it('downloading is silent in the status bar', () => {
    expect(
      statusBarUpdate(up({ state: 'downloading', version: '0.0.2', progress: { downloadedBytes: 1, totalBytes: 2 } }))
    ).toEqual({ text: 'v0.0.1', restart: false })
  })
  it('install failed and forced do not offer a restart', () => {
    expect(statusBarUpdate(up({ state: 'install_failed', version: '0.0.2', error: 'x', manualUrl })).restart).toBe(false)
    expect(
      statusBarUpdate(up({ state: 'forced', version: '0.0.3', progress: null, error: null, manualUrl })).restart
    ).toBe(false)
  })
  it('before the first status: placeholder from the build version', () => {
    expect(statusBarUpdate(null, '0.0.1')).toEqual({ text: 'v0.0.1', restart: false })
  })
  it('does not double a v prefix', () => {
    expect(statusBarUpdate(up({ currentVersion: 'v0.0.1' })).text).toBe('v0.0.1')
  })
})

describe('settingsVersion', () => {
  it('up to date', () => {
    expect(settingsVersion(up(), false)).toMatchObject({
      text: '0.0.1，已是最新',
      checkLabel: '检查更新',
      checkDisabled: false,
      restart: false,
      manualUrl: null,
      tone: ''
    })
  })
  it('checking disables the button', () => {
    expect(settingsVersion(up(), true)).toMatchObject({ checkLabel: '正在检查', checkDisabled: true })
  })
  it('downloading shows a percentage when known', () => {
    const v = settingsVersion(
      up({ state: 'downloading', version: '0.0.2', progress: { downloadedBytes: 5 * MB, totalBytes: 10 * MB } }),
      false
    )
    expect(v.text).toBe('0.0.1，正在后台下载 0.0.2（50%）')
  })
  it('downloading without a known total omits the percentage', () => {
    const v = settingsVersion(
      up({ state: 'downloading', version: '0.0.2', progress: { downloadedBytes: 5, totalBytes: null } }),
      false
    )
    expect(v.text).toBe('0.0.1，正在后台下载 0.0.2')
  })
  it('ready offers restart and says when it takes effect', () => {
    expect(settingsVersion(up({ state: 'ready', version: '0.0.2' }), false)).toMatchObject({
      text: '0.0.1，0.0.2 已下载，下次启动生效',
      restart: true
    })
  })
  it('install failed shows the failure and a manual download', () => {
    const v = settingsVersion(up({ state: 'install_failed', version: '0.0.2', error: '更新安装失败', manualUrl }), false)
    expect(v).toMatchObject({
      text: '0.0.1，更新安装失败',
      tone: 'bad',
      manualUrl,
      manualLabel: '手动下载安装包'
    })
  })
  it('shows the error from Rust as it is, without adding a second prefix', () => {
    const v = settingsVersion(up({ state: 'install_failed', version: '0.0.2', error: '磁盘空间不足', manualUrl }), false)
    expect(v.text).toBe('0.0.1，磁盘空间不足')
    expect(v.text).not.toContain('更新安装失败：')
  })
  it('forced shows a short pointer to the overlay', () => {
    const v = settingsVersion(up({ state: 'forced', version: '0.0.3', progress: null, error: null, manualUrl }), false)
    expect(v.text).toBe('0.0.1，需要升级到 0.0.3')
    expect(v.checkDisabled).toBe(true)
  })
  it('refuses a non-https manual link', () => {
    const v = settingsVersion(
      up({ state: 'install_failed', version: '0.0.2', error: 'x', manualUrl: 'javascript:alert(1)' }),
      false
    )
    expect(v.manualUrl).toBeNull()
  })
  it('before the first status', () => {
    expect(settingsVersion(null, false, '0.0.1').text).toBe('0.0.1')
  })
})

describe('forcedOverlay', () => {
  const forced = (over: Record<string, unknown> = {}) =>
    up({ state: 'forced', version: '0.0.3', progress: null, error: null, manualUrl, ...over })

  it('hidden unless forced', () => {
    expect(forcedOverlay(up(), false)).toBeNull()
    expect(forcedOverlay(null, false)).toBeNull()
  })

  it('config flag alone (first check failed or not done) says it is checking and offers a retry', () => {
    for (const status of [null, up(), up({ state: 'ready', version: '0.0.2' })]) {
      const o = forcedOverlay(status, true)!
      expect(o.title).toBe('需要升级')
      expect(o.progress).toEqual({ percent: null, label: '正在检查新版本' })
      expect(o.error).toBeNull()
      expect(o.retry).toBe(true)
      expect(o.manual).toBeNull()
    }
  })

  it('the forced state keeps its own behaviour: retry only after a failure', () => {
    expect(forcedOverlay(forced(), true)!.retry).toBe(false)
    expect(forcedOverlay(forced({ error: '下载失败' }), true)!.retry).toBe(true)
  })

  it('downloading with known size', () => {
    const o = forcedOverlay(forced({ progress: { downloadedBytes: 17.4 * MB, totalBytes: 28.1 * MB } }), true)!
    expect(o.title).toBe('正在升级到 0.0.3')
    expect(o.text).toBe('当前版本已停止使用，升级完成后才能继续用。完成后会自动重新打开。')
    expect(o.progress).toEqual({ percent: 61, label: '已下载 17.4 / 28.1 MB' })
    expect(o.error).toBeNull()
    expect(o.retry).toBe(false)
  })

  it('downloading with unknown size is indeterminate', () => {
    const o = forcedOverlay(forced({ progress: { downloadedBytes: 3 * MB, totalBytes: null } }), true)!
    expect(o.progress).toEqual({ percent: null, label: '已下载 3.0 MB' })
  })

  it('not started yet shows an indeterminate wait', () => {
    expect(forcedOverlay(forced(), true)!.progress).toEqual({ percent: null, label: '正在准备下载' })
  })

  it('failure shows the reason, retry and the manual installer link', () => {
    const o = forcedOverlay(forced({ error: '网络连接超时' }), true)!
    expect(o.error).toBe('网络连接超时')
    expect(o.retry).toBe(true)
    expect(o.progress).toBeNull()
    expect(o.manual).toEqual({ url: manualUrl, label: 'Kiro-Gateway_0.0.3.dmg' })
  })

  it.each([
    ['下载失败', '下载失败'],
    ['更新安装失败', '更新安装失败']
  ])('shows the Rust error %s once, never as "下载失败：%s"', (error, expected) => {
    const o = forcedOverlay(forced({ error }), true)!
    expect(o.error).toBe(expected)
    expect(o.error).not.toMatch(/：/)
  })

  it('always offers the manual link, even while downloading', () => {
    expect(forcedOverlay(forced(), true)!.manual).not.toBeNull()
  })

  it('manual label falls back when the URL has no file name; non-https is dropped', () => {
    expect(forcedOverlay(forced({ manualUrl: 'https://x.example.com/' }), true)!.manual!.label).toBe('安装包')
    expect(forcedOverlay(forced({ manualUrl: 'http://x.example.com/a.dmg' }), true)!.manual).toBeNull()
  })

  it('cannot be closed: the view model has no dismiss', () => {
    const o = forcedOverlay(forced(), true)!
    expect(Object.keys(o)).not.toContain('dismiss')
    expect(Object.keys(o)).not.toContain('closable')
  })
})

describe('retryCommandFor', () => {
  const forcedState = up({ state: 'forced', version: '0.0.3', progress: null, error: '下载失败', manualUrl })

  it('uses update_retry only where Rust accepts it (forced, install_failed)', () => {
    expect(retryCommandFor(forcedState)).toBe('retry')
    expect(retryCommandFor(up({ state: 'install_failed', version: '0.0.2', error: 'x', manualUrl }))).toBe('retry')
  })

  it('falls back to a fresh check when there is no forced state yet (update_retry would be rejected)', () => {
    expect(retryCommandFor(null)).toBe('check')
    expect(retryCommandFor(up())).toBe('check')
    expect(retryCommandFor(up({ state: 'ready', version: '0.0.2' }))).toBe('check')
    expect(retryCommandFor(up({ state: 'downloading', version: '0.0.2', progress: { downloadedBytes: 1, totalBytes: 2 } }))).toBe('check')
  })
})

describe('installing state (pending update being applied at start-up)', () => {
  const installing = () => up({ state: 'installing', version: '0.0.2' })

  it('settings says the update is being finished and disables the check', () => {
    const s = settingsVersion(installing(), false)
    expect(s.text).toBe('0.0.1，正在完成更新到 0.0.2')
    expect(s.checkDisabled).toBe(true)
    expect(s.restart).toBe(false)
  })
  it('status bar says the update is being finished', () => {
    expect(statusBarUpdate(installing())).toEqual({ text: 'v0.0.1，正在完成更新', restart: false })
  })
  it('a check during installing is not offered as the retry action', () => {
    expect(retryCommandFor(installing())).toBe('check')
  })
})
