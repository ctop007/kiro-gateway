import type { AppState } from '@/state/useAppState'
import { summaryView } from '@/features/request-log/view'
import type { TabId } from '@/features/shell/tabs'

export function UsageStrip({ app, goto }: { app: AppState; goto: (t: TabId) => void }) {
  const v = app.log ? summaryView(app.log) : null
  return (
    <div className="panel">
      <div className="usage-strip">
        <div>
          <span className="num">{v?.todayValue ?? '—'}</span>
          <span className="lbl">{v ? `今日 credits，${app.log!.today.requests} 次请求` : '今日 credits'}</span>
        </div>
        <div>
          <span className="num">{v?.monthValue ?? '—'}</span>
          <span className="lbl">本月 credits</span>
        </div>
        <button type="button" className="btn" onClick={() => goto('logs')}>
          查看日志
        </button>
      </div>
    </div>
  )
}
