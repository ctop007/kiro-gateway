import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { App } from './App'

describe('App shell (server render, before any IPC answer)', () => {
  const html = renderToStaticMarkup(<App />)

  it('renders the three tabs and the switch as a labelled, disabled role=switch', () => {
    for (const label of ['主页', '请求日志', '设置']) expect(html).toContain(label)
    expect(html).toMatch(/role="switch"[^>]*aria-label="KiroGateway 开关"/)
    expect(html).toMatch(/class="rocker"[^>]*disabled/)
  })

  it('shows no upgrade overlay and no failure notice before data arrives', () => {
    expect(html).not.toContain('aria-modal')
    expect(html).not.toContain('class="notice')
  })

  it('has the status bar with the version', () => {
    expect(html).toContain('v0.0.0-test')
  })
})
