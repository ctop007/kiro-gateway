/** UpdateStatus → status bar, settings version row and forced-upgrade overlay (pure). */
import type { DownloadProgress, UpdateStatus } from '@/lib/ipc-types'
import { formatBytesProgress } from '@/features/shared/format'
import { versionLabel } from '@/features/shell/tabs'

/** Whole percent 0..100, or null when the total size is unknown. */
export function progressPercent(p: DownloadProgress): number | null {
  if (p.totalBytes === null || !Number.isFinite(p.totalBytes) || p.totalBytes <= 0) return null
  const pct = Math.floor((Math.max(0, p.downloadedBytes) / p.totalBytes) * 100)
  return Math.min(100, Math.max(0, pct))
}

/** The overlay shows when the update module says so, or config already has the flag. */
export function isUpgradeForced(update: UpdateStatus | null, upgradeRequired: boolean): boolean {
  return upgradeRequired || update?.state === 'forced'
}

function isHttps(url: string): boolean {
  try {
    return new URL(url).protocol === 'https:'
  } catch {
    return false
  }
}

function safeUrl(url: string): string | null {
  return isHttps(url) ? url : null
}

export interface StatusBarUpdate {
  text: string
  /** Show the 「立即重启」 button. */
  restart: boolean
}

export function statusBarUpdate(update: UpdateStatus | null, fallbackVersion = ''): StatusBarUpdate {
  if (!update) return { text: versionLabel(fallbackVersion), restart: false }
  const v = versionLabel(update.currentVersion)
  if (update.state === 'ready') return { text: `${v}，${update.version} 已下载`, restart: true }
  if (update.state === 'installing') return { text: `${v}，正在完成更新`, restart: false }
  return { text: v, restart: false }
}

export interface SettingsVersion {
  text: string
  tone: '' | 'bad'
  checkLabel: string
  checkDisabled: boolean
  restart: boolean
  manualUrl: string | null
  manualLabel: string
}

export function settingsVersion(
  update: UpdateStatus | null,
  checking: boolean,
  fallbackVersion = ''
): SettingsVersion {
  const base = {
    tone: '' as const,
    checkLabel: checking ? '正在检查' : '检查更新',
    checkDisabled: checking,
    restart: false,
    manualUrl: null as string | null,
    manualLabel: '手动下载安装包'
  }
  if (!update) return { ...base, text: fallbackVersion.replace(/^v/, ''), checkDisabled: true }
  const v = update.currentVersion.replace(/^v/, '')
  switch (update.state) {
    case 'downloading': {
      const pct = progressPercent(update.progress)
      return { ...base, text: `${v}，正在后台下载 ${update.version}${pct === null ? '' : `（${pct}%）`}` }
    }
    case 'ready':
      return { ...base, text: `${v}，${update.version} 已下载，下次启动生效`, restart: true }
    case 'install_failed':
      return {
        ...base,
        tone: 'bad',
        // Rust's error already reads "更新安装失败" or a concrete reason: show it as is.
        text: `${v}，${update.error}`,
        manualUrl: safeUrl(update.manualUrl)
      }
    case 'forced':
      return { ...base, text: `${v}，需要升级到 ${update.version}`, checkDisabled: true }
    case 'installing':
      return { ...base, text: `${v}，正在完成更新到 ${update.version}`, checkDisabled: true }
    case 'none':
    default:
      return { ...base, text: `${v}，已是最新` }
  }
}

export interface ForcedOverlay {
  title: string
  text: string
  /** Progress bar; `percent: null` means indeterminate. Null when the download failed. */
  progress: { percent: number | null; label: string } | null
  error: string | null
  retry: boolean
  manual: { url: string; label: string } | null
}

const FORCED_TEXT = '当前版本已停止使用，升级完成后才能继续用。完成后会自动重新打开。'

function installerName(url: string): string {
  try {
    const last = new URL(url).pathname.split('/').filter(Boolean).pop()
    return last ? decodeURIComponent(last) : '安装包'
  } catch {
    return '安装包'
  }
}

/**
 * Which command the overlay "重试" button calls. `update_retry` is only accepted in
 * `forced` and `install_failed`; in any other state a fresh check is the retry.
 */
export function retryCommandFor(update: UpdateStatus | null): 'retry' | 'check' {
  return update?.state === 'forced' || update?.state === 'install_failed' ? 'retry' : 'check'
}

/**
 * Content of the non-dismissable overlay, or null when no upgrade is forced.
 * The returned model has no close action on purpose.
 */
export function forcedOverlay(update: UpdateStatus | null, upgradeRequired: boolean): ForcedOverlay | null {
  if (!isUpgradeForced(update, upgradeRequired)) return null
  if (!update || update.state !== 'forced') {
    return {
      title: '需要升级',
      text: FORCED_TEXT,
      // No forced state yet (the first check failed or has not finished): the overlay must
      // not be a dead end, so it offers a retry. Rust has no manual link to give here.
      progress: { percent: null, label: '正在检查新版本' },
      error: null,
      retry: true,
      manual: null
    }
  }
  const manualUrl = safeUrl(update.manualUrl)
  const manual = manualUrl ? { url: manualUrl, label: installerName(manualUrl) } : null
  const title = `正在升级到 ${update.version}`
  if (update.error !== null) {
    return { title, text: FORCED_TEXT, progress: null, error: update.error, retry: true, manual }
  }
  const progress = update.progress
    ? { percent: progressPercent(update.progress), label: formatBytesProgress(update.progress.downloadedBytes, update.progress.totalBytes) }
    : { percent: null, label: '正在准备下载' }
  return { title, text: FORCED_TEXT, progress, error: null, retry: false, manual }
}
