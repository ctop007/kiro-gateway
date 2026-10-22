/**
 * Wires the IPC wrappers to React state. No business decisions live here: it
 * loads data, subscribes to events and exposes handlers; what to show is
 * decided by the pure functions in `src/features/*`.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  AppConfig,
  ConfigPatch,
  EnvReport,
  GatewayStatus,
  KclaudeStatus,
  RequestLogSnapshot,
  UpdateStatus
} from '@/lib/ipc-types'
import { toIpcError } from '@/lib/ipc-error'
import { configGet, configUpdate } from '@/lib/tauri-config'
import {
  gatewayModels,
  gatewayRevealLog,
  gatewayStart,
  gatewayStatus,
  gatewayStop,
  onGatewayStatus
} from '@/lib/tauri-gateway'
import { envCheck } from '@/lib/tauri-env'
import { kclaudeAdopt, kclaudeInstall, kclaudeRemove, kclaudeStatus } from '@/lib/tauri-kclaude'
import { onRequestLogAppended, requestLogClear, requestLogSnapshot } from '@/lib/tauri-request-log'
import { onUpdateStatus, updateCheckNow, updateRestartNow, updateRetry, updateStatus } from '@/lib/tauri-update'
import { writeClipboardText } from '@/lib/tauri-clipboard'
import { appendedAction, indexSnapshot, mergeAppended } from '@/features/request-log/view'
import { retryCommandFor } from '@/features/update/view'
import { startErrorView } from '@/features/gateway/view'
import { useAsyncAction } from './useAsyncAction'
import { createRaceGuard } from './race-guard'

export const LOG_LIMIT = 200
const CLOCK_TICK_MS = 30_000

/** Re-render periodically so uptime and relative times stay fresh. */
function useNow(): Date {
  const [now, setNow] = useState(() => new Date())
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), CLOCK_TICK_MS)
    return () => clearInterval(id)
  }, [])
  return now
}

/** Subscribe to an event for the lifetime of the component. */
function useEvent<T>(subscribe: (h: (p: T) => void) => Promise<() => void>, handler: (p: T) => void) {
  const ref = useRef(handler)
  ref.current = handler
  useEffect(() => {
    let off: (() => void) | null = null
    let cancelled = false
    void subscribe((p) => ref.current(p)).then((fn) => {
      if (cancelled) fn()
      else off = fn
    })
    return () => {
      cancelled = true
      off?.()
    }
  }, [subscribe])
}

