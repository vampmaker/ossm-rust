<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Loader2 } from '@lucide/vue'
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
])
</script>

<template>
  <div class="space-y-4 bg-white p-4 shadow-md">
    <div class="flex items-center justify-between">
      <div class="flex items-center space-x-2.5">
        <h2 class="text-xl font-bold">{{ t('mainControl.title') }}</h2>
        <span
          v-if="isMutating"
          id="control-busy-indicator"
          class="flex items-center space-x-1.5 rounded-full bg-blue-50 px-2.5 py-0.5 text-xs font-semibold text-blue-600 border border-blue-200 shadow-sm"
          :title="t('mainControl.busyTitle')"
        >
          <Loader2 class="h-3.5 w-3.5 animate-spin" />
          <span>{{ t('mainControl.updating') }}</span>
        </span>
      </div>
      <button
        class="px-4 py-2 font-bold text-white"
        :class="modelValue.paused ? 'bg-green-500 hover:bg-green-600' : 'bg-red-500 hover:bg-red-600'"
        :disabled="!connected" @click="emit('setPaused', !modelValue.paused)"
      >
        {{ modelValue.paused ? t('mainControl.start') : t('mainControl.stop') }}
      </button>
    </div>

    <!-- Sliders -->
    <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <div>
        <label :for="`speed-slider`" class="mb-1 block">{{ t('mainControl.speed', { bpm: modelValue.bpm.toFixed(0) }) }}</label>
        <input
          :id="`speed-slider`" type="range" min="10" max="300" :value="modelValue.bpm" class="w-full"
          :disabled="!connected" @input="updateField('bpm', Number.parseInt(($event.target as HTMLInputElement).value))"
        >
      </div>
      <div>
        <label :for="`depth-slider`" class="mb-1 block">{{ t('mainControl.depth', { depth: (modelValue.depth * 100).toFixed(0) }) }}</label>
        <input
          :id="`depth-slider`" type="range" min="0" max="1" step="0.01" :value="modelValue.depth"
          class="w-full" :disabled="!connected"
          @input="updateField('depth', Number.parseFloat(($event.target as HTMLInputElement).value))"
        >
      </div>
    </div>

    <!-- Paused Position Slider -->
    <div v-if="modelValue.paused">
      <label :for="`paused-position-slider`" class="mb-1 block">{{ t('mainControl.position', { position: (modelValue.paused_position * 100).toFixed(0) }) }}</label>
      <input
        :id="`paused-position-slider`"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="modelValue.paused_position"
        class="w-full"
        :disabled="!connected"
        @input="setPausedPosition(Number.parseFloat(($event.target as HTMLInputElement).value))"
      >
    </div>

    <!-- Wave Selection -->
    <div>
      <label class="mb-1 block">{{ t('mainControl.waveform') }}</label>
      <div class="flex space-x-2">
        <button
          v-for="wave in waveFunctions" :key="wave.value"
          class="px-3 py-1"
          :class="{
            'bg-blue-600 text-white': modelValue.wave_func === wave.value,
            'bg-gray-200 hover:bg-gray-300': modelValue.wave_func !== wave.value,
          }"
          :disabled="!connected" @click="updateField('wave_func', wave.value)"
        >
          {{ wave.name }}
        </button>
      </div>
    </div>

    <!-- Checkboxes -->
    <div class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <div class="flex items-center">
        <input
          :id="`reversed-checkbox`" type="checkbox" :checked="modelValue.reversed" class="mr-2"
          :disabled="!connected" @input="updateField('reversed', ($event.target as HTMLInputElement).checked)"
        >
        <label :for="`reversed-checkbox`">{{ t('mainControl.reversed') }}</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          :title="t('mainControl.reversedHint')"
        >?</span>
      </div>
      <div class="flex items-center">
        <input
          :id="`depth-top-checkbox`" type="checkbox" :checked="modelValue.depth_top" class="mr-2"
          :disabled="!connected" @input="updateField('depth_top', ($event.target as HTMLInputElement).checked)"
        >
        <label :for="`depth-top-checkbox`">{{ t('mainControl.depthFromTop') }}</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          :title="t('mainControl.depthFromTopHint')"
        >?</span>
      </div>
      <div class="flex items-center">
        <input
          :id="`pause-mode-in-place-checkbox`"
          type="checkbox"
          :checked="pauseMode === 'in-place'"
          class="mr-2"
          :disabled="!connected"
          @input="emit('setPauseMode', ($event.target as HTMLInputElement).checked ? 'in-place' : 'fixed-position')"
        >
        <label :for="`pause-mode-in-place-checkbox`">{{ t('mainControl.pauseInPlace') }}</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          :title="t('mainControl.pauseInPlaceHint')"
        >?</span>
      </div>
    </div>
  </div>
</template>
