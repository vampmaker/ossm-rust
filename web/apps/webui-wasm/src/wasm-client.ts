import type {
  OssmControl,
  LinkStats,
  MotorControllerConfig,
  MotorState,
  PausedControlPayload,
  StreamWaypoint,
} from '@ossm/shared'
import init, { OssmShell } from './pkg/ossm_wasm'
import wasmBytes from './pkg/ossm_wasm_bg.wasm?arraybuffer'
import { openRs485Ws, openSerialPort, openModbusWs, type ByteBus } from './rtu-bus'

const IDB_NAME = 'ossm-wasm'
const IDB_STORE = 'motor'

export type WasmConnectKind = 'serial' | 'rs485' | 'modbus-ws' | 'mock'

export type WasmClientEvent = {
  type: 'state' | 'homing' | 'ready' | 'error' | 'disconnected'
  payload?: unknown
}

function nowUs(): bigint {
  return BigInt(Math.floor(performance.now() * 1000))
}

async function loadWasm(): Promise<void> {
  await init({ module_or_path: wasmBytes })
}

async function idbGet(): Promise<string | null> {
  return new Promise((resolve) => {
    const req = indexedDB.open(IDB_NAME, 1)
    req.onupgradeneeded = () => {
      req.result.createObjectStore(IDB_STORE)
    }
    req.onsuccess = () => {
      const db = req.result
      const tx = db.transaction(IDB_STORE, 'readonly')
      const g = tx.objectStore(IDB_STORE).get('config')
      g.onsuccess = () => resolve((g.result as string) ?? null)
      g.onerror = () => resolve(null)
    }
    req.onerror = () => resolve(null)
  })
}

async function idbSet(json: string): Promise<void> {
  await new Promise<void>((resolve) => {
    const req = indexedDB.open(IDB_NAME, 1)
    req.onupgradeneeded = () => {
      req.result.createObjectStore(IDB_STORE)
    }
    req.onsuccess = () => {
      const db = req.result
      const tx = db.transaction(IDB_STORE, 'readwrite')
      tx.objectStore(IDB_STORE).put(json, 'config')
      tx.oncomplete = () => resolve()
      tx.onerror = () => resolve()
    }
    req.onerror = () => resolve()
  })
}

export class WasmClient implements OssmControl {
  private shell: OssmShell | null = null
  private bus: ByteBus | null = null
  private looping = false
  private loopPromise: Promise<void> | null = null
  private exclusiveChain: Promise<void> = Promise.resolve()
  private stateListeners = new Set<(state: MotorState) => void>()
  private eventListeners = new Set<(ev: WasmClientEvent) => void>()
  /** `home()` holds `&mut` the wasm shell; skip `recordBusSample` during homing. */
  private recordBusSamples = true

  onEvent(cb: (ev: WasmClientEvent) => void): () => void {
    this.eventListeners.add(cb)
    return () => this.eventListeners.delete(cb)
  }

  private emit(ev: WasmClientEvent): void {
    this.eventListeners.forEach((cb) => cb(ev))
  }

  private exclusive<T>(fn: () => Promise<T> | T): Promise<T> {
    const run = this.exclusiveChain.then(() => fn())
    this.exclusiveChain = run.then(
      () => undefined,
      () => undefined,
    )
    return run
  }

  private emitState(): void {
    if (!this.shell) return
    const json = this.shell.stateNotificationJson()
    if (!json) return
    try {
      const parsed = JSON.parse(json) as { method?: string; params?: MotorState }
      const state = parsed.params
      if (parsed.method === 'state' && state) {
        this.stateListeners.forEach((cb) => cb(state))
        this.emit({ type: 'state', payload: state })
      }
    } catch {
      /* ignore */
    }
  }

  private async ensureShell(): Promise<OssmShell> {
    if (this.shell) return this.shell
    return this.exclusive(async () => {
      if (this.shell) return this.shell
      await loadWasm()
      const shell = new OssmShell()
      const saved = await idbGet()
      if (saved) shell.loadMotorJson(saved)
      this.shell = shell
      return shell
    })
  }

