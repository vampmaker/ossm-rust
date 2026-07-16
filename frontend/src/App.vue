<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Settings, Bluetooth, Wifi, Activity } from '@lucide/vue'
import MotorPositionDiagram from './components/MotorPositionDiagram.vue'
import MainControl from './components/MainControl.vue'
import SplineEditor from './components/SplineEditor.vue'
import UnifiedSettings from './components/UnifiedSettings.vue'
import FunscriptPlayer from './components/FunscriptPlayer.vue'
import * as api from './api'
import { stateMachine } from './connectionStateMachine'
import { setLocale, type AppLocale } from './i18n'
import type { MotorControllerConfig, PauseMode, PinConfiguration, NetworkConfiguration, MotorState } from './types'

const { t, locale } = useI18n()

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
  modbus_rx_timeout_us: 0,
  modbus_scan_delay_us: 0,
  modbus_inter_frame_delay_us: 0,
  ble_enabled: true,
  operating_mode: 'servo',
}

const defaultNetConfig: NetworkConfiguration = {
  wifi_enabled: true,
  ssid: '',
  password: '',
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
const BLE_STATE_INTERVAL_MS = 100

const isWifiModeAvailable = computed(() => {
  return !isHttps || window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1'
})

const isBleModeAvailable = computed(() => {
  return typeof navigator !== 'undefined' && 'bluetooth' in navigator && (window.isSecureContext || window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1')
})

const motorState = ref<MotorState | null>(null)

function onMotorStateUpdate(state: MotorState) {
  motorState.value = state
}

async function syncLiveStateSubscription() {
  if (!isInitialized.value) return
  if (showDiagram.value || showSettings.value) {
    try {
      const intervalMs = mode.value === 'bluetooth' ? BLE_STATE_INTERVAL_MS : 33
      await api.subscribeState(onMotorStateUpdate, intervalMs)
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
  if (mode.value === newMode) {
    return
  }

  try {
    await api.unsubscribeState(onMotorStateUpdate)
  } catch (e) {
    console.warn('Failed to clean previous state subscription during mode switch:', e)
  }
  api.disconnectBle()

  mode.value = newMode
  api.setConnectionMode(newMode)
  connected.value = false
  motorReady.value = false
  isInitialized.value = false
  motorState.value = null
  error.value = null
  
  if (newMode === 'wifi') {
    await fetchConfig()
  }
}

async function connectToBle() {
  bleConnecting.value = true
  error.value = null
  try {
    const ok = await api.connectBle(() => {
      void api.unsubscribeState(onMotorStateUpdate)
      isInitialized.value = false
      connected.value = false
      motorReady.value = false
      motorState.value = null
      error.value = t('errors.bluetoothDisconnected')
    })
    if (ok) {
      await fetchConfig()
    }
  } catch (e: any) {
    console.error(e)
    error.value = e.message || t('errors.bleConnectFailed')
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
      await stateMachine.executeControl(() => api.setConfig(config.value))
    }
    catch (e) {
      console.error(e)
      error.value = t('errors.setConfigFailed')
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
      const state = await stateMachine.executeControl(() => api.getState())
      const pausePos =
        typeof state.y === 'number'
          ? state.y
          : typeof state.shaped_y === 'number'
            ? state.shaped_y
            : config.value.paused_position
      updatedConfig = await stateMachine.executeControl(() =>
        api.setPaused({ paused: true, position: pausePos }),
      )
    }
    else {
      updatedConfig = await stateMachine.executeControl(() => api.setPaused({ paused }))
    }
    config.value = updatedConfig
  }
  catch (e) {
    console.error(e)
    error.value = t('errors.setPausedFailed')
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
      error.value = t('errors.setPausedPositionFailed')
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
    isInitialized.value = false
    motorState.value = null
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
    isInitialized.value = false
    motorState.value = null
    error.value = t('errors.deviceConnectFailed')
    return
  }

  try {
    config.value = await api.getConfig()
    try {
      motorState.value = await api.getState()
    } catch (e) {
      console.warn('Failed to fetch initial state:', e)
    }
    motorReady.value = true
    isInitialized.value = true
    await syncLiveStateSubscription()
  }
  catch (e) {
    console.error(e)
    motorReady.value = false
    isInitialized.value = false
    error.value = t('errors.motorNotInitialized')
  }
}

const motorUpdateRate = computed<number | null>(() => {
  if (typeof motorState.value?.ups === 'number') {
    return motorState.value.ups
  }
  if (!motorState.value?.update_history || motorState.value.update_history.length === 0) {
    return null
  }
  return motorState.value.update_history[motorState.value.update_history.length - 1] ?? null
})

const isMotorConnected = computed<boolean>(() => {
  if (typeof motorState.value?.motor_connected === 'boolean') {
    return motorState.value.motor_connected
  }
  return motorUpdateRate.value !== null && motorUpdateRate.value > 0
})

const isRelayMode = computed(
  () => (pinConfig.value.operating_mode ?? 'servo') === 'rtu_relay',
)

async function saveAllSettings() {
  pinConfigSaving.value = true
  pinConfigSaved.value = false
  netConfigSaved.value = false
  error.value = null
  try {
    pinConfig.value = await api.setPinConfig(pinConfig.value)
    netConfig.value = await api.setNetworkConfig(netConfig.value)
    pinConfigSaved.value = true
    netConfigSaved.value = true
  }
  catch (e) {
    console.error(e)
    error.value = t('errors.saveSettingsFailed')
  }
  finally {
    pinConfigSaving.value = false
  }
}

async function resetAllSettings() {
  pinConfig.value = { ...defaultPinConfig }
  netConfig.value = { ...defaultNetConfig }
  await saveAllSettings()
}

async function restartDevice() {
  try {
    error.value = null
    await api.restartDevice()
    connected.value = false
    motorReady.value = false
    error.value = t('errors.restartSent')
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
    error.value = t('errors.restartFailed')
  }
}

function switchLocale(next: AppLocale) {
  setLocale(next)
}

stateMachine.onReconnect(async () => {
  await fetchConfig()
})

watch(connected, (isConn, wasConn) => {
  if (isConn && !wasConn) {
    stateMachine.setConnectionStatus('CONNECTED')
  } else if (!isConn) {
    stateMachine.setConnectionStatus('DISCONNECTED')
  }
})

onMounted(() => {
  if (mode.value === 'wifi' && !isWifiModeAvailable.value && isBleModeAvailable.value) {
    switchMode('bluetooth')
  } else if (mode.value === 'bluetooth' && !isBleModeAvailable.value && isWifiModeAvailable.value) {
    switchMode('wifi')
  } else if (mode.value === 'wifi') {
    fetchConfig()
  }
})

onBeforeUnmount(() => {
  void api.unsubscribeState(onMotorStateUpdate)
  api.disconnectBle()
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
          {{ t('app.title') }}
        </h1>
        <div class="flex items-center space-x-3">
          <!-- Locale switcher -->
          <div class="flex rounded bg-gray-200 p-0.5 text-sm font-medium">
            <button
              class="rounded px-2 py-1 transition"
              :class="locale === 'en' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchLocale('en')"
            >
              {{ t('locale.en') }}
            </button>
            <button
              class="rounded px-2 py-1 transition"
              :class="locale === 'zh' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchLocale('zh')"
            >
              {{ t('locale.zh') }}
            </button>
          </div>

          <!-- Mode switcher -->
          <div
            v-if="isWifiModeAvailable && isBleModeAvailable"
            class="flex rounded bg-gray-200 p-0.5 text-sm font-medium"
          >
            <button
              v-if="isWifiModeAvailable"
              class="flex items-center space-x-1 rounded px-2 py-1 transition"
              :class="mode === 'wifi' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchMode('wifi')"
            >
              <Wifi :size="14" />
              <span>{{ t('app.wifi') }}</span>
            </button>
            <button
              v-if="isBleModeAvailable"
              class="flex items-center space-x-1 rounded px-2 py-1 transition"
              :class="mode === 'bluetooth' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchMode('bluetooth')"
            >
              <Bluetooth :size="14" />
              <span>{{ t('app.ble') }}</span>
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
            <span>{{ bleConnecting ? t('app.connecting') : t('app.connectBle') }}</span>
          </button>
          <button
            class="rounded p-1.5 disabled:opacity-50"
            :class="showDiagram ? 'bg-blue-500 text-white hover:bg-blue-600' : 'bg-gray-200 hover:bg-gray-300'"
            :title="showDiagram ? t('app.hideDiagram') : t('app.showDiagram')"
            :aria-label="t('app.toggleDiagram')"
            @click="showDiagram = !showDiagram"
          >
            <Activity :size="18" :stroke-width="2" />
          </button>
          <button
            class="rounded p-1.5 disabled:opacity-50"
            :class="showSettings ? 'bg-blue-500 text-white hover:bg-blue-600' : 'bg-gray-200 hover:bg-gray-300'"
            :title="showSettings ? t('app.hideSettings') : t('app.showSettings')"
            :aria-label="t('app.toggleSettings')"
            @click="showSettings = !showSettings"
          >
            <Settings :size="18" :stroke-width="2" />
          </button>
          <span
            class="h-3 w-3 rounded-full"
            :class="{ 'bg-green-500': connected, 'bg-red-500': !connected }"
          />
          <span class="text-sm font-medium">{{ connected ? (motorReady ? t('app.connected') : t('app.modbusOnly')) : t('app.disconnected') }}</span>
        </div>
      </header>

      <div v-if="mode === 'wifi' && isHttps" class="mb-4 rounded bg-amber-500 p-2.5 text-sm text-white shadow">
        <strong>{{ t('app.httpsWifiNoteStrong') }}</strong> {{ t('app.httpsWifiNote') }}
      </div>

      <div v-if="error" class="mb-4 bg-red-500 p-2 text-white">
        {{ error }}
        <button class="ml-4 font-bold" @click="fetchConfig">
          {{ t('app.retry') }}
        </button>
      </div>

      <!-- Alert Banner: RTU relay mode -->
      <div
        v-if="connected && isRelayMode"
        id="alert-rtu-relay"
        class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-amber-800 shadow-sm"
      >
        <div class="flex items-center space-x-3">
          <span class="flex h-3 w-3 rounded-full bg-amber-500 animate-ping"></span>
          <div>
            <p class="font-bold">{{ t('app.rtuRelayTitle') }}</p>
            <p class="text-xs text-amber-700">
              {{ t('app.rtuRelayBody') }}
            </p>
          </div>
        </div>
        <button
          @click="showSettings = true"
          class="rounded bg-amber-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-700 shadow-sm"
        >
          {{ t('app.openSettings') }}
        </button>
      </div>

      <!-- Alert Banner: Motor disconnected or low update rate (<50 Hz) -->
      <div
        v-if="connected && !isRelayMode && (!isMotorConnected || (motorUpdateRate !== null && motorUpdateRate < 50))"
        id="alert-motor-status"
        class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-amber-800 shadow-sm"
      >
        <div class="flex items-center space-x-3">
          <span class="flex h-3 w-3 rounded-full bg-amber-500 animate-ping"></span>
          <div>
            <p class="font-bold">
              {{ !isMotorConnected ? t('app.motorNotConnectedTitle') : t('app.lowUpdateRateTitle') }}
            </p>
            <p class="text-xs text-amber-700">
              {{ !isMotorConnected
                ? t('app.motorNotConnectedBody')
                : t('app.lowUpdateRateBody', { rate: motorUpdateRate })
              }}
            </p>
          </div>
        </div>
        <button
          @click="showSettings = true"
          class="rounded bg-amber-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-700 shadow-sm"
        >
          {{ t('app.checkSettings') }}
        </button>
      </div>

      <div class="space-y-4">
        <MotorPositionDiagram
          v-if="showDiagram && !isRelayMode"
          :config="config"
          :state="motorState"
          :connected="motorReady"
        />
        <MainControl
          v-if="!isRelayMode"
          v-model="config"
          :pause-mode="pauseMode"
          :connected="motorReady"
          :is-mutating="stateMachine.isMutating.value"
          @update:model-value="setConfig"
          @set-paused="setPaused" @set-paused-position="setPausedPosition"
          @set-pause-mode="pauseMode = $event"
        />
        <SplineEditor
          v-if="!isRelayMode && config.wave_func === 'spline'"
          v-model="config" :connected="motorReady"
          @update:model-value="setConfig"
        />
        <FunscriptPlayer
          v-if="!isRelayMode && config.wave_func === 'funscript'"
        />
        <UnifiedSettings
          v-if="showSettings"
          v-model:pin-config="pinConfig"
          v-model:net-config="netConfig"
          :state="motorState"
          :connected="connected"
          :saving="pinConfigSaving || netConfigSaving"
          :saved="pinConfigSaved && netConfigSaved"
          :mode="mode === 'bluetooth' ? 'bluetooth' : 'wifi'"
          @save="saveAllSettings"
          @reset="resetAllSettings"
          @restart="restartDevice"
        />
      </div>
    </div>
  </div>
</template>

<style scoped></style>
