import { describe, expect, it, vi } from 'vitest'
import { GENERIC_ERROR_MESSAGE, errorMessage, hasErrorCode, toIpcError } from './ipc-error'

describe('toIpcError', () => {
  it('passes a contract-shaped rejection through unchanged', () => {
    expect(toIpcError({ code: 'port_in_use_not_a_code', message: 'x' }).code).toBe('internal')
    expect(toIpcError({ code: 'env_not_ready', message: '先登录 kiro-cli' })).toEqual({
      code: 'env_not_ready',
      message: '先登录 kiro-cli'
    })
  })

  it('maps an unknown code to internal but keeps the displayable message', () => {
    expect(toIpcError({ code: 'brand_new', message: '出错了' })).toEqual({ code: 'internal', message: '出错了' })
  })

  it('falls back to a generic message for shapes outside the contract', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    for (const raw of ['boom', 42, null, undefined, new Error('stack trace here'), { code: 7 }, { message: '' }]) {
      expect(toIpcError(raw)).toEqual({ code: 'internal', message: GENERIC_ERROR_MESSAGE })
    }
    warn.mockRestore()
  })

  it('never leaks the raw text of a non-contract rejection', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    expect(toIpcError('http://127.0.0.1:8000 secret').message).toBe(GENERIC_ERROR_MESSAGE)
    warn.mockRestore()
  })

  it('treats an empty message with a valid code as the generic message', () => {
    expect(toIpcError({ code: 'write_failed', message: '' })).toEqual({
      code: 'write_failed',
      message: GENERIC_ERROR_MESSAGE
    })
  })
})

describe('errorMessage / hasErrorCode', () => {
  it('extracts the displayable message', () => {
    expect(errorMessage({ code: 'write_failed', message: '没能写入' })).toBe('没能写入')
  })

  it('matches on the code, not the message', () => {
    expect(hasErrorCode({ code: 'upgrade_required', message: 'x' }, 'upgrade_required')).toBe(true)
    expect(hasErrorCode({ code: 'internal', message: 'upgrade_required' }, 'upgrade_required')).toBe(false)
    expect(hasErrorCode('nope', 'internal')).toBe(true)
  })
})
