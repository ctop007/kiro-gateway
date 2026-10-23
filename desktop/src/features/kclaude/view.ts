/** kclaude status and model options → panel text and buttons (pure). Every sentence uses the current command name. */
import type { FixAction, KclaudeStatus } from '@/lib/ipc-types'
import { DEFAULT_COMMAND_NAME } from '@/features/command-name/view'

export type KclaudeActionId = 'install' | 'adopt'

export interface KclaudeAction {
  id: KclaudeActionId
  label: string
  primary: boolean
}

export interface KclaudeView {
  tone: '' | 'warn'
  text: string
  /** Text next to the command chip. */
  hint: string
  actions: KclaudeAction[]
  canRemove: boolean
  verifyLine: { text: string; fix: FixAction | null } | null
}

const USAGE_HINT = '用法和 claude 一样'

function foreignText(files: string[], name: string): string {
  const where = files.length > 0 ? `${files.join('、')} 里` : '启动文件里'
  return `${where}已经有一个你自己加的 ${name}，App 没有改动它。`
}

/**
 * @param status  latest `kclaude_status`; null before the first answer
 * @param gatewayRunning  whether the gateway is `running` (changes the ready sentence)
 * @param commandName  the current command name (`AppConfig.commandName`); defaults to the status's own, then `cclaude`
 */
export function kclaudeView(
  status: KclaudeStatus | null,
  gatewayRunning: boolean,
  commandName?: string
): KclaudeView {
  const name = commandName?.trim() || status?.commandName?.trim() || DEFAULT_COMMAND_NAME
  if (!status) {
    return { tone: '', text: `正在检查 ${name}`, hint: USAGE_HINT, actions: [], canRemove: false, verifyLine: null }
  }

  const verifyLine =
    status.verify.status === 'fail'
      ? { text: status.verify.detail.trim() || `新开的终端里没能找到 ${name}`, fix: status.verify.fix }
      : null

  switch (status.state) {
    case 'foreign':
      return {
        tone: 'warn',
        text: foreignText(status.rcFiles, name),
        hint: '当前用的是你自己的版本',
        actions: [{ id: 'adopt', label: '改由 App 管理', primary: false }],
        canRemove: false,
        verifyLine: null
      }
    case 'not_installed':
      return {
        tone: '',
        text: `还没有安装 ${name}。`,
        hint: USAGE_HINT,
        actions: [{ id: 'install', label: '安装', primary: true }],
        canRemove: false,
        verifyLine: null
      }
    case 'outdated':
      return {
        tone: '',
        text: `${name} 的内容不是最新的，重新安装一次就好。`,
        hint: USAGE_HINT,
        actions: [{ id: 'install', label: '重新安装', primary: true }],
        canRemove: true,
        verifyLine
      }
    case 'installed':
    default: {
      if (!status.claudeInstalled) {
        return {
          tone: 'warn',
          text: '已安装，但要先装好 Claude Code 才能用。',
          hint: USAGE_HINT,
          actions: [],
          canRemove: true,
          verifyLine
        }
      }
      return {
        tone: '',
        text: gatewayRunning
          ? '已就绪。新开一个终端就能用，原来的 claude 不受影响。'
          : '已安装。网关关着的时候运行会提示你先打开开关。',
        hint: USAGE_HINT,
        actions: [],
        canRemove: true,
        verifyLine
      }
    }
  }
}

/**
 * Show a Kiro model id the way the mockup does: `claude-opus-5-5[1m]` →
 * `claude-opus-5.5 [1m]`. Only a single-digit minor version right after a
 * single-digit major is rewritten, so dated ids like `-20251001` stay intact.
 */
export function displayModel(raw: string): string {
  const dotted = raw.replace(/^(claude-[a-z]+-\d)-(\d)(?=$|\[|-)/, '$1.$2')
  return dotted.replace(/\[1m\]$/, ' [1m]')
}

export interface ModelOption {
  value: string
  label: string
}

export interface ModelPicker {
  disabled: boolean
  hint: string | null
  options: ModelOption[]
}

/**
 * Options for one tier's dropdown. The saved value is always present so it is
 * never silently replaced; the list comes from `gateway_models` and exists only
 * while the gateway runs.
 *
 * @param saved  the value stored in config
 * @param available  ids from `gateway_models`, or null when not loaded
 * @param gatewayRunning  whether the gateway is running
 */
export function modelPicker(saved: string, available: string[] | null, gatewayRunning: boolean): ModelPicker {
  const ids: string[] = []
  const add = (id: string) => {
    const v = id.trim()
    if (v !== '' && !ids.includes(v)) ids.push(v)
  }
  add(saved)
  for (const id of available ?? []) add(id)
  const options = ids.map((value) => ({ value, label: displayModel(value) }))

  if (!gatewayRunning) return { disabled: true, hint: '启动网关后可选择模型', options }
  if (!available || available.length === 0) return { disabled: true, hint: '正在读取模型列表', options }
  return { disabled: false, hint: null, options }
}

/** Settings row description: where the marked block lives. */
export function rcDescription(status: KclaudeStatus | null): string {
  const files = status?.rcFiles ?? []
  return files.length > 0 ? `写在 ${files.join('、')} 里带标记的一段` : '写在启动文件里带标记的一段'
}
