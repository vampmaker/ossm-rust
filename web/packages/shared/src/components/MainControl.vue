<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Loader2, Play, Square, Gauge, Maximize2, Navigation } from '@lucide/vue'
import type { MotorControllerConfig, PauseMode, WaveFunc } from '../types'

const props = defineProps<{
  modelValue: MotorControllerConfig
  connected: boolean
  pauseMode: PauseMode
  isMutating?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: MotorControllerConfig): void
  (e: 'setPaused', paused: boolean): void
  (e: 'setPausedPosition', position: number): void
  (e: 'setPauseMode', mode: PauseMode): void
}>()

const { t } = useI18n()

function updateField<K extends keyof MotorControllerConfig>(
  key: K,
  value: MotorControllerConfig[K],
) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}

function setPausedPosition(position: number) {
  emit('setPausedPosition', position)
}

const waveFunctions = computed(() => [
  { name: t('mainControl.waveSine'), value: 'sine' as WaveFunc },
  { name: t('mainControl.waveThrust'), value: 'thrust' as WaveFunc },
  { name: t('mainControl.waveSpline'), value: 'spline' as WaveFunc },
  { name: t('mainControl.waveFunscript'), value: 'funscript' as WaveFunc },
  { name: t('mainControl.waveMacro'), value: 'macro' as WaveFunc },
])
</script>

