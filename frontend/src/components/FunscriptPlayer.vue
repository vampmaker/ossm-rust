<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { setConfig, sendWaypoints, subscribeState, unsubscribeState } from '../api'
import type { FunscriptDocument, FunscriptAction, StreamWaypoint, MotorState } from '../types'

const scriptName = ref<string>('')
const actions = ref<FunscriptAction[]>([])
const isPlaying = ref(false)
const currentTime = ref(0) // seconds
const speedMultiplier = ref(1.0)
const minDepth = ref(0.0) // 0% default on load
const maxDepth = ref(1.0) // 100% default on load
const lastSentIndex = ref(0)
const bufferedCount = ref(0)
const streamTime = ref(0)
const statusMessage = ref('Load a .funscript file to get started')

const canvasRef = ref<HTMLCanvasElement | null>(null)
let streamTimer: number | null = null

const totalDuration = computed(() => {
  if (actions.value.length === 0) return 0
  const last = actions.value[actions.value.length - 1]
  return last ? last.at / 1000.0 : 0
})

const formatTime = (secs: number) => {
  const m = Math.floor(secs / 60)
  const s = (secs % 60).toFixed(1)
  return `${m}:${s.padStart(4, '0')}`
}

function handleFileUpload(e: Event) {
  const input = e.target as HTMLInputElement
  if (!input.files || input.files.length === 0) return
  const file = input.files[0]
  if (!file) return
  scriptName.value = file.name
  const reader = new FileReader()
  reader.onload = (ev) => {
    try {
      const text = ev.target?.result as string
      const doc = JSON.parse(text) as FunscriptDocument
      if (!doc.actions || !Array.isArray(doc.actions)) {
        statusMessage.value = 'Invalid .funscript format: missing actions array'
        return
      }
      actions.value = doc.actions.slice().sort((a, b) => a.at - b.at)
      currentTime.value = 0
      lastSentIndex.value = 0
      statusMessage.value = `Loaded ${actions.value.length} actions (${formatTime(totalDuration.value)})`
      drawTimeline()
    } catch (err) {
      statusMessage.value = 'Failed to parse .funscript file'
      console.error(err)
    }
  }
  reader.readAsText(file)
}

function mapPos(rawPos: number): number {
  const normalized = Math.max(0, Math.min(100, rawPos)) / 100.0
  return Math.max(0.0, Math.min(1.0, minDepth.value + normalized * (maxDepth.value - minDepth.value)))
}

function drawTimeline() {
  const canvas = canvasRef.value
  if (!canvas) return
  const ctx = canvas.getContext('2d')
  if (!ctx) return

  const dpr = window.devicePixelRatio || 1
  const rect = canvas.getBoundingClientRect()
  canvas.width = rect.width * dpr
  canvas.height = rect.height * dpr
  ctx.scale(dpr, dpr)

  const w = rect.width
  const h = rect.height

  ctx.clearRect(0, 0, w, h)

  // Background grid
  ctx.strokeStyle = 'rgba(255, 255, 255, 0.05)'
  ctx.lineWidth = 1
  for (let i = 0; i <= 4; i++) {
    const y = (h * i) / 4
    ctx.beginPath()
    ctx.moveTo(0, y)
    ctx.lineTo(w, y)
    ctx.stroke()
  }

  if (actions.value.length < 2) return

  const maxT = totalDuration.value || 1
  const paddingY = 10

  // Draw gradient fill under trajectory
  const grad = ctx.createLinearGradient(0, 0, 0, h)
  grad.addColorStop(0, 'rgba(16, 185, 129, 0.35)')
  grad.addColorStop(1, 'rgba(16, 185, 129, 0.02)')

  ctx.beginPath()
  ctx.moveTo(0, h)
  for (let i = 0; i < actions.value.length; i++) {
    const a = actions.value[i]
    if (!a) continue
    const t = a.at / 1000.0
    const x = (t / maxT) * w
    const yVal = mapPos(a.pos)
    const y = h - paddingY - yVal * (h - 2 * paddingY)
    if (i === 0) ctx.lineTo(x, y)
    else ctx.lineTo(x, y)
  }
  ctx.lineTo(w, h)
  ctx.closePath()
  ctx.fillStyle = grad
  ctx.fill()

  // Draw curve stroke
  ctx.beginPath()
  for (let i = 0; i < actions.value.length; i++) {
    const a = actions.value[i]
    if (!a) continue
    const t = a.at / 1000.0
    const x = (t / maxT) * w
    const yVal = mapPos(a.pos)
    const y = h - paddingY - yVal * (h - 2 * paddingY)
    if (i === 0) ctx.moveTo(x, y)
    else ctx.lineTo(x, y)
  }
  ctx.strokeStyle = '#10b981'
  ctx.lineWidth = 2
  ctx.stroke()

  // Draw playhead line
  const headX = (currentTime.value / maxT) * w
  ctx.beginPath()
  ctx.moveTo(headX, 0)
  ctx.lineTo(headX, h)
  ctx.strokeStyle = '#f59e0b'
  ctx.lineWidth = 2
  ctx.stroke()
}

