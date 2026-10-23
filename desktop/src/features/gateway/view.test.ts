import { describe, expect, it } from 'vitest'
import { failure, gw, LOGIN_CMD } from '@/test-support/fixtures'
import { gatewayView, startErrorView, type GatewayViewInput } from './view'

const SH = 'Asia/Shanghai'
const NOW = new Date('2026-10-22T14:14:30+08:00')
const allowed = { allowed: true, reason: null }

function view(over: Partial<GatewayViewInput> = {}) {
  return gatewayView({
    status: gw(),
    gate: allowed,
    upgradeRequired: false,
    loginCommand: LOGIN_CMD,
    now: NOW,
    timeZone: SH,
    ...over
  })
}

describe('gatewayView: the five states', () => {
  it('stopped: switch off and enabled, hint about the command', () => {
    const v = view()
    expect(v).toMatchObject({
      title: '已停止',
      switchOn: false,
      busy: false,
      disabled: false,
      toggle: 'start',
      pulse: '',
      notice: null
    })
    expect(v.sub).toEqual({ code: null, text: '打开开关后，就能在终端用 cclaude' })
    expect(v.statusBar).toEqual({ text: '已停止', pulse: '' })
  })

  it('stopped: the hint uses the current command name', () => {
    const v = view({ commandName: 'kclaude-test' })
    expect(v.sub).toEqual({ code: null, text: '打开开关后，就能在终端用 kclaude-test' })
    expect(JSON.stringify(v)).not.toContain('cclaude')
  })

  it('starting: switch on and busy, still clickable so the user can cancel', () => {
    const v = view({ status: gw({ state: 'starting', pid: 10, since: '2026-10-22T14:14:00+08:00' }) })
    expect(v).toMatchObject({ title: '启动中', switchOn: true, busy: true, disabled: false, toggle: 'stop' })
    expect(v.sub).toEqual({ code: null, text: '正在等待网关就绪' })
    expect(v.statusBar).toEqual({ text: '启动中', pulse: '' })
  })

  it('running: shows address and uptime, status bar shows the address', () => {
    const v = view({ status: gw({ state: 'running', pid: 10, since: '2026-10-22T12:00:00+08:00', port: 8100 }) })
    expect(v).toMatchObject({ title: '运行中', switchOn: true, busy: false, toggle: 'stop', pulse: 'on' })
    expect(v.sub).toEqual({ code: '127.0.0.1:8100', text: '，已运行 2 小时 14 分' })
    expect(v.statusBar).toEqual({ text: '127.0.0.1:8100', pulse: 'on' })
  })

  it('running without a usable since drops the uptime', () => {
    const v = view({ status: gw({ state: 'running', since: null }) })
    expect(v.sub).toEqual({ code: '127.0.0.1:6868', text: '' })
  })

  it('start_failed (other): failure message, log excerpt, retry and open-log actions', () => {
    const v = view({
      status: gw({
        state: 'start_failed',
        failure: failure({ kind: 'other', message: '网关进程立即退出了', logExcerpt: ['ERROR | boom'] })
      })
    })
    expect(v).toMatchObject({ title: '没能启动', switchOn: false, pulse: 'bad', toggle: 'start' })
    expect(v.sub.text).toBe('网关进程立即退出了')
    expect(v.notice).toMatchObject({ tone: 'bad', logLines: ['ERROR | boom'] })
    expect(v.notice!.actions.map((a) => a.id)).toEqual(['retry', 'open_log'])
    expect(v.notice!.actions[0].primary).toBe(true)
    expect(v.statusBar).toEqual({ text: '没能启动', pulse: 'bad' })
  })

  it('start_failed (timeout): explains the 30 second wait', () => {
    const v = view({
      status: gw({ state: 'start_failed', failure: failure({ kind: 'timeout', message: '30 秒内网关没有就绪' }) })
    })
    expect(v.notice!.segments.map((s) => s.text).join('')).toContain('30 秒')
    expect(v.notice!.actions.map((a) => a.id)).toEqual(['retry', 'open_log'])
  })

  it('crashed: shows the exit time and the last log lines', () => {
    const v = view({
      status: gw({
        state: 'crashed',
        failure: failure({ at: '2026-10-22T14:32:10+08:00', logExcerpt: ['ERROR | a', 'ERROR | b'] })
      })
    })
    expect(v).toMatchObject({ title: '意外退出', switchOn: false, pulse: 'bad' })
    expect(v.sub.text).toBe('网关进程在 14:32 退出了')
    expect(v.notice!.segments.map((s) => s.text).join('')).toBe('网关进程意外退出。最后几行日志：')
    expect(v.notice!.logLines).toEqual(['ERROR | a', 'ERROR | b'])
    expect(v.notice!.actions.map((a) => [a.id, a.label])).toEqual([
      ['restart', '重新启动'],
      ['open_log', '打开完整日志']
    ])
  })

  it('crashed with an empty log skips the "last lines" sentence', () => {
    const v = view({ status: gw({ state: 'crashed', failure: failure({ logExcerpt: [] }) }) })
    expect(v.notice!.segments.map((s) => s.text).join('')).toBe('网关进程意外退出。')
    expect(v.notice!.logLines).toEqual([])
  })

  it('crashed with an unparseable exit time falls back to a plain sentence', () => {
    const v = view({ status: gw({ state: 'crashed', failure: failure({ at: 'garbage' }) }) })
    expect(v.sub.text).toBe('网关进程退出了')
  })
})

