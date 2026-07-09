export interface MotorControllerConfig {
  bpm: number
  depth: number
  depth_top: boolean
  reversed: boolean
  wave_func: 'sine' | 'square' | 'triangle' | 'sawtooth' | 'spline'
  sharpness: number
  spline_points: number[]
  paused: boolean
  paused_position: number
  streaming: boolean
}

export interface PausedControlPayload {
  paused: boolean
  paused_position: number
}

export interface MotorState {
  config: MotorControllerConfig
  t: number
  x: number
  y: number
  shaped_y: number
  position: number
  speed: number
  update_history?: number[]
  position_history?: number[]
  pos_min?: number
  pos_max?: number
  ups?: number
  dt_min_ms?: number
  dt_max_ms?: number
  dt_avg_ms?: number
  dt_mdev_ms?: number
}

export interface StreamWaypoint {
  ts?: number
  pos?: number
  vel?: number
}

export interface Deferred<T> {
  resolve: (value: T) => void
  reject: (reason?: any) => void
}

/**
 * Single-owner WebSocket manager for OSSM JSON-RPC 2.0 connection.
 */
export class WsDataManager {
  private url: string
  private ws: WebSocket | null = null
  private connectingPromise: Promise<void> | null = null
  private pendingRequests = new Map<number, Deferred<any>>()
  private pendingStateWaiters = new Set<Deferred<MotorState>>()
  private cachedState: MotorState | null = null
  private cachedStateTime = 0
  private stateListeners = new Set<(state: MotorState) => void>()
  private activeIntervalMs = 33
  private watchdogTimer: ReturnType<typeof setInterval> | null = null
  private lastMessageTime = 0
  private isReconnecting = false
  private idleTimer: ReturnType<typeof setTimeout> | null = null
  private readonly IDLE_TIMEOUT_MS: number
  private nextId = 1

  constructor(url: string, idleTimeoutMs = 3000) {
    this.url = url
    this.IDLE_TIMEOUT_MS = idleTimeoutMs
  }

  public async fetchData(maxAgeMs = 100): Promise<MotorState> {
    this.resetIdleTimer()
    if (this.cachedState && (Date.now() - this.cachedStateTime <= maxAgeMs)) {
      return this.cachedState
    }
    await this.ensureConnected()

    return new Promise<MotorState>((resolve, reject) => {
      const deferred: Deferred<MotorState> = { resolve, reject }
      this.pendingStateWaiters.add(deferred)

      if (this.stateListeners.size === 0) {
        this.sendRpc('get-state').then(state => {
          if (state) {
            this.updateCachedState(state as MotorState)
          }
        }).catch(err => {
          this.pendingStateWaiters.delete(deferred)
          reject(err)
        })
      }
    })
  }

  public ensureConnected(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      return Promise.resolve()
    }
    if (this.connectingPromise) {
      return this.connectingPromise
    }

    this.connectingPromise = new Promise((resolve, reject) => {
      const ws = new WebSocket(this.url)
      const timeout = setTimeout(() => {
        this.connectingPromise = null
        try {
          ws.onclose = null
          ws.close()
        } catch (e) {}
        reject(new Error('WebSocket connection timed out'))
      }, 3000)

      ws.onopen = () => {
        clearTimeout(timeout)
        this.connectingPromise = null
        this.ws = ws
        this.lastMessageTime = Date.now()
        this.resetIdleTimer()
        resolve()
      }

      ws.onmessage = (event) => {
        this.lastMessageTime = Date.now()
        this.resetIdleTimer()
        this.handleMessage(event.data)
      }

      ws.onerror = (err) => {
        clearTimeout(timeout)
        this.connectingPromise = null
        reject(err)
      }

      ws.onclose = () => {
        clearTimeout(timeout)
        this.connectingPromise = null
        this.ws = null
        this.stopWatchdog()
        this.flushPendingRequests(new Error('WebSocket disconnected unexpectedly'))
        if (this.stateListeners.size > 0) {
          void this.triggerReconnect()
        }
      }
    })

