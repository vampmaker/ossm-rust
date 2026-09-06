import { i18n } from '../i18n'
import { parseResponse, type ModbusResponse } from './modbus-rtu'
import type { CommLogEntry, LogCallback, LogDirection, ModbusTransport } from './transport'

/**
 * Binary WebSocket Modbus RTU pass-through client for OSSM `/ws/modbus`.
 * Each request/response is a full RTU frame (including CRC).
 */
export class WebSocketModbusClient implements ModbusTransport {
  private ws: WebSocket | null = null
  private isClosing = false
  private url = 'ws://ossm.lan/ws/modbus'

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

  private async processQueue() {
    if (this.isProcessingQueue || this.requestQueue.length === 0 || !this.ws) {
      return
    }
    this.isProcessingQueue = true
    const req = this.requestQueue.shift()!

    try {
      this.currentExpectedDevice = req.frame[0] ?? null
      this.currentExpectedFunction = (req.frame[1] ?? 0) & 0x7f
      this.emitLog('TX', req.frame)
      this.ws.send(req.frame)

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
    } else if (ev.data instanceof Blob) {
      // Unexpected; drop
      return
    } else if (typeof ev.data === 'string') {
      const bytes = new TextEncoder().encode(ev.data)
      data = bytes
    } else {
      return
    }

    this.emitLog('RX', data)
    if (!this.currentResolve) {
      return
    }
    const response = parseResponse(data)
    if (!response) {
      this.currentReject?.(new Error(String(i18n.global.t('errors.invalidRtuResponse'))))
      this.cleanupCurrentRequest()
      setTimeout(() => this.processQueue(), 0)
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