function handleCanvasClick(e: MouseEvent) {
  if (actions.value.length === 0) return
  const canvas = canvasRef.value
  if (!canvas) return
  const rect = canvas.getBoundingClientRect()
  const clickX = e.clientX - rect.left
  const ratio = Math.max(0, Math.min(1, clickX / rect.width))
  currentTime.value = ratio * totalDuration.value
  drawTimeline()
  if (isPlaying.value) {
    resyncStream(true)
  }
}

async function startPlayback() {
  if (actions.value.length === 0) return
  isPlaying.value = true
  statusMessage.value = 'Starting streaming mode...'

  try {
    await setConfig({
      bpm: 30,
      depth: 1.0,
      depth_top: false,
      reversed: false,
      wave_func: 'sine',
      sharpness: 0.3,
      spline_points: [0.0, 1.0],
      paused: false,
      paused_position: 0.0,
      streaming: true
    })

    await subscribeState((st: MotorState) => {
      if (st.stream) {
        bufferedCount.value = st.stream.buffered
        streamTime.value = st.stream.stream_time
      }
    }, 200)

    resyncStream(true)

    streamTimer = window.setInterval(() => {
      if (!isPlaying.value) return
      currentTime.value += 0.2 * speedMultiplier.value
      if (currentTime.value >= totalDuration.value) {
        stopPlayback()
        return
      }
      drawTimeline()
      feedWaypoints()
    }, 200)

    statusMessage.value = 'Playing script...'
  } catch (err) {
    console.error(err)
    statusMessage.value = 'Error starting streaming playback'
    isPlaying.value = false
  }
}

async function stopPlayback() {
  isPlaying.value = false
  if (streamTimer) {
    clearInterval(streamTimer)
    streamTimer = null
  }
  try {
    await unsubscribeState()
    await setConfig({
      bpm: 30,
      depth: 1.0,
      depth_top: false,
      reversed: false,
      wave_func: 'sine',
      sharpness: 0.3,
      spline_points: [0.0, 1.0],
      paused: true,
      paused_position: 0.0,
      streaming: false
    })
  } catch (e) {
    console.warn('Error stopping playback:', e)
  }
  statusMessage.value = 'Stopped'
  drawTimeline()
}

function resyncStream(reset = false) {
  const currentMs = currentTime.value * 1000
  let idx = actions.value.findIndex(a => a.at >= currentMs)
  if (idx === -1) idx = actions.value.length - 1
  lastSentIndex.value = idx

  const baseAct = actions.value[idx]
  if (!baseAct) return

  const chunk: StreamWaypoint[] = []
  const maxChunk = 40
  const baseAt = baseAct.at
  for (let i = 0; i < maxChunk && idx + i < actions.value.length; i++) {
    const act = actions.value[idx + i]
    if (!act) continue
    const relMs = (act.at - baseAt) / speedMultiplier.value
    chunk.push({
      ts: Math.round(relMs),
      pos: mapPos(act.pos)
    })
  }
  lastSentIndex.value = Math.min(actions.value.length, idx + maxChunk)
  if (chunk.length > 0) {
    sendWaypoints(chunk, reset)
  }
}

function feedWaypoints() {
  if (bufferedCount.value > 50) return
  if (lastSentIndex.value >= actions.value.length) return

  const firstAct = actions.value[0]
  if (!firstAct) return

  const chunk: StreamWaypoint[] = []
  const maxChunk = 35
  const baseAt = firstAct.at
  for (let i = 0; i < maxChunk && lastSentIndex.value < actions.value.length; i++) {
    const act = actions.value[lastSentIndex.value++]
    if (!act) continue
    const relMs = (act.at - baseAt) / speedMultiplier.value
    chunk.push({
      ts: Math.round(relMs),
      pos: mapPos(act.pos)
    })
  }
  if (chunk.length > 0) {
    sendWaypoints(chunk, false)
  }
}

watch([minDepth, maxDepth], () => {
  drawTimeline()
})

onMounted(() => {
  window.addEventListener('resize', drawTimeline)
  drawTimeline()
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', drawTimeline)
  stopPlayback()
})
</script>

