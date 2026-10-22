import { readdirSync, readFileSync, statSync } from 'node:fs'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { build } from 'vite'
import react from '@vitejs/plugin-react'

const SRC = path.resolve(__dirname, '..')
const ROOT = path.resolve(SRC, '..')

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = path.join(dir, name)
    if (statSync(p).isDirectory()) return sources(p)
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [p] : []
  })
}

describe('mock backend isolation', () => {
  it('only the lib/ wrappers import the mock, and only dynamically behind the DEV guard', () => {
    const offenders: string[] = []
    for (const file of sources(SRC)) {
      const rel = path.relative(SRC, file)
      if (rel.startsWith('mock/') || rel.startsWith('test-support/')) continue
      const text = readFileSync(file, 'utf-8')
      if (!/mock\/backend/.test(text)) continue
      const staticImport = /^\s*import\s[^(]*from\s+['"]@\/mock\//m.test(text)
      const guarded = /import\.meta\.env\.DEV && import\.meta\.env\.VITE_MOCK_IPC === '1'/.test(text)
      if (staticImport || !guarded || !rel.startsWith('lib/')) offenders.push(rel)
    }
    expect(offenders).toEqual([])
  })

  it('a production bundle (even built with VITE_MOCK_IPC=1) contains none of the mock', async () => {
    const prev = process.env.VITE_MOCK_IPC
    const prevNodeEnv = process.env.NODE_ENV
    process.env.VITE_MOCK_IPC = '1'
    // Vitest runs with NODE_ENV=test; a release build must see production.
    process.env.NODE_ENV = 'production'
    try {
      const result = await build({
        root: ROOT,
        configFile: false,
        mode: 'production',
        logLevel: 'silent',
        plugins: [react()],
        resolve: { alias: { '@': SRC } },
        define: { __APP_VERSION__: JSON.stringify('0.0.0-test') },
        build: { write: false, minify: false, target: 'es2022' }
      })
      const outputs = (Array.isArray(result) ? result : [result]).flatMap((r) =>
        'output' in r ? r.output : []
      )
      const code = outputs
        .map((o) => ('code' in o ? o.code : ''))
        .join('\n')
      expect(code.length).toBeGreaterThan(1000)
      for (const marker of ['__kgwMock', 'createMockBackend', 'Monthly request limit exceeded', 'python3.13']) {
        expect(code).not.toContain(marker)
      }
      expect(outputs.some((o) => o.fileName.includes('backend'))).toBe(false)
    } finally {
      if (prev === undefined) delete process.env.VITE_MOCK_IPC
      else process.env.VITE_MOCK_IPC = prev
      process.env.NODE_ENV = prevNodeEnv
    }
  }, 60_000)
})
