/** Top-level tabs of the single main window (mockup: 主页 / 请求日志 / 设置). */
export type TabId = 'home' | 'logs' | 'settings'

export interface TabDef {
  id: TabId
  label: string
}

export const TABS: readonly TabDef[] = [
  { id: 'home', label: '主页' },
  { id: 'logs', label: '请求日志' },
  { id: 'settings', label: '设置' }
]

/** Arrow-key navigation inside the tablist; wraps at both ends. */
export function nextTab(current: TabId, step: 1 | -1): TabId {
  const idx = TABS.findIndex((t) => t.id === current)
  const next = (idx + step + TABS.length) % TABS.length
  return TABS[next].id
}

/** Status-bar version text, e.g. `v0.0.1`. */
export function versionLabel(raw: string): string {
  const v = raw.trim()
  if (!v) return '版本未知'
  return v.startsWith('v') ? v : `v${v}`
}
