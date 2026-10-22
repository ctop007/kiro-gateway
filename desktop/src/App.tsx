import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { TABS, nextTab, type TabId } from '@/features/shell/tabs'
import { isUpgradeForced } from '@/features/update/view'
import { useAppState } from '@/state/useAppState'
import { HomeView } from '@/components/HomeView'
import { LogsView } from '@/components/LogsView'
import { SettingsView } from '@/components/SettingsView'
import { StatusBar } from '@/components/StatusBar'
import { UpgradeOverlay } from '@/components/UpgradeOverlay'

/** Title bar with tabs, one view per tab, status bar, and the forced-upgrade overlay. */
export function App() {
  const [tab, setTab] = useState<TabId>('home')
  const app = useAppState()
  const forced = isUpgradeForced(app.update, app.config?.upgradeRequired ?? false)

  // While the overlay is up, everything behind it must not be reachable by keyboard or AT.
  const titleRef = useRef<HTMLElement>(null)
  const mainRef = useRef<HTMLElement>(null)
  const footRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    for (const el of [titleRef.current, mainRef.current, footRef.current]) {
      if (!el) continue
      if (forced) el.setAttribute('inert', '')
      else el.removeAttribute('inert')
    }
  }, [forced])

  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return
    e.preventDefault()
    const target = nextTab(tab, e.key === 'ArrowRight' ? 1 : -1)
    setTab(target)
    document.getElementById(`tab-${target}`)?.focus()
  }

  return (
    <div className="window">
      <header className="titlebar" ref={titleRef}>
        <span className="app-name">Kiro Gateway</span>
        <nav className="tabs" role="tablist" aria-label="页面">
          {TABS.map((t) => (
            <button
              key={t.id}
              id={`tab-${t.id}`}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              aria-controls={`view-${t.id}`}
              tabIndex={tab === t.id ? 0 : -1}
              onClick={() => setTab(t.id)}
              onKeyDown={onTabKey}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <main ref={mainRef}>
        <section id="view-home" role="tabpanel" aria-labelledby="tab-home" className={`view${tab === 'home' ? ' active' : ''}`} hidden={tab !== 'home'}>
          <HomeView app={app} goto={setTab} />
        </section>
        <section id="view-logs" role="tabpanel" aria-labelledby="tab-logs" className={`view${tab === 'logs' ? ' active' : ''}`} hidden={tab !== 'logs'}>
          <LogsView app={app} />
        </section>
        <section id="view-settings" role="tabpanel" aria-labelledby="tab-settings" className={`view${tab === 'settings' ? ' active' : ''}`} hidden={tab !== 'settings'}>
          <SettingsView app={app} />
        </section>
      </main>

      <div ref={footRef} style={{ display: 'contents' }}>
        <StatusBar app={app} />
      </div>

      <UpgradeOverlay app={app} />
    </div>
  )
}
