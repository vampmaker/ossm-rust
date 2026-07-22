import type {
  MacroAction,
  MacroDocument,
  MacroInstruction,
  MacroSetParams,
  MacroWaveFunc,
  MotorControllerConfig,
} from './types'
import * as api from './api'
import { stateMachine } from './connectionStateMachine'
import { domainToWireConfig, wireToDomainConfig } from './mapper'

export const MACRO_STORAGE_KEY = 'ossm_macros_v1'
export const MACRO_LAST_SELECTED_KEY = 'ossm_macros_last_selected'
export const MACRO_DRAFT_KEY = 'ossm_macro_draft_v1'

/** Deep-clone plain JSON data (Vue proxies are not structuredClone-able). */
export function cloneJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

const VALID_ACTIONS: MacroAction[] = ['start', 'stop', 'set']
const VALID_WAVES: MacroWaveFunc[] = ['sine', 'thrust', 'spline']

export interface MacroValidationResult {
  ok: boolean
  document: MacroDocument | null
  errors: string[]
}

export function emptyMacro(name = 'Untitled'): MacroDocument {
  return {
    version: 1,
    name,
    loop: false,
    instructions: [],
  }
}

export const BUILTIN_PRESETS: MacroDocument[] = [
  {
    version: 1,
    name: 'Warm up',
    loop: false,
    instructions: [
      { at: 0, action: 'set', params: { bpm: 40, depth: 0.6, wave_func: 'sine' } },
      { at: 0, action: 'start' },
      { at: 15000, action: 'set', params: { bpm: 90, sharpness: 0.2, wave_func: 'thrust' } },
      { at: 30000, action: 'set', params: { wave_func: 'spline', spline_points: [0, 1, 0.3, 0.8] } },
      { at: 60000, action: 'stop', params: { position: 0.0 } },
    ],
  },
  {
    version: 1,
    name: 'Intervals',
    loop: true,
    instructions: [
      { at: 0, action: 'set', params: { bpm: 60, depth: 0.8, wave_func: 'sine' } },
      { at: 0, action: 'start' },
      { at: 20000, action: 'set', params: { bpm: 120, wave_func: 'thrust', sharpness: 0.15 } },
      { at: 40000, action: 'set', params: { bpm: 60, wave_func: 'sine' } },
      { at: 60000, action: 'stop', params: { position: 0.5 } },
    ],
  },
]

function clamp(n: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, n))
}

function normalizeSetParams(raw: Record<string, unknown> | undefined, errors: string[], idx: number): MacroSetParams {
  const out: MacroSetParams = {}
  if (!raw || typeof raw !== 'object') return out

  if ('bpm' in raw) {
    const v = Number(raw.bpm)
    if (Number.isFinite(v)) out.bpm = clamp(v, 10, 300)
    else errors.push(`instructions[${idx}].params.bpm must be a number`)
  }
  if ('depth' in raw) {
    const v = Number(raw.depth)
    if (Number.isFinite(v)) out.depth = clamp(v, 0, 1)
    else errors.push(`instructions[${idx}].params.depth must be a number`)
  }
  if ('depth_top' in raw) {
    if (typeof raw.depth_top === 'boolean') out.depth_top = raw.depth_top
    else errors.push(`instructions[${idx}].params.depth_top must be a boolean`)
  }
  if ('reversed' in raw) {
    if (typeof raw.reversed === 'boolean') out.reversed = raw.reversed
    else errors.push(`instructions[${idx}].params.reversed must be a boolean`)
  }
  if ('wave_func' in raw) {
    const w = String(raw.wave_func)
    if (VALID_WAVES.includes(w as MacroWaveFunc)) out.wave_func = w as MacroWaveFunc
    else errors.push(`instructions[${idx}].params.wave_func must be sine|thrust|spline`)
  }
  if ('sharpness' in raw) {
    const v = Number(raw.sharpness)
    if (Number.isFinite(v)) out.sharpness = clamp(v, 0, 1)
    else errors.push(`instructions[${idx}].params.sharpness must be a number`)
  }
  if ('spline_points' in raw) {
    if (Array.isArray(raw.spline_points)) {
      const pts = raw.spline_points
        .map((p) => Number(p))
        .filter((n) => Number.isFinite(n))
        .map((n) => clamp(n, 0, 1))
      if (pts.length >= 2) out.spline_points = pts
      else errors.push(`instructions[${idx}].params.spline_points needs >= 2 numbers`)
    } else {
      errors.push(`instructions[${idx}].params.spline_points must be an array`)
    }
  }
  return out
}

