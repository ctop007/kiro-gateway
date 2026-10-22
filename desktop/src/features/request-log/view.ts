/** Request log records and summaries → table rows, cards and advice (pure). */
import { UNKNOWN_MODEL_LABEL } from '@/lib/ipc-types'
import type {
  ApiKind,
  ModelUsage,
  Outcome,
  RequestLogAppended,
  RequestLogSnapshot,
  RequestRecord,
  UsageSummary
} from '@/lib/ipc-types'
import {
  formatCreditsRow,
  formatCreditsSummary,
  formatDuration,
  formatLogTime,
  formatMonthLabel
} from '@/features/shared/format'

/** Same label the backend groups by (contract section 5); the filter dropdown shows one entry. */
export const UNKNOWN_MODEL = UNKNOWN_MODEL_LABEL

/** A record as held by the list: `seq` is assigned when it enters, and never changes. */
export type KeyedRecord = RequestRecord & { seq?: number }

/** Model name of a record; null and empty both count as unknown, as in the backend. */
export function modelLabel(r: RequestRecord): string {
  return r.model ? r.model : UNKNOWN_MODEL
}

export interface ErrorAdvice {
  summary: string
  tip: string
}

// Anthropic path: "prompt is too long: ..."; OpenAI path (kiro/kiro_errors.py):
// "Model context limit reached. Conversation size exceeds model capacity."
const CONTEXT_PATTERN =
  /too long|context (window|length|limit)|exceed.*context|exceeds model capacity/i

/**
 * Error summary plus one actionable sentence for a failed or refused record.
 * Matching uses the HTTP status first and the error text second, so a 402 is
 * never mistaken for a context error.
 */
export function errorAdvice(r: RequestRecord): ErrorAdvice {
  const text = r.error?.trim() ?? ''
  if (r.outcome === 'refused') {
    return { summary: text || '模型拒绝回答。', tip: '换个问法，或换用 sonnet 再试。' }
  }
  const summary = text || `HTTP ${r.status}`
  if (r.status === 402) return { summary, tip: 'Kiro 本月额度已用完。可以在 kiro-cli 里查看用量和重置时间。' }
  if (r.status === 401 || r.status === 403) {
    return { summary, tip: 'Kiro 的登录失效了。在终端运行 kiro-cli login 重新登录，然后重试。' }
  }
  if (r.status === 429) return { summary, tip: '请求太频繁了。稍等一会儿再试。' }
  if (r.status === 400 && CONTEXT_PATTERN.test(text)) {
    return { summary, tip: '对话超出了模型的上下文窗口。在 Claude Code 里执行 /compact 压缩一下。' }
  }
  if (r.status >= 500) return { summary, tip: '网关没能从 Kiro 拿到结果。检查网络后重试。' }
  return { summary, tip: '没有对应的常见原因，重试一次看看。' }
}

export interface OutcomeTag {
  cls: 'ok' | 'err' | 'ref' | 'gone'
  label: string
}

const TAGS: Record<Outcome, OutcomeTag> = {
  ok: { cls: 'ok', label: '成功' },
  error: { cls: 'err', label: '失败' },
  refused: { cls: 'ref', label: '拒答' },
  client_disconnected: { cls: 'gone', label: '中途断开' }
}

export function outcomeTag(o: Outcome): OutcomeTag {
  return TAGS[o]
}

/** Everything but a success counts for the "仅失败" filter. */
export function isFailure(r: RequestRecord): boolean {
  return r.outcome !== 'ok'
}

/** Only errors and refusals have details worth expanding. */
export function isExpandable(r: RequestRecord): boolean {
  return r.outcome === 'error' || r.outcome === 'refused'
}

export interface LogFilters {
  result: 'all' | 'failed'
  /** Model to show, or null for all. `UNKNOWN_MODEL` matches records without a model. */
  model: string | null
}

/** Filter the list only; summaries come from the backend and are not filtered. */
export function applyFilters(records: RequestRecord[], filters: LogFilters): RequestRecord[] {
  return records.filter((r) => {
    if (filters.result === 'failed' && !isFailure(r)) return false
    if (filters.model !== null && modelLabel(r) !== filters.model) return false
    return true
  })
}

/** Distinct model names for the dropdown: recent records plus this month's groups. */
export function modelFilterOptions(records: RequestRecord[], byModel: ModelUsage[]): string[] {
  const set = new Set<string>()
  for (const r of records) set.add(modelLabel(r))
  for (const m of byModel) set.add(m.model)
  return [...set].sort()
}

export interface SummaryView {
  todayValue: string
  todaySub: string
  monthValue: string
  monthSub: string
}

/** Card numbers; the template appends the unit "credits". */
export function summaryView(s: UsageSummary): SummaryView {
  return {
    todayValue: formatCreditsSummary(s.today.credits),
    todaySub: `今日，${s.today.requests} 次请求`,
    monthValue: formatCreditsSummary(s.month.credits),
    monthSub: `${formatMonthLabel(s.month.month)}，${s.month.requests} 次请求`
  }
}

export interface ModelBar {
  model: string
  value: string
  widthPct: number
}

