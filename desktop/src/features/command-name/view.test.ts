import { describe, expect, it } from 'vitest'
import {
  COMMAND_NAME_EMPTY,
  COMMAND_NAME_FORMAT,
  DEFAULT_COMMAND_NAME,
  RESERVED_COMMAND_NAMES,
  canSaveCommandName,
  commandNameOutcome,
  isReservedCommandName,
  logsEmptyHint,
  panelTitle,
  parseCommandName,
  removeButtonLabel,
  reservedMessage,
  resolveCommandName,
  stoppedHint
} from './view'

describe('parseCommandName: accepted names', () => {
  it.each(['cclaude', 'kclaude-test', 'ab', 'a_b', 'Ab9', 'x'.repeat(32), 'claude2', 'my-claude', ' cclaude '])(
    'accepts %j',
    (raw) => {
      expect(parseCommandName(raw)).toEqual({ ok: true, name: raw.trim() })
    }
  )

  it('the default name is itself valid', () => {
    expect(DEFAULT_COMMAND_NAME).toBe('cclaude')
    expect(parseCommandName(DEFAULT_COMMAND_NAME)).toEqual({ ok: true, name: 'cclaude' })
  })
})

describe('parseCommandName: format errors', () => {
  it.each(['', '   ', '\t'])('asks for a name when empty: %j', (raw) => {
    expect(parseCommandName(raw)).toEqual({ ok: false, message: COMMAND_NAME_EMPTY })
  })

  it.each([
    ['a', 'too short'],
    ['x'.repeat(33), 'too long'],
    ['9abc', 'starts with a digit'],
    ['-abc', 'starts with a dash'],
    ['_abc', 'starts with an underscore'],
    ['my claude', 'contains a space'],
    ['my\tclaude', 'contains a tab'],
    ['a.b', 'contains a dot'],
    ['a/b', 'contains a slash'],
    ['a;b', 'contains a semicolon'],
    ['a$b', 'contains a dollar sign'],
    ['a`b', 'contains a backtick'],
    ["a'b", 'contains a quote'],
    ['密码', 'non-ASCII letters'],
    ['cclaude\nrm', 'contains a newline']
  ])('rejects %j (%s)', (raw) => {
    expect(parseCommandName(raw)).toEqual({ ok: false, message: COMMAND_NAME_FORMAT })
  })

  it('32 characters is the longest allowed length', () => {
    expect(parseCommandName('a'.repeat(32)).ok).toBe(true)
    expect(parseCommandName('a'.repeat(33)).ok).toBe(false)
  })
})

describe('parseCommandName: reserved names', () => {
  it.each(['claude', 'ls'])('rejects %s with the reason', (raw) => {
    expect(parseCommandName(raw)).toEqual({ ok: false, message: reservedMessage(raw) })
    expect(reservedMessage(raw)).toContain(raw)
  })

  it('rejects every name on the reserved list', () => {
    for (const name of RESERVED_COMMAND_NAMES) {
      const r = parseCommandName(name)
      // single-letter names do not exist; every entry has 2+ characters so it reaches the reserved check
      expect(r, name).toEqual({ ok: false, message: reservedMessage(name) })
    }
  })

  it('covers at least the names the spec lists', () => {
    for (const name of [
      'claude', 'kiro-cli', 'ls', 'cd', 'cat', 'cp', 'mv', 'rm', 'git', 'node', 'npm', 'python',
      'python3', 'sh', 'bash', 'zsh', 'sudo', 'open', 'kill', 'test', 'which', 'type', 'echo'
    ]) {
      expect(RESERVED_COMMAND_NAMES).toContain(name)
    }
  })

  it('is case-insensitive', () => {
    expect(isReservedCommandName('Claude')).toBe(true)
    expect(isReservedCommandName('LS')).toBe(true)
    expect(parseCommandName('CLAUDE').ok).toBe(false)
  })

  it('names that merely contain a reserved word are fine', () => {
    expect(parseCommandName('claude-kiro').ok).toBe(true)
    expect(parseCommandName('lsx').ok).toBe(true)
    expect(isReservedCommandName('cclaude')).toBe(false)
  })

  it('a format error wins over the reserved check', () => {
    expect(parseCommandName('9').ok).toBe(false)
  })
})

describe('resolveCommandName', () => {
  it('prefers config, then status, then the default', () => {
    expect(resolveCommandName({ commandName: 'a-one' }, { commandName: 'b-two' })).toBe('a-one')
    expect(resolveCommandName(null, { commandName: 'b-two' })).toBe('b-two')
    expect(resolveCommandName({ commandName: '' }, { commandName: 'b-two' })).toBe('b-two')
    expect(resolveCommandName(null, null)).toBe('cclaude')
    expect(resolveCommandName({}, {})).toBe('cclaude')
  })
})

describe('sentences use the current name', () => {
  it.each(['cclaude', 'kclaude-test'])('for %s', (name) => {
    expect(removeButtonLabel(name)).toBe(`移除 ${name}`)
    expect(stoppedHint(name)).toBe(`打开开关后，就能在终端用 ${name}`)
    expect(panelTitle(name)).toBe(`${name} 命令`)
    const h = logsEmptyHint(name)
    expect(h.code).toBe(name)
    expect(h.before + h.code + h.after).toBe(`还没有请求。在终端运行 ${name} 试试，请求结束后会出现在这里。`)
  })

  it('no sentence keeps a hard-coded old name', () => {
    const all = [removeButtonLabel('cclaude'), stoppedHint('cclaude'), panelTitle('cclaude')]
    for (const s of all) expect(s).not.toContain('kclaude')
  })
})

describe('commandNameOutcome: save result mapping', () => {
  it('success says the new name and that a new terminal is needed', () => {
    expect(commandNameOutcome({ ok: true, name: 'kclaude-test' })).toEqual({
      tone: 'ok',
      text: '已改为 kclaude-test，新开一个终端即可使用'
    })
  })

  it('failure shows the message Rust returned', () => {
    expect(
      commandNameOutcome({ ok: false, error: { code: 'invalid_argument', message: '这个名字已经被占用' } })
    ).toEqual({ tone: 'bad', text: '这个名字已经被占用' })
  })

  it('a backup failure keeps its own sentence', () => {
    expect(
      commandNameOutcome({ ok: false, error: { code: 'backup_failed', message: '备份 ~/.zshrc 失败，未改动' } })
    ).toEqual({ tone: 'bad', text: '备份 ~/.zshrc 失败，未改动' })
  })

  it('a non-contract rejection falls back to the generic sentence, never raw text', () => {
    const r = commandNameOutcome({ ok: false, error: new Error('http://secret/stack') })
    expect(r?.tone).toBe('bad')
    expect(r?.text).not.toContain('secret')
  })

  it('a skipped call (save already pending) shows nothing', () => {
    expect(commandNameOutcome({ ok: false, error: null })).toBeNull()
  })
})

describe('canSaveCommandName', () => {
  it('needs a valid, changed name and no pending save', () => {
    expect(canSaveCommandName('kclaude-test', 'cclaude', false)).toBe(true)
    expect(canSaveCommandName(' kclaude-test ', 'cclaude', false)).toBe(true)
    expect(canSaveCommandName('cclaude', 'cclaude', false)).toBe(false)
    expect(canSaveCommandName('claude', 'cclaude', false)).toBe(false)
    expect(canSaveCommandName('9abc', 'cclaude', false)).toBe(false)
    expect(canSaveCommandName('', 'cclaude', false)).toBe(false)
    expect(canSaveCommandName('kclaude-test', 'cclaude', true)).toBe(false)
  })
})
