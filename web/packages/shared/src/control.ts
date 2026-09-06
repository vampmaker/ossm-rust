import type { InjectionKey, Ref } from 'vue'
import type {
  MotorControllerConfig,
  MotorState,
  PausedControlPayload,
  StreamWaypoint,
} from './types'

/** Motion control surface shared by firmware, desktop, and WASM shells. */
export interface OssmControl {
  getConfig(): Promise<MotorControllerConfig>
  setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig>
  setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig>
  getState(): Promise<MotorState>
  subscribeState(onState: (state: MotorState) => void, intervalMs?: number): Promise<void>
  unsubscribeState(onState?: (state: MotorState) => void): Promise<void>
  sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp?: boolean): Promise<void>
  /** Optional process/device restart (ossm-std exit, WASM re-home, firmware soft reset). */
  restart?(): Promise<void>
}

export const OSSM_CONTROL_KEY: InjectionKey<OssmControl> = Symbol('ossmControl')

/** Stable wrapper so Vue injection can outlive transport swaps (WASM local ↔ remote API). */
export class DelegatingOssmControl implements OssmControl {
  constructor(private target: Ref<OssmControl>) {}

  setTarget(next: OssmControl): void {
    this.target.value = next
  }

  getConfig(): Promise<MotorControllerConfig> {
    return this.target.value.getConfig()
  }

  setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
    return this.target.value.setConfig(config)
  }

  setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
    return this.target.value.setPaused(payload)
  }

  getState(): Promise<MotorState> {
    return this.target.value.getState()
  }

  subscribeState(onState: (state: MotorState) => void, intervalMs?: number): Promise<void> {
    return this.target.value.subscribeState(onState, intervalMs)
  }

  unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
    return this.target.value.unsubscribeState(onState)
  }

  sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp?: boolean): Promise<void> {
    return this.target.value.sendWaypoints(waypoints, resetTimestamp)
  }

  restart(): Promise<void> {
    const fn = this.target.value.restart
    if (!fn) return Promise.resolve()
    return fn.call(this.target.value)
  }
}
