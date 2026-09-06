<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus, Pencil, Trash2, Upload, Download, Sparkles, ChevronDown } from '@lucide/vue'
import type { MotorControllerConfig } from '../types'

const props = defineProps<{
  modelValue: MotorControllerConfig
  connected: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: MotorControllerConfig): void
}>()

const { t } = useI18n()

interface SplinePreset {
  name: string
  points: number[]
}

const presets = ref<SplinePreset[]>([
  { name: 'Default Linear', points: [0, 1] },
  { name: 'Thrust', points: [0, 0, 1, 0.8, 0.5, 0.2] },
  { name: 'Triangle', points: [0, 0.2, 0.4, 0.6, 0.8, 1.0, 0.8, 0.6, 0.4, 0.2] },
])
const selectedPreset = ref<string>('')
const pointsStr = ref<string>('')

function displayPresetName(name: string): string {
  switch (name) {
    case 'Default Linear':
      return t('spline.defaultLinear')
    case 'Thrust':
      return t('spline.thrust')
    case 'Triangle':
      return t('spline.triangle')
    case 'Default':
      return t('spline.default')
    default:
      return name
  }
}

onMounted(() => {
  const savedPresets = localStorage.getItem('splinePresets')
  if (savedPresets) {
    presets.value = JSON.parse(savedPresets)
  }
  const lastSelected = localStorage.getItem('lastSelectedSpline')
  if (lastSelected && presets.value.some(p => p.name === lastSelected)) {
    selectedPreset.value = lastSelected
  }
  else if (presets.value.length > 0) {
    const firstPreset = presets.value[0]
    if (firstPreset) {
      selectedPreset.value = firstPreset.name
    }
  }
  updatePointsFromPreset()
})

watch(presets, (newPresets) => {
  localStorage.setItem('splinePresets', JSON.stringify(newPresets))
}, { deep: true })

watch(selectedPreset, (newName) => {
  localStorage.setItem('lastSelectedSpline', newName)
  updatePointsFromPreset()
})

watch(() => props.modelValue.spline_points, (newPoints) => {
  pointsStr.value = newPoints.join(' ')
}, { immediate: true })

function updatePointsFromPreset() {
  const preset = presets.value.find(p => p.name === selectedPreset.value)
  if (preset) {
    const newConfig = { ...props.modelValue, spline_points: preset.points }
    emit('update:modelValue', newConfig)
  }
}

function applyPoints() {
  const points = pointsStr.value.trim().split(/\s+/).map(Number.parseFloat).filter(n => !Number.isNaN(n) && n >= 0 && n <= 1)
  if (points.length > 1) {
    const newConfig = { ...props.modelValue, spline_points: points }
    emit('update:modelValue', newConfig)

    // also update current preset
    const preset = presets.value.find(p => p.name === selectedPreset.value)
    if (preset) {
      preset.points = points
    }
  }
}

function addPreset() {
  const name = prompt(t('spline.promptNewName'))
  if (name && !presets.value.some(p => p.name === name)) {
    presets.value.push({ name, points: [0, 1] })
    selectedPreset.value = name
  }
}

function renamePreset() {
  const oldName = selectedPreset.value
  const newName = prompt(t('spline.promptRename'), oldName)
  if (newName && newName !== oldName && !presets.value.some(p => p.name === newName)) {
    const preset = presets.value.find(p => p.name === oldName)
    if (preset) {
      preset.name = newName
      selectedPreset.value = newName
    }
  }
}

function deletePreset() {
  if (confirm(t('spline.confirmDelete', { name: selectedPreset.value }))) {
    presets.value = presets.value.filter(p => p.name !== selectedPreset.value)
    if (presets.value.length > 0) {
      const firstPreset = presets.value[0]
      if (firstPreset) {
        selectedPreset.value = firstPreset.name
      }
    }
    else {
      presets.value.push({ name: 'Default', points: [0, 1] })
      selectedPreset.value = 'Default'
    }
  }
}

