/** tauri-plugin-clipboard-manager thin wrapper. Returns whether the copy succeeded. */
import { writeText } from '@tauri-apps/plugin-clipboard-manager'

export async function writeClipboardText(text: string): Promise<boolean> {
  try {
    if (import.meta.env.DEV && import.meta.env.VITE_MOCK_IPC === '1') {
      const mock = await import('@/mock/backend')
      return mock.mockWriteText(text)
    }
    await writeText(text)
    return true
  } catch (err) {
    console.warn('clipboard write failed', err)
    return false
  }
}