<template>
  <div class="space-y-6">
    <!-- Header & Uploader -->
    <div class="flex flex-col md:flex-row md:items-center md:justify-between gap-4 bg-gray-900/60 p-5 rounded-2xl border border-gray-800 backdrop-blur-xl">
      <div>
        <h2 class="text-xl font-bold text-white tracking-wide">Funscript Player</h2>
        <p class="text-sm text-gray-400 mt-0.5">{{ scriptName || 'No script selected' }}</p>
      </div>
      <label class="cursor-pointer bg-gradient-to-r from-emerald-600 to-teal-600 hover:from-emerald-500 hover:to-teal-500 text-white text-sm font-medium px-4 py-2.5 rounded-xl shadow-lg shadow-emerald-900/20 transition-all active:scale-95 inline-flex items-center gap-2">
        <span>Load .funscript</span>
        <input type="file" accept=".funscript,.json" class="hidden" @change="handleFileUpload" />
      </label>
    </div>

    <!-- Timeline Canvas -->
    <div class="bg-gray-900/80 p-4 rounded-2xl border border-gray-800 shadow-xl relative overflow-hidden">
      <div class="flex items-center justify-between text-xs font-semibold text-gray-400 mb-2 px-1">
        <span>Current: {{ formatTime(currentTime) }}</span>
        <span>Duration: {{ formatTime(totalDuration) }}</span>
      </div>
      <canvas
        ref="canvasRef"
        class="w-full h-44 cursor-pointer rounded-xl bg-gray-950 border border-gray-800/80"
        @click="handleCanvasClick"
      ></canvas>
    </div>

    <!-- Playback Controls -->
    <div class="bg-gray-900/60 p-5 rounded-2xl border border-gray-800 flex flex-wrap items-center justify-between gap-4">
      <div class="flex items-center gap-3">
        <button
          v-if="!isPlaying"
          @click="startPlayback"
          :disabled="actions.length === 0"
          class="bg-emerald-500 hover:bg-emerald-400 disabled:bg-gray-800 disabled:text-gray-500 text-gray-950 font-bold px-6 py-2.5 rounded-xl transition-all shadow-lg active:scale-95"
        >
          Play
        </button>
        <button
          v-else
          @click="stopPlayback"
          class="bg-amber-500 hover:bg-amber-400 text-gray-950 font-bold px-6 py-2.5 rounded-xl transition-all shadow-lg active:scale-95"
        >
          Pause
        </button>
        <button
          @click="stopPlayback"
          class="bg-gray-800 hover:bg-gray-700 text-gray-300 font-medium px-4 py-2.5 rounded-xl transition-all"
        >
          Stop
        </button>
      </div>

      <!-- Speed Multiplier -->
      <div class="flex items-center gap-2">
        <span class="text-xs font-semibold text-gray-400 uppercase tracking-wider">Speed:</span>
        <select
          v-model.number="speedMultiplier"
          class="bg-gray-950 border border-gray-800 text-white text-sm rounded-xl px-3 py-1.5 focus:outline-none focus:border-emerald-500"
        >
          <option :value="0.5">0.5x</option>
          <option :value="1.0">1.0x</option>
          <option :value="1.25">1.25x</option>
          <option :value="1.5">1.5x</option>
          <option :value="2.0">2.0x</option>
        </select>
      </div>
    </div>

    <!-- Stroke Range Limits -->
    <div class="bg-gray-900/60 p-5 rounded-2xl border border-gray-800 space-y-4">
      <h3 class="text-sm font-semibold text-gray-300 tracking-wide uppercase">Stroke Range Limits</h3>
      <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div>
          <div class="flex justify-between text-xs text-gray-400 mb-1 font-medium">
            <span>Minimum Depth</span>
            <span class="text-emerald-400 font-bold">{{ Math.round(minDepth * 100) }}%</span>
          </div>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            v-model.number="minDepth"
            class="w-full accent-emerald-500 bg-gray-800 rounded-lg h-2 cursor-pointer"
          />
        </div>

        <div>
          <div class="flex justify-between text-xs text-gray-400 mb-1 font-medium">
            <span>Maximum Depth</span>
            <span class="text-emerald-400 font-bold">{{ Math.round(maxDepth * 100) }}%</span>
          </div>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            v-model.number="maxDepth"
            class="w-full accent-emerald-500 bg-gray-800 rounded-lg h-2 cursor-pointer"
          />
        </div>
      </div>
    </div>

    <!-- Status Bar -->
    <div class="bg-gray-950/80 px-4 py-3 rounded-xl border border-gray-800/80 flex items-center justify-between text-xs">
      <span class="text-gray-400">{{ statusMessage }}</span>
      <span v-if="isPlaying" class="text-emerald-400 font-mono">Buffered: {{ bufferedCount }}</span>
    </div>
  </div>
</template>
