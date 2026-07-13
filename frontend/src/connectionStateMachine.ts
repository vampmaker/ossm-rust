import { ref, computed } from 'vue'

export type ConnectionStatus = 'DISCONNECTED' | 'CONNECTING' | 'CONNECTED' | 'RECONNECTING'
export type ControlRequestStatus = 'IDLE' | 'MUTATING' | 'ERROR'

export class ConnectionStateMachine {
  private _connectionStatus = ref<ConnectionStatus>('DISCONNECTED')
  private _controlStatus = ref<ControlRequestStatus>('IDLE')
  private _inFlightCount = ref<number>(0)
  private _onReconnectCallbacks: Array<() => Promise<void> | void> = []

  public readonly connectionStatus = computed(() => this._connectionStatus.value)
  public readonly controlStatus = computed(() => this._controlStatus.value)
  public readonly isConnected = computed(() => this._connectionStatus.value === 'CONNECTED')
  public readonly isMutating = computed(() => this._inFlightCount.value > 0 || this._controlStatus.value === 'MUTATING')

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
}

export const stateMachine = new ConnectionStateMachine()
