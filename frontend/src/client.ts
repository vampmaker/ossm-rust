import type {
  MotorControllerConfig,
  PausedControlPayload,
  MotorState,
  PinConfiguration,
  NetworkConfiguration,
  StreamWaypoint
} from './types'

export interface Deferred<T> {
  resolve: (value: T) => void
  reject: (reason?: any) => void
}

/**
 * Single-owner WebSocket manager for OSSM JSON-RPC 2.0 connection.
 *
 * Implements:
 * - Deferred Pattern: Routes JSON-RPC responses and shared live state waiters.
 * - In-flight Promise Lock: Acts as a mutex to prevent concurrent connection attempts.
 * - Idle Timer / Debouncer: Automatically disconnects idle connections after a configurable timeout.
 * - Shared Data Caching: Caches recent MotorState updates for immediate async retrieval.
 */
export class WsDataManager {
  private url: string
  private ws: WebSocket | null = null

  // In-flight Promise lock: prevents multiple simultaneous WebSocket connection attempts
  private connectingPromise: Promise<void> | null = null

  // Routing table: maps JSON-RPC message IDs to pending Deferred coroutines
  private pendingRequests = new Map<number, Deferred<any>>()

  // Deferred waiters waiting for the next shared MotorState push notification
  private pendingStateWaiters = new Set<Deferred<MotorState>>()

  // Cached shared motor state for immediate asynchronous retrieval
  private cachedState: MotorState | null = null
  private cachedStateTime = 0

  // Active push subscribers and heartbeat watchdog
  private stateListeners = new Set<(state: MotorState) => void>()
  private activeIntervalMs = 33
  private watchdogTimer: ReturnType<typeof setInterval> | null = null
  private lastMessageTime = 0
  private isReconnecting = false

  // Timer / Debouncer: closes WebSocket when idle with no active consumers
  private idleTimer: ReturnType<typeof setTimeout> | null = null
  private readonly IDLE_TIMEOUT_MS: number
  private nextId = 1

  constructor(url: string, idleTimeoutMs = 3000) {
    this.url = url
    this.IDLE_TIMEOUT_MS = idleTimeoutMs
  }

  /**
   * Public async function to fetch the latest shared MotorState.
   * Returns immediately if cached state is fresh (within maxAgeMs); otherwise
   * suspends via a Deferred promise until the next WebSocket push or fetch completes.
   */
  public async fetchData(maxAgeMs = 100): Promise<MotorState> {
    this.resetIdleTimer()
    if (this.cachedState && (Date.now() - this.cachedStateTime <= maxAgeMs)) {
      return this.cachedState
    }
    await this.ensureConnected()

    return new Promise<MotorState>((resolve, reject) => {
      const deferred: Deferred<MotorState> = { resolve, reject }
      this.pendingStateWaiters.add(deferred)

      // If no active push subscription is running, explicitly request state via JSON-RPC
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

  /**
   * Singleton connection manager (In-flight Promise mutex).
   * Ensures only one WebSocket handshake is initiated even under concurrent calls.
   */
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

  /**
   * Sends a JSON-RPC 2.0 command over WebSocket and correlates response via Deferred ID map.
   */
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

  /**
   * Dispatches incoming WebSocket messages to push listeners or Deferred request handlers.
   */
  private handleMessage(rawData: string): void {
    try {
      const data = JSON.parse(rawData)
      // 1. Handle server-pushed MotorState notifications ("method": "state")
      if (data.method === 'state' && data.params) {
        this.updateCachedState(data.params as MotorState)
        return
      }
      // 2. Handle JSON-RPC request-response matching via ID
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
    // Wake up all coroutines suspended on shared state updates
    this.pendingStateWaiters.forEach(waiter => waiter.resolve(state))
    this.pendingStateWaiters.clear()
    // Notify active stream subscribers
    this.stateListeners.forEach(listener => listener(state))
  }

  /**
   * Subscribes to periodic state notifications pushed by the device firmware.
   */
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

  /**
   * Unsubscribes a listener or all listeners from periodic state push notifications.
   */
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

  /**
   * Resets the idle debouncer timer. Automatically closes connection after IDLE_TIMEOUT_MS
   * when no active subscribers or pending Deferred requests remain.
   */
  public resetIdleTimer(): void {
    if (this.idleTimer) {
      clearTimeout(this.idleTimer)
      this.idleTimer = null
    }
    this.idleTimer = setTimeout(() => {
      if (this.stateListeners.size === 0 && this.pendingRequests.size === 0 && this.pendingStateWaiters.size === 0 && this.ws) {
        console.log(`WebSocket idle for ${this.IDLE_TIMEOUT_MS}ms with no active consumers; closing connection.`)
        this.close()
      } else if (this.stateListeners.size > 0 || this.pendingRequests.size > 0 || this.pendingStateWaiters.size > 0) {
        this.resetIdleTimer()
      }
    }, this.IDLE_TIMEOUT_MS)
  }

  /**
   * Closes the underlying WebSocket connection and rejects any pending Deferred coroutines.
   */
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
      } catch (e) {
        console.warn('Error closing WebSocket:', e)
      }
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
        console.warn('WebSocket silent timeout detected; triggering automatic reconnection...')
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
 * Encapsulates REST endpoints and WebSocket JSON-RPC communication into a single interface.
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

  public async getPinConfig(): Promise<PinConfiguration> {
    return this.getJson<PinConfiguration>('/pin-config')
  }

  public async setPinConfig(config: PinConfiguration): Promise<PinConfiguration> {
    return this.postJson<PinConfiguration>('/pin-config', config)
  }

  public async getNetworkConfig(): Promise<NetworkConfiguration> {
    return this.getJson<NetworkConfiguration>('/network-config')
  }

  public async setNetworkConfig(config: NetworkConfiguration): Promise<NetworkConfiguration> {
    return this.postJson<NetworkConfiguration>('/network-config', config)
  }

  public async restartDevice(): Promise<void> {
    const response = await fetch(`${this.baseUrl}/restart`, { method: 'POST' })
    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`)
    }
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
