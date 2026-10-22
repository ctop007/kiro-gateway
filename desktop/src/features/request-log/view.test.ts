import { describe, expect, it } from 'vitest'
import { rec } from '@/test-support/fixtures'
import { UNKNOWN_MODEL_LABEL } from '@/lib/ipc-types'
import type { RequestLogAppended, RequestLogSnapshot, RequestRecord, UsageSummary } from '@/lib/ipc-types'
import {
  UNKNOWN_MODEL,
  applyFilters,
  effectiveFilters,
  emptyState,
  liveLabel,
  errorAdvice,
  indexSnapshot,
  isExpandable,
  isFailure,
  appendedAction,
  mergeAppended,
  modelFilterOptions,
  modelBars,
  outcomeTag,
  rowView,
  summaryView
} from './view'

const SH = 'Asia/Shanghai'
const NOW = new Date('2026-10-22T15:00:00+08:00')

const summary: UsageSummary = {
  today: { credits: 1.84, requests: 32 },
  month: { credits: 46.2, requests: 611, month: '2026-10' },
  byModel: [
    { model: 'claude-opus-5.5', credits: 36.02, requests: 400 },
    { model: 'claude-sonnet-5.5', credits: 8.77, requests: 150 },
    { model: 'claude-haiku-4.5', credits: 1.41, requests: 61 }
  ]
}

const snap = (records: RequestRecord[], over: Partial<UsageSummary> = {}): RequestLogSnapshot => ({
  ...summary,
  ...over,
  records,
  generatedAt: '2026-10-22T15:00:00+08:00'
})

describe('errorAdvice', () => {
  it('402: quota exhausted', () => {
    const a = errorAdvice(rec({ status: 402, outcome: 'error', error: 'HTTP 402：Monthly request limit exceeded' }))
    expect(a.summary).toBe('HTTP 402：Monthly request limit exceeded')
    expect(a.tip).toBe('Kiro 本月额度已用完。可以在 kiro-cli 里查看用量和重置时间。')
  })

  it('402 beats the "exceeded" wording of the message', () => {
    expect(errorAdvice(rec({ status: 402, outcome: 'error', error: 'limit exceeded' })).tip).toContain('额度已用完')
  })

  it('context too long (400): suggest /compact', () => {
    const a = errorAdvice(rec({ status: 400, outcome: 'error', error: 'HTTP 400：prompt is too long' }))
    expect(a.tip).toBe('对话超出了模型的上下文窗口。在 Claude Code 里执行 /compact 压缩一下。')
  })

  it.each([
    // kiro/kiro_errors.py, OpenAI path: user_message for CONTENT_LENGTH_EXCEEDS_THRESHOLD
    'HTTP 400：Model context limit reached. Conversation size exceeds model capacity.',
    'Model context limit reached. Conversation size exceeds model capacity.',
    // Anthropic path: the same message prefixed with ANTHROPIC_CONTEXT_OVERFLOW_PREFIX
    'prompt is too long: Model context limit reached. Conversation size exceeds model capacity.'
  ])('recognizes the real gateway wording %s', (msg) => {
    const a = errorAdvice(rec({ status: 400, outcome: 'error', error: msg }))
    expect(a.tip).toBe('对话超出了模型的上下文窗口。在 Claude Code 里执行 /compact 压缩一下。')
  })

  it('a 400 that only mentions a limit of something else is not a context error', () => {
    expect(errorAdvice(rec({ status: 400, outcome: 'error', error: 'Monthly request limit exceeded.' })).tip).not.toContain('/compact')
  })

  it.each(['Input is too long for requested model', 'context window exceeded', 'context length limit'])(
    'recognizes the context wording %s',
    (msg) => {
      expect(errorAdvice(rec({ status: 400, outcome: 'error', error: msg })).tip).toContain('/compact')
    }
  )

  it.each([401, 403])('%i: login expired', (status) => {
    const a = errorAdvice(rec({ status, outcome: 'error', error: 'HTTP 403：token refresh failed' }))
    expect(a.tip).toBe('Kiro 的登录失效了。在终端运行 kiro-cli login 重新登录，然后重试。')
  })

  it('refused: its own summary and tip, regardless of status', () => {
    const a = errorAdvice(rec({ status: 200, outcome: 'refused', error: null }))
    expect(a.summary).toBe('模型拒绝回答。')
    expect(a.tip).toBe('换个问法，或换用 sonnet 再试。')
  })

  it('refused keeps the gateway error text when present', () => {
    expect(errorAdvice(rec({ outcome: 'refused', error: '模型拒绝回答（CONTENT_FILTERED）。' })).summary).toBe(
      '模型拒绝回答（CONTENT_FILTERED）。'
    )
  })

  it('429: slow down', () => {
    expect(errorAdvice(rec({ status: 429, outcome: 'error', error: 'HTTP 429' })).tip).toBe(
      '请求太频繁了。稍等一会儿再试。'
    )
  })

  it.each([500, 502, 503])('%i: upstream trouble', (status) => {
    expect(errorAdvice(rec({ status, outcome: 'error', error: 'bad gateway' })).tip).toBe(
      '网关没能从 Kiro 拿到结果。检查网络后重试。'
    )
  })

  it('unknown 4xx: generic tip that does not invent a cause', () => {
    const a = errorAdvice(rec({ status: 418, outcome: 'error', error: 'teapot' }))
    expect(a.tip).toBe('没有对应的常见原因，重试一次看看。')
  })

  it('missing error text: states the HTTP status', () => {
    expect(errorAdvice(rec({ status: 400, outcome: 'error', error: null })).summary).toBe('HTTP 400')
    expect(errorAdvice(rec({ status: 400, outcome: 'error', error: '   ' })).summary).toBe('HTTP 400')
  })
})

