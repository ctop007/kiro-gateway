import type { AccountSnapshot } from '@/lib/ipc-types'

/** Mask both email parts by default; the user can explicitly reveal their account. */
export function maskEmail(email: string | null): string {
  if (!email) return '未获取账号'
  const [name, domain] = email.split('@')
  if (!name || !domain) return '已登录账号'
  const suffix = domain.includes('.') ? domain.slice(domain.lastIndexOf('.')) : ''
  return `${name.slice(0, Math.min(2, name.length))}***@***${suffix}`
}

/** Unknown quantities remain unknown; percentages may exceed the visual bar's maximum. */
export function accountView(account: AccountSnapshot | null) {
  const number = (n: number | null | undefined) => n != null && Number.isFinite(n)
    ? new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 2 }).format(n) : '—'
  const date = (value: string | null | undefined) => value && Number.isFinite(Date.parse(value))
    ? new Date(value).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false }) : '—'
  return {
    used: number(account?.used), total: number(account?.total), remaining: number(account?.remaining),
    percentage: account?.usagePercent != null ? `${number(account.usagePercent)}%` : '—',
    progress: Math.min(100, Math.max(0, account?.usagePercent ?? 0)),
    reset: date(account?.resetAt), expiry: date(account?.tokenExpiresAt), checked: date(account?.checkedAt),
    overage: number(account?.overageUsed)
  }
}
