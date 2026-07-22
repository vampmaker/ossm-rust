import { ref, computed } from 'vue'

export type ConnectionStatus = 'DISCONNECTED' | 'CONNECTING' | 'CONNECTED' | 'RECONNECTING'
export type ControlRequestStatus = 'IDLE' | 'MUTATING' | 'ERROR'
export type EngineMode = 'IDLE' | 'MACRO' | 'FUNSCRIPT'

export class ConnectionStateMachine {
  private _connectionStatus = ref<ConnectionStatus>('DISCONNECTED')
  private _controlStatus = ref<ControlRequestStatus>('IDLE')
  private _engineMode = ref<EngineMode>('IDLE')
  private _inFlightCount = ref<number>(0)
  private _localEditCount = ref<number>(0)
  private _authoritativeVersion = ref<number>(0)
  private _onReconnectCallbacks: Array<() => Promise<void> | void> = []
  private _debounceTimers = new Map<string, ReturnType<typeof setTimeout>>()

  public readonly connectionStatus = computed(() => this._connectionStatus.value)
  public readonly controlStatus = computed(() => this._controlStatus.value)
  public readonly engineMode = computed(() => this._engineMode.value)
  public readonly authoritativeVersion = computed(() => this._authoritativeVersion.value)
  public readonly isEnginePlaying = computed(() => this._engineMode.value !== 'IDLE')
  public readonly isConnected = computed(() => this._connectionStatus.value === 'CONNECTED')
  public readonly isMutating = computed(() => this._inFlightCount.value > 0 || this._controlStatus.value === 'MUTATING')
  /** True when remote device config may safely overwrite the UI controls. */
  public readonly canApplyRemoteConfig = computed(
    () =>
      this._inFlightCount.value === 0
      && this._controlStatus.value !== 'MUTATING'
      && this._localEditCount.value === 0
      && this._engineMode.value === 'IDLE',
  )

  public trackAuthoritativeVersion(version: number | undefined): void {
    if (typeof version === 'number' && Number.isFinite(version) && version > this._authoritativeVersion.value) {
      this._authoritativeVersion.value = version
    }
  }

  public getNextVersion(): number {
    if (this._authoritativeVersion.value > 0) {
      this._authoritativeVersion.value += 1
      return this._authoritativeVersion.value
    }
    return 0
  }

  public shouldAcceptRemoteConfig(remoteConfig: { version?: number } | undefined): boolean {
    if (!remoteConfig) return false
    if (!this.canApplyRemoteConfig.value) return false
    if (remoteConfig.version !== undefined && remoteConfig.version < this._authoritativeVersion.value) {
      return false
    }
    return true
  }

  public setConnectionStatus(newStatus: ConnectionStatus) {
    const prevStatus = this._connectionStatus.value
    this._connectionStatus.value = newStatus

    // Trigger refresh callback when reconnecting or transitioning to CONNECTED
    if (newStatus === 'CONNECTED' && prevStatus !== 'CONNECTED') {
      this.triggerReconnectRefresh()
    }
  }

  public onReconnect(callback: () => Promise<void> | void) {
    this._onReconnectCallbacks.push(callback)
  }

  public async triggerReconnectRefresh() {
    for (const cb of this._onReconnectCallbacks) {
      try {
        await cb()
      } catch (err) {
        console.error('[StateMachine] Reconnect refresh callback failed:', err)
      }
    }
  }

  /**
   * Mark when a virtual playback engine (macro or funscript) is actively running.
   * Prevents background telemetry from overwriting UI controls during motion.
   */
  public beginEnginePlayback(mode: EngineMode) {
    this._engineMode.value = mode
  }

  /**
   * Mark that virtual playback has stopped or paused.
   */
  public endEnginePlayback() {
    this._engineMode.value = 'IDLE'
  }

  /**
   * Mark that the UI has started an optimistic local config edit.
   * Suppresses remote config merge until matching endLocalEdit().
   */
  public beginLocalEdit() {
    this._localEditCount.value++
  }

  /**
   * End one local-edit scope. Nested begin/end pairs are supported.
   */
  public endLocalEdit() {
    this._localEditCount.value = Math.max(0, this._localEditCount.value - 1)
  }

  /**
   * Wrap any HTTP request or control mutation so the state machine tracks
   * in-flight requests and sets MUTATING state with a spinning indicator.
   */
  public async executeControl<T>(fn: () => Promise<T>): Promise<T> {
    this._inFlightCount.value++
    this._controlStatus.value = 'MUTATING'
    try {
      const result = await fn()
      return result
    } catch (err) {
      this._controlStatus.value = 'ERROR'
      throw err
    } finally {
      this._inFlightCount.value = Math.max(0, this._inFlightCount.value - 1)
      if (this._inFlightCount.value === 0) {
        this._controlStatus.value = 'IDLE'
      }
    }
  }

  /**
   * Schedule a debounced mutation with automatic begin/end local edit tracking.
   * Cancels any pending mutation for the same key.
   */
  public scheduleOptimisticMutation<T>(
    key: string,
    delayMs: number,
    fn: () => Promise<T>,
    onSuccess: (result: T) => void,
    onError?: (error: unknown) => void,
  ): void {
    const existing = this._debounceTimers.get(key)
    if (existing !== undefined) {
      clearTimeout(existing)
    } else {
      // First trigger for this key starts a local edit mutex
      this.beginLocalEdit()
    }

    const timer = setTimeout(async () => {
      this._debounceTimers.delete(key)
      try {
        const result = await this.executeControl(fn)
        // Only apply server response if no newer debounce was scheduled for this key during the POST
        if (!this._debounceTimers.has(key)) {
          onSuccess(result)
        }
      } catch (err) {
        if (onError) onError(err)
        else console.error(`[StateMachine] Mutation '${key}' failed:`, err)
      } finally {
        this.endLocalEdit()
      }
    }, delayMs)

    this._debounceTimers.set(key, timer)
  }

  /**
   * Clear any pending debounced mutation for a given key without executing it.
   */
  public clearOptimisticMutation(key: string): void {
    const existing = this._debounceTimers.get(key)
    if (existing !== undefined) {
      clearTimeout(existing)
      this._debounceTimers.delete(key)
      this.endLocalEdit()
    }
  }
}

export const stateMachine = new ConnectionStateMachine()