describe('gatewayView: port in use', () => {
  const portFail = (occupant: { name: string | null; pid: number | null } | null) =>
    gw({
      state: 'start_failed',
      port: 6868,
      failure: failure({ kind: 'port_in_use', message: '6868 端口被别的程序占用', occupant })
    })

  it('names the occupying process and offers change-port and retry', () => {
    const v = view({ status: portFail({ name: 'python3.13', pid: 89984 }) })
    expect(v.title).toBe('没能启动')
    expect(v.sub.text).toBe('6868 端口被别的程序占用')
    expect(v.notice!.tone).toBe('warn')
    expect(v.notice!.segments).toEqual([
      { text: '占用端口的是 ' },
      { text: 'python3.13', strong: true },
      { text: '（pid 89984），可能是你手动起的网关。可以先关掉它，或者换一个端口。' }
    ])
    expect(v.notice!.actions.map((a) => [a.id, a.label, a.primary])).toEqual([
      ['change_port', '换个端口', true],
      ['retry', '重试', false]
    ])
  })

  it('handles an occupant whose name is unknown', () => {
    const v = view({ status: portFail({ name: null, pid: 77 }) })
    expect(v.notice!.segments.map((s) => s.text).join('')).toBe(
      '占用端口的程序（pid 77）没能识别，可以先关掉它，或者换一个端口。'
    )
  })

  it('handles an occupant that could not be found at all', () => {
    const v = view({ status: portFail({ name: null, pid: null }) })
    expect(v.notice!.segments.map((s) => s.text).join('')).toBe(
      '有别的程序占用了这个端口。可以先关掉它，或者换一个端口。'
    )
  })

  it('handles a missing occupant object', () => {
    const v = view({ status: portFail(null) })
    expect(v.notice!.actions[0].id).toBe('change_port')
  })

  it('never offers to kill the occupying process', () => {
    const v = view({ status: portFail({ name: 'python3.13', pid: 1 }) })
    expect(v.notice!.actions.map((a) => a.id)).not.toContain('kill')
  })
})