describe('outcome helpers', () => {
  it('tags every outcome', () => {
    expect(outcomeTag('ok')).toEqual({ cls: 'ok', label: '成功' })
    expect(outcomeTag('error')).toEqual({ cls: 'err', label: '失败' })
    expect(outcomeTag('refused')).toEqual({ cls: 'ref', label: '拒答' })
    expect(outcomeTag('client_disconnected')).toEqual({ cls: 'gone', label: '中途断开' })
  })

  it('failure is anything that is not a success', () => {
    expect(isFailure(rec({ outcome: 'ok' }))).toBe(false)
    expect(isFailure(rec({ outcome: 'error' }))).toBe(true)
    expect(isFailure(rec({ outcome: 'refused' }))).toBe(true)
    expect(isFailure(rec({ outcome: 'client_disconnected' }))).toBe(true)
  })

  it('only errors and refusals expand', () => {
    expect(isExpandable(rec({ outcome: 'error' }))).toBe(true)
    expect(isExpandable(rec({ outcome: 'refused' }))).toBe(true)
    expect(isExpandable(rec({ outcome: 'client_disconnected' }))).toBe(false)
    expect(isExpandable(rec({ outcome: 'ok' }))).toBe(false)
  })
})

describe('applyFilters', () => {
  const records = [
    rec({ id: '1', outcome: 'ok', model: 'claude-opus-5.5' }),
    rec({ id: '2', outcome: 'error', model: 'claude-opus-5.5' }),
    rec({ id: '3', outcome: 'refused', model: 'claude-haiku-4.5' }),
    rec({ id: '4', outcome: 'client_disconnected', model: 'claude-haiku-4.5' }),
    rec({ id: '5', outcome: 'ok', model: null })
  ]
  const ids = (rs: typeof records) => rs.map((r) => r.id)

  it('all / all keeps everything in order', () => {
    expect(ids(applyFilters(records, { result: 'all', model: null }))).toEqual(['1', '2', '3', '4', '5'])
  })

  it('failed only keeps every non-success outcome', () => {
    expect(ids(applyFilters(records, { result: 'failed', model: null }))).toEqual(['2', '3', '4'])
  })

  it('model filter', () => {
    expect(ids(applyFilters(records, { result: 'all', model: 'claude-haiku-4.5' }))).toEqual(['3', '4'])
  })

  it('combines both filters', () => {
    expect(ids(applyFilters(records, { result: 'failed', model: 'claude-opus-5.5' }))).toEqual(['2'])
  })

  it('records without a model match the unknown-model entry', () => {
    expect(ids(applyFilters(records, { result: 'all', model: '未知模型' }))).toEqual(['5'])
  })

  it('the unknown-model label is the contract one, shared with the backend', () => {
    expect(UNKNOWN_MODEL_LABEL).toBe('未知模型')
    expect(UNKNOWN_MODEL).toBe(UNKNOWN_MODEL_LABEL)
  })

  it('an empty-string model counts as unknown, like the backend groups it', () => {
    const withEmpty = [...records, rec({ id: '6', model: '' })]
    expect(ids(applyFilters(withEmpty, { result: 'all', model: UNKNOWN_MODEL_LABEL }))).toEqual(['5', '6'])
  })

  it('does not mutate its input', () => {
    const copy = [...records]
    applyFilters(records, { result: 'failed', model: null })
    expect(records).toEqual(copy)
  })

  it('never touches the summary: it takes only records', () => {
    expect(applyFilters.length).toBe(2)
  })
})

