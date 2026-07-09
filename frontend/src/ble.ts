import type { MotorControllerConfig, PausedControlPayload, MotorState, PinConfiguration, NetworkConfiguration } from './types'

const SERVICE_UUID = '6e400001-b5a3-f393-e0a9-e50e24dcca9e'
const CONFIG_UUID  = '6e400002-b5a3-f393-e0a9-e50e24dcca9e'
const STATE_UUID   = '6e400003-b5a3-f393-e0a9-e50e24dcca9e'
const PAUSED_UUID  = '6e400004-b5a3-f393-e0a9-e50e24dcca9e'
const PIN_UUID     = '6e400005-b5a3-f393-e0a9-e50e24dcca9e'
const RPC_UUID     = '6e400006-b5a3-f393-e0a9-e50e24dcca9e'
const NET_UUID     = '6e400007-b5a3-f393-e0a9-e50e24dcca9e'

let device: BluetoothDevice | null = null
let gattServer: BluetoothRemoteGATTServer | null = null
let service: BluetoothRemoteGATTService | null = null

let configChar: BluetoothRemoteGATTCharacteristic | null = null
let stateChar: BluetoothRemoteGATTCharacteristic | null = null
let pausedChar: BluetoothRemoteGATTCharacteristic | null = null
let pinConfigChar: BluetoothRemoteGATTCharacteristic | null = null
let rpcChar: BluetoothRemoteGATTCharacteristic | null = null
let netConfigChar: BluetoothRemoteGATTCharacteristic | null = null

export async function connectBle(onDisconnect?: () => void): Promise<boolean> {
  if (!navigator.bluetooth) {
    throw new Error('Web Bluetooth is not supported in this browser.')
  }

  device = await navigator.bluetooth.requestDevice({
    filters: [{ name: 'OSSM' }, { namePrefix: 'OSSM' }],
    optionalServices: [SERVICE_UUID]
  })

  if (onDisconnect) {
    device.addEventListener('gattserverdisconnected', () => {
      gattServer = null
      onDisconnect()
    })
  }

  gattServer = await device.gatt!.connect()
  service = await gattServer.getPrimaryService(SERVICE_UUID)

  configChar = await service.getCharacteristic(CONFIG_UUID)
  stateChar = await service.getCharacteristic(STATE_UUID)
  pausedChar = await service.getCharacteristic(PAUSED_UUID)
  pinConfigChar = await service.getCharacteristic(PIN_UUID)
  rpcChar = await service.getCharacteristic(RPC_UUID)
  netConfigChar = await service.getCharacteristic(NET_UUID)

  return true
}

export function isBleConnected(): boolean {
  return gattServer !== null && gattServer.connected
}

export function disconnectBle(): void {
  if (gattServer && gattServer.connected) {
    gattServer.disconnect()
  }
  device = null
  gattServer = null
  service = null
}

async function readJson<T>(char: BluetoothRemoteGATTCharacteristic | null): Promise<T> {
  if (!char) throw new Error('BLE not connected')
  const val = await char.readValue()
  const decoder = new TextDecoder('utf-8')
  const jsonStr = decoder.decode(val)
  return JSON.parse(jsonStr)
}

async function writeJson(char: BluetoothRemoteGATTCharacteristic | null, data: unknown): Promise<void> {
  if (!char) throw new Error('BLE not connected')
  const encoder = new TextEncoder()
  const bytes = encoder.encode(JSON.stringify(data))
  await char.writeValue(bytes)
}

export async function getConfig(): Promise<MotorControllerConfig> {
  return readJson<MotorControllerConfig>(configChar)
}

export async function setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
  await writeJson(configChar, config)
  await new Promise(r => setTimeout(r, 50))
  return readJson<MotorControllerConfig>(configChar)
}

export async function setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
  await writeJson(pausedChar, payload)
  await new Promise(r => setTimeout(r, 50))
  return readJson<MotorControllerConfig>(configChar)
}

export async function getState(): Promise<MotorState> {
  return readJson<MotorState>(stateChar)
}

export async function getPinConfig(): Promise<PinConfiguration> {
  return readJson<PinConfiguration>(pinConfigChar)
}

export async function setPinConfig(config: PinConfiguration): Promise<PinConfiguration> {
  await writeJson(pinConfigChar, config)
  await new Promise(r => setTimeout(r, 50))
  return readJson<PinConfiguration>(pinConfigChar)
}

export async function getNetworkConfig(): Promise<NetworkConfiguration> {
  return readJson<NetworkConfiguration>(netConfigChar)
}

export async function setNetworkConfig(config: NetworkConfiguration): Promise<NetworkConfiguration> {
  await writeJson(netConfigChar, config)
  await new Promise(r => setTimeout(r, 50))
  return readJson<NetworkConfiguration>(netConfigChar)
}

export async function restartDevice(): Promise<void> {
  const req = { jsonrpc: '2.0', method: 'restart', id: 1 }
  await writeJson(rpcChar, req)
}

let rpcCounter = 100
export async function sendRpc(method: string, params?: unknown): Promise<void> {
  const req = { jsonrpc: '2.0', method, params, id: rpcCounter++ }
  await writeJson(rpcChar, req)
}

const bleStateListeners = new Set<(state: MotorState) => void>()
let bleNotificationStarted = false

export async function subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
  if (!stateChar || !rpcChar) throw new Error('BLE not connected')
  bleStateListeners.add(onState)
  if (!bleNotificationStarted) {
    stateChar.addEventListener('characteristicvaluechanged', (event) => {
      const target = event.target as BluetoothRemoteGATTCharacteristic
      if (target && target.value) {
        try {
          const str = new TextDecoder('utf-8').decode(target.value)
          const state = JSON.parse(str) as MotorState
          bleStateListeners.forEach(listener => listener(state))
        } catch (e) {
          console.error('BLE notification parse error:', e)
        }
      }
    })
    await stateChar.startNotifications()
    bleNotificationStarted = true
  }
  const req = { jsonrpc: '2.0', method: 'subscribe-state', params: { interval_ms: intervalMs }, id: 1 }
  await writeJson(rpcChar, req)
}

export async function unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
  if (onState) {
    bleStateListeners.delete(onState)
  } else {
    bleStateListeners.clear()
  }
  if (bleStateListeners.size === 0 && stateChar && rpcChar) {
    try {
      const req = { jsonrpc: '2.0', method: 'unsubscribe-state', id: 1 }
      await writeJson(rpcChar, req)
      await stateChar.stopNotifications()
      bleNotificationStarted = false
    } catch (e) {
      console.warn('Error unsubscribing BLE state:', e)
    }
  }
}
