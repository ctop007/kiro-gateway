import { useState, type KeyboardEvent } from 'react'
import { TABS, nextTab, versionLabel, type TabId } from '@/features/shell/tabs'

/**
 * Minimal shell: title bar with tabs, an empty view per tab, and the status bar.
 * Feature panels (switch, checks, kclaude, logs, settings) land in later batches.
 */
export function App() {
  const [tab, setTab] = useState<TabId>('home')

  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return
    e.preventDefault()
    const target = nextTab(tab, e.key === 'ArrowRight' ? 1 : -1)
    setTab(target)
    document.getElementById(`tab-${target}`)?.focus()
  }

  return (
    <div className="window">
      <header className="titlebar">
        <span className="app-name">Kiro Gateway</span>
        <nav className="tabs" role="tablist" aria-label="页面">
          {TABS.map((t) => (
            <button
              key={t.id}
              id={`tab-${t.id}`}
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

      <main>
        {TABS.map((t) => (
          <section
            key={t.id}
            id={`view-${t.id}`}
            role="tabpanel"
            aria-labelledby={`tab-${t.id}`}
            className={`view${tab === t.id ? ' active' : ''}`}
            hidden={tab !== t.id}
          />
        ))}
      </main>

      <footer className="statusbar">
        <span>
          <span className="pulse" />
          已停止
        </span>
        <span className="grow" />
        <span>{versionLabel(__APP_VERSION__)}</span>
      </footer>
    </div>
  )
}