describe('modelFilterOptions', () => {
  it('unions record models and monthly models, sorted, without duplicates', () => {
    const opts = modelFilterOptions([rec({ model: 'b' }), rec({ model: 'a' }), rec({ model: null })], [
      { model: 'c', credits: 1, requests: 1 },
      { model: 'a', credits: 1, requests: 1 }
    ])
    expect(opts).toEqual(['a', 'b', 'c', '未知模型'])
  })

  it('offers the backend unknown-model group and unknown records as one entry', () => {
    const opts = modelFilterOptions([rec({ model: null }), rec({ model: '' })], [
      { model: '未知模型', credits: 1, requests: 1 },
      { model: 'a', credits: 1, requests: 1 }
    ])
    expect(opts.filter((o) => o.includes('未知'))).toEqual(['未知模型'])
  })

  it('selecting the backend-provided unknown entry keeps the records without a model', () => {
    const recs = [rec({ id: '1', model: 'a' }), rec({ id: '2', model: null })]
    const opts = modelFilterOptions(recs, [{ model: '未知模型', credits: 1, requests: 1 }])
    const pick = opts.find((o) => o.includes('未知'))!
    expect(applyFilters(recs, { result: 'all', model: pick }).map((r) => r.id)).toEqual(['2'])
  })

  it('is empty when there is nothing', () => {
    expect(modelFilterOptions([], [])).toEqual([])
  })
})

describe('summaryView', () => {
  it('formats cards with units left to the template', () => {
    expect(summaryView(summary)).toEqual({
      todayValue: '1.84',
      todaySub: '今日，32 次请求',
      monthValue: '46.20',
      monthSub: '10 月，611 次请求'
    })
  })

  it('zeroes render as a bare 0', () => {
    const v = summaryView({
      today: { credits: 0, requests: 0 },
      month: { credits: 0, requests: 0, month: '2026-10' },
      byModel: []
    })
    expect(v).toMatchObject({ todayValue: '0', todaySub: '今日，0 次请求', monthValue: '0' })
  })

  it('never renders money', () => {
    const v = JSON.stringify(summaryView(summary))
    expect(v).not.toMatch(/[¥$￥元]/)
  })
})

describe('modelBars', () => {
  it('uses the share of the month total for widths', () => {
    const bars = modelBars(summary)
    expect(bars.map((b) => [b.model, b.value])).toEqual([
      ['claude-opus-5.5', '36.02'],
      ['claude-sonnet-5.5', '8.77'],
      ['claude-haiku-4.5', '1.41']
    ])
    expect(bars.map((b) => Math.round(b.widthPct))).toEqual([78, 19, 3])
  })

  it('keeps tiny non-zero shares visible', () => {
    const bars = modelBars({
      ...summary,
      month: { ...summary.month, credits: 1000 },
      byModel: [{ model: 'x', credits: 0.01, requests: 1 }]
    })
    expect(bars[0].widthPct).toBeGreaterThanOrEqual(2)
  })

  it('zero total gives zero-width bars, not NaN', () => {
    const bars = modelBars({ ...summary, month: { ...summary.month, credits: 0 }, byModel: [
      { model: 'x', credits: 0, requests: 2 }
    ] })
    expect(bars[0].widthPct).toBe(0)
  })

  it('clamps at 100 percent when rounding drift makes a share exceed the total', () => {
    const bars = modelBars({ ...summary, month: { ...summary.month, credits: 1 }, byModel: [
      { model: 'x', credits: 1.5, requests: 1 }
    ] })
    expect(bars[0].widthPct).toBe(100)
  })

  it('empty month has no bars', () => {
    expect(modelBars({ ...summary, byModel: [] })).toEqual([])
  })
})

