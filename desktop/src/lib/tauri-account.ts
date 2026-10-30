import { call } from './tauri-core'

export const accountGet = () => call('account_get')
export const accountRefreshToken = () => call('account_refresh_token')
