import { useState } from 'react'
import type { AppState } from '@/state/useAppState'
import { accountView, maskEmail } from '@/features/account/view'

/** Provider account quota, separate from this gateway's request-log totals. */
export function AccountPanel({ app }: { app: AppState }) {
  const { data, pending, error, refreshed } = app.account
  const [revealedEmail, setRevealedEmail] = useState<string | null>(null)
  const revealed = !!data?.email && revealedEmail === data.email
  const view = accountView(data)
  const disabled = !app.gatewayRunning || !!pending
  return (
    <section className="panel account-panel" aria-labelledby="account-heading" aria-busy={!!pending}>
      <div className="account-heading"><h2 id="account-heading">当前账号</h2>{data && <span className="account-badge">{data.provider}</span>}</div>
      {!app.gatewayRunning ? <p className="account-empty">启动网关后查看账号和额度</p> : <>
        <div className="account-identity">
          <strong>{revealed ? data?.email : maskEmail(data?.email ?? null)}</strong>
          {data?.email && <button className="btn ghost" type="button" onClick={() => setRevealedEmail(revealed ? null : data.email)} aria-label={revealed ? '隐藏账号邮箱' : '显示账号邮箱'}>{revealed ? '隐藏' : '显示'}</button>}
        </div>
        <div className="account-quota-row"><span>{data?.subscription ?? '套餐待获取'}</span><span>{view.percentage}</span></div>
        <progress className="account-progress" aria-label="账号额度使用比例" max={100} value={view.progress} />
        <div className="account-quota-row"><span>{view.used} / {view.total} credits</span><strong>剩余 {view.remaining}</strong></div>
        {data?.overageEnabled && <p className="account-meta">已开启按量付费 · 超额使用 {view.overage} credits</p>}
        <div className="account-meta">{data?.resetAt && <span>额度重置 {view.reset}</span>}<span>Token 到期 {view.expiry}</span></div>
        <div className="account-actions">
          <button className="btn" type="button" disabled={disabled} onClick={() => void app.account.refresh('usage')}>{pending === 'usage' ? '刷新中…' : '刷新额度'}</button>
          <button className="btn" type="button" disabled={disabled || (data !== null && !data.canRefreshToken)} onClick={() => void app.account.refresh('token')}>{pending === 'token' ? '刷新中…' : '刷新 Token'}</button>
        </div>
        <div role="status" className="account-meta">{refreshed ? 'Token 已刷新' : data ? `${error ? '上次获取' : '更新于'} ${view.checked}` : pending ? '正在获取账号…' : ''}</div>
        {(error || data?.usageError) && <p className="err" role="alert">{error ?? data?.usageError}</p>}
      </>}
    </section>
  )
}
