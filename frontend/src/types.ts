export type WaveFunc = 'sine' | 'thrust' | 'spline'

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
}

export interface PausedControlPayload {
  paused?: boolean
  position?: number
  adjust?: number
}

export interface MotorState {
  config: MotorControllerConfig
  t: number
  x: number
  y: number
  shaped_y: number
  position: number
  speed: number
}
