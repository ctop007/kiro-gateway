/** tauri-plugin-opener thin wrapper. Business code imports this, never the plugin. */
import { openUrl } from '@tauri-apps/plugin-opener'

export async function openExternalUrl(url: string): Promise<void> {
  if (import.meta.env.DEV && import.meta.env.VITE_MOCK_IPC === '1') {
    const mock = await import('@/mock/backend')
    mock.mockOpenUrl(url)
    return
  }
  await openUrl(url)
}