describe('gatewayView: login expired', () => {
  const expired = (state: 'start_failed' | 'crashed') =>
    gw({ state, failure: failure({ kind: 'login_expired', message: 'kiro-cli 的登录已失效' }) })

  it('start_failed: shows the failure message and offers copying the login command', () => {
    const v = view({ status: expired('start_failed') })
    expect(v.title).toBe('没能启动')
    expect(v.sub.text).toBe('kiro-cli 的登录已失效')
    expect(v.notice!.command).toBe(LOGIN_CMD)
    expect(v.notice!.actions.map((a) => a.id)).toEqual(['copy_login'])
    expect(v.notice!.actions[0].label).toBe('复制登录命令')
  })

  it('crashed with an expired login uses the login notice too', () => {
    const v = view({ status: expired('crashed') })
    expect(v.title).toBe('意外退出')
    expect(v.notice!.actions.map((a) => a.id)).toEqual(['copy_login'])
  })

  it('without a known kiro-cli path, offers no copy button instead of a broken one', () => {
    const v = view({ status: expired('start_failed'), loginCommand: null })
    expect(v.notice!.command).toBeNull()
    expect(v.notice!.actions.map((a) => a.id)).toEqual([])
  })

  it('refuses a relative login command', () => {
    const v = view({ status: expired('start_failed'), loginCommand: 'kiro-cli login' })
    expect(v.notice!.command).toBeNull()
  })
})

describe('gatewayView: switch availability', () => {
  it('blocked environment disables a stopped switch and shows the reason', () => {
    const v = view({ gate: { allowed: false, reason: '先登录 kiro-cli 才能打开' } })
    expect(v).toMatchObject({ title: '已停止', disabled: true, switchOn: false })
    expect(v.sub).toEqual({ code: null, text: '先登录 kiro-cli 才能打开' })
  })

  it('blocked environment never traps a running gateway: it can still be turned off', () => {
    const v = view({
      status: gw({ state: 'running', since: '2026-10-22T14:00:00+08:00' }),
      gate: { allowed: false, reason: '先登录 kiro-cli 才能打开' }
    })
    expect(v.disabled).toBe(false)
    expect(v.toggle).toBe('stop')
  })

  it('blocked environment disables retry actions after a failure', () => {
    const v = view({
      status: gw({ state: 'crashed', failure: failure({ logExcerpt: [] }) }),
      gate: { allowed: false, reason: '先登录 kiro-cli 才能打开' }
    })
    expect(v.disabled).toBe(true)
    expect(v.notice!.actions.find((a) => a.id === 'restart')!.disabled).toBe(true)
    expect(v.notice!.actions.find((a) => a.id === 'open_log')!.disabled).toBe(false)
  })

  it('forced upgrade disables the switch regardless of environment', () => {
    const v = view({ upgradeRequired: true })
    expect(v.disabled).toBe(true)
    expect(v.sub.text).toBe('需要先升级到新版本才能使用')
  })

  it('forced upgrade takes precedence over the environment reason', () => {
    const v = view({ upgradeRequired: true, gate: { allowed: false, reason: '先登录 kiro-cli 才能打开' } })
    expect(v.sub.text).toBe('需要先升级到新版本才能使用')
  })

  it('shows a neutral disabled switch before the first status arrives', () => {
    const v = view({ status: null })
    expect(v).toMatchObject({ title: '正在读取状态', disabled: true, switchOn: false, notice: null })
  })
})

describe('startErrorView', () => {
  it('env_not_ready re-runs the environment check and shows the backend sentence', () => {
    expect(startErrorView({ code: 'env_not_ready', message: '先登录 kiro-cli' })).toEqual({
      message: '先登录 kiro-cli',
      recheckEnv: true
    })
  })
  it('upgrade_required points at the upgrade', () => {
    expect(startErrorView({ code: 'upgrade_required', message: 'x' })).toEqual({
      message: '需要先升级到新版本才能使用',
      recheckEnv: false
    })
  })
  it('anything else shows the backend sentence without a recheck', () => {
    expect(startErrorView({ code: 'internal', message: '没能启动网关进程' })).toEqual({
      message: '没能启动网关进程',
      recheckEnv: false
    })
  })
})
