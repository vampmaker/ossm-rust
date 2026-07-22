import type { MotorControllerConfig, PausedControlPayload, MotorState, PinConfiguration, NetworkConfiguration } from './types'
import { i18n } from './i18n'

const SERVICE_UUID = '6e400001-b5a3-f393-e0a9-e50e24dcca9e'
const CONFIG_UUID  = '6e400002-b5a3-f393-e0a9-e50e24dcca9e'
const STATE_UUID   = '6e400003-b5a3-f393-e0a9-e50e24dcca9e'
const PAUSED_UUID  = '6e400004-b5a3-f393-e0a9-e50e24dcca9e'
const PIN_UUID     = '6e400005-b5a3-f393-e0a9-e50e24dcca9e'
const RPC_UUID     = '6e400006-b5a3-f393-e0a9-e50e24dcca9e'
const NET_UUID     = '6e400007-b5a3-f393-e0a9-e50e24dcca9e'
const BLE_DISCOVERY_TIMEOUT_MS = 45_000
const BLE_CONNECT_TIMEOUT_MS = 15_000
const BLE_GATT_OP_TIMEOUT_MS = 8_000
const BLE_NOTIFICATION_OP_TIMEOUT_MS = 10_000
const BLE_IO_RETRIES = 3

const textDecoder = new TextDecoder('utf-8')
const textEncoder = new TextEncoder()

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
      return textDecoder.decode(fragment)
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
      return textDecoder.decode(combined)
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
const rpcReassembler = new BleChunkReassembler()
const bleStateListeners = new Set<(state: MotorState) => void>()
let bleNotificationStarted = false
let connectInProgress = false
let disconnectCallback: (() => void) | null = null
let deviceDisconnectHandler: ((event: Event) => void) | null = null
let stateNotificationHandler: ((event: Event) => void) | null = null
let rpcNotificationHandler: ((event: Event) => void) | null = null
let gattLock = Promise.resolve()

function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}

function withTimeout<T>(operation: Promise<T>, timeoutMs: number, operationName: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`BLE ${operationName} timed out after ${timeoutMs}ms`))
    }, timeoutMs)

    operation.then(
      result => {
        clearTimeout(timeout)
        resolve(result)
      },
      error => {
        clearTimeout(timeout)
        reject(error)
      },
    )
  })
}

function normalizeBleError(error: unknown): Error {
  if (error instanceof DOMException) {
    if (error.name === 'NotFoundError') {
      return new Error(i18n.global.t('errors.bleNoDevice'))
    }
    if (error.name === 'SecurityError') {
      return new Error(i18n.global.t('errors.bleSecureContext'))
    }
    if (error.name === 'NotSupportedError') {
      return new Error(i18n.global.t('errors.bleNotSupported'))
    }
    if (error.name === 'NetworkError') {
      return new Error(i18n.global.t('errors.bleNetworkError'))
    }
  }
  if (error instanceof Error) {
    return error
  }
  return new Error(String(error))
}

function detachDeviceDisconnectListener(): void {
  if (device && deviceDisconnectHandler) {
    device.removeEventListener('gattserverdisconnected', deviceDisconnectHandler)
  }
  deviceDisconnectHandler = null
}

function detachStateNotificationListener(): void {
  if (stateChar && stateNotificationHandler) {
    stateChar.removeEventListener('characteristicvaluechanged', stateNotificationHandler)
  }
  stateNotificationHandler = null
}

function detachRpcNotificationListener(): void {
  if (rpcChar && rpcNotificationHandler) {
    rpcChar.removeEventListener('characteristicvaluechanged', rpcNotificationHandler)
  }
  rpcNotificationHandler = null
}

function clearBleSession(clearDevice = true): void {
  detachStateNotificationListener()
  detachRpcNotificationListener()
  if (clearDevice) {
    detachDeviceDisconnectListener()
  }
  bleStateListeners.clear()
  stateReassembler.reset()
  rpcReassembler.reset()
  bleNotificationStarted = false

  configChar = null
  stateChar = null
  pausedChar = null
  pinConfigChar = null
  rpcChar = null
  netConfigChar = null
  service = null
  gattServer = null

  if (clearDevice) {
    device = null
    disconnectCallback = null
  }

  // Ensure stale hung operations do not permanently block future operations.
  gattLock = Promise.resolve()
}

