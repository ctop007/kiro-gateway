/**
 * In-memory implementation of the whole IPC contract, used when the UI runs
 * outside Tauri (`VITE_MOCK_IPC=1`, e.g. `pnpm dev` in a browser for review and
 * screenshots). It is only reachable through dynamic imports behind the
 * build-time flag, so release bundles do not contain it.
 *
 * Scenarios mirror the mockup's prototype panel. They can be switched at
 * runtime from the console: `__kgwMock.setScenario({ gateway: 'crashed' })`, or
 * with URL query parameters (`?gateway=port_in_use&env=no_login`).
 */
import type {
  AppConfig,
  CheckItem,
  CommandMap,
  CommandName,
  ConfigPatch,
  EnvReport,
  EventMap,
  EventName,
  GatewayFailure,
  GatewayStatus,
  IpcError,
  IpcErrorCode,
  KclaudeStatus,
  ModelTier,
  RequestLogSnapshot,
  RequestRecord,
  UpdateStatus,
  UsageSummary
} from '@/lib/ipc-types'
import { DEFAULT_COMMAND_NAME, parseCommandName } from '@/features/command-name/view'

export interface Scenario {
  gateway: 'stopped' | 'running' | 'crashed' | 'port_in_use' | 'login_expired' | 'timeout'
  env: 'ok' | 'no_cli' | 'no_login' | 'expired' | 'no_claude'
  kclaude: 'installed' | 'not_installed' | 'foreign'
  update: 'none' | 'ready' | 'forced' | 'forced_failed' | 'install_failed'
  log: 'some' | 'empty'
}

export interface MockOptions {
  /** How long `starting` lasts before the scenario outcome. */
  startDelayMs?: number
  scenario?: Partial<Scenario>
}

export interface MockBackend {
  invoke(command: CommandName, args?: unknown): Promise<unknown>
  listen<E extends EventName>(name: E, handler: (payload: EventMap[E]) => void): () => void
  setScenario(patch: Partial<Scenario>): void
}

const DEFAULT_SCENARIO: Scenario = {
  gateway: 'stopped',
  env: 'ok',
  kclaude: 'installed',
  update: 'none',
  log: 'some'
}

const KIRO_CLI_ABS = '/Users/dev/.local/bin/kiro-cli'
const MODELS = ['claude-haiku-4-5', 'claude-opus-5-5[1m]', 'claude-sonnet-4-6', 'claude-sonnet-5-5[1m]']
const APPEND_EVERY_MS = 6000
const MANUAL_URL = 'https://example.invalid/Kiro-Gateway_0.0.3.dmg'

class MockError implements IpcError {
  constructor(
    public code: IpcErrorCode,
    public message: string
  ) {}
}