  private async attachBus(
    next: ByteBus | null,
    link?: { transport: string; endpoint: string },
  ): Promise<void> {
    if (this.bus) {
      try {
        await this.bus.close()
      } catch {
        /* ignore */
      }
    }
    this.bus = next
    if (!this.shell) return
    if (!next) {
      this.shell.clearExchange()
      return
    }
    if (link) {
      this.shell.setLinkInfo(link.transport, link.endpoint)
    }
    this.shell.pllReset()
    const b = next
    this.shell.setExchange(async (req: Uint8Array, timeoutMs: number) => {
      const t0 = performance.now()
      const now = nowUs()
      try {
        const rsp = await b.exchange(req, timeoutMs)
        if (this.recordBusSamples) {
          this.shell!.recordBusSample(
            Math.round((performance.now() - t0) * 1000),
            true,
            req.length,
            rsp.length,
            now,
          )
        }
        return rsp
      } catch (e) {
        if (this.recordBusSamples) {
          this.shell!.recordBusSample(
            Math.round((performance.now() - t0) * 1000),
            false,
            req.length,
            0,
            now,
          )
        }
        throw e
      }
    })
  }

  private async motionLoop(): Promise<void> {
    this.looping = true
    while (this.looping && this.shell) {
      const step = await this.exclusive(() => {
        if (!this.shell) return { write: false, pos: 0, persist: undefined as string | undefined }
        const now = nowUs()
        const pos = this.shell.tick(now)
        if (this.shell.shouldNotify(now)) this.emitState()
        const persist = this.shell.takePersistJson(now)
        return { write: this.shell.writeBus(), pos, persist }
      })
      if (step.persist) await idbSet(step.persist)
      if (step.write && this.shell) {
        try {
          await this.exclusive(() => this.shell!.writePosition(step.pos))
        } catch {
          /* CRC miss / timeout — tick continues */
        }
        const us = Number(this.shell.nextWakeUs(nowUs()))
        if (us > 0) {
          await new Promise((r) => setTimeout(r, us / 1000))
        }
      } else {
        await new Promise((r) => setTimeout(r, 5))
      }
    }
  }

  private async stopLoop(): Promise<void> {
    this.looping = false
    if (this.loopPromise) {
      await this.loopPromise.catch(() => undefined)
      this.loopPromise = null
    }
  }

  private async connectKind(kind: WasmConnectKind, opts: { baud?: number; url?: string; port?: SerialPort }): Promise<void> {
    await this.ensureShell()
    await this.stopLoop()
    await this.exclusive(async () => {
      const baud = opts.baud ?? 115200
      if (kind === 'mock') {
        await this.attachBus(null)
        this.shell!.applyMockHome()
        this.emit({ type: 'ready', payload: { kind: 'mock' } })
        this.loopPromise = this.motionLoop()
        return
      }
      this.emit({ type: 'homing' })
      this.recordBusSamples = false
      try {
        if (kind === 'serial') {
          const port = opts.port
          if (!port || typeof port.open !== 'function') {
            throw new Error('serial port missing')
          }
          await this.attachBus(await openSerialPort(port, baud), {
            transport: 'serial',
            endpoint: 'web-serial',
          })
        } else if (kind === 'rs485') {
          const url = opts.url
          if (!url) throw new Error('rs485 url required')
          await this.attachBus(await openRs485Ws(url, baud), {
            transport: 'rs485-ws',
            endpoint: url,
          })
        } else if (kind === 'modbus-ws') {
          const url = opts.url
          if (!url) throw new Error('modbus ws url required')
          await this.attachBus(await openModbusWs(url), {
            transport: 'modbus-ws',
            endpoint: url,
          })
        } else {
          throw new Error(`unknown connect kind ${kind}`)
        }
        await this.shell!.home(nowUs())
        this.recordBusSamples = true
        this.emit({ type: 'ready', payload: { kind } })
        this.loopPromise = this.motionLoop()
      } catch (e) {
        this.recordBusSamples = true
        await this.attachBus(null)
        const msg = e instanceof Error ? e.message : String(e)
        this.emit({ type: 'error', payload: { message: msg } })
        throw e instanceof Error ? e : new Error(msg)
      }
    })
  }

