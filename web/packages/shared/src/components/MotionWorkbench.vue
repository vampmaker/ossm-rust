<script setup lang="ts">
import type { MotorControllerConfig, MotorState, PauseMode } from '../types'
import MotorPositionDiagram from './MotorPositionDiagram.vue'
import MainControl from './MainControl.vue'
import SplineEditor from './SplineEditor.vue'
import FunscriptPlayer from './FunscriptPlayer.vue'
import MacroPlayer from './MacroPlayer.vue'

const config = defineModel<MotorControllerConfig>('config', { required: true })
const pauseMode = defineModel<PauseMode>('pauseMode', { required: true })

defineProps<{
  state: MotorState | null
  connected: boolean
  showDiagram: boolean
  isMutating?: boolean
  hideMotion?: boolean
}>()

const emit = defineEmits<{
  setPaused: [paused: boolean]
  setPausedPosition: [position: number]
  postedConfig: [config: MotorControllerConfig]
  macroConfig: [config: MotorControllerConfig]
}>()

function onPostedConfig(next: MotorControllerConfig) {
  config.value = next
  emit('postedConfig', next)
}

function onMacroConfig(next: MotorControllerConfig) {
  config.value = next
  emit('macroConfig', next)
}
</script>

<template>
  <div class="space-y-4">
    <MotorPositionDiagram
      v-if="showDiagram && !hideMotion"
      :config="config"
      :state="state"
      :connected="connected"
    />
    <MainControl
      v-if="!hideMotion"
      v-model="config"
      :pause-mode="pauseMode"
      :connected="connected"
      :is-mutating="isMutating"
      @update:model-value="onPostedConfig"
      @set-paused="emit('setPaused', $event)"
      @set-paused-position="emit('setPausedPosition', $event)"
      @set-pause-mode="pauseMode = $event"
    />
    <SplineEditor
      v-if="!hideMotion && config.wave_func === 'spline'"
      v-model="config"
      :connected="connected"
      @update:model-value="onPostedConfig"
    />
    <FunscriptPlayer v-if="!hideMotion && config.wave_func === 'funscript'" />
    <MacroPlayer
      v-if="!hideMotion && config.wave_func === 'macro'"
      v-model="config"
      :connected="connected"
      @update:model-value="onMacroConfig"
    />
  </div>
</template>