function iso(d: Date): string {
  const pad = (n: number, w = 2) => String(n).padStart(w, '0')
  const off = -d.getTimezoneOffset()
  const sign = off >= 0 ? '+' : '-'
  const abs = Math.abs(off)
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}` +
    `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`
  )
}

function checkItems(env: Scenario['env'], commandName: string): CheckItem[] {
  const cliOk = env !== 'no_cli'
  const loginOk = cliOk && env !== 'no_login'
  const validOk = loginOk && env !== 'expired'
  const claudeOk = env !== 'no_claude'
  const loginFix: CheckItem['fix'] = { kind: 'copy_command', label: '复制', command: `${KIRO_CLI_ABS} login` }
  return [
    cliOk
      ? { id: 'kiro_cli_installed', status: 'ok', title: 'kiro-cli 已安装', detail: '~/.local/bin/kiro-cli', fix: null }
      : {
          id: 'kiro_cli_installed',
          status: 'fail',
          title: '没找到 kiro-cli',
          detail: '网关要借用 kiro-cli 的登录',
          fix: { kind: 'open_url', label: '打开安装页面', url: 'https://kiro.dev/downloads/' }
        },
    !cliOk
      ? { id: 'kiro_cli_logged_in', status: 'skipped', title: 'kiro-cli 登录', detail: '装好 kiro-cli 后再检测', fix: null }
      : loginOk
        ? { id: 'kiro_cli_logged_in', status: 'ok', title: 'kiro-cli 已登录', detail: 'AWS SSO 登录', fix: null }
        : {
            id: 'kiro_cli_logged_in',
            status: 'fail',
            title: 'kiro-cli 还没登录',
            detail: '在终端运行这条命令，浏览器里完成登录后点「重新检测」',
            fix: loginFix
          },
    !loginOk
      ? { id: 'login_valid', status: 'skipped', title: '登录有效', detail: '登录后再检测', fix: null }
      : validOk
        ? { id: 'login_valid', status: 'ok', title: '登录有效', detail: '到期后会自动续期', fix: null }
        : { id: 'login_valid', status: 'fail', title: '登录已失效', detail: '重新登录一次就好', fix: loginFix },
    claudeOk
      ? { id: 'claude_installed', status: 'ok', title: 'Claude Code 已安装', detail: '~/.local/bin/claude', fix: null }
      : {
          id: 'claude_installed',
          status: 'fail',
          title: '没找到 Claude Code',
          detail: `网关照样能开，但 ${commandName} 要等装好 Claude Code 才能用`,
          fix: { kind: 'open_url', label: '查看安装方法', url: 'https://docs.claude.com/en/docs/claude-code/setup' }
        }
  ]
}

function seedRecords(now: Date): RequestRecord[] {
  const at = (minutesAgo: number) => iso(new Date(now.getTime() - minutesAgo * 60_000))
  const r = (
    i: number,
    minutesAgo: number,
    model: string,
    outcome: RequestRecord['outcome'],
    status: number,
    durationMs: number,
    credits: number,
    error: string | null,
    api: RequestRecord['api'] = 'anthropic'
  ): RequestRecord => ({
    ts: at(minutesAgo),
    id: `mock-${i}`,
    api,
    model,
    stream: true,
    status,
    outcome,
    durationMs,
    credits,
    error
  })
  return [
    r(8, 1, 'claude-opus-5.5', 'ok', 200, 12_400, 0.3112, null),
    r(7, 2, 'claude-opus-5.5', 'ok', 200, 3_100, 0.0874, null),
    r(6, 4, 'claude-haiku-4.5', 'ok', 200, 900, 0.0034, null),
    r(5, 6, 'claude-opus-5.5', 'client_disconnected', 200, 41_000, 0.2201, null),
    r(4, 20, 'claude-opus-5.5', 'refused', 200, 2_200, 0.041, '模型拒绝回答（CONTENT_FILTERED）。'),
    r(3, 40, 'claude-sonnet-5.5', 'ok', 200, 5_800, 0.0921, null, 'openai'),
    r(2, 60, 'claude-opus-5.5', 'error', 402, 400, 0, 'HTTP 402：Monthly request limit exceeded'),
    r(1, 70, 'claude-opus-5.5', 'error', 400, 1_200, 0.012, 'HTTP 400：prompt is too long')
  ]
}

/** Names the mock pretends are already on PATH, to exercise the "already taken" message. */
const TAKEN_NAMES = ['rg', 'fd', 'make']

export function createMockBackend(options: MockOptions = {}): MockBackend {
  const startDelayMs = options.startDelayMs ?? 1400
  let scenario: Scenario = { ...DEFAULT_SCENARIO, ...options.scenario }
  const listeners = new Map<EventName, Set<(p: never) => void>>()

  let port = 6868
  let commandName = DEFAULT_COMMAND_NAME
  const models: Record<ModelTier, string> = {
    opus: 'claude-opus-5-5[1m]',
    sonnet: 'claude-sonnet-5-5[1m]',
    haiku: 'claude-haiku-4-5'
  }
  let autostart = false
  let gatewayWanted = false

  let gateway: GatewayStatus = { state: 'stopped', port, pid: null, since: null, failure: null }
  let startTimer: ReturnType<typeof setTimeout> | null = null
  let appendTimer: ReturnType<typeof setInterval> | null = null

  let kclaudeState: KclaudeStatus['state'] = 'installed'
  let records: RequestRecord[] = []
  let usage: UsageSummary = {
    today: { credits: 0, requests: 0 },
    month: { credits: 0, requests: 0, month: '' },
    byModel: []
  }
  let update: UpdateStatus = { state: 'none', currentVersion: '0.0.1', lastCheckedAt: null }
  let nextId = 100

  const emit = <E extends EventName>(name: E, payload: EventMap[E]) => {
    for (const h of listeners.get(name) ?? []) (h as (p: EventMap[E]) => void)(payload)
  }

  const setGateway = (next: GatewayStatus) => {
    gateway = next
    emit('gateway-status', gateway)
  }

  const failureFor = (kind: Scenario['gateway'] | 'other'): GatewayFailure => {
    const at = iso(new Date())
    switch (kind) {
      case 'port_in_use':
        return {
          kind: 'port_in_use',
          message: `${port} 端口被别的程序占用`,
          occupant: { name: 'python3.13', pid: 89984 },
          logExcerpt: [],
          at
        }
      case 'login_expired':
        return { kind: 'login_expired', message: 'kiro-cli 的登录已失效', occupant: null, logExcerpt: [], at }
      case 'timeout':
        return {
          kind: 'timeout',
          message: '30 秒内网关没有就绪',
          occupant: null,
          logExcerpt: ['INFO | main:lifespan - Loading model cache', 'INFO | kiro.auth:refresh - Refreshing token'],
          at
        }
      default:
        return {
          kind: 'other',
          message: '网关进程退出了',
          occupant: null,
          logExcerpt: [
            'ERROR | kiro.auth:refresh - Token refresh failed: Connection reset',
            'ERROR | main:lifespan - Startup aborted'
          ],
          at
        }
    }
  }

  const stopAppending = () => {
    if (appendTimer) clearInterval(appendTimer)
    appendTimer = null
  }

  const startAppending = () => {
    stopAppending()
    appendTimer = setInterval(() => {
      const now = new Date()
      const credits = 0.05 + (nextId % 7) * 0.02
      const rec: RequestRecord = {
        ts: iso(now),
        id: `mock-${nextId++}`,
        api: 'anthropic',
        model: 'claude-sonnet-5.5',
        stream: true,
        status: 200,
        outcome: 'ok',
        durationMs: 2_000 + (nextId % 5) * 700,
        credits,
        error: null
      }
      records = [rec, ...records]
      usage = {
        today: { credits: usage.today.credits + credits, requests: usage.today.requests + 1 },
        month: { ...usage.month, credits: usage.month.credits + credits, requests: usage.month.requests + 1 },
        byModel: usage.byModel.map((m) =>
          m.model === rec.model ? { ...m, credits: m.credits + credits, requests: m.requests + 1 } : m
        )
      }
      emit('request-log-appended', { ...usage, records: [rec], reset: false })
    }, APPEND_EVERY_MS)
  }

  const applyUpdateScenario = () => {
    const base = { currentVersion: '0.0.1', lastCheckedAt: null }
    switch (scenario.update) {
      case 'ready':
        update = { state: 'ready', version: '0.0.2', ...base }
        break
      case 'forced':
        update = {
          state: 'forced',
          version: '0.0.3',
          progress: { downloadedBytes: 17.4 * 1024 * 1024, totalBytes: 28.1 * 1024 * 1024 },
          error: null,
          manualUrl: MANUAL_URL,
          ...base
        }
        break
      case 'forced_failed':
        update = {
          state: 'forced',
          version: '0.0.3',
          progress: null,
          error: '网络连接超时',
          manualUrl: MANUAL_URL,
          ...base
        }
        break
      case 'install_failed':
        update = { state: 'install_failed', version: '0.0.2', error: '磁盘空间不足', manualUrl: MANUAL_URL, ...base }
        break
      default:
        update = { state: 'none', ...base }
    }
  }

  const applyLogScenario = () => {
    const now = new Date()
    if (scenario.log === 'empty') {
      records = []
      usage = {
        today: { credits: 0, requests: 0 },
        month: { credits: 0, requests: 0, month: `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}` },
        byModel: []
      }
      return
    }
    records = seedRecords(now)
    usage = {
      today: { credits: 1.84, requests: 32 },
      month: {
        credits: 46.2,
        requests: 611,
        month: `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`
      },
      byModel: [
        { model: 'claude-opus-5.5', credits: 36.02, requests: 400 },
        { model: 'claude-sonnet-5.5', credits: 8.77, requests: 150 },
        { model: 'claude-haiku-4.5', credits: 1.41, requests: 61 }
      ]
    }
  }

  const applyGatewayScenario = () => {
    if (startTimer) clearTimeout(startTimer)
    startTimer = null
    stopAppending()
    const now = new Date()
    switch (scenario.gateway) {
      case 'running':
        gateway = { state: 'running', port, pid: 4242, since: iso(new Date(now.getTime() - 134 * 60_000)), failure: null }
        startAppending()
        break
      case 'crashed':
        gateway = { state: 'crashed', port, pid: null, since: iso(now), failure: failureFor('other') }
        break
      default:
        gateway = { state: 'stopped', port, pid: null, since: null, failure: null }
    }
    emit('gateway-status', gateway)
  }

  const applyKclaudeScenario = () => {
    kclaudeState = scenario.kclaude
  }

  const applyAll = () => {
    applyUpdateScenario()
    applyLogScenario()
    applyKclaudeScenario()
    applyGatewayScenario()
    emit('update-status', update)
  }

  const kclaude = (): KclaudeStatus => ({
    commandName,
    state: kclaudeState,
    shell: 'zsh',
    rcFiles: ['~/.zshrc'],
    claudeInstalled: scenario.env !== 'no_claude',
    verify:
      kclaudeState === 'installed'
        ? { status: 'ok', detail: '', fix: null }
        : { status: 'skipped', detail: '', fix: null }
  })

  const envReport = (): EnvReport => {
    const items = checkItems(scenario.env, commandName)
    const cliOk = scenario.env !== 'no_cli'
    return {
      checkedAt: iso(new Date()),
      items,
      kiroCliPath: cliOk ? '~/.local/bin/kiro-cli' : null,
      claudePath: scenario.env === 'no_claude' ? null : '~/.local/bin/claude',
      loginExpiresAt: null,
      loginCommand: cliOk ? `${KIRO_CLI_ABS} login` : null
    }
  }

  const envReady = () => ['ok', 'no_claude'].includes(scenario.env)

  const config = (): AppConfig => ({
    port,
    commandName,
    models: { ...models },
    autostart,
    gatewayWanted,
    upgradeRequired: update.state === 'forced'
  })

  const snapshot = (limit: number): RequestLogSnapshot => ({
    ...usage,
    records: records.slice(0, limit),
    generatedAt: iso(new Date())
  })

  const handlers: { [C in CommandName]: (args: CommandMap[C]['args']) => CommandMap[C]['result'] } = {
    config_get: () => config(),
    config_update: ({ patch }: { patch: ConfigPatch }) => {
      if (patch.port !== undefined && (!Number.isInteger(patch.port) || patch.port < 1024 || patch.port > 65535)) {
        throw new MockError('invalid_argument', '端口要在 1024 到 65535 之间')
      }
      let nextName: string | null = null
      if (patch.commandName !== undefined) {
        const parsed = parseCommandName(patch.commandName)
        if (!parsed.ok) throw new MockError('invalid_argument', parsed.message)
        if (parsed.name !== commandName && TAKEN_NAMES.includes(parsed.name)) {
          throw new MockError('invalid_argument', '这个名字已经被占用')
        }
        nextName = parsed.name
      }
      for (const [tier, id] of Object.entries(patch.models ?? {})) {
        if (!id || !id.trim()) throw new MockError('invalid_argument', `${tier} 的模型不能为空`)
      }
      const live = gateway.state === 'running' || gateway.state === 'starting'
      const portChanged = patch.port !== undefined && patch.port !== port
      if (patch.port !== undefined) port = patch.port
      if (nextName !== null) commandName = nextName
      Object.assign(models, patch.models ?? {})
      if (patch.autostart !== undefined) autostart = patch.autostart
      if (portChanged && !live) setGateway({ ...gateway, port })
      return { config: config(), restartRequired: portChanged && live }
    },

    gateway_status: () => gateway,
    gateway_start: () => {
      if (update.state === 'forced') throw new MockError('upgrade_required', '需要先升级到新版本')
      if (!envReady()) throw new MockError('env_not_ready', '先完成 kiro-cli 的安装和登录')
      if (gateway.state === 'starting' || gateway.state === 'running') return gateway
      gatewayWanted = true
      setGateway({ state: 'starting', port, pid: 4242, since: iso(new Date()), failure: null })
      startTimer = setTimeout(() => {
        startTimer = null
        if (scenario.gateway === 'port_in_use' || scenario.gateway === 'login_expired' || scenario.gateway === 'timeout') {
          setGateway({
            state: 'start_failed',
            port,
            pid: null,
            since: iso(new Date()),
            failure: failureFor(scenario.gateway)
          })
          return
        }
        setGateway({ state: 'running', port, pid: 4242, since: iso(new Date()), failure: null })
        startAppending()
      }, startDelayMs)
      return gateway
    },
    gateway_stop: () => {
      if (startTimer) clearTimeout(startTimer)
      startTimer = null
      stopAppending()
      gatewayWanted = false
      setGateway({ state: 'stopped', port, pid: null, since: null, failure: null })
      return gateway
    },
    gateway_models: () => {
      if (gateway.state !== 'running') throw new MockError('gateway_not_running', '网关还没有运行')
      return MODELS.map((id) => ({ id }))
    },
    gateway_reveal_log: () => null,

    env_check: () => envReport(),

    kclaude_status: () => kclaude(),
    kclaude_install: () => {
      if (kclaudeState === 'foreign') {
        throw new MockError('foreign_present', `~/.zshrc 里已有不是本 App 写入的 ${commandName}`)
      }
      kclaudeState = 'installed'
      return kclaude()
    },
    kclaude_adopt: () => {
      if (kclaudeState !== 'foreign') throw new MockError('invalid_argument', `当前没有需要接管的 ${commandName}`)
      kclaudeState = 'installed'
      return kclaude()
    },
    kclaude_remove: () => {
      kclaudeState = 'not_installed'
      return kclaude()
    },

    request_log_snapshot: ({ limit }: { limit?: number }) => {
      const n = limit ?? 200
      if (!Number.isInteger(n) || n < 1 || n > 1000) throw new MockError('invalid_argument', 'limit 要在 1 到 1000 之间')
      return snapshot(n)
    },
    request_log_clear: () => {
      records = []
      usage = {
        today: { credits: 0, requests: 0 },
        month: { credits: 0, requests: 0, month: usage.month.month },
        byModel: []
      }
      return snapshot(200)
    },

    update_status: () => update,
    update_check_now: () => {
      update = { ...update, lastCheckedAt: iso(new Date()) } as UpdateStatus
      return update
    },
    update_restart_now: () => {
      if (update.state !== 'ready') throw new MockError('no_pending_update', '还没有已下载的更新')
      return null
    },
    update_retry: () => {
      if (update.state !== 'forced' && update.state !== 'install_failed') {
        throw new MockError('invalid_argument', '当前没有需要重试的更新')
      }
      update =
        update.state === 'forced'
          ? { ...update, error: null, progress: { downloadedBytes: 1024 * 1024, totalBytes: 28.1 * 1024 * 1024 } }
          : { state: 'none', currentVersion: update.currentVersion, lastCheckedAt: update.lastCheckedAt }
      emit('update-status', update)
      return update
    }
  }

  applyAll()

  return {
    async invoke(command, args) {
      const handler = handlers[command] as ((a: unknown) => unknown) | undefined
      if (!handler) throw new MockError('internal', `未知命令 ${String(command)}`)
      return handler(args)
    },
    listen(name, handler) {
      const set = listeners.get(name) ?? new Set()
      set.add(handler as (p: never) => void)
      listeners.set(name, set)
      return () => set.delete(handler as (p: never) => void)
    },
    setScenario(patch) {
      scenario = { ...scenario, ...patch }
      applyAll()
    }
  }
}

// ---------- Singleton used by tauri-core / plugin wrappers ----------

function scenarioFromQuery(): Partial<Scenario> {
  if (typeof location === 'undefined') return {}
  const q = new URLSearchParams(location.search)
  const out: Record<string, string> = {}
  for (const key of ['gateway', 'env', 'kclaude', 'update', 'log']) {
    const v = q.get(key)
    if (v) out[key] = v
  }
  return out as Partial<Scenario>
}

let instance: MockBackend | null = null

function backend(): MockBackend {
  if (!instance) {
    instance = createMockBackend({ scenario: scenarioFromQuery() })
    if (typeof window !== 'undefined') (window as unknown as { __kgwMock: MockBackend }).__kgwMock = instance
  }
  return instance
}

export const mockInvoke = (command: CommandName, args?: unknown): Promise<unknown> => backend().invoke(command, args)
export const mockListen = <E extends EventName>(name: E, handler: (p: EventMap[E]) => void): (() => void) =>
  backend().listen(name, handler)
export const mockOpenUrl = (url: string): void => {
  console.info('[mock] open url', url)
}
export const mockWriteText = (text: string): boolean => {
  console.info('[mock] copied', text)
  return true
}
