<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from 'vue'
import { Settings, Bluetooth, Wifi, Activity } from '@lucide/vue'
import MotorPositionDiagram from './components/MotorPositionDiagram.vue'
import MainControl from './components/MainControl.vue'
import SplineEditor from './components/SplineEditor.vue'
import ModbusSettings from './components/ModbusSettings.vue'
import NetworkSettings from './components/NetworkSettings.vue'
import FunscriptPlayer from './components/FunscriptPlayer.vue'
import * as api from './api'
import type { MotorControllerConfig, PauseMode, PinConfiguration, NetworkConfiguration, MotorState } from './types'

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
  ble_enabled: true,
}

const defaultNetConfig: NetworkConfiguration = {
  hostname: 'ossm',
  dhcp_enabled: true,
  static_ip: '192.168.1.100',
  static_mask: '255.255.255.0',
  static_gateway: '192.168.1.1',
  static_dns: '8.8.8.8',
}

const config = ref<MotorControllerConfig>(defaultConfig)
const pinConfig = ref<PinConfiguration>(defaultPinConfig)
const netConfig = ref<NetworkConfiguration>(defaultNetConfig)
const connected = ref(false)
const motorReady = ref(false)
const showSettings = ref(false)
const DIAGRAM_STORAGE_KEY = 'ossm_show_position_diagram'
const showDiagram = ref(localStorage.getItem(DIAGRAM_STORAGE_KEY) !== 'false')
watch(showDiagram, (val) => {
  localStorage.setItem(DIAGRAM_STORAGE_KEY, String(val))
})
const pauseMode = ref<PauseMode>('fixed-position')
const pinConfigSaving = ref(false)
const pinConfigSaved = ref(false)
const netConfigSaving = ref(false)
const netConfigSaved = ref(false)
const error = ref<string | null>(null)
const isInitialized = ref(false)
const mode = ref<api.ConnectionMode>(api.getConnectionMode())
const bleConnecting = ref(false)
const isHttps = window.location.protocol === 'https:'
const motorState = ref<MotorState | null>(null)

function onMotorStateUpdate(state: MotorState) {
  motorState.value = state
}

async function syncLiveStateSubscription() {
  if (!isInitialized.value) return
  if (showDiagram.value || showSettings.value) {
    try {
      await api.subscribeState(onMotorStateUpdate, 33)
    } catch (e) {
      console.warn('Could not subscribe to live motor state:', e)
    }
  } else {
    void api.unsubscribeState(onMotorStateUpdate)
  }
}

watch([showDiagram, showSettings], () => {
  void syncLiveStateSubscription()
})

async function switchMode(newMode: api.ConnectionMode) {
  mode.value = newMode
  api.setConnectionMode(newMode)
  connected.value = false
  motorReady.value = false
  motorState.value = null
  error.value = null
  
  if (newMode === 'wifi') {
    api.disconnectBle()
    await fetchConfig()
  } else {
    api.disconnectBle()
  }
}

