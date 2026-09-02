import type { MotorControllerConfig, PausedControlPayload, MotorState, PinConfiguration, NetworkConfiguration, StreamWaypoint } from './types'
import * as ble from './ble'

export type ConnectionMode = 'wifi' | 'bluetooth'

const hostname = window.location.hostname
const isPrivateOrLocal = hostname === 'localhost' || hostname === '127.0.0.1' || hostname === 'ossm.lan' ||
  hostname.startsWith('192.168.') || hostname.startsWith('10.') ||
  /^172\.(1[6-9]|2\d|3[01])\./.test(hostname) || hostname.endsWith('.local') || hostname.endsWith('.lan')

export let currentMode: ConnectionMode = isPrivateOrLocal ? 'wifi' : 'bluetooth'

export function getConnectionMode(): ConnectionMode {
  return currentMode
}

export function setConnectionMode(mode: ConnectionMode): void {
  if (currentMode !== mode) {
    disconnectWs()
  }
  currentMode = mode
}

const api_base = import.meta.env.DEV ? 'http://ossm.lan' : document.location.href;

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(new URL(path, api_base).toString())
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
  return await response.json()
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(new URL(path, api_base).toString(), {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
  return await response.json()
}

async function postNoContent(path: string): Promise<void> {
  const response = await fetch(new URL(path, api_base).toString(), {
    method: 'POST',
  })
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
}

export async function getConfig(): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') {
    return ble.getConfig()
  }
  return getJson<MotorControllerConfig>('/config')
}

export async function setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') {
    return ble.setConfig(config)
  }
  return postJson<MotorControllerConfig>('/config', config)
}

export async function setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') {
    return ble.setPaused(payload)
  }
  return postJson<MotorControllerConfig>('/paused', payload)
}

export async function getState(): Promise<MotorState> {
  if (currentMode === 'bluetooth') {
    return ble.getState()
  }
  return getJson<MotorState>('/state')
}

export async function getPinConfig(): Promise<PinConfiguration> {
  if (currentMode === 'bluetooth') {
    return ble.getPinConfig()
  }
  return getJson<PinConfiguration>('/pin-config')
}

export async function setPinConfig(config: PinConfiguration): Promise<PinConfiguration> {
  if (currentMode === 'bluetooth') {
    return ble.setPinConfig(config)
  }
  return postJson<PinConfiguration>('/pin-config', config)
}

export async function getNetworkConfig(): Promise<NetworkConfiguration> {
  if (currentMode === 'bluetooth') {
    return ble.getNetworkConfig()
  }
  return getJson<NetworkConfiguration>('/network-config')
}

export async function setNetworkConfig(config: NetworkConfiguration): Promise<NetworkConfiguration> {
  if (currentMode === 'bluetooth') {
    return ble.setNetworkConfig(config)
  }
  return postJson<NetworkConfiguration>('/network-config', config)
}

export async function restartDevice(): Promise<void> {
  if (currentMode === 'bluetooth') {
    return ble.restartDevice()
  }
  await postNoContent('/restart')
}

export async function connectBle(onDisconnect?: () => void): Promise<boolean> {
  return ble.connectBle(onDisconnect)
}

export function isBleConnected(): boolean {
  return ble.isBleConnected()
}

export function disconnectBle(): void {
  ble.disconnectBle()
}

export { WsDataManager, OssmClient } from './client'
export type { Deferred } from './client'
import { WsDataManager } from './client'

function getWsUrl(): string {
  const u = new URL(api_base)
  const protocol = u.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${u.host}/ws/command`
}

/**
 * Singleton WebSocket data manager instance for the web UI.
 * Acts as the single owner of the WebSocket connection and shared state cache.
 */
export const wsManager = new WsDataManager(getWsUrl())

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', () => {
    wsManager.close()
  })
}

/**
 * Async function to retrieve the latest shared MotorState.
 * Returns cached state immediately if fresh (< maxAgeMs); otherwise awaits the next update.
 */
export async function fetchSharedMotorState(maxAgeMs = 100): Promise<MotorState> {
  if (currentMode === 'bluetooth') {
    return ble.getState()
  }
  return wsManager.fetchData(maxAgeMs)
}

/**
 * Subscribes to periodic live state updates over WebSocket or BLE.
 */
export async function subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
  if (currentMode === 'bluetooth') {
    return ble.subscribeState(onState, intervalMs)
  }
  return wsManager.subscribe(onState, intervalMs)
}

/**
 * Resets the idle connection timer to prevent immediate disconnection during activity.
 */
export function cancelWsIdleDisconnect(): void {
  wsManager.resetIdleTimer()
}

/**
 * Schedules an idle disconnect check after inactivity.
 */
export async function closeWsIfIdle(): Promise<void> {
  wsManager.resetIdleTimer()
}

/**
 * Immediately closes the WebSocket connection.
 */
export function disconnectWs(): void {
  wsManager.close()
}

/**
 * Unsubscribes a listener from live state updates.
 */
export async function unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
  if (currentMode === 'bluetooth') {
    return ble.unsubscribeState(onState)
  }
  return wsManager.unsubscribe(onState)
}

/**
 * Sends motion trajectory waypoints over WebSocket or BLE.
 */
export async function sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp = false): Promise<void> {
  const method = resetTimestamp ? 'set-waypoints' : 'append-waypoints'
  const params = resetTimestamp ? { waypoints, 'reset-timestamp': true } : waypoints
  if (currentMode === 'bluetooth') {
    await ble.sendRpc(method, params)
    return
  }
  await wsManager.sendRpc(method, params)
}