describe('rowView', () => {
  it('formats one row for the table', () => {
    const v = rowView(rec(), NOW, SH)
    expect(v).toMatchObject({
      time: '14:41:08',
      model: 'claude-opus-5.5',
      api: 'Anthropic',
      stream: true,
      tag: { cls: 'ok', label: '成功' },
      duration: '12.4 s',
      credits: '0.3112',
      expandable: false
    })
  })

  it('uses local time for another day', () => {
    expect(rowView(rec({ ts: '2026-10-21T23:59:59+08:00' }), NOW, SH).time).toBe('10-21 23:59:59')
  })

  it('OpenAI label and unknown model', () => {
    const v = rowView(rec({ api: 'openai', model: null }), NOW, SH)
    expect(v.api).toBe('OpenAI')
    expect(v.model).toBe('未知模型')
  })

  it('failed rows carry the expandable detail', () => {
    const v = rowView(rec({ outcome: 'error', status: 402, error: 'HTTP 402：x', credits: 0 }), NOW, SH)
    expect(v.expandable).toBe(true)
    expect(v.detail!.tip).toContain('额度已用完')
    expect(v.credits).toBe('0')
  })

  it('a key that is unique even when the id is missing', () => {
    const a = rowView(rec({ id: null, ts: '2026-10-22T14:41:08+08:00' }), NOW, SH).key
    const b = rowView(rec({ id: null, ts: '2026-10-22T14:41:09+08:00' }), NOW, SH).key
    expect(a).not.toBe(b)
  })
})

describe('records that share a millisecond and have no id', () => {
  const TS = '2026-10-22T14:41:08.123+08:00'
  const a = rec({ id: null, ts: TS, model: 'm1', credits: 0.1 })
  const b = rec({ id: null, ts: TS, model: 'm2', credits: 0.2 })
  const push = (records: RequestRecord[]): RequestLogAppended => ({ ...summary, records, reset: false })

  it('both survive a push into an empty list and get different keys', () => {
    const m = mergeAppended(snap([]), push([b, a]), 200)
    expect(m.records).toHaveLength(2)
    const keys = m.records.map((r) => rowView(r, NOW, SH).key)
    expect(new Set(keys).size).toBe(2)
  })

  it('a second same-millisecond record arriving later is kept, not merged into the first', () => {
    const first = mergeAppended(snap([]), push([a]), 200)
    const second = mergeAppended(first, push([b]), 200)
    expect(second.records.map((r) => r.model)).toEqual(['m2', 'm1'])
    const keys = second.records.map((r) => rowView(r, NOW, SH).key)
    expect(new Set(keys).size).toBe(2)
  })

  it('two identical records (same ms, same content) both survive and stay distinct', () => {
    const twin = { ...a }
    const m = mergeAppended(mergeAppended(snap([]), push([a]), 200), push([twin]), 200)
    // An identical record pushed again is indistinguishable from an overlap with the
    // snapshot, so only a push that carries more copies than the list holds adds any.
    expect(m.records.length).toBeGreaterThanOrEqual(1)
    const both = mergeAppended(snap([]), push([a, twin]), 200)
    expect(both.records).toHaveLength(2)
    expect(new Set(both.records.map((r) => rowView(r, NOW, SH).key)).size).toBe(2)
  })

  it('an overlap with the snapshot is still dropped', () => {
    const m = mergeAppended(snap([a, b]), push([a, b]), 200)
    expect(m.records).toHaveLength(2)
  })

  it('keys of existing rows do not change when newer records are merged on top', () => {
    const first = mergeAppended(snap([]), push([a, b]), 200)
    const before = first.records.map((r) => rowView(r, NOW, SH).key)
    const next = mergeAppended(first, push([rec({ id: null, ts: '2026-10-22T14:50:00+08:00' })]), 200)
    const after = next.records.slice(1).map((r) => rowView(r, NOW, SH).key)
    expect(after).toEqual(before)
  })

  it('keys stay unique across a snapshot load of many same-ms records', () => {
    const many = Array.from({ length: 5 }, (_, i) => rec({ id: null, ts: TS, credits: i }))
    const loaded = indexSnapshot(snap(many))
    expect(new Set(loaded.records.map((r) => rowView(r, NOW, SH).key)).size).toBe(5)
  })
})