export function normalizeMacro(input: unknown): MacroValidationResult {
  const errors: string[] = []
  if (!input || typeof input !== 'object') {
    return { ok: false, document: null, errors: ['Root must be an object'] }
  }
  const raw = input as Record<string, unknown>
  const name = typeof raw.name === 'string' && raw.name.trim() ? raw.name.trim() : 'Untitled'
  const loop = Boolean(raw.loop)
  const version = typeof raw.version === 'number' && Number.isFinite(raw.version) ? Math.floor(raw.version) : 1

  if (!Array.isArray(raw.instructions)) {
    return { ok: false, document: null, errors: ['Missing instructions array'] }
  }

  const instructions: MacroInstruction[] = []
  raw.instructions.forEach((item, idx) => {
    if (!item || typeof item !== 'object') {
      errors.push(`instructions[${idx}] must be an object`)
      return
    }
    const row = item as Record<string, unknown>
    const at = Number(row.at)
    if (!Number.isFinite(at) || at < 0) {
      errors.push(`instructions[${idx}].at must be a non-negative number`)
      return
    }
    const action = String(row.action) as MacroAction
    if (!VALID_ACTIONS.includes(action)) {
      errors.push(`instructions[${idx}].action must be start|stop|set`)
      return
    }
    const paramsRaw = row.params as Record<string, unknown> | undefined
    const inst: MacroInstruction = { at: Math.round(at), action }
    if (action === 'set') {
      inst.params = normalizeSetParams(paramsRaw, errors, idx)
    } else if (action === 'stop' && paramsRaw && 'position' in paramsRaw) {
      const pos = Number(paramsRaw.position)
      if (Number.isFinite(pos)) inst.params = { position: clamp(pos, 0, 1) }
      else errors.push(`instructions[${idx}].params.position must be a number`)
    }
    instructions.push(inst)
  })

  instructions.sort((a, b) => a.at - b.at)
  const document: MacroDocument = { version, name, loop, instructions }
  return { ok: errors.length === 0, document, errors }
}

export function parseMacroJson(text: string): MacroValidationResult {
  try {
    return normalizeMacro(JSON.parse(text))
  } catch (e) {
    return { ok: false, document: null, errors: [`JSON parse error: ${e instanceof Error ? e.message : String(e)}`] }
  }
}

export function macroDurationMs(doc: MacroDocument): number {
  if (doc.instructions.length === 0) return 0
  return doc.instructions[doc.instructions.length - 1]!.at
}

export function formatMacroTime(ms: number): string {
  const totalSec = Math.max(0, ms) / 1000
  const m = Math.floor(totalSec / 60)
  const s = totalSec % 60
  return `${String(m).padStart(2, '0')}:${s.toFixed(1).padStart(4, '0')}`
}

export function parseMacroTime(text: string): number | null {
  const trimmed = text.trim()
  if (!trimmed) return null
  // Check for explicit milliseconds suffix (e.g. "5000ms", "5000 ms")
  const msMatch = trimmed.match(/^(\d+(?:\.\d+)?)\s*ms$/i)
  if (msMatch) {
    const ms = Number(msMatch[1])
    return Number.isFinite(ms) && ms >= 0 ? Math.round(ms) : null
  }
  // Check for mm:ss or m:s format (e.g. "01:23.4", "1:23")
  const mmss = trimmed.match(/^(\d+):(\d+(?:\.\d+)?)$/)
  if (mmss) {
    const m = Number(mmss[1])
    const s = Number(mmss[2])
    if (!Number.isFinite(m) || !Number.isFinite(s)) return null
    return Math.round((m * 60 + s) * 1000)
  }
  // Check for seconds with or without 's' suffix (e.g. "12.5s", "12.5 s", "12.5")
  const secMatch = trimmed.match(/^(\d+(?:\.\d+)?)\s*s?$/i)
  if (secMatch) {
    const sec = Number(secMatch[1])
    if (Number.isFinite(sec) && sec >= 0) return Math.round(sec * 1000)
  }
  return null
}

