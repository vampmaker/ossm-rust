<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import MainControl from './components/MainControl.vue'
import SplineEditor from './components/SplineEditor.vue'
import * as api from './api'
import type { MotorControllerConfig } from './types'

const defaultConfig: MotorControllerConfig = {
  bpm: 60.0,
  depth: 1.0,
  depth_top: true,
  reversed: false,
  wave_func: 'sine',
  sharpness: 0.5,
  spline_points: [0.0, 1.0],
  paused: true,
  paused_position: 0.5,
}

const config = ref<MotorControllerConfig>(defaultConfig)
const connected = ref(false)
const error = ref<string | null>(null)
const isInitialized = ref(false)

let debounceTimer: number | undefined
let pausedPositionDebounceTimer: number | undefined

function setConfig(newConfig: MotorControllerConfig) {
  config.value = newConfig
  error.value = null

  clearTimeout(debounceTimer)
  debounceTimer = setTimeout(async () => {
    try {
      await api.setConfig(config.value)
    }
    catch (e) {
      console.error(e)
      error.value = 'Failed to set config'
      connected.value = false
    }
  }, 200)
}

async function setPaused(paused: boolean) {
  const newConfig = { ...config.value, paused }
  config.value = newConfig // update UI immediately
  error.value = null

  clearTimeout(debounceTimer)
  try {
    const updatedConfig = await api.setPaused({ paused })
    config.value = updatedConfig
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to set paused state'
    connected.value = false
  }
}

function setPausedPosition(position: number) {
  if (config.value) {
    config.value.paused_position = position
  }
  error.value = null

  clearTimeout(pausedPositionDebounceTimer)
  pausedPositionDebounceTimer = setTimeout(async () => {
    try {
      // we don't need the returned config as it might be out of date if the user is still sliding
      await api.setPaused({ position })
    }
    catch (e) {
      console.error(e)
      error.value = 'Failed to set paused position'
      connected.value = false
    }
  }, 100)
}

async function fetchConfig() {
  try {
    config.value = await api.getConfig()
    connected.value = true
    error.value = null
    isInitialized.value = true
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to connect to device'
    connected.value = false
  }
}

onMounted(() => {
  fetchConfig()
})

// when wave_func is changed to thrust, set sharpness to 0.1
watch(() => config.value.wave_func, (newWaveFunc, oldWaveFunc) => {
  if (isInitialized.value && newWaveFunc === 'thrust' && oldWaveFunc !== 'thrust') {
    setConfig({ ...config.value, sharpness: 0.1 })
  }
})
</script>

<template>
  <div class="min-h-screen bg-gray-100 text-gray-800">
    <div class="container mx-auto max-w-2xl p-4">
      <header class="mb-4 flex items-center justify-between">
        <h1 class="text-2xl font-bold">
          OSSM-Rust Controller
        </h1>
        <div class="flex items-center space-x-2">
          <span
            class="h-3 w-3"
            :class="{ 'bg-green-500': connected, 'bg-red-500': !connected }"
          />
          <span>{{ connected ? 'Connected' : 'Disconnected' }}</span>
        </div>
      </header>

      <div v-if="error" class="mb-4 bg-red-500 p-2 text-white">
        {{ error }}
        <button class="ml-4 font-bold" @click="fetchConfig">
          Retry
        </button>
      </div>

      <div class="space-y-4">
        <MainControl
          v-model="config" :connected="connected" @update:model-value="setConfig"
          @set-paused="setPaused" @set-paused-position="setPausedPosition"
        />
        <SplineEditor
          v-if="config.wave_func === 'spline'"
          v-model="config" :connected="connected"
          @update:model-value="setConfig"
        />
      </div>
    </div>
  </div>
</template>

<style scoped></style>
