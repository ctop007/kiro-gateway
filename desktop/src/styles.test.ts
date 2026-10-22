import { readFileSync } from 'node:fs'
import path from 'node:path'
import { describe, expect, it } from 'vitest'

const css = readFileSync(path.resolve(__dirname, 'styles.css'), 'utf-8')

/** Variables declared in the first rule whose selector starts with `selector`. */
function vars(selector: string): Record<string, string> {
  const start = css.indexOf(selector)
  if (start < 0) throw new Error(`selector not found: ${selector}`)
  const open = css.indexOf('{', start)
  const close = css.indexOf('}', open)
  const out: Record<string, string> = {}
  for (const m of css.slice(open + 1, close).matchAll(/--([a-z-]+)\s*:\s*(#[0-9a-fA-F]{6})\s*;/g)) out[m[1]] = m[2].toLowerCase()
  return out
}

function channel(v: number): number {
  const c = v / 255
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16)
  return 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255)
}

/** WCAG 2.x contrast ratio. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
  return (hi + 0.05) / (lo + 0.05)
}

const THEMES: Record<string, Record<string, string>> = {
  light: vars(':root {'),
  dark: vars(':root[data-theme="dark"]'),
  'dark (system)': vars(':root:not([data-theme="light"])')
}

describe('contrast helper', () => {
  it('matches the WCAG reference values', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 1)
    expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 1)
  })
})

describe.each(Object.entries(THEMES))('text colors, %s theme', (_name, t) => {
  it('defines every color used below', () => {
    for (const k of ['paper', 'window', 'panel', 'faint', 'muted', 'warn', 'warn-soft', 'bad', 'on']) expect(t[k]).toMatch(/^#/)
  })

  // 12px text (statusbar, table headers, small hints) sits on the three surfaces.
  it.each(['window', 'panel', 'paper'])('--faint on --%s is at least 4.5:1', (bg) => {
    expect(contrast(t.faint, t[bg])).toBeGreaterThanOrEqual(4.5)
  })

  // .notice.warn: --warn text on --warn-soft; .tag.ref and .kc-state.warn use it on surfaces.
  it.each(['warn-soft', 'window', 'panel', 'paper'])('--warn on --%s is at least 4.5:1', (bg) => {
    expect(contrast(t.warn, t[bg])).toBeGreaterThanOrEqual(4.5)
  })

  it.each(['window', 'panel', 'paper'])('--muted on --%s stays at least 4.5:1', (bg) => {
    expect(contrast(t.muted, t[bg])).toBeGreaterThanOrEqual(4.5)
  })
})

describe('theme blocks stay in step', () => {
  it('the system-dark block and the forced-dark block define the same colors', () => {
    expect(THEMES['dark (system)']).toEqual(THEMES.dark)
  })
})
