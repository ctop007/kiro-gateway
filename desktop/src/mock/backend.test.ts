import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createMockBackend, type MockBackend } from './backend'
import { IPC_ERROR_CODES, type GatewayStatus, type RequestLogAppended, type UpdateStatus } from '@/lib/ipc-types'

let b: MockBackend

beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-10-22T14:41:30+08:00'))
  b = createMockBackend({ startDelayMs: 1000 })
})
afterEach(() => {
  vi.useRealTimers()
})

async function rejection(p: Promise<unknown>) {
  try {
    await p
  } catch (e) {
    return e as { code: string; message: string }
  }
  throw new Error('expected rejection')
}

describe('mock backend: gateway', () => {
  it('start goes stopped -> starting -> running and emits both transitions', async () => {
    const seen: GatewayStatus[] = []
    b.listen('gateway-status', (s) => seen.push(s))
    const first = (await b.invoke('gateway_start')) as GatewayStatus
    expect(first.state).toBe('starting')
    await vi.advanceTimersByTimeAsync(1000)
    expect(seen.map((s) => s.state)).toEqual(['starting', 'running'])
    expect(((await b.invoke('gateway_status')) as GatewayStatus).state).toBe('running')
  })

  it('start is idempotent while starting or running', async () => {
    await b.invoke('gateway_start')
    const again = (await b.invoke('gateway_start')) as GatewayStatus
    expect(again.state).toBe('starting')
    await vi.advanceTimersByTimeAsync(1000)
    expect(((await b.invoke('gateway_start')) as GatewayStatus).state).toBe('running')
  })

  it('stop during starting cancels the pending transition', async () => {
    await b.invoke('gateway_start')
    await b.invoke('gateway_stop')
    await vi.advanceTimersByTimeAsync(5000)
    expect(((await b.invoke('gateway_status')) as GatewayStatus).state).toBe('stopped')
  })

  it('refuses to start when the environment is not ready, with the contract error', async () => {
    b.setScenario({ env: 'no_login' })
    const err = await rejection(b.invoke('gateway_start'))
    expect(err.code).toBe('env_not_ready')
    expect(IPC_ERROR_CODES).toContain(err.code)
  })

  it('refuses to start during a forced upgrade', async () => {
    b.setScenario({ update: 'forced' })
    expect((await rejection(b.invoke('gateway_start'))).code).toBe('upgrade_required')
  })

  it('port scenario ends in start_failed with an occupant and keeps the switch off', async () => {
    b.setScenario({ gateway: 'port_in_use' })
    await b.invoke('gateway_start')
    await vi.advanceTimersByTimeAsync(1000)
    const s = (await b.invoke('gateway_status')) as GatewayStatus
    expect(s.state).toBe('start_failed')
    expect(s.failure).toMatchObject({ kind: 'port_in_use', occupant: { name: 'python3.13', pid: 89984 } })
  })

  it('stop clears a failure', async () => {
    b.setScenario({ gateway: 'crashed' })
    expect(((await b.invoke('gateway_status')) as GatewayStatus).state).toBe('crashed')
    const s = (await b.invoke('gateway_stop')) as GatewayStatus
    expect(s).toMatchObject({ state: 'stopped', failure: null })
  })

  it('models are only available while running', async () => {
    expect((await rejection(b.invoke('gateway_models'))).code).toBe('gateway_not_running')
    await b.invoke('gateway_start')
    await vi.advanceTimersByTimeAsync(1000)
    const models = (await b.invoke('gateway_models')) as { id: string }[]
    expect(models.length).toBeGreaterThan(1)
  })

  it('changing the port while running asks for a restart', async () => {
    await b.invoke('gateway_start')
    await vi.advanceTimersByTimeAsync(1000)
    const r = (await b.invoke('config_update', { patch: { port: 8100 } })) as { restartRequired: boolean }
    expect(r.restartRequired).toBe(true)
  })

  it('config_update validates the port', async () => {
    expect((await rejection(b.invoke('config_update', { patch: { port: 80 } }))).code).toBe('invalid_argument')
  })
})