function exportPresets() {
  const json = JSON.stringify(presets.value, null, 2)
  const blob = new Blob([json], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'spline_presets.json'
  a.click()
  URL.revokeObjectURL(url)
}

function importPresets() {
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = 'application/json'
  input.onchange = async (e) => {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (file) {
      try {
        const text = await file.text()
        const importedPresets: SplinePreset[] = JSON.parse(text)
        if (Array.isArray(importedPresets) && importedPresets.every(p => typeof p.name === 'string' && Array.isArray(p.points))) {
          presets.value = importedPresets
          if (presets.value.length > 0) {
            const firstPreset = presets.value[0]
            if (firstPreset) {
              selectedPreset.value = firstPreset.name
            }
          }
        }
        else {
          alert(t('spline.invalidFile'))
        }
      }
      catch (err) {
        alert(t('spline.importFailed'))
        console.error(err)
      }
    }
  }
  input.click()
}

// Visual curve coordinates calculation for SVG
const svgWidth = 500
const svgHeight = 130
const padX = 25
const padY = 20

const curvePoints = computed(() => {
  const pts = props.modelValue.spline_points ?? [0, 1]
  if (pts.length < 2) return []
  const innerW = svgWidth - padX * 2
  const innerH = svgHeight - padY * 2
  return pts.map((val, idx) => {
    const x = padX + (idx / (pts.length - 1)) * innerW
    const clamped = Math.max(0, Math.min(1, val))
    const y = svgHeight - padY - clamped * innerH
    return { x, y, val: clamped }
  })
})

const curvePath = computed(() => {
  const pts = curvePoints.value
  if (pts.length === 0) return ''
  return pts.reduce((acc, pt, i) => `${acc} ${i === 0 ? 'M' : 'L'} ${pt.x.toFixed(1)} ${pt.y.toFixed(1)}`, '')
})

const curveAreaPath = computed(() => {
  const pts = curvePoints.value
  if (pts.length < 2) return ''
  const first = pts[0]
  const last = pts[pts.length - 1]
  if (!first || !last) return ''
  const bottomY = svgHeight - padY
  return `${curvePath.value} L ${last.x.toFixed(1)} ${bottomY} L ${first.x.toFixed(1)} ${bottomY} Z`
})
</script>

<template>
  <div class="space-y-6 rounded-2xl bg-white p-6 shadow-sm border border-slate-200/80 hover:shadow-md transition-all">
    <!-- Header -->
    <div class="flex items-center justify-between gap-4">
      <div class="flex items-center space-x-3">
        <h2 class="text-xl font-extrabold text-slate-900 tracking-tight">{{ t('spline.title') }}</h2>
        <span class="inline-flex items-center gap-1 rounded-full bg-indigo-50 px-2.5 py-0.5 text-xs font-bold text-indigo-700 border border-indigo-200 shadow-2xs">
          <Sparkles :size="12" />
          <span>Hermite Spline</span>
        </span>
      </div>
    </div>

    <!-- Visual Curve Preview -->
    <div class="rounded-xl bg-slate-900 p-4 shadow-inner border border-slate-800 relative overflow-hidden">
      <div class="flex items-center justify-between text-[11px] font-mono font-bold text-slate-400 mb-1 px-2">
        <span>Stroke Motion Curve</span>
        <span class="text-indigo-400 font-semibold">{{ curvePoints.length }} points</span>
      </div>
      <svg :viewBox="`0 0 ${svgWidth} ${svgHeight}`" class="w-full h-28 select-none">
        <defs>
          <linearGradient id="splineGradient" x1="0%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stop-color="#6366f1" stop-opacity="0.45" />
            <stop offset="100%" stop-color="#6366f1" stop-opacity="0.02" />
          </linearGradient>
        </defs>
        <!-- Horizontal Guide Lines -->
        <line :x1="padX" :y1="padY" :x2="svgWidth - padX" :y2="padY" stroke="#334155" stroke-dasharray="3,3" stroke-width="1" />
        <line :x1="padX" :y1="svgHeight / 2" :x2="svgWidth - padX" :y2="svgHeight / 2" stroke="#334155" stroke-dasharray="3,3" stroke-width="1" />
        <line :x1="padX" :y1="svgHeight - padY" :x2="svgWidth - padX" :y2="svgHeight - padY" stroke="#475569" stroke-width="1" />

        <!-- Area Fill -->
        <path v-if="curveAreaPath" :d="curveAreaPath" fill="url(#splineGradient)" />

        <!-- Curve Line -->
        <path v-if="curvePath" :d="curvePath" fill="none" stroke="#818cf8" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />

        <!-- Point Markers -->
        <g v-for="(pt, idx) in curvePoints" :key="idx">
          <circle :cx="pt.x" :cy="pt.y" r="4.5" fill="#ffffff" stroke="#4f46e5" stroke-width="2.5" />
        </g>
      </svg>
      <div class="flex justify-between text-[10px] font-mono text-slate-400 mt-1 px-4">
        <span>0% (Cycle Start)</span>
        <span>50%</span>
        <span>100% (Cycle End)</span>
      </div>
    </div>

    <!-- Presets & Action Buttons (bounded for wide-screen ergonomics) -->
    <div class="space-y-3">
      <div class="flex flex-col sm:flex-row sm:items-end justify-between gap-3">
        <!-- Preset Dropdown -->
        <div class="flex-1 max-w-sm">
          <label :for="`preset-select`" class="text-xs font-bold text-slate-700 block mb-1.5 select-none">
            {{ t('spline.preset') }}
          </label>
          <div class="relative">
            <select
              :id="`preset-select`"
              v-model="selectedPreset"
              class="w-full appearance-none bg-slate-50 border border-slate-300 hover:border-slate-400 text-slate-900 font-semibold text-sm rounded-xl pl-3.5 pr-9 py-2.5 shadow-2xs focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500 transition-all cursor-pointer disabled:opacity-50"
              :disabled="!connected"
            >
              <option v-for="preset in presets" :key="preset.name" :value="preset.name">
                {{ displayPresetName(preset.name) }}
              </option>
            </select>
            <div class="absolute right-3 top-1/2 -translate-y-1/2 pointer-events-none text-slate-400">
              <ChevronDown :size="16" />
            </div>
          </div>
        </div>

        <!-- Preset Management Buttons -->
        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-emerald-600 text-white hover:bg-emerald-700 shadow-2xs hover:shadow active:scale-95 transition-all disabled:opacity-40 cursor-pointer"
            :disabled="!connected"
            @click="addPreset"
          >
            <Plus :size="14" class="shrink-0" />
            <span>{{ t('spline.new') }}</span>
          </button>
          <button
            type="button"
            class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-amber-500 text-white hover:bg-amber-600 shadow-2xs hover:shadow active:scale-95 transition-all disabled:opacity-40 cursor-pointer"
            :disabled="!connected"
            @click="renamePreset"
          >
            <Pencil :size="14" class="shrink-0" />
            <span>{{ t('spline.rename') }}</span>
          </button>
          <button
            type="button"
            class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-rose-50 text-rose-700 border border-rose-200 hover:bg-rose-100 shadow-2xs hover:shadow active:scale-95 transition-all disabled:opacity-40 cursor-pointer"
            :disabled="!connected"
            @click="deletePreset"
          >
            <Trash2 :size="14" class="shrink-0" />
            <span>{{ t('spline.delete') }}</span>
          </button>
        </div>
      </div>
    </div>

    <!-- Points Textarea -->
    <div class="space-y-1.5">
      <div class="flex items-center justify-between">
        <label :for="`points-textarea`" class="text-xs font-bold text-slate-700 select-none">
          {{ t('spline.points') }}
        </label>
        <span class="text-[11px] text-slate-500 font-medium">
          Values between 0.0 and 1.0
        </span>
      </div>
      <textarea
        :id="`points-textarea`"
        v-model="pointsStr"
        class="w-full font-mono text-sm border border-slate-300 bg-slate-50/70 focus:bg-white text-slate-900 rounded-xl p-3 shadow-2xs focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500 transition-all outline-none"
        rows="2"
        :disabled="!connected"
        @blur="applyPoints"
      />
    </div>

    <!-- Import / Export Actions -->
    <div class="flex items-center gap-3 pt-1 border-t border-slate-100">
      <button
        type="button"
        class="inline-flex items-center justify-center gap-1.5 px-4 py-2 text-xs font-bold rounded-xl bg-slate-100 text-slate-700 hover:bg-slate-200 border border-slate-300 shadow-2xs hover:shadow-sm active:scale-95 transition-all cursor-pointer"
        @click="importPresets"
      >
        <Upload :size="14" class="shrink-0 text-slate-600" />
        <span>{{ t('spline.import') }}</span>
      </button>
      <button
        type="button"
        class="inline-flex items-center justify-center gap-1.5 px-4 py-2 text-xs font-bold rounded-xl bg-slate-100 text-slate-700 hover:bg-slate-200 border border-slate-300 shadow-2xs hover:shadow-sm active:scale-95 transition-all cursor-pointer"
        @click="exportPresets"
      >
        <Download :size="14" class="shrink-0 text-slate-600" />
        <span>{{ t('spline.export') }}</span>
      </button>
    </div>
  </div>
</template>
