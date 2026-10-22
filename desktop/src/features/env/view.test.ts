import { describe, expect, it } from 'vitest'
import { item, okEnv, withItem, KIRO_CLI, LOGIN_CMD } from '@/test-support/fixtures'
import { checkRows, isAbsoluteCommand, loginCommandFor, switchGate } from './view'

const loginFix = { kind: 'copy_command' as const, label: '复制', command: LOGIN_CMD }

describe('switchGate', () => {
  it('allows the switch when all kiro-cli checks pass', () => {
    expect(switchGate(okEnv())).toEqual({ allowed: true, reason: null })
  })

  it('does not depend on Claude Code being installed', () => {
    const r = withItem(okEnv(), item({ id: 'claude_installed', status: 'fail' }))
    expect(switchGate(r).allowed).toBe(true)
  })

  it('blocks and asks to install kiro-cli when it is missing', () => {
    let r = withItem(okEnv(), item({ id: 'kiro_cli_installed', status: 'fail' }))
    r = withItem(r, item({ id: 'kiro_cli_logged_in', status: 'skipped' }))
    r = withItem(r, item({ id: 'login_valid', status: 'skipped' }))
    expect(switchGate(r)).toEqual({ allowed: false, reason: '先安装 kiro-cli 才能打开' })
  })

  it('blocks and asks to log in when kiro-cli is installed but logged out', () => {
    let r = withItem(okEnv(), item({ id: 'kiro_cli_logged_in', status: 'fail' }))
    r = withItem(r, item({ id: 'login_valid', status: 'skipped' }))
    expect(switchGate(r)).toEqual({ allowed: false, reason: '先登录 kiro-cli 才能打开' })
  })

  it('blocks and asks to log in again when the login is no longer valid', () => {
    const r = withItem(okEnv(), item({ id: 'login_valid', status: 'fail' }))
    expect(switchGate(r)).toEqual({ allowed: false, reason: '先重新登录 kiro-cli 才能打开' })
  })

  it('reports the first blocking check when several fail', () => {
    let r = withItem(okEnv(), item({ id: 'kiro_cli_logged_in', status: 'fail' }))
    r = withItem(r, item({ id: 'login_valid', status: 'fail' }))
    expect(switchGate(r).reason).toBe('先登录 kiro-cli 才能打开')
  })

  it('treats a skipped kiro-cli check as blocking', () => {
    const r = withItem(okEnv(), item({ id: 'login_valid', status: 'skipped' }))
    expect(switchGate(r).allowed).toBe(false)
  })

  it('treats a missing kiro-cli item as blocking', () => {
    const r = okEnv()
    r.items = r.items.filter((i) => i.id !== 'login_valid')
    expect(switchGate(r).allowed).toBe(false)
  })

  it('blocks with a waiting reason before the first report arrives', () => {
    expect(switchGate(null)).toEqual({ allowed: false, reason: '正在检测运行环境' })
  })
})

describe('isAbsoluteCommand', () => {
  it.each([
    ['/Users/x/.local/bin/kiro-cli login', true],
    ['"/Users/x y/bin/kiro-cli" login', true],
    ['C:\\Users\\x\\kiro-cli.exe login', true],
    ['"C:\\Users\\x y\\kiro-cli.exe" login', true],
    ["'/Users/John Smith/.local/bin/kiro-cli' login", true],
    ["'/Users/O'\\''Brien/.local/bin/kiro-cli' login", true],
    ["  '/Users/John Smith/.local/bin/kiro-cli' login", true],
    ["'~/.local/bin/kiro-cli' login", false],
    ["'kiro-cli' login", false],
    ["'", false],
    ['\\\\server\\share\\kiro-cli.exe login', true],
    ['kiro-cli login', false],
    ['~/.local/bin/kiro-cli login', false],
    ['./kiro-cli login', false],
    ['', false],
    ['   ', false]
  ])('%s -> %s', (cmd, expected) => {
    expect(isAbsoluteCommand(cmd)).toBe(expected)
  })
})