export function loadMacroLibrary(): MacroDocument[] {
  try {
    const raw = localStorage.getItem(MACRO_STORAGE_KEY)
    if (!raw) return BUILTIN_PRESETS.map((p) => cloneJson(p))
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return BUILTIN_PRESETS.map((p) => cloneJson(p))
    const docs: MacroDocument[] = []
    for (const item of parsed) {
      const res = normalizeMacro(item)
      if (res.document) docs.push(res.document)
    }
    return docs.length > 0 ? docs : BUILTIN_PRESETS.map((p) => cloneJson(p))
  } catch {
    return BUILTIN_PRESETS.map((p) => cloneJson(p))
  }
}

export function saveMacroLibrary(docs: MacroDocument[]): void {
  localStorage.setItem(MACRO_STORAGE_KEY, JSON.stringify(docs))
}

export function loadDraft(): MacroDocument | null {
  try {
    const raw = localStorage.getItem(MACRO_DRAFT_KEY)
    if (!raw) return null
    const res = normalizeMacro(JSON.parse(raw))
    return res.document
  } catch {
    return null
  }
}

export function saveDraft(doc: MacroDocument): void {
  localStorage.setItem(MACRO_DRAFT_KEY, JSON.stringify(doc))
}

export function captureConfigAsSet(config: MotorControllerConfig): MacroInstruction {
  const wave: MacroWaveFunc =
    config.wave_func === 'thrust' || config.wave_func === 'spline' ? config.wave_func : 'sine'
  const params: MacroSetParams = {
    bpm: config.bpm,
    depth: config.depth,
    depth_top: config.depth_top,
    reversed: config.reversed,
    wave_func: wave,
    sharpness: config.sharpness,
  }
  if (wave === 'spline') {
    params.spline_points = [...config.spline_points]
  }
  return { at: 0, action: 'set', params }
}

export function applySetParams(
  base: MotorControllerConfig,
  params: MacroSetParams | undefined,
): MotorControllerConfig {
  const next = { ...base, spline_points: [...base.spline_points] }
  if (!params) return next
  if (params.bpm !== undefined) next.bpm = params.bpm
  if (params.depth !== undefined) next.depth = params.depth
  if (params.depth_top !== undefined) next.depth_top = params.depth_top
  if (params.reversed !== undefined) next.reversed = params.reversed
  if (params.wave_func !== undefined) next.wave_func = params.wave_func
  if (params.sharpness !== undefined) next.sharpness = params.sharpness
  if (params.spline_points !== undefined) next.spline_points = [...params.spline_points]
  // Keep streaming off during macro config patches
  next.streaming = false
  return next
}

export type MacroPlayerCallbacks = {
  onConfig?: (config: MotorControllerConfig) => void
  onTick?: (elapsedMs: number, nextIndex: number) => void
  onStatus?: (message: string) => void
  onDone?: () => void
}

/**
 * Drift-corrected macro playback engine.
 * Maintains a local config copy and posts full configs (no per-step get_status).
 */
export class MacroPlayerEngine {
  private doc: MacroDocument
  private baseConfig: MotorControllerConfig
  private liveConfig: MotorControllerConfig
  private callbacks: MacroPlayerCallbacks
  private playing = false
  private startWall = 0
  private startElapsed = 0
  private elapsedMs = 0
  private nextIndex = 0
  private timer: number | null = null
  private speed = 1

  constructor(
    doc: MacroDocument,
    baseConfig: MotorControllerConfig,
    callbacks: MacroPlayerCallbacks = {},
  ) {
    this.doc = cloneJson(doc)
    this.baseConfig = { ...baseConfig, spline_points: [...baseConfig.spline_points] }
    this.liveConfig = { ...this.baseConfig }
    this.callbacks = callbacks
    stateMachine.trackAuthoritativeVersion(baseConfig.version)
  }

  private bumpConfigVersion(): void {
    if (stateMachine.authoritativeVersion.value > 0 || this.liveConfig.version !== undefined) {
      this.liveConfig = { ...this.liveConfig, version: stateMachine.getNextVersion() }
    }
  }