async function requestDeviceWithFallback(bluetooth: Bluetooth): Promise<BluetoothDevice> {
  // One chooser with OR filters: name-based discovery is reliable for OSSM's
  // CompleteLocalName, and the service UUID filter covers scanners that match
  // services in advertising data. Avoid stacked requestDevice() calls — each
  // opens a new chooser and leaves the previous attempt hanging.
  const options: BluetoothRequestDeviceOptions = {
    filters: [
      { name: 'OSSM' },
      { namePrefix: 'OSSM' },
      { services: [SERVICE_UUID] },
    ],
    optionalServices: [SERVICE_UUID],
  }

  try {
    return await withTimeout(
      bluetooth.requestDevice(options),
      BLE_DISCOVERY_TIMEOUT_MS,
      'device discovery',
    )
  } catch (error) {
    throw normalizeBleError(error)
  }
}

function normalizeMotorState(raw: unknown): MotorState {
  const state = (raw ?? {}) as MotorState & Record<string, unknown>
  const config = (state.config ?? {}) as MotorState['config']
  const y =
    typeof state.y === 'number'
      ? state.y
      : typeof state.shaped_y === 'number'
        ? state.shaped_y
        : typeof config.paused_position === 'number'
          ? config.paused_position
          : 0
  const shapedY = typeof state.shaped_y === 'number' ? state.shaped_y : y

  return {
    ...state,
    config,
    y,
    shaped_y: shapedY,
    position: typeof state.position === 'number' ? state.position : 0,
    speed: typeof state.speed === 'number' ? state.speed : 0,
    t: typeof state.t === 'number' ? state.t : 0,
    x: typeof state.x === 'number' ? state.x : 0,
  } as MotorState
}

