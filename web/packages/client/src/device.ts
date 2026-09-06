import type {
  MotorControllerConfig,
  MotorState,
  NetworkConfiguration,
  OssmControl,
  PausedControlPayload,
  PinConfiguration,
  ShellConfig,
  StreamWaypoint,
} from '@ossm/shared'
import * as ble from './ble'
import { WsDataManager } from './client'

export type ConnectionMode = 'wifi' | 'bluetooth'

const hostname = typeof window !== 'undefined' ? window.location.hostname : ''
const isPrivateOrLocal =
  hostname === 'localhost' ||
  hostname === '127.0.0.1' ||
  hostname === 'ossm.lan' ||
  hostname.startsWith('192.168.') ||
  hostname.startsWith('10.') ||
  /^172\.(1[6-9]|2\d|3[01])\./.test(hostname) ||
  hostname.endsWith('.local') ||
  hostname.endsWith('.lan')

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

const api_base =
  (typeof import.meta !== 'undefined' && import.meta.env?.VITE_OSSM_API) ||
  (typeof window !== 'undefined' && import.meta.env?.DEV
    ? 'http://ossm.lan'
    : typeof document !== 'undefined'
      ? document.location.href
      : 'http://ossm.lan')

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
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
  return await response.json()
}

async function postNoContent(path: string): Promise<void> {
  const response = await fetch(new URL(path, api_base).toString(), { method: 'POST' })
  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`)
  }
}

export async function getConfig(): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') return ble.getConfig()
  return getJson<MotorControllerConfig>('/config')
}

export async function setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') return ble.setConfig(config)
  return postJson<MotorControllerConfig>('/config', config)
}

export async function setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
  if (currentMode === 'bluetooth') return ble.setPaused(payload)
  return postJson<MotorControllerConfig>('/paused', payload)
}

export async function getState(): Promise<MotorState> {
  if (currentMode === 'bluetooth') return ble.getState()
  return getJson<MotorState>('/state')
}

function pinSlice(config: ShellConfig): PinConfiguration {
  return {
    modbus_tx: config.modbus_tx,
    modbus_rx: config.modbus_rx,
    modbus_de_re: config.modbus_de_re,
    modbus_timeout_ms: config.modbus_timeout_ms,
    modbus_rx_timeout_us: config.modbus_rx_timeout_us,
    modbus_scan_delay_us: config.modbus_scan_delay_us,
    modbus_inter_frame_delay_us: config.modbus_inter_frame_delay_us,
    ble_enabled: config.ble_enabled,
    modbus_debug: config.modbus_debug,
    operating_mode: config.operating_mode,
    modbus_baud: config.modbus_baud,
  }
}

function netSlice(config: ShellConfig): NetworkConfiguration {
  return {
    wifi_enabled: config.wifi_enabled,
    ssid: config.ssid,
    password: config.password,
    hostname: config.hostname,
    dhcp_enabled: config.dhcp_enabled,
    static_ip: config.static_ip,
    static_mask: config.static_mask,
    static_gateway: config.static_gateway,
    static_dns: config.static_dns,
  }
}

export async function getShellConfig(): Promise<ShellConfig> {
  if (currentMode === 'bluetooth') {
    const pin = await ble.getPinConfig()
    const net = await ble.getNetworkConfig()
    return { ...net, ...pin }
  }
  return getJson<ShellConfig>('/shell-config')
}

export async function setShellConfig(config: ShellConfig): Promise<ShellConfig> {
  if (currentMode === 'bluetooth') {
    await ble.setPinConfig(pinSlice(config))
    await ble.setNetworkConfig(netSlice(config))
    return getShellConfig()
  }
  return postJson<ShellConfig>('/shell-config', config)
}

export async function restartDevice(): Promise<void> {
  if (currentMode === 'bluetooth') return ble.restartDevice()
  await postNoContent('/restart')
}

export async function getRuntimeConfig(): Promise<import('@ossm/shared').StdRuntimeConfig> {
  return getJson('/runtime-config')
}

export async function getLinkStats(): Promise<import('@ossm/shared').LinkStats> {
  return getJson('/link-stats')
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

function getWsUrl(): string {
  const u = new URL(api_base)
  const protocol = u.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${u.host}/ws/command`
}

export const wsManager = new WsDataManager(getWsUrl())

if (typeof window !== 'undefined') {
  window.addEventListener('beforeunload', () => {
    wsManager.close()
  })
}

export async function fetchSharedMotorState(maxAgeMs = 100): Promise<MotorState> {
  if (currentMode === 'bluetooth') return ble.getState()
  return wsManager.fetchData(maxAgeMs)
}

export async function subscribeState(
  onState: (state: MotorState) => void,
  intervalMs = 33,
): Promise<void> {
  if (currentMode === 'bluetooth') return ble.subscribeState(onState, intervalMs)
  return wsManager.subscribe(onState, intervalMs)
}

export function cancelWsIdleDisconnect(): void {
  wsManager.resetIdleTimer()
}

export async function closeWsIfIdle(): Promise<void> {
  wsManager.resetIdleTimer()
}

export function disconnectWs(): void {
  wsManager.close()
}

export async function unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
  if (currentMode === 'bluetooth') return ble.unsubscribeState(onState)
  return wsManager.unsubscribe(onState)
}

export async function sendWaypoints(
  waypoints: StreamWaypoint[],
  resetTimestamp = false,
): Promise<void> {
  const method = resetTimestamp ? 'set-waypoints' : 'append-waypoints'
  const params = resetTimestamp ? { waypoints, 'reset-timestamp': true } : waypoints
  if (currentMode === 'bluetooth') {
    await ble.sendRpc(method, params)
    return
  }
  await wsManager.sendRpc(method, params)
}

/** Device (WiFi/BLE) control surface used by the embedded frontend. */
export const deviceControl: OssmControl = {
  getConfig,
  setConfig,
  setPaused,
  getState,
  subscribeState,
  unsubscribeState,
  sendWaypoints,
  restart: restartDevice,
}
