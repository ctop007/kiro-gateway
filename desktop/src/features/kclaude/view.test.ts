import { describe, expect, it } from 'vitest'
import { kc } from '@/test-support/fixtures'
import { displayModel, kclaudeView, modelPicker, rcDescription } from './view'

describe('kclaudeView: states', () => {
  it('installed and gateway running: ready sentence, remove allowed, no install button', () => {
    const v = kclaudeView(kc(), true)
    expect(v).toMatchObject({
      tone: '',
      text: '已就绪。新开一个终端就能用，原来的 claude 不受影响。',
      hint: '用法和 claude 一样',
      canRemove: true,
      verifyLine: null
    })
    expect(v.actions).toEqual([])
  })

  it('installed and gateway stopped: tells what happens when run', () => {
    expect(kclaudeView(kc(), false).text).toBe('已安装。网关关着的时候运行会提示你先打开开关。')
  })

  it('installed but Claude Code missing: warns that Claude Code is needed first', () => {
    const v = kclaudeView(kc({ claudeInstalled: false }), true)
    expect(v).toMatchObject({ tone: 'warn', text: '已安装，但要先装好 Claude Code 才能用。' })
  })

  it('foreign: names the rc file, did not touch it, offers adopt only', () => {
    const v = kclaudeView(kc({ state: 'foreign', rcFiles: ['~/.zshrc'] }), true)
    expect(v).toMatchObject({
      tone: 'warn',
      text: '~/.zshrc 里已经有一个你自己加的 kclaude，App 没有改动它。',
      hint: '当前用的是你自己的版本',
      canRemove: false
    })
    expect(v.actions).toEqual([{ id: 'adopt', label: '改由 App 管理', primary: false }])
  })

  it('foreign wins over Claude Code missing', () => {
    const v = kclaudeView(kc({ state: 'foreign', claudeInstalled: false }), true)
    expect(v.text).toContain('你自己加的 kclaude')
  })

  it('foreign with two PowerShell profiles lists both', () => {
    const v = kclaudeView(kc({ state: 'foreign', shell: 'powershell', rcFiles: ['~/a.ps1', '~/b.ps1'] }), true)
    expect(v.text).toContain('~/a.ps1、~/b.ps1')
  })

  it('foreign with no rc file reported still reads sensibly', () => {
    expect(kclaudeView(kc({ state: 'foreign', rcFiles: [] }), true).text).toContain('启动文件里已经有一个')
  })

  it('not installed: offers install, nothing to remove', () => {
    const v = kclaudeView(kc({ state: 'not_installed', verify: { status: 'skipped', detail: '', fix: null } }), true)
    expect(v.text).toBe('还没有安装 kclaude。')
    expect(v.actions).toEqual([{ id: 'install', label: '安装', primary: true }])
    expect(v.canRemove).toBe(false)
  })

  it('outdated: says it is being refreshed and can be reinstalled', () => {
    const v = kclaudeView(kc({ state: 'outdated' }), true)
    expect(v.text).toBe('kclaude 的内容不是最新的，重新安装一次就好。')
    expect(v.actions.map((a) => a.id)).toEqual(['install'])
    expect(v.canRemove).toBe(true)
  })

  it('before the first status: neutral line, no actions', () => {
    const v = kclaudeView(null, false)
    expect(v.text).toBe('正在检查 kclaude')
    expect(v.actions).toEqual([])
    expect(v.canRemove).toBe(false)
  })
})

describe('kclaudeView: verify result', () => {
  it('shows nothing for ok or skipped', () => {
    expect(kclaudeView(kc({ verify: { status: 'ok', detail: '', fix: null } }), true).verifyLine).toBeNull()
    expect(kclaudeView(kc({ verify: { status: 'skipped', detail: 'x', fix: null } }), true).verifyLine).toBeNull()
  })

  it('shows the failure detail and a copyable fix', () => {
    const fix = { kind: 'copy_command' as const, label: '复制', command: 'C:\\x\\fix.cmd' }
    const v = kclaudeView(kc({ verify: { status: 'fail', detail: '新开的终端里找不到 kclaude', fix } }), true)
    expect(v.verifyLine).toEqual({ text: '新开的终端里找不到 kclaude', fix })
  })

  it('substitutes a sentence when the failure detail is empty', () => {
    const v = kclaudeView(kc({ verify: { status: 'fail', detail: '  ', fix: null } }), true)
    expect(v.verifyLine!.text).toBe('新开的终端里没能找到 kclaude')
  })
})

describe('displayModel', () => {
  it.each([
    ['claude-opus-5-5[1m]', 'claude-opus-5.5 [1m]'],
    ['claude-sonnet-5-5[1m]', 'claude-sonnet-5.5 [1m]'],
    ['claude-haiku-4-5', 'claude-haiku-4.5'],
    ['claude-haiku-4.5', 'claude-haiku-4.5'],
    ['claude-haiku-4-5-20251001', 'claude-haiku-4.5-20251001'],
    ['claude-sonnet-4.6', 'claude-sonnet-4.6'],
    ['auto', 'auto'],
    ['', '']
  ])('%s -> %s', (raw, shown) => {
    expect(displayModel(raw)).toBe(shown)
  })
})

describe('modelPicker', () => {
  it('gateway stopped: disabled, shows only the saved value and the hint', () => {
    expect(modelPicker('claude-opus-5-5[1m]', null, false)).toEqual({
      disabled: true,
      hint: '启动网关后可选择模型',
      options: [{ value: 'claude-opus-5-5[1m]', label: 'claude-opus-5.5 [1m]' }]
    })
  })

  it('gateway running with a list: options come from the list, saved value selected', () => {
    const p = modelPicker('claude-haiku-4-5', ['claude-haiku-4-5', 'claude-sonnet-4-6'], true)
    expect(p.disabled).toBe(false)
    expect(p.hint).toBeNull()
    expect(p.options.map((o) => o.value)).toEqual(['claude-haiku-4-5', 'claude-sonnet-4-6'])
  })

  it('keeps a saved value that the gateway no longer lists, so it is not silently changed', () => {
    const p = modelPicker('claude-old-1', ['claude-haiku-4-5'], true)
    expect(p.options.map((o) => o.value)).toEqual(['claude-old-1', 'claude-haiku-4-5'])
  })

  it('does not duplicate or include blank ids', () => {
    const p = modelPicker('a', ['a', 'a', '', '  ', 'b'], true)
    expect(p.options.map((o) => o.value)).toEqual(['a', 'b'])
  })

  it('gateway running but list not loaded yet: disabled with a loading hint', () => {
    const p = modelPicker('a', null, true)
    expect(p.disabled).toBe(true)
    expect(p.hint).toBe('正在读取模型列表')
    expect(p.options.map((o) => o.value)).toEqual(['a'])
  })

  it('gateway running but the list is empty: disabled', () => {
    expect(modelPicker('a', [], true).disabled).toBe(true)
  })
})

describe('rcDescription', () => {
  it('names the rc files the marked block lives in', () => {
    expect(rcDescription(kc({ rcFiles: ['~/.zshrc'] }))).toBe('写在 ~/.zshrc 里带标记的一段')
    expect(rcDescription(kc({ shell: 'powershell', rcFiles: ['~/a.ps1', '~/b.ps1'] }))).toBe(
      '写在 ~/a.ps1、~/b.ps1 里带标记的一段'
    )
  })

  it('falls back when nothing is known', () => {
    expect(rcDescription(null)).toBe('写在启动文件里带标记的一段')
    expect(rcDescription(kc({ rcFiles: [] }))).toBe('写在启动文件里带标记的一段')
  })
})