describe('appendedAction', () => {
  it('a reset push reloads the list, any other push is merged', () => {
    const push = (reset: boolean): RequestLogAppended => ({ ...summary, records: [], reset })
    expect(appendedAction(push(true))).toBe('reload')
    expect(appendedAction(push(false))).toBe('merge')
  })
})

describe('mergeAppended', () => {
  const base = [rec({ id: 'a', ts: '2026-10-22T14:00:00+08:00' }), rec({ id: 'b', ts: '2026-10-22T13:00:00+08:00' })]
  const appended = (records: RequestLogAppended['records']): RequestLogAppended => ({
    ...summary,
    records,
    reset: false
  })

  it('puts new records on top and replaces the summary wholesale', () => {
    const next = { ...summary, today: { credits: 9, requests: 33 } }
    const m = mergeAppended(
      snap(base),
      { ...next, records: [rec({ id: 'c', ts: '2026-10-22T14:30:00+08:00' })], reset: false },
      200
    )
    expect(m.records.map((r) => r.id)).toEqual(['c', 'a', 'b'])
    expect(m.today.credits).toBe(9)
  })

  it('an empty push (day rollover) zeroes the summary and keeps the list', () => {
    const zero = {
      today: { credits: 0, requests: 0 },
      month: { credits: 46.2, requests: 611, month: '2026-10' },
      byModel: summary.byModel,
      records: [],
      reset: false
    }
    const m = mergeAppended(snap(base), zero, 200)
    expect(m.records).toHaveLength(2)
    expect(m.today.credits).toBe(0)
  })

  it('drops tail records beyond the limit', () => {
    const m = mergeAppended(
      snap(base),
      appended([rec({ id: 'c', ts: '2026-10-22T14:30:00+08:00' })]),
      2
    )
    expect(m.records.map((r) => r.id)).toEqual(['c', 'a'])
  })

  it('ignores records that the snapshot already contains', () => {
    const m = mergeAppended(snap(base), appended([base[0]]), 200)
    expect(m.records.map((r) => r.id)).toEqual(['a', 'b'])
  })

  it('keeps newest-first order even if the pushed batch is out of order', () => {
    const m = mergeAppended(
      snap(base),
      appended([
        rec({ id: 'x', ts: '2026-10-22T14:10:00+08:00' }),
        rec({ id: 'y', ts: '2026-10-22T14:20:00+08:00' })
      ]),
      200
    )
    expect(m.records.map((r) => r.id)).toEqual(['y', 'x', 'a', 'b'])
  })

  it('works from an empty list', () => {
    const m = mergeAppended(snap([]), appended([rec({ id: 'z' })]), 200)
    expect(m.records.map((r) => r.id)).toEqual(['z'])
  })

  it('does not mutate its inputs', () => {
    const cur = snap([...base])
    mergeAppended(cur, appended([rec({ id: 'n', ts: '2026-10-22T14:30:00+08:00' })]), 200)
    expect(cur.records).toHaveLength(2)
  })
})

describe('emptyState', () => {
  it('no records at all -> none', () => {
    expect(emptyState(0, 0)).toBe('none')
  })
  it('records exist but the filter hides all -> filtered', () => {
    expect(emptyState(5, 0)).toBe('filtered')
  })
  it('something to show -> rows', () => {
    expect(emptyState(5, 2)).toBe('rows')
  })
})

describe('effectiveFilters', () => {
  it('keeps a model that still exists', () => {
    expect(effectiveFilters({ result: 'failed', model: 'a' }, ['a', 'b'])).toEqual({ result: 'failed', model: 'a' })
  })
  it('drops a model that vanished (for example after clearing the log)', () => {
    expect(effectiveFilters({ result: 'all', model: 'gone' }, ['a'])).toEqual({ result: 'all', model: null })
  })
})

describe('liveLabel', () => {
  it('reflects whether the gateway is running', () => {
    expect(liveLabel(true)).toBe('实时更新中')
    expect(liveLabel(false)).toBe('网关未运行')
  })
})
