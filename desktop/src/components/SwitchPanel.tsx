import type { AppState } from '@/state/useAppState'
import { gatewayView, noticeText, type GatewayActionId } from '@/features/gateway/view'
import { loginCommandFor, switchGate } from '@/features/env/view'

interface Props {
  app: AppState
  onChangePort: () => void
}

/** Home: the gateway switch, state text and failure notice. */
export function SwitchPanel({ app, onChangePort }: Props) {
  const view = gatewayView({
    status: app.gateway,
    gate: switchGate(app.env),
    upgradeRequired: app.config?.upgradeRequired ?? false,
    loginCommand: loginCommandFor(app.env),
    now: app.now,
    commandName: app.commandName
  })
  const busy = app.toggle.pending

  const onSwitch = () => {
    if (busy) return
    void (view.toggle === 'start' ? app.startGateway() : app.stopGateway())
  }

  const onAction = (id: GatewayActionId) => {
    switch (id) {
      case 'retry':
      case 'restart':
        void app.startGateway()
        return
      case 'open_log':
        app.openLog()
        return
      case 'change_port':
        onChangePort()
        return
      case 'copy_login':
        if (view.notice?.command) void app.copyText(view.notice.command, 'notice-login')
    }
  }

  return (
    <div className="panel switch-panel">
      <div className="switch-row">
        <button
          type="button"
          className="rocker"
          role="switch"
          aria-checked={view.switchOn}
          aria-label="KiroGateway 开关"
          aria-describedby="state-sub"
          data-busy={view.busy}
          disabled={view.disabled || busy}
          onClick={onSwitch}
        />
        <div>
          <p className="state-title" id="state-title">
            {view.title}
          </p>
          <p className="state-sub" id="state-sub">
            {view.sub.code && <code>{view.sub.code}</code>}
            {view.sub.text}
          </p>
        </div>
      </div>

      <div aria-live="polite">
        {app.toggle.error && <p className="action-error" role="alert" style={{ marginTop: 12 }}>{app.toggle.error}</p>}
        {view.notice && (
          <div className={`notice ${view.notice.tone}`}>
            <p>
              {view.notice.segments.map((s, i) => (s.strong ? <strong key={i}>{s.text}</strong> : <span key={i}>{s.text}</span>))}
            </p>
            {view.notice.command && <code className="cmdline">{view.notice.command}</code>}
            {view.notice.logLines.length > 0 && (
              <pre aria-label="最后几行日志">{view.notice.logLines.join('\n')}</pre>
            )}
            {view.notice.actions.length > 0 && (
              <div className="actions">
                {view.notice.actions.map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    className={`btn${a.primary ? ' primary' : ''}`}
                    disabled={a.disabled || busy}
                    onClick={() => onAction(a.id)}
                  >
                    {a.label}
                  </button>
                ))}
                {app.copied === 'notice-login' && <span className="copied">已复制</span>}
              </div>
            )}
            <span className="visually-hidden">{noticeText(view.notice)}</span>
          </div>
        )}
      </div>
    </div>
  )
}
