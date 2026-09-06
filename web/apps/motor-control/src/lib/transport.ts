import type { ModbusResponse } from './modbus-rtu'

export type LogDirection = 'TX' | 'RX'

export interface CommLogEntry {
  timestamp: number
  direction: LogDirection
  data: Uint8Array
}

export type LogCallback = (entry: CommLogEntry) => void

export interface ModbusTransport {
  connect(options?: { baudRate?: number; url?: string }): Promise<void>
  disconnect(): Promise<void>
  readonly isConnected: boolean
  sendRequest(frame: Uint8Array, timeoutMs?: number): Promise<ModbusResponse>
  sendRawFrame(
    frame: Uint8Array,
    timeoutMs?: number,
    waitForResponse?: boolean,
  ): Promise<ModbusResponse | null>
  onLog(callback: LogCallback): () => void
}