  private applyPostedConfig(posted: MotorControllerConfig): void {
    stateMachine.trackAuthoritativeVersion(posted.version)
    this.liveConfig = wireToDomainConfig(posted, this.liveConfig)
    this.callbacks.onConfig?.(this.liveConfig)
  }

  get isPlaying(): boolean {
    return this.playing
  }

  get elapsed(): number {
    return this.elapsedMs
  }

  get nextInstructionIndex(): number {
    return this.nextIndex
  }

  seek(ms: number, exactIndex?: number): void {
    const duration = macroDurationMs(this.doc)
    this.elapsedMs = Math.max(0, Math.min(ms, duration))
    if (exactIndex !== undefined && exactIndex >= 0 && exactIndex <= this.doc.instructions.length) {
      this.nextIndex = exactIndex
    } else if (this.elapsedMs <= 0) {
      this.nextIndex = 0
    } else if (this.elapsedMs >= duration && duration > 0) {
      this.nextIndex = this.doc.instructions.length
    } else {
      let idx = 0
      while (idx < this.doc.instructions.length && this.doc.instructions[idx]!.at <= this.elapsedMs) {
        idx++
      }
      this.nextIndex = idx
    }
    this.callbacks.onTick?.(this.elapsedMs, this.nextIndex)
  }

  async start(fromMs = 0): Promise<void> {
    if (this.playing) return
    this.seek(fromMs)
    this.playing = true
    stateMachine.beginEnginePlayback('MACRO')
    this.startWall = performance.now()
    this.startElapsed = this.elapsedMs
    this.callbacks.onStatus?.('playing')
    this.scheduleTick()
  }

  async stop(): Promise<void> {
    this.playing = false
    stateMachine.endEnginePlayback()
    if (this.timer !== null) {
      clearTimeout(this.timer)
      this.timer = null
    }
    try {
      const updated = await api.setPaused({ paused: true })
      this.applyPostedConfig(updated)
    } catch (e) {
      console.warn('Macro stop park failed:', e)
    }
    this.callbacks.onStatus?.('stopped')
    this.callbacks.onDone?.()
  }

  private scheduleTick(): void {
    if (!this.playing) return
    this.timer = window.setTimeout(() => {
      void this.tick()
    }, 25)
  }

  private async tick(): Promise<void> {
    if (!this.playing) return
    const wallElapsed = (performance.now() - this.startWall) * this.speed
    this.elapsedMs = this.startElapsed + wallElapsed
    this.callbacks.onTick?.(this.elapsedMs, this.nextIndex)

    while (this.nextIndex < this.doc.instructions.length) {
      const inst = this.doc.instructions[this.nextIndex]!
      if (inst.at > this.elapsedMs) break
      try {
        await this.execute(inst)
      } catch (e) {
        console.error('Macro instruction failed:', e)
        this.callbacks.onStatus?.('error')
        await this.stop()
        return
      }
      this.nextIndex++
      this.callbacks.onTick?.(this.elapsedMs, this.nextIndex)
    }

    const duration = macroDurationMs(this.doc)
    if (this.nextIndex >= this.doc.instructions.length && this.elapsedMs >= duration) {
      if (this.doc.loop && duration > 0) {
        this.startWall = performance.now()
        this.startElapsed = 0
        this.elapsedMs = 0
        this.nextIndex = 0
        this.callbacks.onTick?.(0, 0)
        this.scheduleTick()
        return
      }
      await this.stop()
      return
    }

    this.scheduleTick()
  }

  private async execute(inst: MacroInstruction): Promise<void> {
    stateMachine.beginLocalEdit()
    try {
      if (inst.action === 'start') {
        const updated = await api.setPaused({ paused: false })
        this.applyPostedConfig(updated)
        return
      }
      if (inst.action === 'stop') {
        const position = inst.params?.position
        const updated = position !== undefined
          ? await api.setPaused({ paused: true, position })
          : await api.setPaused({ paused: true })
        this.applyPostedConfig(updated)
        return
      }
      // set
      this.liveConfig = applySetParams(this.liveConfig, inst.params)
      this.bumpConfigVersion()
      const posted = await api.setConfig(domainToWireConfig(this.liveConfig))
      this.applyPostedConfig(posted)
    } finally {
      stateMachine.endLocalEdit()
    }
  }
}
