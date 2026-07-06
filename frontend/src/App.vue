<script setup lang="ts">
import { ref, onMounted, watch } from 'vue'
import { Settings } from '@lucide/vue'
import MainControl from './components/MainControl.vue'
import SplineEditor from './components/SplineEditor.vue'
import ModbusSettings from './components/ModbusSettings.vue'
import * as api from './api'
import type { MotorControllerConfig, PauseMode, PinConfiguration } from './types'

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

const defaultPinConfig: PinConfiguration = {
  modbus_tx: 18,
  modbus_rx: 19,
  modbus_de_re: 20,
  modbus_timeout_ms: 0,
  modbus_scan_delay_us: 0,
}

const config = ref<MotorControllerConfig>(defaultConfig)
const pinConfig = ref<PinConfiguration>(defaultPinConfig)
const connected = ref(false)
const motorReady = ref(false)
const showSettings = ref(false)
const pauseMode = ref<PauseMode>('fixed-position')
const pinConfigSaving = ref(false)
const pinConfigSaved = ref(false)
const error = ref<string | null>(null)
const isInitialized = ref(false)

let debounceTimer: ReturnType<typeof setTimeout> | undefined
let pausedPositionDebounceTimer: ReturnType<typeof setTimeout> | undefined

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
      motorReady.value = false
    }
  }, 200)
}

async function setPaused(paused: boolean) {
  const newConfig = { ...config.value, paused }
  config.value = newConfig // update UI immediately
  error.value = null

  clearTimeout(debounceTimer)
  try {
    let updatedConfig
    if (paused && pauseMode.value === 'in-place') {
      const state = await api.getState()
      updatedConfig = await api.setPaused({ paused: true, position: state.y })
    }
    else {
      updatedConfig = await api.setPaused({ paused })
    }
    config.value = updatedConfig
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to set paused state'
    motorReady.value = false
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
      motorReady.value = false
    }
  }, 100)
}

async function fetchConfig() {
  error.value = null
  pinConfigSaved.value = false

  try {
    pinConfig.value = await api.getPinConfig()
    connected.value = true
  }
  catch (e) {
    console.error(e)
    connected.value = false
    motorReady.value = false
    error.value = 'Failed to connect to device'
    return
  }

  try {
    config.value = await api.getConfig()
    motorReady.value = true
    isInitialized.value = true
  }
  catch (e) {
    console.error(e)
    motorReady.value = false
    isInitialized.value = false
    error.value = 'Motor not initialized — Modbus settings are still available below'
  }
}

async function savePinConfig() {
  pinConfigSaving.value = true
  pinConfigSaved.value = false
  error.value = null
  try {
    pinConfig.value = await api.setPinConfig(pinConfig.value)
    pinConfigSaved.value = true
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to save Modbus settings'
  }
  finally {
    pinConfigSaving.value = false
  }
}

async function resetPinConfig() {
  pinConfig.value = { ...defaultPinConfig }
  await savePinConfig()
}

async function restartDevice() {
  try {
    error.value = null
    await api.restartDevice()
    connected.value = false
    motorReady.value = false
    error.value = 'Restart command sent. Reconnecting...'
    setTimeout(() => {
      void fetchConfig()
    }, 3000)
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to restart device'
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
        <div class="flex items-center space-x-3">
          <button
            class="rounded p-1.5 disabled:opacity-50"
            :class="showSettings ? 'bg-blue-500 text-white hover:bg-blue-600' : 'bg-gray-200 hover:bg-gray-300'"
            :disabled="!connected"
            :title="showSettings ? 'Hide settings' : 'Show settings'"
            aria-label="Toggle settings"
            @click="showSettings = !showSettings"
          >
            <Settings :size="18" :stroke-width="2" />
          </button>
          <span
            class="h-3 w-3"
            :class="{ 'bg-green-500': connected, 'bg-red-500': !connected }"
          />
          <span>{{ connected ? (motorReady ? 'Connected' : 'Modbus only') : 'Disconnected' }}</span>
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
          v-model="config"
          :pause-mode="pauseMode"
          :connected="motorReady"
          @update:model-value="setConfig"
          @set-paused="setPaused" @set-paused-position="setPausedPosition"
          @set-pause-mode="pauseMode = $event"
        />
        <SplineEditor
          v-if="config.wave_func === 'spline'"
          v-model="config" :connected="motorReady"
          @update:model-value="setConfig"
        />
        <ModbusSettings
          v-if="showSettings"
          v-model="pinConfig"
          :connected="connected"
          :saving="pinConfigSaving"
          :saved="pinConfigSaved"
          @save="savePinConfig"
          @reset="resetPinConfig"
          @restart="restartDevice"
        />
      </div>
    </div>
  </div>
</template>

<style scoped></style>
