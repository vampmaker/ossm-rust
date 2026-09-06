import { i18n } from '../i18n'
import { parseResponse, type ModbusResponse } from './modbus-rtu'
import type { CommLogEntry, LogCallback, LogDirection, ModbusTransport } from './transport'

const PKT_TX = 0
const PKT_RX = 1
const PKT_CFG = 2

function pack(typ: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(3 + payload.length)
  out[0] = typ
  out[1] = payload.length & 0xff
  out[2] = (payload.length >> 8) & 0xff
  out.set(payload, 3)
  return out
}

function unpack(frame: Uint8Array): { typ: number; payload: Uint8Array } | null {
  if (frame.length < 3) return null
  const typ = frame[0]!
  const len = frame[1]! | (frame[2]! << 8)
  if (frame.length < 3 + len) return null
  return { typ, payload: frame.slice(3, 3 + len) }
}

/**
 * Raw RS-485 WebSocket client for OSSM `/ws/rs485` (TX/RX/CFG packets).
 */
export class WebSocketRs485Client implements ModbusTransport {
  private ws: WebSocket | null = null
  private isClosing = false
  private url = 'ws://ossm.lan/ws/rs485'
  private baudRate = 115200
  private rxBuffer = new Uint8Array(1024)
  private rxBufferLength = 0

  private requestQueue: Array<{
    frame: Uint8Array
    resolve: (res: ModbusResponse | null) => void
    reject: (err: Error) => void
    timeoutMs: number
    expectResponse: boolean
  }> = []
  private isProcessingQueue = false
  private currentResolve: ((res: ModbusResponse | null) => void) | null = null
  private currentReject: ((err: Error) => void) | null = null
  private currentTimeout: number | null = null
  private currentExpectedDevice: number | null = null
  private currentExpectedFunction: number | null = null

  private logCallbacks: Set<LogCallback> = new Set()

  public onLog(callback: LogCallback) {
    this.logCallbacks.add(callback)
    return () => this.logCallbacks.delete(callback)
  }

  private emitLog(direction: LogDirection, data: Uint8Array) {
    const entry: CommLogEntry = {
      timestamp: Date.now(),
      direction,
      data: new Uint8Array(data),
    }
    this.logCallbacks.forEach((cb) => cb(entry))
  }

