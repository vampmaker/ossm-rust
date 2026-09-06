import type {
  MotorControllerConfig,
  MotorState,
  OssmControl,
  PausedControlPayload,
  StreamWaypoint,
} from '@ossm/shared'
import { OssmClient } from '@ossm/client'

/** Remote motion API client (ossm-std or ESP32 firmware). */
export class RemoteApiControl implements OssmControl {
  private client: OssmClient

  constructor(baseUrl: string) {
    this.client = new OssmClient(baseUrl.replace(/\/$/, ''))
  }

  setBaseUrl(baseUrl: string): void {
    this.client.close()
    this.client = new OssmClient(baseUrl.replace(/\/$/, ''))
  }

  close(): void {
    this.client.close()
  }

  getConfig(): Promise<MotorControllerConfig> {
    return this.client.getConfig()
  }

  setConfig(config: MotorControllerConfig): Promise<MotorControllerConfig> {
    return this.client.setConfig(config)
  }

  setPaused(payload: PausedControlPayload): Promise<MotorControllerConfig> {
    return this.client.setPaused(payload)
  }

  getState(): Promise<MotorState> {
    return this.client.getState()
  }

  subscribeState(onState: (state: MotorState) => void, intervalMs = 33): Promise<void> {
    return this.client.subscribeState(onState, intervalMs)
  }

  unsubscribeState(onState?: (state: MotorState) => void): Promise<void> {
    return this.client.unsubscribeState(onState)
  }

  sendWaypoints(waypoints: StreamWaypoint[], resetTimestamp = false): Promise<void> {
    return this.client.sendWaypoints(waypoints, resetTimestamp)
  }

  restart(): Promise<void> {
    return this.client.restartDevice()
  }
}
