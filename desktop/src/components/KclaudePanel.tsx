import type { AppState } from '@/state/useAppState'
import { MODEL_TIERS } from '@/lib/ipc-types'
import { displayModel, kclaudeView } from '@/features/kclaude/view'
import { panelTitle } from '@/features/command-name/view'

export function KclaudePanel({ app }: { app: AppState }) {
  const view = kclaudeView(app.kclaude, app.gatewayRunning, app.commandName)
  const models = app.config?.models

  const run = (id: 'install' | 'adopt') => void (id === 'install' ? app.installKclaude() : app.adoptKclaude())

  return (
    <div className="panel">
      <h2>{panelTitle(app.commandName)}</h2>
      <p className={`kc-state${view.tone ? ` ${view.tone}` : ''}`}>{view.text}</p>
      {view.actions.length > 0 && (
        <div className="kc-actions" style={{ marginBottom: 10 }}>
          {view.actions.map((a) => (
            <button
              key={a.id}
              type="button"
              className={`btn${a.primary ? ' primary' : ''}`}
              disabled={app.kclaudeAction.pending}
              onClick={() => run(a.id)}
            >
              {a.label}
            </button>
          ))}
        </div>
      )}
      <div className="cmd">
        <code>{app.commandName}</code>
        <span className="hint">{view.hint}</span>
      </div>
      {models && (
        <dl className="mapping">
          {MODEL_TIERS.map((t) => (
            <div key={t} style={{ display: 'contents' }}>
              <dt>{t}</dt>
              <dd>{displayModel(models[t])}</dd>
            </div>
          ))}
        </dl>
      )}
      {view.verifyLine && <p className="kc-verify">{view.verifyLine.text}</p>}
      {app.kclaudeAction.error && (
        <p className="action-error" role="alert" style={{ marginTop: 8 }}>
          {app.kclaudeAction.error}
        </p>
      )}
    </div>
  )
}
