import { describe, expect, it } from 'vitest'
import { createRaceGuard } from './race-guard'

function setup() {
  const seen: string[] = []
  const guard = createRaceGuard<string>((v) => seen.push(v))
  return { seen, guard }
}

describe('createRaceGuard', () => {
  it('applies a request result when no event arrived meanwhile', () => {
    const { seen, guard } = setup()
    guard.begin()('result')
    expect(seen).toEqual(['result'])
  })

  it('events are always applied', () => {
    const { seen, guard } = setup()
    guard.onEvent('e1')
    guard.onEvent('e2')
    expect(seen).toEqual(['e1', 'e2'])
  })

  it('drops a slow response that was issued before an event and arrives after it', () => {
    const { seen, guard } = setup()
    const finish = guard.begin() // gateway_status issued
    guard.onEvent('running') // a newer gateway-status event arrives first
    finish('stopped') // the slow, older answer
    expect(seen).toEqual(['running'])
  })

  it('keeps a response that was issued after the last event', () => {
    const { seen, guard } = setup()
    guard.onEvent('starting')
    guard.begin()('running')
    expect(seen).toEqual(['starting', 'running'])
  })

  it('drops the result of gateway_start (starting) when running was already pushed', () => {
    const { seen, guard } = setup()
    const finishStart = guard.begin()
    guard.onEvent('starting')
    guard.onEvent('running')
    finishStart('starting')
    expect(seen).toEqual(['starting', 'running'])
  })

  it('an older request finishing after a newer one does not overwrite it', () => {
    const { seen, guard } = setup()
    const older = guard.begin()
    const newer = guard.begin()
    newer('new')
    older('old')
    expect(seen).toEqual(['new'])
  })

  it('an older request finishing before a newer one is applied, then superseded', () => {
    const { seen, guard } = setup()
    const older = guard.begin()
    const newer = guard.begin()
    older('old')
    newer('new')
    expect(seen).toEqual(['old', 'new'])
  })

  it('an applier can be called only once effectively: a repeated call applies again only if still current', () => {
    const { seen, guard } = setup()
    const finish = guard.begin()
    finish('a')
    guard.onEvent('e')
    finish('b')
    expect(seen).toEqual(['a', 'e'])
  })
})
