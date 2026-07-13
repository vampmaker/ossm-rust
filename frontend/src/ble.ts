import type { MotorControllerConfig, PausedControlPayload, MotorState, PinConfiguration, NetworkConfiguration } from './types'

const SERVICE_UUID = '6e400001-b5a3-f393-e0a9-e50e24dcca9e'
const CONFIG_UUID  = '6e400002-b5a3-f393-e0a9-e50e24dcca9e'
const STATE_UUID   = '6e400003-b5a3-f393-e0a9-e50e24dcca9e'
const PAUSED_UUID  = '6e400004-b5a3-f393-e0a9-e50e24dcca9e'
const PIN_UUID     = '6e400005-b5a3-f393-e0a9-e50e24dcca9e'
const RPC_UUID     = '6e400006-b5a3-f393-e0a9-e50e24dcca9e'
const NET_UUID     = '6e400007-b5a3-f393-e0a9-e50e24dcca9e'

class BleChunkReassembler {
  private chunks = new Map<number, Uint8Array>()
  private total = 0

  feed(value: DataView): string | null {
    if (value.byteLength < 2) return null
    const index = value.getUint8(0)
    const total = value.getUint8(1)
    const fragment = new Uint8Array(value.buffer, value.byteOffset + 2, value.byteLength - 2)

    if (total === 0) return null

    if (total === 1 && index === 0) {
      this.chunks.clear()
      this.total = 0
      return new TextDecoder('utf-8').decode(fragment)
    }

    if (total !== this.total) {
      this.chunks.clear()
      this.total = total
    }

    this.chunks.set(index, fragment)

    if (this.chunks.size === total) {
      let totalLen = 0
      for (let i = 0; i < total; i++) {
        const c = this.chunks.get(i)
        if (!c) return null
        totalLen += c.length
      }
      const combined = new Uint8Array(totalLen)
      let offset = 0
      for (let i = 0; i < total; i++) {
        const c = this.chunks.get(i)!
        combined.set(c, offset)
        offset += c.length
      }
      this.chunks.clear()
      this.total = 0
      return new TextDecoder('utf-8').decode(combined)
    }

    return null
  }

  reset() {
    this.chunks.clear()
    this.total = 0
  }
}

let device: BluetoothDevice | null = null
let gattServer: BluetoothRemoteGATTServer | null = null
let service: BluetoothRemoteGATTService | null = null

let configChar: BluetoothRemoteGATTCharacteristic | null = null
let stateChar: BluetoothRemoteGATTCharacteristic | null = null
let pausedChar: BluetoothRemoteGATTCharacteristic | null = null
let pinConfigChar: BluetoothRemoteGATTCharacteristic | null = null
let rpcChar: BluetoothRemoteGATTCharacteristic | null = null
let netConfigChar: BluetoothRemoteGATTCharacteristic | null = null

const stateReassembler = new BleChunkReassembler()
const bleStateListeners = new Set<(state: MotorState) => void>()
let bleNotificationStarted = false
let connectInProgress = false
let gattLock = Promise.resolve()

async function withGattLock<T>(fn: () => Promise<T>): Promise<T> {
  const previous = gattLock
  let release: () => void
  gattLock = new Promise<void>(resolve => {
    release = resolve
  })
  await previous
  try {
    return await fn()
  } finally {
    release!()
  }
}

export async function connectBle(onDisconnect?: () => void): Promise<boolean> {
  if (!navigator.bluetooth) {
    throw new Error('Web Bluetooth is not supported in this browser.')
  }
  if (isBleConnected()) {
    return true
  }
  if (connectInProgress) {
    throw new Error('BLE connection already in progress.')
  }

  connectInProgress = true
  try {
    device = await navigator.bluetooth.requestDevice({
      filters: [
        { name: 'OSSM' },
        { namePrefix: 'OSSM' }
      ],
      optionalServices: [SERVICE_UUID]
    })

    if (onDisconnect) {
      device.addEventListener('gattserverdisconnected', () => {
        gattServer = null
        stateReassembler.reset()
        bleNotificationStarted = false
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
  } finally {
    connectInProgress = false
  }
}

export function isBleConnected(): boolean {
  return gattServer !== null && gattServer.connected
}

export function disconnectBle(): void {
  if (gattServer && gattServer.connected) {
    gattServer.disconnect()
  }
  stateReassembler.reset()
  bleNotificationStarted = false
  device = null
  gattServer = null
  service = null
}

async function readJson<T>(char: BluetoothRemoteGATTCharacteristic | null): Promise<T> {
  if (!char) throw new Error('BLE not connected')
  return withGattLock(async () => {
    let lastErr: unknown
    for (let attempt = 1; attempt <= 3; attempt++) {
      try {
        const val = await char.readValue()
        const decoder = new TextDecoder('utf-8')
        const jsonStr = decoder.decode(val)
        return JSON.parse(jsonStr)
      } catch (e) {
        lastErr = e
        if (attempt < 3) {
          await new Promise(r => setTimeout(r, 100 * attempt))
        }
      }
    }
    throw lastErr || new Error('BLE read failed')
  })
}

async function writeJson(char: BluetoothRemoteGATTCharacteristic | null, data: unknown): Promise<void> {
  if (!char) throw new Error('BLE not connected')
  const encoder = new TextEncoder()
  const bytes = encoder.encode(JSON.stringify(data))
  return withGattLock(async () => {
    let lastErr: unknown
    for (let attempt = 1; attempt <= 3; attempt++) {
      try {
        if ('writeValueWithResponse' in char && typeof (char as any).writeValueWithResponse === 'function') {
          await (char as any).writeValueWithResponse(bytes)
        } else if ('writeValueWithoutResponse' in char && typeof (char as any).writeValueWithoutResponse === 'function') {
          await (char as any).writeValueWithoutResponse(bytes)
        } else {
          await char.writeValue(bytes)
        }
        return
      } catch (e) {
        lastErr = e
        if (attempt < 3) {
          await new Promise(r => setTimeout(r, 100 * attempt))
        }
      }
    }
    throw lastErr || new Error('BLE write failed')
  })
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

export async function subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
  if (!stateChar || !rpcChar) throw new Error('BLE not connected')
  bleStateListeners.add(onState)
  if (!bleNotificationStarted) {
    await withGattLock(async () => {
      if (!bleNotificationStarted) {
        stateChar!.addEventListener('characteristicvaluechanged', (event) => {
          const target = event.target as BluetoothRemoteGATTCharacteristic
          if (target && target.value) {
            try {
              const reassembled = stateReassembler.feed(target.value)
              if (reassembled) {
                const state = JSON.parse(reassembled) as MotorState
                bleStateListeners.forEach(listener => listener(state))
              }
            } catch (e) {
              console.error('BLE notification parse error:', e)
            }
          }
        })
        await stateChar!.startNotifications()
        bleNotificationStarted = true
      }
    })
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
      await withGattLock(async () => {
        await stateChar!.stopNotifications()
      })
      bleNotificationStarted = false
      stateReassembler.reset()
    } catch (e) {
      console.warn('Error unsubscribing BLE state:', e)
    }
  }
}