  async connectSerial(port: SerialPort, baud = 115200): Promise<void> {
    await this.connectKind('serial', { port, baud })
  }

  async connectRs485(url: string, baud = 115200): Promise<void> {
    await this.connectKind('rs485', { url, baud })
  }

  async connectModbusWs(url: string): Promise<void> {
    await this.connectKind('modbus-ws', { url })
  }

  async connectMock(): Promise<void> {
    await this.connectKind('mock', {})
  }

  async disconnect(): Promise<void> {
    await this.ensureShell()
    await this.stopLoop()
    await this.exclusive(async () => {
      await this.attachBus(null)
      this.emit({ type: 'disconnected' })
    })
  }

  async request(cmd: string, params?: unknown): Promise<unknown> {
    await this.ensureShell()
    if (cmd === 'rpc') {
      return this.rpc(params as Record<string, unknown>)
    }
    if (cmd === 'paused') {
      return this.setPaused(params as PausedControlPayload)
    }
    throw new Error(`unknown cmd ${cmd}`)
  }

  private async rpc(params: Record<string, unknown>): Promise<unknown> {
    await this.ensureShell()
    return this.exclusive(() => {
      const req = JSON.stringify({ jsonrpc: '2.0', id: params.id ?? 1, ...params })
      const body = this.shell!.rpc(req)
      const parsed = JSON.parse(body) as { result?: unknown; error?: { message: string } }
      if (parsed.error) throw new Error(parsed.error.message)
      if (this.shell!.lastAction() === 'subscribe') this.emitState()
      return parsed.result
    })
  }

  async getConfig(): Promise<MotorControllerConfig> {
    const state = await this.getState()
    return state.config
  }

  async setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
    return (await this.rpc({
      jsonrpc: '2.0',
      method: 'set-config',
      params: config,
    })) as MotorControllerConfig
  }

  async setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
    await this.ensureShell()
    return this.exclusive(() => {
      const json = this.shell!.paused(JSON.stringify(payload))
      return JSON.parse(json) as MotorControllerConfig
    })
  }

  async getState(): Promise<MotorState> {
    await this.ensureShell()
    return (await this.rpc({
      jsonrpc: '2.0',
      method: 'get-state',
    })) as MotorState
  }

  getLinkStats(): LinkStats {
    if (!this.shell) {
      throw new Error('not connected')
    }
    return JSON.parse(this.shell.linkStatsJson(nowUs())) as LinkStats
  }

  async subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
    this.stateListeners.add(onState)
    await this.ensureShell()
    await this.rpc({
      jsonrpc: '2.0',
      method: 'subscribe-state',
      params: { interval_ms: intervalMs },
    })
  }

  async unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
    if (onState) this.stateListeners.delete(onState)
    else this.stateListeners.clear()
    if (this.stateListeners.size === 0) {
      await this.ensureShell()
      await this.rpc({ jsonrpc: '2.0', method: 'unsubscribe-state' })
    }
  }

  async sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp = false): Promise<void> {
    const method = resetTimestamp ? 'set-waypoints' : 'append-waypoints'
    const params = resetTimestamp ? { waypoints, 'reset-timestamp': true } : waypoints
    await this.ensureShell()
    await this.rpc({ jsonrpc: '2.0', method, params })
  }

  async restart(): Promise<void> {
    await this.ensureShell()
    await this.stopLoop()
    await this.exclusive(async () => {
      if (this.bus) {
        this.emit({ type: 'homing' })
        await this.shell!.home(nowUs())
        this.emit({ type: 'ready' })
      } else {
        this.shell!.applyMockHome()
      }
    })
    this.loopPromise = this.motionLoop()
  }

  terminate(): void {
    this.looping = false
    void this.attachBus(null)
  }
}