async function connectToBle() {
  bleConnecting.value = true
  error.value = null
  try {
    const ok = await api.connectBle(() => {
      connected.value = false
      motorReady.value = false
      error.value = 'Bluetooth disconnected'
    })
    if (ok) {
      await fetchConfig()
    }
  } catch (e: any) {
    console.error(e)
    error.value = e.message || 'Failed to connect to Bluetooth device'
    connected.value = false
    motorReady.value = false
  } finally {
    bleConnecting.value = false
  }
}

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

  if (mode.value === 'bluetooth' && !api.isBleConnected()) {
    connected.value = false
    motorReady.value = false
    return
  }

  try {
    pinConfig.value = await api.getPinConfig()
    try {
      netConfig.value = await api.getNetworkConfig()
    } catch (e) {
      console.warn('Failed to fetch network config:', e)
    }
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
    await syncLiveStateSubscription()
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

async function saveNetConfig() {
  netConfigSaving.value = true
  netConfigSaved.value = false
  error.value = null
  try {
    netConfig.value = await api.setNetworkConfig(netConfig.value)
    netConfigSaved.value = true
  }
  catch (e) {
    console.error(e)
    error.value = 'Failed to save Network settings'
  }
  finally {
    netConfigSaving.value = false
  }
}

async function resetNetConfig() {
  netConfig.value = { ...defaultNetConfig }
  await saveNetConfig()
}

async function restartDevice() {
  try {
    error.value = null
    await api.restartDevice()
    connected.value = false
    motorReady.value = false
    error.value = 'Restart command sent. Reconnecting...'
    setTimeout(() => {
      if (mode.value === 'wifi') {
        void fetchConfig()
      } else {
        api.disconnectBle()
      }
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

onBeforeUnmount(() => {
  void api.unsubscribeState(onMotorStateUpdate)
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
      <header class="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h1 class="text-2xl font-bold">
          OSSM-Rust Controller
        </h1>
        <div class="flex items-center space-x-3">
          <!-- Mode switcher -->
          <div class="flex rounded bg-gray-200 p-0.5 text-sm font-medium">
            <button
              class="flex items-center space-x-1 rounded px-2 py-1 transition"
              :class="mode === 'wifi' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchMode('wifi')"
            >
              <Wifi :size="14" />
              <span>WiFi</span>
            </button>
            <button
              class="flex items-center space-x-1 rounded px-2 py-1 transition"
              :class="mode === 'bluetooth' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchMode('bluetooth')"
            >
              <Bluetooth :size="14" />
              <span>BLE</span>
            </button>
          </div>

          <!-- BLE Connect Button -->
          <button
            v-if="mode === 'bluetooth' && !connected"
            class="flex items-center space-x-1 rounded bg-blue-500 px-3 py-1.5 text-sm font-semibold text-white shadow hover:bg-blue-600 disabled:opacity-50"
            :disabled="bleConnecting"
            @click="connectToBle"
          >
            <Bluetooth :size="16" />
            <span>{{ bleConnecting ? 'Connecting...' : 'Connect BLE' }}</span>
          </button>
          <button
            class="rounded p-1.5 disabled:opacity-50"
            :class="showDiagram ? 'bg-blue-500 text-white hover:bg-blue-600' : 'bg-gray-200 hover:bg-gray-300'"
            :title="showDiagram ? 'Hide motor position diagram' : 'Show motor position diagram'"
            aria-label="Toggle motor position diagram"
            @click="showDiagram = !showDiagram"
          >
            <Activity :size="18" :stroke-width="2" />
          </button>
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
            class="h-3 w-3 rounded-full"
            :class="{ 'bg-green-500': connected, 'bg-red-500': !connected }"
          />
          <span class="text-sm font-medium">{{ connected ? (motorReady ? 'Connected' : 'Modbus only') : 'Disconnected' }}</span>
        </div>
      </header>

      <div v-if="mode === 'wifi' && isHttps" class="mb-4 rounded bg-amber-500 p-2.5 text-sm text-white shadow">
        <strong>Note:</strong> You have switched to WiFi mode on an HTTPS webpage. This will not work if the device is hosted on HTTP due to browser Mixed Content restrictions. Convenient for debugging or plain HTTP hosting.
      </div>

      <div v-if="error" class="mb-4 bg-red-500 p-2 text-white">
        {{ error }}
        <button class="ml-4 font-bold" @click="fetchConfig">
          Retry
        </button>
      </div>

      <div class="space-y-4">
        <MotorPositionDiagram
          v-if="showDiagram"
          :config="config"
          :state="motorState"
          :connected="motorReady"
        />
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
        <FunscriptPlayer
          v-if="config.wave_func === 'funscript'"
        />
        <ModbusSettings
          v-if="showSettings"
          v-model="pinConfig"
          :state="motorState"
          :connected="connected"
          :saving="pinConfigSaving"
          :saved="pinConfigSaved"
          @save="savePinConfig"
          @reset="resetPinConfig"
          @restart="restartDevice"
        />
        <NetworkSettings
          v-if="showSettings"
          v-model="netConfig"
          :connected="connected"
          :saving="netConfigSaving"
          :saved="netConfigSaved"
          @save="saveNetConfig"
          @reset="resetNetConfig"
        />
      </div>
    </div>
  </div>
</template>

<style scoped></style>
