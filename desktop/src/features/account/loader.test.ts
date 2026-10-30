import { describe, expect, it } from 'vitest'
import { createAccountLoader } from './loader'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}

describe('account session ownership', () => {
  it('discards old-session success without clearing the new pending request', async () => {
    const loader = createAccountLoader<string>()
    const old = deferred<string>(), fresh = deferred<string>()
    loader.session('one')
    const first = loader.run(() => old.promise)
    loader.session('two')
    const second = loader.run(() => fresh.promise)
    old.resolve('wrong account')
    expect(await first).toBeNull()
    expect(await loader.run(async () => 'duplicate')).toBeNull()
    fresh.resolve('current account')
    expect(await second).toEqual({ ok: true, value: 'current account' })
  })

  it('ignores stopped sessions and rejects duplicate refreshes', async () => {
    const loader = createAccountLoader<string>()
    expect(await loader.run(async () => 'unused')).toBeNull()
    loader.session('one')
    const request = deferred<string>()
    const pending = loader.run(() => request.promise)
    expect(await loader.run(async () => 'duplicate')).toBeNull()
    loader.session(null)
    request.resolve('stopped account')
    expect(await pending).toBeNull()
  })

  it('returns current failures and allows retry', async () => {
    const loader = createAccountLoader<string>()
    loader.session('one')
    expect(await loader.run(async () => { throw new Error('failure') })).toMatchObject({ ok: false })
    expect(await loader.run(async () => 'retry')).toEqual({ ok: true, value: 'retry' })
  })
})
