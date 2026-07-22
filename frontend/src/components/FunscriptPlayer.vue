<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { setConfig, sendWaypoints, subscribeState, unsubscribeState } from '../api'
import { stateMachine } from '../connectionStateMachine'
import type { FunscriptDocument, FunscriptAction, StreamWaypoint, MotorState } from '../types'

const { t } = useI18n()

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
const statusMessage = ref(t('funscript.loadPrompt'))

const canvasRef = ref<HTMLCanvasElement | null>(null)
let streamTimer: number | null = null
let streamStateListener: ((st: MotorState) => void) | null = null

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
        statusMessage.value = t('funscript.invalidFormat')
        return
      }
      actions.value = doc.actions.slice().sort((a, b) => a.at - b.at)
      currentTime.value = 0
      lastSentIndex.value = 0
      statusMessage.value = `${actions.value.length} · ${formatTime(totalDuration.value)}`
      drawTimeline()
    } catch (err) {
      statusMessage.value = t('funscript.parseFailed')
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
  stateMachine.beginEnginePlayback('FUNSCRIPT')
  statusMessage.value = t('funscript.starting')

  try {
    stateMachine.beginLocalEdit()
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
    } finally {
      stateMachine.endLocalEdit()
    }

    const onStreamState = (st: MotorState) => {
      if (st.stream) {
        bufferedCount.value = st.stream.buffered
        streamTime.value = st.stream.stream_time
      }
    }
    streamStateListener = onStreamState
    await subscribeState(onStreamState, 200)

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

    statusMessage.value = t('funscript.playing')
  } catch (err) {
    console.error(err)
    statusMessage.value = t('funscript.playError')
    isPlaying.value = false
  }
}

async function stopPlayback() {
  isPlaying.value = false
  stateMachine.endEnginePlayback()
  if (streamTimer) {
    clearInterval(streamTimer)
    streamTimer = null
  }
  try {
    if (streamStateListener) {
      await unsubscribeState(streamStateListener)
      streamStateListener = null
    }
    stateMachine.beginLocalEdit()
    try {
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
    } finally {
      stateMachine.endLocalEdit()
    }
  } catch (e) {
    console.warn('Error stopping playback:', e)
  }
  statusMessage.value = t('funscript.stopped')
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
    <div class="flex flex-col md:flex-row md:items-center md:justify-between gap-4 bg-white p-6 rounded-2xl border border-gray-200 shadow-md transition-all">
      <div>
        <h2 class="text-xl font-extrabold text-gray-900 tracking-tight">{{ t('funscript.title') }}</h2>
        <p class="text-sm font-medium text-gray-500 mt-0.5">{{ scriptName || t('funscript.noScript') }}</p>
      </div>
      <label class="cursor-pointer bg-gradient-to-r from-emerald-500 to-teal-600 hover:from-emerald-600 hover:to-teal-700 text-white text-sm font-bold px-5 py-2.5 rounded-xl shadow-sm hover:shadow transition-all active:scale-95 inline-flex items-center gap-2">
        <span>{{ t('funscript.load') }}</span>
        <input type="file" accept=".funscript,.json" class="hidden" @change="handleFileUpload" />
      </label>
    </div>

    <!-- Timeline Canvas -->
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-md relative overflow-hidden transition-all">
      <div class="flex items-center justify-between text-xs font-bold text-gray-600 mb-2 px-1">
        <span>{{ t('funscript.current', { time: formatTime(currentTime) }) }}</span>
        <span>{{ t('funscript.duration', { time: formatTime(totalDuration) }) }}</span>
      </div>
      <canvas
        ref="canvasRef"
        class="w-full h-44 cursor-pointer rounded-xl bg-gray-50 border border-gray-300 shadow-inner"
        @click="handleCanvasClick"
      ></canvas>
    </div>

    <!-- Playback Controls -->
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-md flex flex-wrap items-center justify-between gap-4 transition-all">
      <div class="flex items-center gap-3">
        <button
          v-if="!isPlaying"
          @click="startPlayback"
          :disabled="actions.length === 0"
          class="bg-gradient-to-r from-emerald-500 to-teal-600 hover:from-emerald-600 hover:to-teal-700 disabled:from-gray-300 disabled:to-gray-300 disabled:text-gray-500 text-white font-extrabold text-sm px-6 py-3 rounded-xl transition-all shadow-md hover:shadow-lg active:scale-95 cursor-pointer"
        >
          {{ t('funscript.play') }}
        </button>
        <button
          v-else
          @click="stopPlayback"
          class="bg-gradient-to-r from-amber-500 to-orange-600 hover:from-amber-600 hover:to-orange-700 text-white font-extrabold text-sm px-6 py-3 rounded-xl transition-all shadow-md hover:shadow-lg active:scale-95 cursor-pointer"
        >
          {{ t('funscript.pause') }}
        </button>
        <button
          @click="stopPlayback"
          class="inline-flex items-center justify-center gap-2 bg-gray-100 hover:bg-gray-200 text-gray-700 font-bold text-sm px-4 py-3 rounded-xl transition-all border border-gray-300 shadow-2xs active:scale-95 cursor-pointer"
        >
          {{ t('funscript.stop') }}
        </button>
      </div>

      <!-- Speed Multiplier -->
      <div class="flex items-center gap-2.5">
        <span class="text-xs font-bold text-gray-600 uppercase tracking-wider">{{ t('funscript.speed') }}</span>
        <select
          v-model.number="speedMultiplier"
          class="bg-gray-50 border border-gray-300 text-gray-900 font-bold text-sm rounded-xl px-3 py-1.5 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all cursor-pointer"
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
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-md space-y-4 transition-all">
      <h3 class="text-sm font-extrabold text-gray-800 tracking-wide uppercase">{{ t('funscript.strokeLimits') }}</h3>
      <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div>
          <div class="flex justify-between text-xs text-gray-600 mb-1 font-bold">
            <span>{{ t('funscript.minDepth') }}</span>
            <span class="text-emerald-600 font-extrabold">{{ Math.round(minDepth * 100) }}%</span>
          </div>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            v-model.number="minDepth"
            class="w-full accent-emerald-600 bg-gray-200 rounded-lg h-2 cursor-pointer"
          />
        </div>

        <div>
          <div class="flex justify-between text-xs text-gray-600 mb-1 font-bold">
            <span>{{ t('funscript.maxDepth') }}</span>
            <span class="text-emerald-600 font-extrabold">{{ Math.round(maxDepth * 100) }}%</span>
          </div>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            v-model.number="maxDepth"
            class="w-full accent-emerald-600 bg-gray-200 rounded-lg h-2 cursor-pointer"
          />
        </div>
      </div>
    </div>

    <!-- Status Bar -->
    <div class="bg-gray-50 px-4 py-3 rounded-xl border border-gray-200 flex items-center justify-between text-xs font-semibold">
      <span class="text-gray-600">{{ statusMessage }}</span>
      <span v-if="isPlaying" class="text-emerald-600 font-mono font-bold">{{ t('funscript.buffered', { count: bufferedCount }) }}</span>
    </div>
  </div>
</template>
