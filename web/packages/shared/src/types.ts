export type WaveFunc = 'sine' | 'thrust' | 'spline' | 'funscript' | 'macro'
export type PauseMode = 'fixed-position' | 'in-place'

export interface MotorControllerConfig {
  version?: number
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

export const DEFAULT_MOTOR_CONFIG: MotorControllerConfig = {
  bpm: 60.0,
  depth: 1.0,
  depth_top: true,
  reversed: false,
  wave_func: 'sine',
  sharpness: 0.5,
  spline_points: [0.0, 1.0],
  paused: true,
  paused_position: 0.5,
}

/** Wave funcs that may appear inside a macro `set` instruction (not funscript/macro). */
export type MacroWaveFunc = 'sine' | 'thrust' | 'spline'

export type MacroAction = 'start' | 'stop' | 'set'

export interface MacroSetParams {
  bpm?: number
  depth?: number
  depth_top?: boolean
  reversed?: boolean
  wave_func?: MacroWaveFunc
  sharpness?: number
  spline_points?: number[]
}

export interface MacroStopParams {
  position?: number
}

export interface MacroInstruction {
  _id?: string
  at: number
  action: MacroAction
  params?: MacroSetParams & MacroStopParams
}

export interface MacroDocument {
  version: number
  name: string
  loop: boolean
  instructions: MacroInstruction[]
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

export interface LoopStats {
  ups: number
  dt_min_ms: number
  dt_avg_ms: number
  dt_max_ms: number
  dt_mdev_ms: number
  update_history: number[]
  position_history: number[]
}

export interface LinkStats {
  transport: string
  connected: boolean
  exchanges: number
  failures: number
  success_rate: number
  exchanges_per_sec: number
  bytes_tx: number
  bytes_rx: number
  reconnects: number
  round_trip: TimingWindowStats
  slave_latency: TimingWindowStats
  rx_duration: TimingWindowStats
  pacing_state?: string
  pacing_period_us?: number
  pacing_delay_us?: number
  rtt_min_us?: number
}

/** @deprecated Use LinkStats.round_trip etc. */
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
  pos_min?: number
  pos_max?: number
  motor_connected?: boolean
  loop_stats?: LoopStats
  link_stats?: LinkStats
}

export interface ShellConfig {
  modbus_tx: number
  modbus_rx: number
  modbus_de_re: number
  modbus_timeout_ms: number
  modbus_rx_timeout_us?: number
  modbus_scan_delay_us: number
  modbus_inter_frame_delay_us: number
  ble_enabled: boolean
  /** Diagnostic Modbus RX capture; requires reboot. Collapses UPS while on. */
  modbus_debug?: boolean
  operating_mode?: 'servo' | 'rtu_relay' | 'rs485'
  /** UART1 boot baud; live baud in rs485 may follow CDC / /ws/rs485 CFG. */
  modbus_baud?: number
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

/** BLE GATT `CHAR_PIN_CONFIG` slice of `ShellConfig`. */
export type PinConfiguration = Pick<
  ShellConfig,
  | 'modbus_tx'
  | 'modbus_rx'
  | 'modbus_de_re'
  | 'modbus_timeout_ms'
  | 'modbus_rx_timeout_us'
  | 'modbus_scan_delay_us'
  | 'modbus_inter_frame_delay_us'
  | 'ble_enabled'
  | 'modbus_debug'
  | 'operating_mode'
  | 'modbus_baud'
>

/** BLE GATT `CHAR_NETWORK_CONFIG` slice of `ShellConfig`. */
export type NetworkConfiguration = Pick<
  ShellConfig,
  | 'wifi_enabled'
  | 'ssid'
  | 'password'
  | 'hostname'
  | 'dhcp_enabled'
  | 'static_ip'
  | 'static_mask'
  | 'static_gateway'
  | 'static_dns'
>

/** Resolved ossm-std startup/runtime settings (read-only via GET /runtime-config). */
export interface StdRuntimeConfig {
  mode: 'servo' | 'rtu-relay'
  mock: boolean
  serial: string | null
  baud: number
  slave_id: number
  bind: string
  modbus_bind: string
  relay_tcp: string | null
  relay_ws: string | null
  rs485_ws: string | null
  config_path: string
  static_dir: string | null
  repl: boolean
  no_repl: boolean
  /** Human-readable active bus selection. */
  transport: string
}