<template>
  <div class="space-y-6 rounded-2xl bg-white p-6 shadow-sm border border-slate-200/80 hover:shadow-md transition-all">
    <!-- Header -->
    <div class="flex items-center justify-between gap-4">
      <div class="flex items-center space-x-3">
        <h2 class="text-xl font-extrabold text-slate-900 tracking-tight">{{ t('mainControl.title') }}</h2>
        <span
          v-if="isMutating"
          id="control-busy-indicator"
          class="flex items-center space-x-1.5 rounded-full bg-blue-50 px-2.5 py-0.5 text-xs font-semibold text-blue-600 border border-blue-200 shadow-2xs"
          :title="t('mainControl.busyTitle')"
        >
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
          <span>{{ t('mainControl.updating') }}</span>
        </span>
      </div>
      <button
        type="button"
        class="inline-flex items-center gap-2 px-5 py-2.5 font-extrabold text-sm rounded-xl text-white shadow-sm hover:shadow active:scale-95 transition-all disabled:opacity-40 disabled:pointer-events-none cursor-pointer"
        :class="modelValue.paused
          ? 'bg-gradient-to-r from-emerald-500 to-teal-600 hover:from-emerald-600 hover:to-teal-700 shadow-emerald-500/20'
          : 'bg-gradient-to-r from-rose-500 to-red-600 hover:from-rose-600 hover:to-red-700 shadow-rose-500/20 animate-pulse'"
        :disabled="!connected"
        @click="emit('setPaused', !modelValue.paused)"
      >
        <Play v-if="modelValue.paused" :size="15" class="fill-white shrink-0" />
        <Square v-else :size="15" class="fill-white shrink-0" />
        <span>{{ modelValue.paused ? t('mainControl.start') : t('mainControl.stop') }}</span>
      </button>
    </div>

    <!-- Sliders Grid (bounded in width for wide-screen ergonomics) -->
    <div class="grid grid-cols-1 gap-5 md:grid-cols-2">
      <!-- Speed Slider -->
      <div class="rounded-xl bg-slate-50/70 border border-slate-200/80 p-4 space-y-2.5 shadow-2xs">
        <div class="flex items-center justify-between">
          <label :for="`speed-slider`" class="flex items-center gap-1.5 text-sm text-slate-700 select-none">
            <Gauge :size="15" class="text-blue-600 shrink-0" />
            <span class="font-extrabold text-slate-900 tracking-tight">{{ t('mainControl.speedLabel') }}:</span>
            <span class="font-extrabold text-blue-600 font-mono text-base">{{ modelValue.bpm.toFixed(0) }}</span>
            <span class="text-xs font-bold text-slate-500">BPM</span>
          </label>
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-mono font-extrabold bg-blue-50 text-blue-700 border border-blue-200 shadow-2xs">
            {{ modelValue.bpm.toFixed(0) }} BPM
          </span>
        </div>
        <input
          :id="`speed-slider`"
          type="range"
          min="10"
          max="300"
          :value="modelValue.bpm"
          class="w-full cursor-pointer"
          :disabled="!connected"
          @input="updateField('bpm', Number.parseInt(($event.target as HTMLInputElement).value))"
        >
      </div>

      <!-- Depth Slider -->
      <div class="rounded-xl bg-slate-50/70 border border-slate-200/80 p-4 space-y-2.5 shadow-2xs">
        <div class="flex items-center justify-between">
          <label :for="`depth-slider`" class="flex items-center gap-1.5 text-sm text-slate-700 select-none">
            <Maximize2 :size="15" class="text-indigo-600 shrink-0" />
            <span class="font-extrabold text-slate-900 tracking-tight">{{ t('mainControl.depthLabel') }}:</span>
            <span class="font-extrabold text-indigo-600 font-mono text-base">{{ (modelValue.depth * 100).toFixed(0) }}</span>
            <span class="text-xs font-bold text-slate-500">%</span>
          </label>
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-mono font-extrabold bg-indigo-50 text-indigo-700 border border-indigo-200 shadow-2xs">
            {{ (modelValue.depth * 100).toFixed(0) }}%
          </span>
        </div>
        <input
          :id="`depth-slider`"
          type="range"
          min="0"
          max="1"
          step="0.01"
          :value="modelValue.depth"
          class="w-full cursor-pointer"
          :disabled="!connected"
          @input="updateField('depth', Number.parseFloat(($event.target as HTMLInputElement).value))"
        >
      </div>
    </div>

    <!-- Paused Position Slider (takes full row space) -->
    <div v-if="modelValue.paused" class="w-full rounded-xl bg-amber-50/60 border border-amber-200/80 p-4 space-y-2.5 shadow-2xs">
      <div class="flex items-center justify-between">
        <label :for="`paused-position-slider`" class="flex items-center gap-1.5 text-sm text-amber-950 select-none">
          <Navigation :size="15" class="text-amber-600 shrink-0" />
          <span class="font-extrabold text-amber-950 tracking-tight">{{ t('mainControl.positionLabel') }}:</span>
          <span class="font-extrabold text-amber-700 font-mono text-base">{{ (modelValue.paused_position * 100).toFixed(0) }}</span>
          <span class="text-xs font-bold text-amber-700/80">%</span>
        </label>
        <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-mono font-extrabold bg-amber-100 text-amber-900 border border-amber-300 shadow-2xs">
          {{ (modelValue.paused_position * 100).toFixed(0) }}%
        </span>
      </div>
      <input
        :id="`paused-position-slider`"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="modelValue.paused_position"
        class="w-full cursor-pointer accent-amber-600"
        :disabled="!connected"
        @input="setPausedPosition(Number.parseFloat(($event.target as HTMLInputElement).value))"
      >
    </div>

    <!-- Wave Selection -->
    <div class="space-y-2">
      <label class="text-xs font-bold text-slate-600 block select-none">
        {{ t('mainControl.waveform') }}
      </label>
      <div class="inline-flex flex-wrap gap-1.5 p-1 bg-slate-100/90 rounded-xl border border-slate-200/80 shadow-inner">
        <button
          v-for="wave in waveFunctions"
          :key="wave.value"
          type="button"
          class="px-3.5 py-1.5 text-xs font-extrabold rounded-lg transition-all cursor-pointer disabled:opacity-40"
          :class="modelValue.wave_func === wave.value
            ? 'bg-blue-600 text-white shadow-xs font-extrabold scale-100'
            : 'text-slate-600 hover:text-slate-900 hover:bg-white/70'"
          :disabled="!connected"
          @click="updateField('wave_func', wave.value)"
        >
          {{ wave.name }}
        </button>
      </div>
    </div>

    <!-- Checkboxes / Mode Options -->
    <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <!-- Reversed Option -->
      <label
        :for="`reversed-checkbox`"
        class="flex items-center justify-between p-3 rounded-xl border bg-slate-50/70 hover:bg-slate-100/70 cursor-pointer transition-all select-none group shadow-2xs"
        :class="modelValue.reversed ? 'border-blue-300 bg-blue-50/40' : 'border-slate-200/80'"
      >
        <div class="flex items-center gap-2.5">
          <input
            :id="`reversed-checkbox`"
            type="checkbox"
            :checked="modelValue.reversed"
            class="rounded text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
            :disabled="!connected"
            @input="updateField('reversed', ($event.target as HTMLInputElement).checked)"
          >
          <span class="text-xs font-bold text-slate-800">{{ t('mainControl.reversed') }}</span>
        </div>
        <span
          class="inline-flex h-4 w-4 items-center justify-center rounded-full bg-slate-200/80 text-[10px] font-bold text-slate-600 group-hover:bg-slate-300 transition-colors"
          :title="t('mainControl.reversedHint')"
        >?</span>
      </label>

      <!-- Depth from Top Option -->
      <label
        :for="`depth-top-checkbox`"
        class="flex items-center justify-between p-3 rounded-xl border bg-slate-50/70 hover:bg-slate-100/70 cursor-pointer transition-all select-none group shadow-2xs"
        :class="modelValue.depth_top ? 'border-blue-300 bg-blue-50/40' : 'border-slate-200/80'"
      >
        <div class="flex items-center gap-2.5">
          <input
            :id="`depth-top-checkbox`"
            type="checkbox"
            :checked="modelValue.depth_top"
            class="rounded text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
            :disabled="!connected"
            @input="updateField('depth_top', ($event.target as HTMLInputElement).checked)"
          >
          <span class="text-xs font-bold text-slate-800">{{ t('mainControl.depthFromTop') }}</span>
        </div>
        <span
          class="inline-flex h-4 w-4 items-center justify-center rounded-full bg-slate-200/80 text-[10px] font-bold text-slate-600 group-hover:bg-slate-300 transition-colors"
          :title="t('mainControl.depthFromTopHint')"
        >?</span>
      </label>

      <!-- Pause In-place Option -->
      <label
        :for="`pause-mode-in-place-checkbox`"
        class="flex items-center justify-between p-3 rounded-xl border bg-slate-50/70 hover:bg-slate-100/70 cursor-pointer transition-all select-none group shadow-2xs"
        :class="pauseMode === 'in-place' ? 'border-blue-300 bg-blue-50/40' : 'border-slate-200/80'"
      >
        <div class="flex items-center gap-2.5">
          <input
            :id="`pause-mode-in-place-checkbox`"
            type="checkbox"
            :checked="pauseMode === 'in-place'"
            class="rounded text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
            :disabled="!connected"
            @input="emit('setPauseMode', ($event.target as HTMLInputElement).checked ? 'in-place' : 'fixed-position')"
          >
          <span class="text-xs font-bold text-slate-800">{{ t('mainControl.pauseInPlace') }}</span>
        </div>
        <span
          class="inline-flex h-4 w-4 items-center justify-center rounded-full bg-slate-200/80 text-[10px] font-bold text-slate-600 group-hover:bg-slate-300 transition-colors"
          :title="t('mainControl.pauseInPlaceHint')"
        >?</span>
      </label>
    </div>
  </div>
</template>