const MIN_VISIBLE_PCT = 2

/** Per-model bars, width = share of the month total. */
export function modelBars(s: UsageSummary): ModelBar[] {
  const total = s.month.credits
  return s.byModel.map((m) => {
    let pct = total > 0 && m.credits > 0 ? (m.credits / total) * 100 : 0
    if (pct > 0) pct = Math.min(100, Math.max(MIN_VISIBLE_PCT, pct))
    return { model: m.model, value: formatCreditsSummary(m.credits), widthPct: pct }
  })
}

export interface RowView {
  key: string
  time: string
  model: string
  api: string
  stream: boolean
  tag: OutcomeTag
  duration: string
  credits: string
  expandable: boolean
  detail: ErrorAdvice | null
}

const API_LABEL: Record<ApiKind, string> = { openai: 'OpenAI', anthropic: 'Anthropic' }

let nextSeq = 1

/** Dedupe identity of a record: id and time when it has an id, every field otherwise. */
function recordSignature(r: RequestRecord): string {
  if (r.id !== null) return `${r.id}|${r.ts}`
  return JSON.stringify([r.ts, r.api, r.model, r.stream, r.status, r.outcome, r.durationMs, r.credits, r.error])
}

/** React key. Records without an id get the sequence number assigned when they entered the list. */
function recordKey(r: KeyedRecord): string {
  if (r.id !== null) return `${r.id}|${r.ts}`
  return `no-id|${r.ts}|${r.seq ?? 0}`
}

function withSeq(r: KeyedRecord): KeyedRecord {
  return r.seq === undefined ? { ...r, seq: nextSeq++ } : r
}

/**
 * Give every record of a freshly loaded snapshot its sequence number. Call on every
 * snapshot that replaces the held list (initial load, reload, clear).
 */
export function indexSnapshot(snapshot: RequestLogSnapshot): RequestLogSnapshot {
  return { ...snapshot, records: snapshot.records.map(withSeq) }
}

export function rowView(r: KeyedRecord, now: Date, timeZone?: string): RowView {
  const expandable = isExpandable(r)
  return {
    key: recordKey(r),
    time: formatLogTime(r.ts, now, timeZone),
    model: modelLabel(r),
    api: API_LABEL[r.api],
    stream: r.stream,
    tag: outcomeTag(r.outcome),
    duration: formatDuration(r.durationMs),
    credits: formatCreditsRow(r.credits),
    expandable,
    detail: expandable ? errorAdvice(r) : null
  }
}

/**
 * What to do with a `request-log-appended` push: a `reset` push (a month file was deleted,
 * truncated or replaced) means the held list may show records that no longer exist, so it
 * is reloaded from a snapshot instead of merged.
 */
export function appendedAction(pushed: RequestLogAppended): 'reload' | 'merge' {
  return pushed.reset ? 'reload' : 'merge'
}

/**
 * Apply a `request-log-appended` push: new records go on top (newest first),
 * duplicates already in the list are ignored, the tail is cut at `limit`, and
 * the summary is replaced wholesale by the pushed one.
 */
export function mergeAppended(
  current: RequestLogSnapshot,
  pushed: RequestLogAppended,
  limit: number
): RequestLogSnapshot {
  const held = current.records.map(withSeq)
  // A pushed record that the list already holds is an overlap with the snapshot and is
  // dropped. Copies are counted, so identical records sharing one push all survive.
  const heldCount = new Map<string, number>()
  for (const r of held) heldCount.set(recordSignature(r), (heldCount.get(recordSignature(r)) ?? 0) + 1)
  const pushedSoFar = new Map<string, number>()
  const fresh: KeyedRecord[] = []
  for (const r of pushed.records) {
    const sig = recordSignature(r)
    const n = (pushedSoFar.get(sig) ?? 0) + 1
    pushedSoFar.set(sig, n)
    if (n > (heldCount.get(sig) ?? 0)) fresh.push(withSeq(r))
  }
  const records = [...fresh, ...held]
    .map((r, i) => ({ r, i }))
    .sort((a, b) => {
      const ta = Date.parse(a.r.ts)
      const tb = Date.parse(b.r.ts)
      if (Number.isNaN(ta) || Number.isNaN(tb) || ta === tb) return a.i - b.i
      return tb - ta
    })
    .map((x) => x.r)
    .slice(0, Math.max(0, limit))
  return {
    ...current,
    records,
    today: pushed.today,
    month: pushed.month,
    byModel: pushed.byModel
  }
}

export type EmptyState = 'none' | 'filtered' | 'rows'

/** What the table area shows: the first-run hint, a filter miss, or rows. */
export function emptyState(total: number, shown: number): EmptyState {
  if (total === 0) return 'none'
  return shown === 0 ? 'filtered' : 'rows'
}

/** A model filter that no longer matches any known model resets to "all". */
export function effectiveFilters(filters: LogFilters, models: string[]): LogFilters {
  return filters.model !== null && !models.includes(filters.model) ? { ...filters, model: null } : filters
}

export function liveLabel(gatewayRunning: boolean): string {
  return gatewayRunning ? '实时更新中' : '网关未运行'
}