  public async connect(options?: { baudRate?: number; url?: string }) {
    if (this.ws) {
      await this.disconnect()
    }
    const url = options?.url?.trim() || this.url
    this.url = url
    this.baudRate = options?.baudRate ?? this.baudRate
    this.isClosing = false

    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(url)
      ws.binaryType = 'arraybuffer'
      const onError = () => {
        cleanup()
        reject(new Error(String(i18n.global.t('errors.wsConnectFailed', { url }))))
      }
      const onOpen = () => {
        cleanup()
        this.ws = ws
        ws.onmessage = (ev) => this.handleMessage(ev)
        ws.onclose = () => {
          if (!this.isClosing) {
            void this.disconnect()
          }
        }
        ws.onerror = () => {
          if (!this.isClosing) {
            void this.disconnect()
          }
        }
        const cfg = new TextEncoder().encode(JSON.stringify({ baud: this.baudRate }))
        ws.send(pack(PKT_CFG, cfg))
        resolve()
      }
      const cleanup = () => {
        ws.removeEventListener('open', onOpen)
        ws.removeEventListener('error', onError)
      }
      ws.addEventListener('open', onOpen)
      ws.addEventListener('error', onError)
    })
  }

  public async disconnect() {
    this.isClosing = true
    const err = new Error(String(i18n.global.t('errors.wsDisconnected')))
    if (this.currentReject) {
      this.currentReject(err)
      this.cleanupCurrentRequest()
    }
    while (this.requestQueue.length > 0) {
      const req = this.requestQueue.shift()
      if (req) req.reject(err)
    }
    this.isProcessingQueue = false
    if (this.ws) {
      try {
        this.ws.close()
      } catch {
        /* ignore */
      }
      this.ws = null
    }
  }

  public get isConnected(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN && !this.isClosing
  }

  public setBaudRate(baud: number) {
    this.baudRate = baud
    if (!this.isConnected || !this.ws) {
      return
    }
    const cfg = new TextEncoder().encode(JSON.stringify({ baud: this.baudRate }))
    this.ws.send(pack(PKT_CFG, cfg))
  }

  public async sendRequest(frame: Uint8Array, timeoutMs: number = 800): Promise<ModbusResponse> {
    const response = await this.sendRawFrame(frame, timeoutMs, true)
    if (!response) {
      throw new Error(String(i18n.global.t('errors.noResponse')))
    }
    return response
  }

  public async sendRawFrame(
    frame: Uint8Array,
    timeoutMs: number = 800,
    waitForResponse: boolean = true,
  ): Promise<ModbusResponse | null> {
    if (!this.isConnected) {
      throw new Error(String(i18n.global.t('errors.notConnected')))
    }
    return new Promise((resolve, reject) => {
      this.requestQueue.push({
        frame,
        resolve,
        reject,
        timeoutMs,
        expectResponse: waitForResponse,
      })
      this.processQueue()
    })
  }

  private processQueue() {
    if (this.isProcessingQueue || this.requestQueue.length === 0 || !this.ws) {
      return
    }
    this.isProcessingQueue = true
    const req = this.requestQueue.shift()!

    try {
      this.rxBufferLength = 0
      this.currentExpectedDevice = req.frame[0] ?? null
      this.currentExpectedFunction = (req.frame[1] ?? 0) & 0x7f
      this.emitLog('TX', req.frame)
      this.ws.send(pack(PKT_TX, req.frame))

      if (!req.expectResponse) {
        req.resolve(null)
        this.cleanupCurrentRequest()
        setTimeout(() => this.processQueue(), 0)
        return
      }

      this.currentResolve = req.resolve
      this.currentReject = req.reject
      this.currentTimeout = window.setTimeout(() => {
        if (this.currentReject) {
          this.currentReject(new Error(String(i18n.global.t('errors.timeout'))))
          this.cleanupCurrentRequest()
          this.processQueue()
        }
      }, req.timeoutMs)
    } catch (e) {
      req.reject(e as Error)
      this.cleanupCurrentRequest()
      this.processQueue()
    }
  }

  private cleanupCurrentRequest() {
    this.currentResolve = null
    this.currentReject = null
    this.currentExpectedDevice = null
    this.currentExpectedFunction = null
    if (this.currentTimeout) {
      clearTimeout(this.currentTimeout)
      this.currentTimeout = null
    }
    this.isProcessingQueue = false
  }

  private handleMessage(ev: MessageEvent) {
    let data: Uint8Array
    if (ev.data instanceof ArrayBuffer) {
      data = new Uint8Array(ev.data)
    } else {
      return
    }
    const pkt = unpack(data)
    if (!pkt || pkt.typ !== PKT_RX) {
      return
    }
    this.emitLog('RX', pkt.payload)
    if (!this.currentResolve) {
      return
    }
    if (this.rxBufferLength + pkt.payload.length > this.rxBuffer.length) {
      this.rxBufferLength = 0
    }
    this.rxBuffer.set(pkt.payload, this.rxBufferLength)
    this.rxBufferLength += pkt.payload.length
    const response = parseResponse(this.rxBuffer.slice(0, this.rxBufferLength))
    if (!response) {
      return
    }
    if (
      (this.currentExpectedDevice !== null &&
        response.deviceAddress !== this.currentExpectedDevice) ||
      (this.currentExpectedFunction !== null &&
        response.functionCode !== this.currentExpectedFunction)
    ) {
      this.currentReject?.(new Error(String(i18n.global.t('errors.unexpectedResponse'))))
      this.cleanupCurrentRequest()
      setTimeout(() => this.processQueue(), 0)
      return
    }
    this.currentResolve(response)
    this.cleanupCurrentRequest()
    setTimeout(() => this.processQueue(), 0)
  }
}
