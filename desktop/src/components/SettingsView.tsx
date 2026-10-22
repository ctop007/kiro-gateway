import { useEffect, useState, type FormEvent } from 'react'
import type { AppState } from '@/state/useAppState'
import { MODEL_TIERS, type ModelTier } from '@/lib/ipc-types'
import { modelPicker, rcDescription } from '@/features/kclaude/view'
import { AUTOSTART_DESCRIPTION, autostartLabel, parsePort, portHint, restartNotice } from '@/features/settings/view'
import { settingsVersion } from '@/features/update/view'
import { openExternalUrl } from '@/lib/tauri-opener'

export function SettingsView({ app }: { app: AppState }) {
  const config = app.config
  const [portText, setPortText] = useState('')
  const [portError, setPortError] = useState<string | null>(null)

  useEffect(() => {
    if (config) setPortText(String(config.port))
  }, [config?.port]) // eslint-disable-line react-hooks/exhaustive-deps

  const submitPort = (e: FormEvent) => {
    e.preventDefault()
    const parsed = parsePort(portText)
    if (!parsed.ok) {
      setPortError(parsed.message)
      return
    }
    setPortError(null)
    if (config && parsed.port !== config.port) void app.saveConfig({ port: parsed.port })
  }

  const ver = settingsVersion(app.update, app.updateAction.pending, __APP_VERSION__)
  const note = restartNotice(app.restartNote === 'restart')
  const kcRemovable = app.kclaude?.state === 'installed' || app.kclaude?.state === 'outdated'

  return (
    <div className="settings">
      <div className="setting">
        <div>
          <label className="name" htmlFor="port-input">
            端口
          </label>
          <div className="desc">{portHint(app.gateway)}</div>
        </div>
        <form onSubmit={submitPort}>
          <div className="inline">
            <input
              id="port-input"
              type="number"
              inputMode="numeric"
              value={portText}
              aria-invalid={portError !== null}
              aria-describedby="port-msg"
              onChange={(e) => {
                setPortText(e.target.value)
                setPortError(null)
              }}
              onBlur={submitPort}
              disabled={!config}
            />
            <span className="desc">只监听本机 127.0.0.1</span>
          </div>
          <div id="port-msg" aria-live="polite">
            {portError && <div className="err">{portError}</div>}
            {note && <div className="ok-note">{note}</div>}
          </div>
        </form>
      </div>

      <div className="setting">
        <div>
          <div className="name">模型对应</div>
          <div className="desc">Claude Code 的三档模型分别用 Kiro 的哪个</div>
        </div>
        <div>
          {MODEL_TIERS.map((tier: ModelTier) => {
            const saved = config?.models[tier] ?? ''
            const picker = modelPicker(saved, app.models, app.gatewayRunning)
            return (
              <div className="field-row" key={tier}>
                <span id={`tier-${tier}`}>{tier}</span>
                <select
                  aria-labelledby={`tier-${tier}`}
                  value={saved}
                  disabled={picker.disabled || !config}
                  onChange={(e) => void app.saveConfig({ models: { [tier]: e.target.value } })}
                >
                  {picker.options.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </div>
            )
          })}
          {!app.gatewayRunning && <div className="desc">启动网关后可选择模型</div>}
        </div>
      </div>

      <div className="setting">
        <div>
          <div className="name" id="autostart-name">
            开机自启
          </div>
          <div className="desc">{AUTOSTART_DESCRIPTION}</div>
        </div>
        <div className="inline">
          <button
            type="button"
            className="mini-switch"
            role="switch"
            aria-checked={config?.autostart ?? false}
            aria-labelledby="autostart-name"
            disabled={!config || app.configAction.pending}
            onClick={() => config && void app.saveConfig({ autostart: !config.autostart })}
          />
          <span className="desc">{autostartLabel(config?.autostart ?? false)}</span>
        </div>
      </div>

      <div className="setting">
        <div>
          <div className="name">kclaude</div>
          <div className="desc">{rcDescription(app.kclaude)}</div>
        </div>
        <div className="inline">
          <button
            type="button"
            className="btn"
            disabled={!kcRemovable || app.kclaudeAction.pending}
            onClick={() => void app.removeKclaude()}
          >
            移除 kclaude
          </button>
          {app.kclaudeAction.error && <span className="action-error" role="alert">{app.kclaudeAction.error}</span>}
        </div>
      </div>

      <div className="setting">
        <div>
          <div className="name">版本</div>
          <div className="desc">新版本会在后台下载，下次启动时生效</div>
        </div>
        <div>
          <div className="inline">
            <span className={ver.tone === 'bad' ? 'bad' : undefined}>{ver.text}</span>
            <button type="button" className="btn ghost" disabled={ver.checkDisabled} onClick={() => void app.checkUpdate()}>
              {ver.checkLabel}
            </button>
            {ver.restart && (
              <button type="button" className="btn" onClick={() => void app.restartNow()}>
                立即重启
              </button>
            )}
          </div>
          {ver.manualUrl && (
            <div className="desc">
              <button type="button" className="link" onClick={() => void openExternalUrl(ver.manualUrl!)}>
                {ver.manualLabel}
              </button>
            </div>
          )}
          {app.updateAction.error && <div className="err">{app.updateAction.error}</div>}
          {app.configAction.error && <div className="err" role="alert">{app.configAction.error}</div>}
        </div>
      </div>
    </div>
  )
}
