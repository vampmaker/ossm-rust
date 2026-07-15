<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import type { DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const props = defineProps<{
  state: DriveState | null
}>()

const canvasRef = ref<HTMLCanvasElement | null>(null)
let animationFrameId: number | null = null

const X_MAX = 100
const Y_MAX = 8

/** Fixed-length ring buffer of raw signed register values (VB plot_line). */
interface Sample {
  current: number
  pwm: number
  speed: number
  voltage: number
}

const samples: Sample[] = []
let writeIndex = 0

// QBColor-ish: current yellow, pwm magenta, speed cyan, voltage gray
const colors = {
  current: '#ca8a04',
  pwm: '#c026d3',
  speed: '#0891b2',
  voltage: '#6b7280',
}

const visibility = ref({
  current: true,
  pwm: true,
  speed: true,
  voltage: true,
})

function clearPlot() {
  samples.length = 0
  writeIndex = 0
}

watch(
  () => props.state,
  (newState) => {
    if (!newState) return

    if (writeIndex >= X_MAX) {
      clearPlot()
    }

    samples[writeIndex] = {
      current: newState.currentRaw,
      pwm: newState.pwmRaw,
      speed: newState.speedRaw,
      voltage: newState.voltageRaw,
    }
    writeIndex += 1
  },
  { deep: true },
)

/** Map raw signed /32768 into canvas Y (0=top), matching VB Pic1.Line formula. */
function rawToY(raw: number, height: number): number {
  return height - (raw / 32768 + 1) * (height / 2)
}

function draw() {
  const canvas = canvasRef.value
  if (!canvas) {
    animationFrameId = requestAnimationFrame(draw)
    return
  }

  const ctx = canvas.getContext('2d')
  if (!ctx) {
    animationFrameId = requestAnimationFrame(draw)
    return
  }

  const dpr = window.devicePixelRatio || 1
  const rect = canvas.getBoundingClientRect()

  if (canvas.width !== rect.width * dpr || canvas.height !== rect.height * dpr) {
    canvas.width = rect.width * dpr
    canvas.height = rect.height * dpr
  }

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  const width = rect.width
  const height = rect.height

  ctx.fillStyle = '#ffffff'
  ctx.fillRect(0, 0, width, height)

  // Grid: Y ±8 divisions (9 lines), X 0..100 (11 lines) — VB pic1_init
  ctx.strokeStyle = '#9ca3af'
  ctx.lineWidth = 1
  ctx.beginPath()
  for (let i = 0; i <= 8; i++) {
    const y = (height * i) / 8
    ctx.moveTo(0, y)
    ctx.lineTo(width, y)
  }
  for (let i = 0; i <= 10; i++) {
    const x = (width * i) / 10
    ctx.moveTo(x, 0)
    ctx.lineTo(x, height)
  }
  ctx.stroke()

  // Axis accents
  ctx.strokeStyle = '#6b7280'
  ctx.beginPath()
  ctx.moveTo(0, height / 2)
  ctx.lineTo(width, height / 2)
  ctx.moveTo(width / 2, 0)
  ctx.lineTo(width / 2, height)
  ctx.stroke()

  const drawChannel = (key: keyof typeof colors) => {
    if (!visibility.value[key] || writeIndex < 2) return
    ctx.beginPath()
    ctx.strokeStyle = colors[key]
    ctx.lineWidth = 1.5
    ctx.lineJoin = 'round'

    for (let i = 0; i < writeIndex; i++) {
      const sample = samples[i]
      if (!sample) continue
      const x = (i * width) / X_MAX
      const y = rawToY(sample[key], height)
      if (i === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    ctx.stroke()
  }

  drawChannel('voltage')
  drawChannel('pwm')
  drawChannel('speed')
  drawChannel('current')

  // Draw VB6-style white label cutouts for Y axis (8, 6, 4, 2, 0, -2, -4, -6, -8) along left edge
  ctx.font = '11px sans-serif'
  ctx.textAlign = 'left'
  ctx.textBaseline = 'middle'
  for (let i = 0; i <= 8; i++) {
    const y = (height * i) / 8
    const val = 8 - i * 2
    const text = String(val)
    const metrics = ctx.measureText(text)
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(1, Math.max(1, Math.min(height - 15, y - 7)), metrics.width + 4, 14)
    ctx.fillStyle = '#000000'
    ctx.fillText(text, 3, Math.max(8, Math.min(height - 8, y)))
  }

  // Draw VB6-style white label cutouts for X axis (0, 10, 20... 100) along bottom edge
  ctx.textAlign = 'center'
  ctx.textBaseline = 'bottom'
  for (let i = 0; i <= 10; i++) {
    const x = (width * i) / 10
    const text = String(i * 10)
    const metrics = ctx.measureText(text)
    const boxW = metrics.width + 6
    const boxLeft = Math.max(1, Math.min(width - boxW - 1, x - boxW / 2))
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(boxLeft, height - 16, boxW, 15)
    ctx.fillStyle = '#000000'
    ctx.fillText(text, boxLeft + boxW / 2, height - 2)
  }

  animationFrameId = requestAnimationFrame(draw)
}

onMounted(() => {
  animationFrameId = requestAnimationFrame(draw)
})

onUnmounted(() => {
  if (animationFrameId) cancelAnimationFrame(animationFrameId)
})
</script>

<template>
  <GroupBox caption="波形显示" class="h-full flex flex-col">
    <div class="flex flex-wrap items-center gap-3 text-[11px] mb-1.5 px-1 select-none">
      <label class="flex items-center gap-1 cursor-pointer font-medium">
        <input v-model="visibility.current" type="checkbox" class="accent-yellow-600" />
        <span style="color: #ca8a04">电流</span>
      </label>
      <label class="flex items-center gap-1 cursor-pointer font-medium">
        <input v-model="visibility.pwm" type="checkbox" class="accent-fuchsia-600" />
        <span style="color: #c026d3">输出脉宽</span>
      </label>
      <label class="flex items-center gap-1 cursor-pointer font-medium">
        <input v-model="visibility.speed" type="checkbox" class="accent-cyan-600" />
        <span style="color: #0891b2">转速</span>
      </label>
      <label class="flex items-center gap-1 cursor-pointer font-medium">
        <input v-model="visibility.voltage" type="checkbox" class="accent-gray-500" />
        <span style="color: #6b7280">电压</span>
      </label>
      <span class="text-gray-600 ml-auto">X: 0..{{ X_MAX }} · Y: ±{{ Y_MAX }} · /32768</span>
    </div>
    <div class="relative flex-1 w-full min-h-[260px] border border-gray-500 bg-white shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] overflow-hidden">
      <canvas ref="canvasRef" class="absolute inset-0 w-full h-full" />
    </div>
  </GroupBox>
</template>

