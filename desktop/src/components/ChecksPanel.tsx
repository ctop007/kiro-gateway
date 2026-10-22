import type { AppState } from '@/state/useAppState'
import { checkRows } from '@/features/env/view'
import { openExternalUrl } from '@/lib/tauri-opener'
import { NoIcon, OkIcon, SkipIcon } from './Icons'

const MARK = { ok: <OkIcon />, bad: <NoIcon />, skip: <SkipIcon /> }
const TONE_LABEL = { ok: '通过', bad: '未通过', skip: '已跳过' }

export function ChecksPanel({ app }: { app: AppState }) {
  const rows = checkRows(app.env)
  return (
    <div className="panel">
      <h2>运行环境</h2>
      {rows.length === 0 ? (
        <p className="check-detail">{app.envAction.error ?? '正在检测…'}</p>
      ) : (
        <ul className="checks">
          {rows.map((r) => (
            <li key={r.id}>
              <span className={`mark ${r.tone}`}>
                {MARK[r.tone]}
                <span className="visually-hidden">{TONE_LABEL[r.tone]}</span>
              </span>
              <div>
                <div className="check-name">{r.title}</div>
                <div className="check-detail">{r.detail}</div>
                {r.fix && (
                  <div className="check-fix">
                    {r.fix.kind === 'copy_command' ? (
                      <>
                        <code>{r.fix.command}</code>
                        <button
                          type="button"
                          className="btn"
                          onClick={() => r.fix?.kind === 'copy_command' && void app.copyText(r.fix.command, r.id)}
                        >
                          {r.fix.label}
                        </button>
                        {app.copied === r.id && <span className="copied">已复制</span>}
                      </>
                    ) : (
                      <button
                        type="button"
                        className={`btn${r.id === 'kiro_cli_installed' ? ' primary' : ''}`}
                        onClick={() => r.fix?.kind === 'open_url' && void openExternalUrl(r.fix.url)}
                      >
                        {r.fix.label}
                      </button>
                    )}
                  </div>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      <button type="button" className="btn recheck" disabled={app.envAction.pending} onClick={() => void app.recheckEnv()}>
        {app.envAction.pending ? '正在检测' : '重新检测'}
      </button>
      {app.envAction.error && rows.length > 0 && (
        <p className="action-error" role="alert" style={{ marginTop: 8 }}>
          {app.envAction.error}
        </p>
      )}
    </div>
  )
}
