export type WaveFunc = 'sine' | 'thrust' | 'spline' | 'funscript'
export type PauseMode = 'fixed-position' | 'in-place'

export interface MotorControllerConfig {
  bpm: number
  depth: number
  depth_top: boolean
  reversed: boolean
  wave_func: WaveFunc
  sharpness: number
  spline_points: number[]
  paused: boolean
  paused_position: number
  streaming?: boolean
}

export interface PausedControlPayload {
  paused?: boolean
  position?: number
  adjust?: number
}

export interface StreamStatus {
  buffered: number
  stream_time: number
  underrun: boolean
}

export interface StreamWaypoint {
  ts: number
  pos: number
  vel?: number
}

export interface FunscriptAction {
  at: number
  pos: number
}

export interface FunscriptDocument {
  version?: string
  actions: FunscriptAction[]
}

export interface TimingWindowStats {
  min: number
  max: number
  pct5: number
  pct10: number
  pct50: number
  pct90: number
  pct95: number
  mean: number
  mdev: number
}

export interface ModbusStats {
  successful_requests: number
  failed_requests: number
  success_rate: number
  round_trip: TimingWindowStats
  slave_latency: TimingWindowStats
  rx_duration: TimingWindowStats
}

export interface MotorState {
  config: MotorControllerConfig
  t: number
  x: number
  y: number
  shaped_y: number
  position: number
  speed: number
  stream?: StreamStatus
  update_history?: number[]
  position_history?: number[]
  pos_min?: number
  pos_max?: number
  ups?: number
  dt_min_ms?: number
  dt_max_ms?: number
  dt_avg_ms?: number
  dt_mdev_ms?: number
  motor_connected?: boolean
  modbus_stats?: ModbusStats
}

export interface PinConfiguration {
  modbus_tx: number
  modbus_rx: number
  modbus_de_re: number
  modbus_timeout_ms: number
  modbus_rx_timeout_us?: number
  modbus_scan_delay_us: number
  modbus_inter_frame_delay_us: number
  ble_enabled: boolean
  operating_mode?: 'servo' | 'rtu_relay'
}

export interface NetworkConfiguration {
  wifi_enabled: boolean
  ssid: string
  password: string
  hostname: string
  dhcp_enabled: boolean
  static_ip: string
  static_mask: string
  static_gateway: string
  static_dns: string
}

