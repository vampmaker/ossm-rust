import { parseResponse, type ModbusResponse } from './modbus-rtu';
import type { CommLogEntry, LogCallback, LogDirection, ModbusTransport } from './transport';

export type { CommLogEntry, LogCallback, LogDirection, ModbusTransport } from './transport';

export class SerialModbusClient implements ModbusTransport {
  private port: any = null;
  private writer: WritableStreamDefaultWriter<Uint8Array> | null = null;
  private reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  private readLoopPromise: Promise<void> | null = null;
  private isClosing = false;
  
  private rxBuffer = new Uint8Array(1024);
  private rxBufferLength = 0;
  
  private requestQueue: Array<{
    frame: Uint8Array;
    resolve: (res: ModbusResponse | null) => void;
    reject: (err: Error) => void;
    timeoutMs: number;
    expectResponse: boolean;
  }> = [];
  private isProcessingQueue = false;
  
  private currentResolve: ((res: ModbusResponse | null) => void) | null = null;
  private currentReject: ((err: Error) => void) | null = null;
  private currentTimeout: number | null = null;
  private currentExpectedDevice: number | null = null;
  private currentExpectedFunction: number | null = null;
  
  private logCallbacks: Set<LogCallback> = new Set();
  
  // Inter-frame delay timing
  private lastRxTime = 0;
  private readonly interFrameDelayMs = 4; // ~3.5 chars at 9600 baud is ~4ms

  public onLog(callback: LogCallback) {
    this.logCallbacks.add(callback);
    return () => this.logCallbacks.delete(callback);
  }

  private emitLog(direction: LogDirection, data: Uint8Array) {
    const entry: CommLogEntry = {
      timestamp: Date.now(),
      direction,
      data: new Uint8Array(data)
    };
    this.logCallbacks.forEach(cb => cb(entry));
  }

  public async connect(options?: { baudRate?: number; url?: string }) {
    const baudRate = options?.baudRate ?? 115200;
    if (this.port) {
      await this.disconnect();
    }
    
    try {
      this.port = await (navigator as any).serial.requestPort();
      await this.port.open({ baudRate });
      
      this.writer = this.port.writable.getWriter();
      this.reader = this.port.readable.getReader();
      this.isClosing = false;
      
      this.readLoopPromise = this.readLoop();
    } catch (e) {
      this.port = null;
      throw e;
    }
  }

  public async disconnect() {
    this.isClosing = true;
    
    if (this.reader) {
      await this.reader.cancel();
      this.reader = null;
    }
    
    if (this.writer) {
      await this.writer.close();
      this.writer = null;
    }
    
    if (this.readLoopPromise) {
      await this.readLoopPromise;
      this.readLoopPromise = null;
    }
    
    if (this.port) {
      await this.port.close();
      this.port = null;
    }
    
    // Reject all pending requests
    const err = new Error('Port disconnected');
    if (this.currentReject) {
      this.currentReject(err);
      this.currentReject = null;
      this.currentResolve = null;
      if (this.currentTimeout) {
        clearTimeout(this.currentTimeout);
        this.currentTimeout = null;
      }
    }
    
    while (this.requestQueue.length > 0) {
      const req = this.requestQueue.shift();
      if (req) req.reject(err);
    }
    
    this.isProcessingQueue = false;
  }

  public get isConnected(): boolean {
    return this.port !== null && !this.isClosing;
  }

  public async sendRequest(frame: Uint8Array, timeoutMs: number = 200): Promise<ModbusResponse> {
    const response = await this.sendRawFrame(frame, timeoutMs, true);
    if (!response) {
      throw new Error('No response received');
    }
    return response;
  }

  public async sendRawFrame(
    frame: Uint8Array,
    timeoutMs: number = 800,
    waitForResponse: boolean = true,
  ): Promise<ModbusResponse | null> {
    if (!this.isConnected) {
      throw new Error('Not connected');
    }

    return new Promise((resolve, reject) => {
      this.requestQueue.push({ frame, resolve, reject, timeoutMs, expectResponse: waitForResponse });
      this.processQueue();
    });
  }

  private async processQueue() {
    if (this.isProcessingQueue || this.requestQueue.length === 0 || !this.writer) {
      return;
    }

    this.isProcessingQueue = true;
    const req = this.requestQueue.shift()!;

    try {
      // Ensure inter-frame delay
      const now = performance.now();
      const timeSinceLastRx = now - this.lastRxTime;
      if (timeSinceLastRx < this.interFrameDelayMs) {
        await new Promise(r => setTimeout(r, this.interFrameDelayMs - timeSinceLastRx));
      }

      this.rxBufferLength = 0; // Clear buffer before sending
      this.currentExpectedDevice = req.frame[0] ?? null;
      this.currentExpectedFunction = (req.frame[1] ?? 0) & 0x7F;

      this.emitLog('TX', req.frame);
      await this.writer.write(req.frame);

      if (!req.expectResponse) {
        req.resolve(null);
        this.cleanupCurrentRequest();
        setTimeout(() => this.processQueue(), 0);
        return;
      }

      this.currentResolve = req.resolve;
      this.currentReject = req.reject;
      this.currentTimeout = window.setTimeout(() => {
        if (this.currentReject) {
          this.currentReject(new Error('Timeout'));
          this.cleanupCurrentRequest();
          this.processQueue();
        }
      }, req.timeoutMs);
    } catch (e) {
      req.reject(e as Error);
      this.cleanupCurrentRequest();
      this.processQueue();
    }
  }

  private cleanupCurrentRequest() {
    this.currentResolve = null;
    this.currentReject = null;
    this.currentExpectedDevice = null;
    this.currentExpectedFunction = null;
    if (this.currentTimeout) {
      clearTimeout(this.currentTimeout);
      this.currentTimeout = null;
    }
    this.isProcessingQueue = false;
  }

  private async readLoop() {
    if (!this.reader) return;

    try {
      while (!this.isClosing) {
        const { value, done } = await this.reader.read();
        if (done) break;
        if (value) {
          this.lastRxTime = performance.now();
          this.handleRxData(value);
        }
      }
    } catch (e) {
      if (!this.isClosing) {
        console.error('Serial read error:', e);
        this.disconnect();
      }
    }
  }

  private handleRxData(data: Uint8Array) {
    if (this.rxBufferLength + data.length > this.rxBuffer.length) {
      // Buffer overflow, reset
      this.rxBufferLength = 0;
      console.warn('RX buffer overflow, resetting');
    }

    this.rxBuffer.set(data, this.rxBufferLength);
    this.rxBufferLength += data.length;

    if (this.currentResolve) {
      const currentData = this.rxBuffer.slice(0, this.rxBufferLength);
      const response = parseResponse(currentData);
      
      if (response) {
        this.emitLog('RX', currentData);
        if (
          (this.currentExpectedDevice !== null && response.deviceAddress !== this.currentExpectedDevice) ||
          (this.currentExpectedFunction !== null && response.functionCode !== this.currentExpectedFunction)
        ) {
          // Drop unexpected frame and keep waiting for the matching response.
          this.rxBufferLength = 0;
          return;
        }
        this.currentResolve(response);
        this.cleanupCurrentRequest();
        
        // Use setTimeout to yield execution and prevent stack overflow
        setTimeout(() => this.processQueue(), 0);
      }
    }
  }
}
