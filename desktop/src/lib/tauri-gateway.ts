import { call, onEvent } from './tauri-core'
import type { GatewayModel, GatewayStatus } from './ipc-types'

export const gatewayStatus = (): Promise<GatewayStatus> => call('gateway_status')
export const gatewayStart = (): Promise<GatewayStatus> => call('gateway_start')
export const gatewayStop = (): Promise<GatewayStatus> => call('gateway_stop')
export const gatewayModels = (): Promise<GatewayModel[]> => call('gateway_models')
export const gatewayRevealLog = (): Promise<null> => call('gateway_reveal_log')
export const onGatewayStatus = (handler: (s: GatewayStatus) => void): Promise<() => void> =>
  onEvent('gateway-status', handler)