    return this.connectingPromise
  }

  public async sendRpc(method: string, params?: any): Promise<any> {
    this.resetIdleTimer()
    await this.ensureConnected()
    const id = this.nextId++
    const payload = {
      jsonrpc: '2.0',
      method,
      params,
      id
    }
    return new Promise((resolve, reject) => {
      this.pendingRequests.set(id, { resolve, reject })
      try {
        this.ws!.send(JSON.stringify(payload))
      } catch (err) {
        this.pendingRequests.delete(id)
        reject(err)
      }
    })
  }

  private handleMessage(rawData: string): void {
    try {
      const data = JSON.parse(rawData)
      if (data.method === 'state' && data.params) {
        this.updateCachedState(data.params as MotorState)
        return
      }
      if (typeof data.id === 'number') {
        const pending = this.pendingRequests.get(data.id)
        if (pending) {
          this.pendingRequests.delete(data.id)
          if (data.error) {
            pending.reject(new Error(data.error.message || JSON.stringify(data.error)))
          } else {
            pending.resolve(data.result)
          }
        }
      }
    } catch (e) {
      console.error('Failed to parse WebSocket message', e)
    }
  }

  private updateCachedState(state: MotorState): void {
    this.cachedState = state
    this.cachedStateTime = Date.now()
    this.pendingStateWaiters.forEach(waiter => waiter.resolve(state))
    this.pendingStateWaiters.clear()
    this.stateListeners.forEach(listener => listener(state))
  }

  public async subscribe(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
    this.resetIdleTimer()
    this.stateListeners.add(onState)
    this.activeIntervalMs = intervalMs
    await this.ensureConnected()
    this.startWatchdog()
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({
        jsonrpc: '2.0',
        method: 'subscribe-state',
        params: { interval_ms: intervalMs },
        id: this.nextId++
      }))
    }
  }

  public async unsubscribe(onState?: (state: MotorState) => void): Promise<void> {
    if (onState) {
      this.stateListeners.delete(onState)
    } else {
      this.stateListeners.clear()
    }
    if (this.stateListeners.size === 0) {
      this.stopWatchdog()
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        try {
          this.ws.send(JSON.stringify({
            jsonrpc: '2.0',
            method: 'unsubscribe-state',
            id: this.nextId++
          }))
        } catch (e) {
          console.warn('Error sending unsubscribe-state command:', e)
        }
      }
      this.resetIdleTimer()
    }
  }

  public resetIdleTimer(): void {
    if (this.idleTimer) {
      clearTimeout(this.idleTimer)
      this.idleTimer = null
    }
    this.idleTimer = setTimeout(() => {
      if (this.stateListeners.size === 0 && this.pendingRequests.size === 0 && this.pendingStateWaiters.size === 0 && this.ws) {
        this.close()
      } else if (this.stateListeners.size > 0 || this.pendingRequests.size > 0 || this.pendingStateWaiters.size > 0) {
        this.resetIdleTimer()
      }
    }, this.IDLE_TIMEOUT_MS)
  }

  public close(): void {
    if (this.idleTimer) {
      clearTimeout(this.idleTimer)
      this.idleTimer = null
    }
    this.stopWatchdog()
    if (this.ws) {
      try {
        this.ws.onclose = null
        this.ws.close()
      } catch (e) {}
      this.ws = null
    }
    this.connectingPromise = null
    this.flushPendingRequests(new Error('WebSocket closed'))
  }

  private flushPendingRequests(error: Error): void {
    this.pendingRequests.forEach(pending => pending.reject(error))
    this.pendingRequests.clear()
    this.pendingStateWaiters.forEach(waiter => waiter.reject(error))
    this.pendingStateWaiters.clear()
  }

  private startWatchdog(): void {
    if (this.watchdogTimer) return
    this.watchdogTimer = setInterval(() => {
      if (this.stateListeners.size === 0) {
        this.stopWatchdog()
        return
      }
      const silentTimeoutMs = Math.max(2000, this.activeIntervalMs * 10)
      if (Date.now() - this.lastMessageTime > silentTimeoutMs) {
        void this.triggerReconnect()
      }
    }, 500)
  }

  private stopWatchdog(): void {
    if (this.watchdogTimer) {
      clearInterval(this.watchdogTimer)
      this.watchdogTimer = null
    }
  }

  private async triggerReconnect(): Promise<void> {
    if (this.isReconnecting || this.stateListeners.size === 0) return
    this.isReconnecting = true
    this.close()
    while (this.stateListeners.size > 0) {
      try {
        await this.ensureConnected()
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
          this.ws.send(JSON.stringify({
            jsonrpc: '2.0',
            method: 'subscribe-state',
            params: { interval_ms: this.activeIntervalMs },
            id: this.nextId++
          }))
        }
        break
      } catch (e) {
        await new Promise(resolve => setTimeout(resolve, 1000))
      }
    }
    this.isReconnecting = false
  }
}

/**
 * High-level OSSM Client SDK suitable for standalone applications or browser usage.
 */
export class OssmClient {
  public readonly baseUrl: string
  public readonly wsManager: WsDataManager

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/$/, '')
    const urlObj = new URL(this.baseUrl)
    const wsProtocol = urlObj.protocol === 'https:' ? 'wss:' : 'ws:'
    const wsUrl = `${wsProtocol}//${urlObj.host}/ws/command`
    this.wsManager = new WsDataManager(wsUrl)
  }

  private async getJson<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`)
    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }
    return await response.json()
  }

  private async postJson<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body)
    })
    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }
    return await response.json()
  }

  public async getConfig(): Promise<MotorControllerConfig> {
    return this.getJson<MotorControllerConfig>('/config')
  }

  public async setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
    return this.postJson<MotorControllerConfig>('/config', config)
  }

  public async setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
    return this.postJson<MotorControllerConfig>('/paused', payload)
  }

  public async getState(): Promise<MotorState> {
    return this.getJson<MotorState>('/state')
  }

  public async getSharedState(maxAgeMs = 100): Promise<MotorState> {
    return this.wsManager.fetchData(maxAgeMs)
  }

  public async subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
    return this.wsManager.subscribe(onState, intervalMs)
  }

  public async unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
    return this.wsManager.unsubscribe(onState)
  }

  public async sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp = false): Promise<void> {
    const method = resetTimestamp ? 'set-waypoints' : 'append-waypoints'
    const params = resetTimestamp ? { waypoints, 'reset-timestamp': true } : waypoints
    await this.wsManager.sendRpc(method, params)
  }

  public close(): void {
    this.wsManager.close()
  }
}
