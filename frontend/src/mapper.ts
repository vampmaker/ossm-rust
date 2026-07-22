import type { MotorControllerConfig, MotorState } from './types'

/**
 * Checks deep equality between two MotorControllerConfig objects.
 * Used across the frontend to prevent unnecessary reactivity updates.
 */
export function isConfigEqual(a: MotorControllerConfig | null | undefined, b: MotorControllerConfig | null | undefined): boolean {
  if (!a || !b) return a === b
  if (a.version !== undefined && b.version !== undefined && a.version !== b.version) return false
  if (a.paused !== b.paused) return false
  if (Math.abs((a.paused_position ?? 0) - (b.paused_position ?? 0)) > 0.0001) return false
  if (!!a.streaming !== !!b.streaming) return false
  if (Math.abs((a.bpm ?? 0) - (b.bpm ?? 0)) > 0.0001) return false
  if (Math.abs((a.depth ?? 0) - (b.depth ?? 0)) > 0.0001) return false
  if (a.depth_top !== b.depth_top) return false
  if (a.reversed !== b.reversed) return false
  if (a.wave_func !== b.wave_func) return false
  if (Math.abs((a.sharpness ?? 0) - (b.sharpness ?? 0)) > 0.0001) return false
  const aSpline = a.spline_points ?? []
  const bSpline = b.spline_points ?? []
  if (aSpline.length !== bSpline.length) return false
  for (let i = 0; i < aSpline.length; i++) {
    if (Math.abs((aSpline[i] ?? 0) - (bSpline[i] ?? 0)) > 0.0001) return false
  }
  return true
}

/**
 * Safely normalizes numerical values; returns default if NaN or non-finite.
 */
function safeFloat(val: unknown, fallback: number): number {
  if (typeof val !== 'number' || !Number.isFinite(val)) return fallback
  return val
}

/**
 * Maps a concrete wire configuration received from the backend (REST or WebSocket)
 * into our frontend domain configuration. Preserves active virtual modes ('macro' or 'funscript')
 * and safely deep-clones spline point arrays.
 */
export function wireToDomainConfig(fromServer: MotorControllerConfig, currentDomain?: MotorControllerConfig | null): MotorControllerConfig {
  const isVirtualMode = currentDomain && (currentDomain.wave_func === 'macro' || currentDomain.wave_func === 'funscript')
  const targetWaveFunc = isVirtualMode ? currentDomain.wave_func : fromServer.wave_func

  return {
    ...fromServer,
    version: fromServer.version !== undefined ? fromServer.version : currentDomain?.version,
    bpm: safeFloat(fromServer.bpm, currentDomain?.bpm ?? 40),
    depth: safeFloat(fromServer.depth, currentDomain?.depth ?? 0.8),
    sharpness: safeFloat(fromServer.sharpness, currentDomain?.sharpness ?? 0.3),
    paused_position: safeFloat(fromServer.paused_position, currentDomain?.paused_position ?? 0.0),
    wave_func: targetWaveFunc,
    spline_points: [...(fromServer.spline_points ?? (currentDomain?.spline_points ?? []))],
  }
}

/**
 * Maps our frontend domain configuration to a clean wire payload suitable for api.setConfig().
 * If the UI is in a virtual mode ('macro' or 'funscript'), converts wave_func to a concrete physical wave
 * (defaults to 'sine' unless the domain already specifies a concrete waveform) to ensure valid firmware state.
 */
export function domainToWireConfig(domain: MotorControllerConfig): MotorControllerConfig {
  const isVirtual = domain.wave_func === 'macro' || domain.wave_func === 'funscript'
  const wireWaveFunc = isVirtual ? 'sine' : domain.wave_func

  return {
    ...domain,
    version: domain.version,
    bpm: safeFloat(domain.bpm, 40),
    depth: safeFloat(domain.depth, 0.8),
    sharpness: safeFloat(domain.sharpness, 0.3),
    paused_position: safeFloat(domain.paused_position, 0.0),
    wave_func: wireWaveFunc,
    spline_points: [...(domain.spline_points ?? [])],
  }
}

/**
 * Sanitizes incoming MotorState telemetry payloads from background polling or WebSocket pushes.
 * Protects charts, position diagrams, and statistics cards from NaN/Infinity or inverted hardware limits.
 */
export function sanitizeMotorState(state: MotorState): MotorState {
  if (!state) return state

  const posMin = safeFloat(state.pos_min, -5.0)
  const posMaxRaw = safeFloat(state.pos_max, 5.0)
  const posMax = posMaxRaw < posMin ? posMin + 10.0 : posMaxRaw

  return {
    ...state,
    t: safeFloat(state.t, 0),
    x: safeFloat(state.x, 0),
    y: safeFloat(state.y, 0),
    shaped_y: safeFloat(state.shaped_y, 0),
    position: safeFloat(state.position, 0),
    speed: safeFloat(state.speed, 0),
    pos_min: posMin,
    pos_max: posMax,
    ups: Math.max(0, Math.round(safeFloat(state.ups, 0))),
    dt_min_ms: safeFloat(state.dt_min_ms, 0),
    dt_max_ms: safeFloat(state.dt_max_ms, 0),
    dt_avg_ms: safeFloat(state.dt_avg_ms, 0),
    dt_mdev_ms: safeFloat(state.dt_mdev_ms, 0),
    config: state.config ? wireToDomainConfig(state.config) : state.config,
    update_history: Array.isArray(state.update_history)
      ? state.update_history.map(v => safeFloat(v, 0))
      : undefined,
    position_history: Array.isArray(state.position_history)
      ? state.position_history.map(v => safeFloat(v, 0))
      : undefined,
  }
}
