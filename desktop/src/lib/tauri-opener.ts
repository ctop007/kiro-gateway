/** tauri-plugin-opener thin wrapper. Business code imports this, never the plugin. */
import { openUrl } from '@tauri-apps/plugin-opener'

export async function openExternalUrl(url: string): Promise<void> {
  await openUrl(url)
}
