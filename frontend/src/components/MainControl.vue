<script setup lang="ts">
import type { MotorControllerConfig, PauseMode, WaveFunc } from '../types'

const props = defineProps<{
  modelValue: MotorControllerConfig
  connected: boolean
  pauseMode: PauseMode
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: MotorControllerConfig): void
  (e: 'setPaused', paused: boolean): void
  (e: 'setPausedPosition', position: number): void
  (e: 'setPauseMode', mode: PauseMode): void
}>()

function updateField<K extends keyof MotorControllerConfig>(
  key: K,
  value: MotorControllerConfig[K],
) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}

function setPausedPosition(position: number) {
  emit('setPausedPosition', position)
}

const waveFunctions: { name: string, value: WaveFunc }[] = [
  { name: 'Sine', value: 'sine' },
  { name: 'Thrust', value: 'thrust' },
  { name: 'Spline', value: 'spline' },
  { name: 'Funscript', value: 'funscript' },
]
</script>

<template>
  <div class="space-y-4 bg-white p-4 shadow-md">
    <div class="flex items-center justify-between">
      <h2 class="text-xl font-bold">Main Control</h2>
      <button
        class="px-4 py-2 font-bold text-white"
        :class="modelValue.paused ? 'bg-green-500 hover:bg-green-600' : 'bg-red-500 hover:bg-red-600'"
        :disabled="!connected" @click="emit('setPaused', !modelValue.paused)"
      >
        {{ modelValue.paused ? 'Start' : 'Stop' }}
      </button>
    </div>

    <!-- Sliders -->
    <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <div>
        <label :for="`speed-slider`" class="mb-1 block">Speed: {{ modelValue.bpm.toFixed(0) }} BPM</label>
        <input
          :id="`speed-slider`" type="range" min="10" max="300" :value="modelValue.bpm" class="w-full"
          :disabled="!connected" @input="updateField('bpm', Number.parseInt(($event.target as HTMLInputElement).value))"
        >
      </div>
      <div>
        <label :for="`depth-slider`" class="mb-1 block">Depth: {{ (modelValue.depth * 100).toFixed(0) }}%</label>
        <input
          :id="`depth-slider`" type="range" min="0" max="1" step="0.01" :value="modelValue.depth"
          class="w-full" :disabled="!connected"
          @input="updateField('depth', Number.parseFloat(($event.target as HTMLInputElement).value))"
        >
      </div>
    </div>

    <!-- Paused Position Slider -->
    <div v-if="modelValue.paused">
      <label :for="`paused-position-slider`" class="mb-1 block">Position: {{ (modelValue.paused_position * 100).toFixed(0) }}%</label>
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
      <label class="mb-1 block">Waveform</label>
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
        <label :for="`reversed-checkbox`">Reversed Direction</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          title="Invert movement direction so the waveform runs in reverse."
        >?</span>
      </div>
      <div class="flex items-center">
        <input
          :id="`depth-top-checkbox`" type="checkbox" :checked="modelValue.depth_top" class="mr-2"
          :disabled="!connected" @input="updateField('depth_top', ($event.target as HTMLInputElement).checked)"
        >
        <label :for="`depth-top-checkbox`">Depth From Top</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          title="Checked: stroke uses the top range. Unchecked: stroke uses the bottom range."
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
        <label :for="`pause-mode-in-place-checkbox`">Pause in-place</label>
        <span
          class="ml-1 inline-flex h-4 w-4 items-center justify-center rounded-full bg-gray-200 text-xs text-gray-700"
          title="Unchecked: Fixed position (pause slider target). Checked: In-place (capture current motor position on Stop)."
        >?</span>
      </div>
    </div>
  </div>
</template>
