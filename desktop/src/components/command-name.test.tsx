import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import type { AppState } from '@/state/useAppState'
import { gw, kc, okEnv } from '@/test-support/fixtures'
import { KclaudePanel } from './KclaudePanel'
import { LogsView } from './LogsView'
import { SettingsView } from './SettingsView'
import { StatusBar } from './StatusBar'
import { SwitchPanel } from './SwitchPanel'

const idle = { pending: false, error: null, run: async () => ({ ok: false, error: null }), clear: () => {} }

/** Only the fields these components read; everything else is left undefined on purpose. */
function fakeApp(name: string, over: Record<string, unknown> = {}): AppState {
  return {
    now: new Date('2026-10-23T10:00:00+08:00'),
    commandName: name,
    config: {
      port: 6868,
      commandName: name,
      models: { opus: 'claude-opus-5-5[1m]', sonnet: 'claude-sonnet-5-5[1m]', haiku: 'claude-haiku-4-5' },
      autostart: false,
      gatewayWanted: false,
      upgradeRequired: false
    },
    gateway: gw({ state: 'stopped' }),
    env: okEnv(),
    kclaude: kc({ commandName: name, state: 'installed' }),
    update: null,
    log: { records: [], byModel: [], today: { credits: 0, requests: 0 }, month: { credits: 0, requests: 0, month: '' }, generatedAt: '' },
    models: null,
    gatewayRunning: false,
    restartNote: null,
    copied: null,
    toggle: idle,
    kclaudeAction: idle,
    configAction: idle,
    commandNameAction: idle,
    commandNameResult: null,
    clearCommandNameResult: () => {},
    updateAction: idle,
    logAction: idle,
    ...over
  } as unknown as AppState
}

describe.each(['cclaude', 'kclaude-test'])('surfaces follow the command name %s', (name) => {
  const other = name === 'cclaude' ? 'kclaude-test' : 'cclaude'

  it('home command card: title, chip and state text', () => {
    const html = renderToStaticMarkup(<KclaudePanel app={fakeApp(name)} />)
    expect(html).toContain(`${name} 命令`)
    expect(html).toContain(`<code>${name}</code>`)
    expect(html).not.toContain(other)
  })

  it('home command card when not installed offers install and names the command', () => {
    const app = fakeApp(name, { kclaude: kc({ commandName: name, state: 'not_installed' }) })
    expect(renderToStaticMarkup(<KclaudePanel app={app} />)).toContain(`还没有安装 ${name}。`)
  })

  it('stopped switch panel hint', () => {
    const html = renderToStaticMarkup(<SwitchPanel app={fakeApp(name)} onChangePort={() => {}} />)
    expect(html).toContain(`打开开关后，就能在终端用 ${name}`)
    expect(html).not.toContain(other)
  })

  it('empty request log hint', () => {
    const html = renderToStaticMarkup(<LogsView app={fakeApp(name)} />)
    expect(html).toContain(`在终端运行 <code>${name}</code> 试试`)
  })

  it('settings: name input shows the current name, remove button and port hint use it', () => {
    const html = renderToStaticMarkup(<SettingsView app={fakeApp(name)} />)
    expect(html).toContain('命令名')
    expect(html).toContain(`移除 ${name}`)
    expect(html).toContain(`${name} 会自动跟上`)
    expect(html).not.toContain(other)
  })

  it('status bar never prints the command name by accident', () => {
    expect(renderToStaticMarkup(<StatusBar app={fakeApp(name)} />)).not.toContain(other)
  })
})

describe('settings page: command name saving', () => {
  it('shows Rust\'s failure sentence in an alert-style line', () => {
    const app = fakeApp('cclaude', { commandNameResult: { tone: 'bad', text: '这个名字已经被占用' } })
    const html = renderToStaticMarkup(<SettingsView app={app} />)
    expect(html).toContain('<div class="err">这个名字已经被占用</div>')
  })

  it('shows the success line', () => {
    const app = fakeApp('kclaude-test', {
      commandNameResult: { tone: 'ok', text: '已改为 kclaude-test，新开一个终端即可使用' }
    })
    const html = renderToStaticMarkup(<SettingsView app={app} />)
    expect(html).toContain('<div class="ok-note">已改为 kclaude-test，新开一个终端即可使用</div>')
  })

  it('the save button is disabled while the name is unchanged', () => {
    const html = renderToStaticMarkup(<SettingsView app={fakeApp('cclaude')} />)
    expect(html).toMatch(/<button type="submit" class="btn" disabled="">保存<\/button>/)
  })

  it('port field defaults to 6868', () => {
    expect(renderToStaticMarkup(<SettingsView app={fakeApp('cclaude')} />)).toContain('value="6868"')
  })
})