describe('mock backend: env and kclaude', () => {
  it('env report always has four items in contract order', async () => {
    for (const env of ['ok', 'no_cli', 'no_login', 'expired', 'no_claude'] as const) {
      b.setScenario({ env })
      const r = (await b.invoke('env_check')) as { items: { id: string }[] }
      expect(r.items.map((i) => i.id)).toEqual([
        'kiro_cli_installed',
        'kiro_cli_logged_in',
        'login_valid',
        'claude_installed'
      ])
    }
  })

  it('adopt only works from foreign', async () => {
    expect((await rejection(b.invoke('kclaude_adopt'))).code).toBe('invalid_argument')
    b.setScenario({ kclaude: 'foreign' })
    expect(((await b.invoke('kclaude_install').catch((e) => e)) as { code: string }).code).toBe('foreign_present')
    expect(((await b.invoke('kclaude_adopt')) as { state: string }).state).toBe('installed')
  })

  it('remove then install round-trips', async () => {
    expect(((await b.invoke('kclaude_remove')) as { state: string }).state).toBe('not_installed')
    expect(((await b.invoke('kclaude_install')) as { state: string }).state).toBe('installed')
  })
})

describe('mock backend: request log', () => {
  it('snapshot honors limit and rejects out-of-range limits', async () => {
    const s = (await b.invoke('request_log_snapshot', { limit: 3 })) as { records: unknown[] }
    expect(s.records).toHaveLength(3)
    expect((await rejection(b.invoke('request_log_snapshot', { limit: 0 }))).code).toBe('invalid_argument')
    expect((await rejection(b.invoke('request_log_snapshot', { limit: 1001 }))).code).toBe('invalid_argument')
  })

  it('records are newest first and the model groups sum to the month total', async () => {
    const s = (await b.invoke('request_log_snapshot', {})) as {
      records: { ts: string }[]
      month: { credits: number }
      byModel: { credits: number }[]
    }
    const ts = s.records.map((r) => Date.parse(r.ts))
    expect([...ts].sort((x, y) => y - x)).toEqual(ts)
    const sum = s.byModel.reduce((a, m) => a + m.credits, 0)
    expect(sum).toBeCloseTo(s.month.credits, 6)
  })

  it('clear zeroes everything and the empty scenario starts empty', async () => {
    const s = (await b.invoke('request_log_clear')) as {
      records: unknown[]
      today: { credits: number }
      byModel: unknown[]
    }
    expect(s.records).toEqual([])
    expect(s.today.credits).toBe(0)
    expect(s.byModel).toEqual([])
    b.setScenario({ log: 'some' })
    expect(((await b.invoke('request_log_snapshot', {})) as { records: unknown[] }).records.length).toBeGreaterThan(0)
  })

  it('pushes request-log-appended while the gateway runs', async () => {
    const pushes: RequestLogAppended[] = []
    b.listen('request-log-appended', (p) => pushes.push(p))
    await b.invoke('gateway_start')
    await vi.advanceTimersByTimeAsync(1000)
    await vi.advanceTimersByTimeAsync(6000)
    expect(pushes.length).toBeGreaterThan(0)
    expect(pushes[0].records).toHaveLength(1)
  })

  it('does not push while the gateway is stopped', async () => {
    const pushes: RequestLogAppended[] = []
    b.listen('request-log-appended', (p) => pushes.push(p))
    await vi.advanceTimersByTimeAsync(20_000)
    expect(pushes).toEqual([])
  })
})

describe('mock backend: update', () => {
  it('check now returns none by default and ready when the scenario says so', async () => {
    expect(((await b.invoke('update_check_now')) as UpdateStatus).state).toBe('none')
    b.setScenario({ update: 'ready' })
    expect(((await b.invoke('update_status')) as UpdateStatus).state).toBe('ready')
  })

  it('restart without a pending update is rejected', async () => {
    expect((await rejection(b.invoke('update_restart_now'))).code).toBe('no_pending_update')
  })

  it('retry is only valid for forced or install_failed', async () => {
    expect((await rejection(b.invoke('update_retry'))).code).toBe('invalid_argument')
    b.setScenario({ update: 'forced_failed' })
    const s = (await b.invoke('update_retry')) as UpdateStatus
    expect(s.state).toBe('forced')
    expect(s.state === 'forced' && s.error).toBeNull()
  })

  it('forced scenario sets upgradeRequired in config', async () => {
    b.setScenario({ update: 'forced' })
    expect(((await b.invoke('config_get')) as { upgradeRequired: boolean }).upgradeRequired).toBe(true)
  })
})

describe('mock backend: dispatch', () => {
  it('rejects unknown commands with the internal code', async () => {
    expect((await rejection(b.invoke('nope' as never))).code).toBe('internal')
  })

  it('listen returns an unsubscribe', async () => {
    const seen: GatewayStatus[] = []
    const off = b.listen('gateway-status', (s) => seen.push(s))
    off()
    await b.invoke('gateway_start')
    expect(seen).toEqual([])
  })
})