describe('checkRows', () => {
  it('maps statuses to tones in contract order', () => {
    let r = withItem(okEnv(), item({ id: 'kiro_cli_logged_in', status: 'fail', title: 'kiro-cli 还没登录', fix: loginFix }))
    r = withItem(r, item({ id: 'login_valid', status: 'skipped' }))
    const rows = checkRows(r)
    expect(rows.map((x) => [x.id, x.tone])).toEqual([
      ['kiro_cli_installed', 'ok'],
      ['kiro_cli_logged_in', 'bad'],
      ['login_valid', 'skip'],
      ['claude_installed', 'ok']
    ])
  })

  it('sorts items into the fixed order even if the backend shuffles them', () => {
    const r = okEnv()
    r.items = [...r.items].reverse()
    expect(checkRows(r).map((x) => x.id)).toEqual([
      'kiro_cli_installed',
      'kiro_cli_logged_in',
      'login_valid',
      'claude_installed'
    ])
  })

  it('keeps the login copy command and its absolute path on the failing row', () => {
    const r = withItem(okEnv(), item({ id: 'kiro_cli_logged_in', status: 'fail', fix: loginFix }))
    const row = checkRows(r).find((x) => x.id === 'kiro_cli_logged_in')!
    expect(row.fix).toEqual({ kind: 'copy_command', label: '复制', command: LOGIN_CMD })
    expect(row.fix && row.fix.kind === 'copy_command' && row.fix.command.startsWith(KIRO_CLI)).toBe(true)
  })

  it('drops a copy command whose executable is not an absolute path', () => {
    const bad = { kind: 'copy_command' as const, label: '复制', command: '~/.local/bin/kiro-cli login' }
    const r = withItem(okEnv(), item({ id: 'kiro_cli_logged_in', status: 'fail', fix: bad }))
    expect(checkRows(r).find((x) => x.id === 'kiro_cli_logged_in')!.fix).toBeNull()
  })

  it('keeps https links and drops other schemes', () => {
    const ok = { kind: 'open_url' as const, label: '查看安装方法', url: 'https://docs.claude.com/claude-code' }
    const r1 = withItem(okEnv(), item({ id: 'claude_installed', status: 'fail', fix: ok }))
    expect(checkRows(r1).find((x) => x.id === 'claude_installed')!.fix).toEqual(ok)
    for (const url of ['http://example.com', 'javascript:alert(1)', 'file:///etc/passwd', 'not a url']) {
      const r2 = withItem(okEnv(), item({ id: 'claude_installed', status: 'fail', fix: { ...ok, url } }))
      expect(checkRows(r2).find((x) => x.id === 'claude_installed')!.fix).toBeNull()
    }
  })

  it('shows no fix on passing or skipped rows even if the backend sent one', () => {
    const r = withItem(okEnv(), item({ id: 'kiro_cli_installed', status: 'ok', fix: loginFix }))
    expect(checkRows(r).find((x) => x.id === 'kiro_cli_installed')!.fix).toBeNull()
  })

  it('returns no rows before the first report', () => {
    expect(checkRows(null)).toEqual([])
  })
})

describe('single-quoted login commands (paths with spaces on macOS)', () => {
  const quoted = "'/Users/John Smith/.local/bin/kiro-cli' login"
  const copyFix = { kind: 'copy_command' as const, label: '复制', command: quoted }

  it('keeps the copy button on the failing row', () => {
    const r = withItem(okEnv({ loginCommand: quoted }), item({ id: 'kiro_cli_logged_in', status: 'fail', fix: copyFix }))
    expect(checkRows(r).find((x) => x.id === 'kiro_cli_logged_in')!.fix).toEqual(copyFix)
  })

  it('loginCommandFor returns the report command and the fallback from a failing item', () => {
    expect(loginCommandFor(okEnv({ loginCommand: quoted }))).toBe(quoted)
    const r = withItem(okEnv({ loginCommand: null }), item({ id: 'login_valid', status: 'fail', fix: copyFix }))
    expect(loginCommandFor(r)).toBe(quoted)
  })

  it('keeps a path that contains an apostrophe', () => {
    const cmd = "'/Users/O'\\''Brien/.local/bin/kiro-cli' login"
    expect(loginCommandFor(okEnv({ loginCommand: cmd }))).toBe(cmd)
  })
})

describe('loginCommandFor', () => {
  it('prefers the report-level login command', () => {
    expect(loginCommandFor(okEnv())).toBe(LOGIN_CMD)
  })

  it('falls back to a copy command on a failing item', () => {
    const r = withItem(
      okEnv({ loginCommand: null }),
      item({ id: 'login_valid', status: 'fail', fix: loginFix })
    )
    expect(loginCommandFor(r)).toBe(LOGIN_CMD)
  })

  it('refuses a non-absolute command everywhere', () => {
    const r = okEnv({ loginCommand: 'kiro-cli login' })
    expect(loginCommandFor(r)).toBeNull()
    expect(loginCommandFor(null)).toBeNull()
  })
})
