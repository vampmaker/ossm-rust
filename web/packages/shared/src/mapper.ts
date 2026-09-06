import type { LoopStats, MotorControllerConfig, MotorState, TimingWindowStats } from './types'

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

function sanitizeTimingWindow(w: TimingWindowStats | undefined): TimingWindowStats | undefined {
  if (!w) return undefined
  return {
    min: safeFloat(w.min, 0),
    max: safeFloat(w.max, 0),
    pct5: safeFloat(w.pct5, 0),
    pct10: safeFloat(w.pct10, 0),
    pct50: safeFloat(w.pct50, 0),
    pct90: safeFloat(w.pct90, 0),
    pct95: safeFloat(w.pct95, 0),
    mean: safeFloat(w.mean, 0),
    mdev: safeFloat(w.mdev, 0),
  }
}

function sanitizeLoopStats(loop: LoopStats | undefined): LoopStats | undefined {
  if (!loop) return undefined
  return {
    ups: Math.max(0, Math.round(safeFloat(loop.ups, 0))),
    dt_min_ms: safeFloat(loop.dt_min_ms, 0),
    dt_avg_ms: safeFloat(loop.dt_avg_ms, 0),
    dt_max_ms: safeFloat(loop.dt_max_ms, 0),
    dt_mdev_ms: safeFloat(loop.dt_mdev_ms, 0),
    update_history: Array.isArray(loop.update_history)
      ? loop.update_history.map((v) => safeFloat(v, 0))
      : [],
    position_history: Array.isArray(loop.position_history)
      ? loop.position_history.map((v) => safeFloat(v, 0))
      : [],
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

  const loop = sanitizeLoopStats(state.loop_stats)
  const link = state.link_stats
    ? {
        ...state.link_stats,
        success_rate: safeFloat(state.link_stats.success_rate, 0),
        round_trip: sanitizeTimingWindow(state.link_stats.round_trip)!,
        slave_latency: sanitizeTimingWindow(state.link_stats.slave_latency)!,
        rx_duration: sanitizeTimingWindow(state.link_stats.rx_duration)!,
      }
    : undefined

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
    config: state.config ? wireToDomainConfig(state.config) : state.config,
    loop_stats: loop,
    link_stats: link,
  }
}