function requireConnectedCharacteristic(
  char: BluetoothRemoteGATTCharacteristic | null,
  label: string,
): BluetoothRemoteGATTCharacteristic {
  if (!char || !gattServer || !gattServer.connected) {
    throw new Error(i18n.global.t('errors.bleNotConnected', { label }))
  }
  return char
}

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
  const bluetooth = navigator.bluetooth
  if (!bluetooth) {
    throw new Error(i18n.global.t('errors.bleNotSupportedBrowser'))
  }
  if (isBleConnected()) {
    return true
  }
  if (connectInProgress) {
    throw new Error(i18n.global.t('errors.bleConnectInProgress'))
  }

  connectInProgress = true
  try {
    clearBleSession()
    disconnectCallback = onDisconnect ?? null

    device = await requestDeviceWithFallback(bluetooth)
    if (!device.gatt) {
      throw new Error(i18n.global.t('errors.bleNoGatt'))
    }

    deviceDisconnectHandler = () => {
      if (connectInProgress) {
        return
      }
      setTimeout(() => {
        if (!connectInProgress && (!gattServer || !gattServer.connected)) {
          const cb = disconnectCallback
          clearBleSession(false)
          if (cb) {
            cb()
          }
        }
      }, 350)
    }
    device.addEventListener('gattserverdisconnected', deviceDisconnectHandler)

    let lastConnectError: unknown
    for (let attempt = 1; attempt <= BLE_IO_RETRIES; attempt++) {
      try {
        if (attempt > 1) {
          clearBleSession(false)
        }
        gattServer = await withTimeout(
          device.gatt.connect(),
          BLE_CONNECT_TIMEOUT_MS,
          'GATT connect',
        )
        await delay(200)

        service = await withTimeout(
          gattServer.getPrimaryService(SERVICE_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'service discovery',
        )

        configChar = await withTimeout(
          service.getCharacteristic(CONFIG_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'config characteristic discovery',
        )
        stateChar = await withTimeout(
          service.getCharacteristic(STATE_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'state characteristic discovery',
        )
        pausedChar = await withTimeout(
          service.getCharacteristic(PAUSED_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'paused characteristic discovery',
        )
        pinConfigChar = await withTimeout(
          service.getCharacteristic(PIN_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'pin characteristic discovery',
        )
        rpcChar = await withTimeout(
          service.getCharacteristic(RPC_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'rpc characteristic discovery',
        )
        netConfigChar = await withTimeout(
          service.getCharacteristic(NET_UUID),
          BLE_GATT_OP_TIMEOUT_MS,
          'network characteristic discovery',
        )

        if (rpcChar) {
          if (!rpcNotificationHandler) {
            rpcNotificationHandler = (event: Event) => {
              const target = event.target as BluetoothRemoteGATTCharacteristic
              if (target && target.value) {
                try {
                  const reassembled = rpcReassembler.feed(target.value)
                  if (reassembled) {
                    console.log('BLE RPC response:', JSON.parse(reassembled))
                  }
                } catch (e) {
                  console.error('BLE RPC notification parse error:', e)
                }
              }
            }
            rpcChar.addEventListener('characteristicvaluechanged', rpcNotificationHandler)
          }
          try {
            await withTimeout(
              rpcChar.startNotifications(),
              BLE_NOTIFICATION_OP_TIMEOUT_MS,
              'start rpc notifications',
            )
          } catch (e) {
            console.warn('Could not start initial RPC notifications during connect:', e)
          }
        }

        await delay(150)
        return true
      } catch (err) {
        lastConnectError = err
        if (attempt < BLE_IO_RETRIES) {
          console.warn(`BLE connect attempt ${attempt} failed, retrying in ${attempt * 500}ms...`, err)
          try {
            if (gattServer && gattServer.connected) {
              gattServer.disconnect()
            }
          } catch (e) {}
          await delay(attempt * 500)
        }
      }
    }
    throw normalizeBleError(lastConnectError)
  } catch (error) {
    disconnectCallback = null
    disconnectBle()
    throw normalizeBleError(error)
  } finally {
    connectInProgress = false
  }
}

export function isBleConnected(): boolean {
  return gattServer !== null && gattServer.connected
}

export function disconnectBle(): void {
  disconnectCallback = null
  detachDeviceDisconnectListener()
  try {
    if (gattServer && gattServer.connected) {
      gattServer.disconnect()
    }
  } catch (error) {
    console.warn('BLE disconnect warning:', error)
  }
  clearBleSession()
}

async function readJson<T>(char: BluetoothRemoteGATTCharacteristic | null): Promise<T> {
  const targetChar = requireConnectedCharacteristic(char, 'read')
  return withGattLock(async () => {
    let lastErr: unknown
    for (let attempt = 1; attempt <= BLE_IO_RETRIES; attempt++) {
      try {
        const val = await withTimeout(
          targetChar.readValue(),
          BLE_GATT_OP_TIMEOUT_MS,
          `read ${targetChar.uuid}`,
        )
        const jsonStr = textDecoder.decode(val)
        return JSON.parse(jsonStr) as T
      } catch (e) {
        lastErr = e
        if (attempt < BLE_IO_RETRIES) {
          if (!gattServer || !gattServer.connected) {
            if (device && device.gatt) {
              try {
                gattServer = await withTimeout(device.gatt.connect(), BLE_CONNECT_TIMEOUT_MS, 'GATT reconnect')
                await delay(150)
              } catch (reconnErr) {
                console.warn('BLE reconnect during read retry failed:', reconnErr)
              }
            }
          }
          await delay(150 * attempt)
        }
      }
    }
    throw normalizeBleError(lastErr || new Error(i18n.global.t('errors.bleReadFailed')))
  })
}

async function writeJson(char: BluetoothRemoteGATTCharacteristic | null, data: unknown): Promise<void> {
  const targetChar = requireConnectedCharacteristic(char, 'write')
  const bytes = textEncoder.encode(JSON.stringify(data))
  return withGattLock(async () => {
    let lastErr: unknown
    for (let attempt = 1; attempt <= BLE_IO_RETRIES; attempt++) {
      try {
        // Prefer write-with-response for reliable motor/config control over BLE.
        if (typeof targetChar.writeValueWithResponse === 'function') {
          await withTimeout(
            targetChar.writeValueWithResponse(bytes),
            BLE_GATT_OP_TIMEOUT_MS,
            `write ${targetChar.uuid}`,
          )
        } else {
          await withTimeout(
            targetChar.writeValue(bytes),
            BLE_GATT_OP_TIMEOUT_MS,
            `write ${targetChar.uuid}`,
          )
        }
        return
      } catch (e) {
        lastErr = e
        if (attempt < BLE_IO_RETRIES) {
          if (!gattServer || !gattServer.connected) {
            if (device && device.gatt) {
              try {
                gattServer = await withTimeout(device.gatt.connect(), BLE_CONNECT_TIMEOUT_MS, 'GATT reconnect')
                await delay(150)
              } catch (reconnErr) {
                console.warn('BLE reconnect during write retry failed:', reconnErr)
              }
            }
          }
          await delay(150 * attempt)
        }
      }
    }
    throw normalizeBleError(lastErr || new Error(i18n.global.t('errors.bleWriteFailed')))
  })
}

export async function getConfig(): Promise<MotorControllerConfig> {
  return readJson<MotorControllerConfig>(configChar)
}

export async function setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
  await writeJson(configChar, config)
  await delay(50)
  return readJson<MotorControllerConfig>(configChar)
}

export async function setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
  await writeJson(pausedChar, payload)
  await delay(50)
  return readJson<MotorControllerConfig>(configChar)
}

export async function getState(): Promise<MotorState> {
  const raw = await readJson<unknown>(stateChar)
  return normalizeMotorState(raw)
}

export async function getPinConfig(): Promise<PinConfiguration> {
  return readJson<PinConfiguration>(pinConfigChar)
}

export async function setPinConfig(config: PinConfiguration): Promise<PinConfiguration> {
  await writeJson(pinConfigChar, config)
  await delay(50)
  return readJson<PinConfiguration>(pinConfigChar)
}

export async function getNetworkConfig(): Promise<NetworkConfiguration> {
  return readJson<NetworkConfiguration>(netConfigChar)
}

export async function setNetworkConfig(config: NetworkConfiguration): Promise<NetworkConfiguration> {
  await writeJson(netConfigChar, config)
  await delay(50)
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
  const targetStateChar = requireConnectedCharacteristic(stateChar, 'state notifications')
  const targetRpcChar = requireConnectedCharacteristic(rpcChar, 'subscribe-state')
  // BLE ATT throughput is limited; keep a floor so notify floods don't drop links.
  const clampedIntervalMs = Math.max(50, Math.min(5000, intervalMs))
  bleStateListeners.add(onState)
  if (!bleNotificationStarted) {
    await withGattLock(async () => {
      if (!bleNotificationStarted) {
        if (!stateNotificationHandler) {
          stateNotificationHandler = (event: Event) => {
            const target = event.target as BluetoothRemoteGATTCharacteristic
            if (target && target.value) {
              try {
                const reassembled = stateReassembler.feed(target.value)
                if (reassembled) {
                  const state = normalizeMotorState(JSON.parse(reassembled))
                  bleStateListeners.forEach(listener => listener(state))
                }
              } catch (e) {
                console.error('BLE notification parse error:', e)
              }
            }
          }
          targetStateChar.addEventListener('characteristicvaluechanged', stateNotificationHandler)
        }
        await withTimeout(
          targetStateChar.startNotifications(),
          BLE_NOTIFICATION_OP_TIMEOUT_MS,
          'start state notifications',
        )
        if (targetRpcChar && !rpcNotificationHandler) {
          rpcNotificationHandler = (event: Event) => {
            const target = event.target as BluetoothRemoteGATTCharacteristic
            if (target && target.value) {
              try {
                const reassembled = rpcReassembler.feed(target.value)
                if (reassembled) {
                  console.log('BLE RPC response:', JSON.parse(reassembled))
                }
              } catch (e) {
                console.error('BLE RPC notification parse error:', e)
              }
            }
          }
          targetRpcChar.addEventListener('characteristicvaluechanged', rpcNotificationHandler)
          await withTimeout(
            targetRpcChar.startNotifications(),
            BLE_NOTIFICATION_OP_TIMEOUT_MS,
            'start rpc notifications',
          )
        }
        bleNotificationStarted = true
      }
    })
  }
  const req = { jsonrpc: '2.0', method: 'subscribe-state', params: { interval_ms: clampedIntervalMs }, id: 1 }
  await writeJson(targetRpcChar, req)
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
        await withTimeout(
          stateChar!.stopNotifications(),
          BLE_NOTIFICATION_OP_TIMEOUT_MS,
          'stop state notifications',
        )
      })
      bleNotificationStarted = false
      stateReassembler.reset()
      detachStateNotificationListener()
    } catch (e) {
      console.warn('Error unsubscribing BLE state:', e)
    }
  }
}
