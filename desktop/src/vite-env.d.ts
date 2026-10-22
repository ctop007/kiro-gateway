/// <reference types="vite/client" />

declare const __APP_VERSION__: string

interface ImportMetaEnv {
  /** "1" serves the UI from the in-memory mock backend (browser preview, screenshots). */
  readonly VITE_MOCK_IPC?: string
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
