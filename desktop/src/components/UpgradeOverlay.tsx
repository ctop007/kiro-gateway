import { useEffect, useRef } from 'react'
import type { AppState } from '@/state/useAppState'
import { forcedOverlay } from '@/features/update/view'
import { openExternalUrl } from '@/lib/tauri-opener'

/**
 * Forced-upgrade overlay. role=dialog + aria-modal, no close control, Escape
 * is swallowed. The rest of the window is made inert by the caller.
 */
export function UpgradeOverlay({ app }: { app: AppState }) {
  const view = forcedOverlay(app.update, app.config?.upgradeRequired ?? false)
  const title = useRef<HTMLHeadingElement>(null)
  const shown = view !== null

  useEffect(() => {
    if (shown) title.current?.focus()
  }, [shown])

  if (!view) return null
  const pct = view.progress?.percent ?? null

  return (
    <div
      className="overlay"
      role="dialog"
      aria-modal="true"
      aria-labelledby="up-title"
      aria-describedby="up-text"
      onKeyDown={(e) => {
        if (e.key === 'Escape') e.preventDefault()
      }}
    >
      <div className="upgrade">
        <h2 id="up-title" ref={title} tabIndex={-1}>
          {view.title}
        </h2>
        <p id="up-text">{view.text}</p>
        {view.error && <p className="error" role="alert">{view.error}</p>}
        {view.retry && (
          <button type="button" className="btn primary" disabled={app.updateAction.pending} onClick={() => void app.retryUpdate()}>
            重试
          </button>
        )}
        {view.progress && (
          <>
            <div
              className={`progress${pct === null ? ' indeterminate' : ''}`}
              role="progressbar"
              aria-label="下载进度"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct ?? undefined}
              aria-valuetext={pct === null ? view.progress.label : `${pct}%`}
            >
              <i style={pct === null ? undefined : { width: `${pct}%` }} />
            </div>
            <div className="small" aria-live="polite">{view.progress.label}</div>
          </>
        )}
        {view.manual && (
          <div className="small">
            也可以手动下载安装包：
            <a
              href={view.manual.url}
              onClick={(e) => {
                e.preventDefault()
                void openExternalUrl(view.manual!.url)
              }}
            >
              {view.manual.label}
            </a>
          </div>
        )}
      </div>
    </div>
  )
}
