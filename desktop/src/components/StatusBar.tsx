import type { AppState } from '@/state/useAppState'
import { gatewayView } from '@/features/gateway/view'
import { switchGate, loginCommandFor } from '@/features/env/view'
import { isUpgradeForced, statusBarUpdate } from '@/features/update/view'

export function StatusBar({ app }: { app: AppState }) {
  const g = gatewayView({
    status: app.gateway,
    gate: switchGate(app.env),
    upgradeRequired: app.config?.upgradeRequired ?? false,
    loginCommand: loginCommandFor(app.env),
    now: app.now
  })
  const u = statusBarUpdate(app.update, __APP_VERSION__)
  const forced = isUpgradeForced(app.update, app.config?.upgradeRequired ?? false)
  return (
    <footer className="statusbar">
      <span>
        <span className={`pulse${g.statusBar.pulse ? ` ${g.statusBar.pulse}` : ''}`} aria-hidden="true" />
        {g.statusBar.text}
      </span>
      <span className="grow" />
      <span>
        {u.text}
        {u.restart && !forced && (
          <>
            {' '}
            <button type="button" onClick={() => void app.restartNow()}>
              立即重启
            </button>
          </>
        )}
      </span>
    </footer>
  )
}
