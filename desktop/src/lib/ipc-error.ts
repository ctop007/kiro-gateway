/**
 * Normalizes whatever a rejected command throws into the contract's
 * `{ code, message }` shape. Pure, so it can be tested without Tauri.
 */
import { IPC_ERROR_CODES, type IpcError, type IpcErrorCode } from './ipc-types'

export const GENERIC_ERROR_MESSAGE = '操作没有成功，请稍后再试'

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null
}

/**
 * Convert a rejection value into an `IpcError`.
 *
 * Only a string `message` on an object is trusted for display (the contract
 * guarantees it is a user-facing sentence). Anything else, including `Error`
 * instances and bare strings, may contain URLs or stack text, so it is replaced
 * by a generic sentence and logged to the console for debugging only.
 */
export function toIpcError(raw: unknown): IpcError {
  if (!isRecord(raw) || raw instanceof Error) {
    console.warn('unexpected command rejection', raw)
    return { code: 'internal', message: GENERIC_ERROR_MESSAGE }
  }
  const code = (IPC_ERROR_CODES as readonly unknown[]).includes(raw.code)
    ? (raw.code as IpcErrorCode)
    : 'internal'
  const message =
    typeof raw.message === 'string' && raw.message.trim() !== '' && typeof raw.code === 'string'
      ? raw.message
      : GENERIC_ERROR_MESSAGE
  if (typeof raw.code !== 'string') console.warn('unexpected command rejection', raw)
  return { code, message }
}

/** The displayable sentence of any rejection. */
export function errorMessage(raw: unknown): string {
  return toIpcError(raw).message
}

/** True when the rejection carries the given contract code. */
export function hasErrorCode(raw: unknown, code: IpcErrorCode): boolean {
  return toIpcError(raw).code === code
}
