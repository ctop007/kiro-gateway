import { Fragment, useMemo, useState, type KeyboardEvent } from 'react'
import type { AppState } from '@/state/useAppState'
import {
  applyFilters,
  effectiveFilters,
  emptyState,
  liveLabel,
  modelBars,
  modelFilterOptions,
  rowView,
  summaryView,
  type LogFilters
} from '@/features/request-log/view'
import { logsEmptyHint } from '@/features/command-name/view'

export function LogsView({ app }: { app: AppState }) {
  const [filters, setFilters] = useState<LogFilters>({ result: 'all', model: null })
  const [open, setOpen] = useState<Set<string>>(() => new Set())
  const [confirming, setConfirming] = useState(false)

  const log = app.log
  const options = useMemo(() => (log ? modelFilterOptions(log.records, log.byModel) : []), [log])
  const active = effectiveFilters(filters, options)
  const shown = useMemo(() => (log ? applyFilters(log.records, active) : []), [log, active])

  const toggle = (key: string) =>
    setOpen((cur) => {
      const next = new Set(cur)
      if (!next.delete(key)) next.add(key)
      return next
    })

  const onRowKey = (e: KeyboardEvent<HTMLTableRowElement>, key: string) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      toggle(key)
    }
  }

  const summary = log ? summaryView(log) : null
  const bars = log ? modelBars(log) : []
  const emptyHint = logsEmptyHint(app.commandName)
  const state = emptyState(log?.records.length ?? 0, shown.length)

  const doClear = async () => {
    if (await app.clearLog()) {
      setConfirming(false)
      setOpen(new Set())
    }
  }

  return (
    <>
      <div className="log-head">
        <div className="panel">
          <div className="big">
            {summary?.todayValue ?? '—'}
            <small>credits</small>
          </div>
          <div className="sub">{summary?.todaySub ?? '今日'}</div>
        </div>
        <div className="panel">
          <div className="big">
            {summary?.monthValue ?? '—'}
            <small>credits</small>
          </div>
          <div className="sub">{summary?.monthSub ?? '本月'}</div>
        </div>
        <div className="panel">
          <ul className="bars" aria-label="本月按模型的 credits">
            {bars.length === 0 ? (
              <li className="none">本月还没有用量</li>
            ) : (
              bars.map((b) => (
                <li key={b.model}>
                  <span className="model" title={b.model}>
                    {b.model}
                  </span>
                  <span className="track" aria-hidden="true">
                    <span className="fill" style={{ width: `${b.widthPct}%` }} />
                  </span>
                  <span className="val">{b.value}</span>
                </li>
              ))
            )}
          </ul>
        </div>
      </div>

      <div className="filters">
        <div className="seg" role="group" aria-label="结果筛选">
          <button type="button" aria-pressed={active.result === 'all'} onClick={() => setFilters({ ...active, result: 'all' })}>
            全部
          </button>
          <button type="button" aria-pressed={active.result === 'failed'} onClick={() => setFilters({ ...active, result: 'failed' })}>
            仅失败
          </button>
        </div>
        <select
          aria-label="模型筛选"
          value={active.model ?? ''}
          onChange={(e) => setFilters({ ...active, model: e.target.value === '' ? null : e.target.value })}
        >
          <option value="">全部模型</option>
          {options.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <span className="live">{liveLabel(app.gatewayRunning)}</span>
      </div>

      {state === 'none' && (
        <div className="empty">
          {emptyHint.before}
          <code>{emptyHint.code}</code>
          {emptyHint.after}
        </div>
      )}
      {state === 'filtered' && <div className="empty">没有符合筛选条件的请求。</div>}
      {state === 'rows' && (
        <table>
          <thead>
            <tr>
              <th scope="col">时间</th>
              <th scope="col">模型</th>
              <th scope="col" className="hide-sm">
                接口
              </th>
              <th scope="col">结果</th>
              <th scope="col" className="num hide-sm">
                耗时
              </th>
              <th scope="col" className="num">
                credits
              </th>
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => {
              const v = rowView(r, app.now)
              const expanded = open.has(v.key)
              return (
                <Fragment key={v.key}>
                  <tr
                    className={v.expandable ? 'fail' : undefined}
                    tabIndex={v.expandable ? 0 : undefined}
                    aria-expanded={v.expandable ? expanded : undefined}
                    aria-controls={v.expandable ? `detail-${v.key}` : undefined}
                    onClick={v.expandable ? () => toggle(v.key) : undefined}
                    onKeyDown={v.expandable ? (e) => onRowKey(e, v.key) : undefined}
                  >
                    <td>{v.time}</td>
                    <td className="model">{v.model}</td>
                    <td className="hide-sm">
                      {v.api}
                      {v.stream ? ' · 流式' : ''}
                    </td>
                    <td>
                      <span className={`tag ${v.tag.cls}`}>{v.tag.label}</span>
                    </td>
                    <td className="num hide-sm">{v.duration}</td>
                    <td className="num">{v.credits}</td>
                  </tr>
                  {v.detail && (
                    <tr className="detail" id={`detail-${v.key}`} hidden={!expanded}>
                      <td colSpan={6}>
                        <strong>{v.detail.summary}</strong>
                        <br />
                        {v.detail.tip}
                      </td>
                    </tr>
                  )}
                </Fragment>
              )
            })}
          </tbody>
        </table>
      )}

      <div className="log-foot">
        {confirming ? (
          <div className="confirm" role="group" aria-label="确认清空日志">
            <span>清空后列表和用量统计都会归零，不能恢复。</span>
            <button type="button" className="btn danger" disabled={app.logAction.pending} onClick={() => void doClear()}>
              确认清空
            </button>
            <button type="button" className="btn" onClick={() => setConfirming(false)}>
              取消
            </button>
          </div>
        ) : (
          <button type="button" className="btn" onClick={() => setConfirming(true)} disabled={!log}>
            清空日志
          </button>
        )}
        {app.logAction.error && (
          <span className="action-error" role="alert">
            {app.logAction.error}
          </span>
        )}
      </div>
    </>
  )
}