export function useAppState() {
  const now = useNow()
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [gateway, setGateway] = useState<GatewayStatus | null>(null)
  const [env, setEnv] = useState<EnvReport | null>(null)
  const [kclaude, setKclaude] = useState<KclaudeStatus | null>(null)
  const [update, setUpdate] = useState<UpdateStatus | null>(null)
  const [log, setLog] = useState<RequestLogSnapshot | null>(null)
  const [models, setModels] = useState<string[] | null>(null)
  const [restartNote, setRestartNote] = useState<string | null>(null)
  const [copied, setCopied] = useState<string | null>(null)

  // Gateway status and config arrive as command results and as pushed state. A pushed
  // value is newer than any request issued before it, so a slow answer must not overwrite it.
  const gatewayGuard = useRef(createRaceGuard<GatewayStatus>((s) => setGateway(s))).current
  const configGuard = useRef(createRaceGuard<AppConfig>((c) => setConfig(c))).current

  const toggle = useAsyncAction()
  const gatewayAction = useAsyncAction()
  const envAction = useAsyncAction()
  const kclaudeAction = useAsyncAction()
  const logAction = useAsyncAction()
  const configAction = useAsyncAction()
  const updateAction = useAsyncAction()

  // ----- initial loads -----
  const refreshEnv = envAction.run
  const loadEnv = useCallback(() => refreshEnv(() => envCheck().then((r) => (setEnv(r), r))), [refreshEnv])
  const refreshLog = useCallback(() => {
    void requestLogSnapshot(LOG_LIMIT).then((snap) => setLog(indexSnapshot(snap)), (e) => console.warn('request_log_snapshot failed', toIpcError(e)))
  }, [])

  /** Re-read config from Rust; the forced-upgrade flag is only ever taken from there. */
  const refreshConfig = useCallback(() => {
    void configGet().then(configGuard.begin(), (e) => console.warn('config_get failed', toIpcError(e)))
  }, [configGuard])
  const refreshGatewayStatus = useCallback(
    (warn: boolean) => {
      void gatewayStatus().then(gatewayGuard.begin(), (e) => warn && console.warn('gateway_status failed', toIpcError(e)))
    },
    [gatewayGuard]
  )

  useEffect(() => {
    refreshConfig()
    refreshGatewayStatus(true)
    void kclaudeStatus().then(setKclaude, (e) => console.warn('kclaude_status failed', toIpcError(e)))
    void updateStatus().then(setUpdate, (e) => console.warn('update_status failed', toIpcError(e)))
    void loadEnv()
    refreshLog()
  }, [loadEnv, refreshLog, refreshConfig, refreshGatewayStatus])

  // Events override whatever the initial calls returned.
  useEvent(onGatewayStatus, gatewayGuard.onEvent)
  useEvent(onUpdateStatus, (u) => {
    setUpdate(u)
    // `upgradeRequired` is persisted by Rust and can outlive the `forced` state (for example
    // when the first check after launch failed). Only Rust clears it, so read it back.
    refreshConfig()
  })
  useEvent(onRequestLogAppended, (p) => {
    // A month file was deleted or rewritten: the held list may show records that are gone.
    if (appendedAction(p) === 'reload') {
      refreshLog()
      return
    }
    setLog((cur) =>
      cur ? mergeAppended(cur, p, LOG_LIMIT) : indexSnapshot({ ...p, generatedAt: new Date().toISOString() })
    )
  })

  // A hidden window suspends the webview and may miss pushes: reload on return.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === 'visible') {
        refreshLog()
        refreshGatewayStatus(false)
      }
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => document.removeEventListener('visibilitychange', onVisible)
  }, [refreshLog, refreshGatewayStatus])

  // The model list exists only while the gateway runs.
  const gatewayState = gateway?.state
  useEffect(() => {
    if (gatewayState !== 'running') {
      setModels(null)
      return
    }
    let cancelled = false
    void gatewayModels().then(
      (list) => !cancelled && setModels(list.map((m) => m.id)),
      (e) => console.warn('gateway_models failed', toIpcError(e))
    )
    return () => {
      cancelled = true
    }
  }, [gatewayState])

  // ----- handlers -----
  const startGateway = useCallback(async () => {
    const r = await toggle.run(() => {
      const finish = gatewayGuard.begin()
      return gatewayStart().then((s) => (finish(s), s))
    })
    if (!r.ok && r.error && startErrorView(toIpcError(r.error)).recheckEnv) void loadEnv()
  }, [toggle, loadEnv, gatewayGuard])

  const stopGateway = useCallback(async () => {
    await toggle.run(() => {
      const finish = gatewayGuard.begin()
      return gatewayStop().then((s) => (finish(s), s))
    })
  }, [toggle, gatewayGuard])

  const flashCopied = useCallback((what: string) => {
    setCopied(what)
    setTimeout(() => setCopied((c) => (c === what ? null : c)), 2000)
  }, [])

  const copyText = useCallback(
    async (text: string, what: string) => {
      if (await writeClipboardText(text)) flashCopied(what)
    },
    [flashCopied]
  )

  const openLog = useCallback(() => {
    void gatewayAction.run(() => gatewayRevealLog())
  }, [gatewayAction])

  const kclaudeDo = useCallback(
    (fn: () => Promise<KclaudeStatus>) => kclaudeAction.run(() => fn().then((s) => (setKclaude(s), s))),
    [kclaudeAction]
  )

  const saveConfig = useCallback(
    async (patch: ConfigPatch) => {
      const finish = configGuard.begin()
      const r = await configAction.run(() => configUpdate(patch))
      if (r.ok) {
        finish(r.value.config)
        setRestartNote(r.value.restartRequired ? 'restart' : null)
        // The gateway status carries the port; reflect a stopped-state change right away.
        refreshGatewayStatus(false)
      }
      return r.ok
    },
    [configAction, configGuard, refreshGatewayStatus]
  )

  const clearLog = useCallback(async () => {
    const r = await logAction.run(() => requestLogClear())
    if (r.ok) setLog(indexSnapshot(r.value))
    return r.ok
  }, [logAction])

  const checkUpdate = useCallback(async () => {
    await updateAction.run(() => updateCheckNow().then((u) => (setUpdate(u), refreshConfig(), u)))
  }, [updateAction, refreshConfig])

  const retryUpdate = useCallback(async () => {
    // `update_retry` is rejected outside forced / install_failed; a fresh check is the retry then.
    const call = retryCommandFor(update) === 'retry' ? updateRetry : updateCheckNow
    await updateAction.run(() => call().then((u) => (setUpdate(u), refreshConfig(), u)))
  }, [updateAction, update, refreshConfig])

  const restartNow = useCallback(async () => {
    await updateAction.run(() => updateRestartNow())
  }, [updateAction])

  const gatewayRunning = gateway?.state === 'running'

  return {
    now,
    config,
    gateway,
    env,
    kclaude,
    update,
    log,
    models,
    gatewayRunning,
    restartNote,
    copied,
    toggle,
    gatewayAction,
    envAction,
    kclaudeAction,
    logAction,
    configAction,
    updateAction,
    startGateway,
    stopGateway,
    copyText,
    openLog,
    recheckEnv: loadEnv,
    installKclaude: () => kclaudeDo(kclaudeInstall),
    adoptKclaude: () => kclaudeDo(kclaudeAdopt),
    removeKclaude: () => kclaudeDo(kclaudeRemove),
    saveConfig,
    clearLog,
    checkUpdate,
    retryUpdate,
    restartNow
  }
}

export type AppState = ReturnType<typeof useAppState>
