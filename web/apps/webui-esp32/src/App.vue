<script setup lang="ts">
import { ref, computed, onMounted, provide, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Settings, Bluetooth, Wifi } from '@lucide/vue'
import {
  ControllerPageShell,
  OSSM_CONTROL_KEY,
  useControllerSession,
  wireToDomainConfig,
  type ShellConfig,
} from '@ossm/shared'
import UnifiedSettings from './components/UnifiedSettings.vue'
import * as api from './api'
import { deviceControl } from '@ossm/client'
import { setLocale } from './i18n'

provide(OSSM_CONTROL_KEY, deviceControl)

const { t } = useI18n()

const defaultShellConfig: ShellConfig = {
  modbus_tx: 18,
  modbus_rx: 19,
  modbus_de_re: 20,
  modbus_timeout_ms: 0,
  modbus_rx_timeout_us: 0,
  modbus_scan_delay_us: 0,
  modbus_inter_frame_delay_us: 0,
  ble_enabled: true,
  modbus_debug: false,
  operating_mode: 'servo',
  modbus_baud: 115200,
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

const shellConfig = ref<ShellConfig>({ ...defaultShellConfig })
const showSettings = ref(false)
const shellConfigSaving = ref(false)
const shellConfigSaved = ref(false)
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

const liveIntervalMs = computed(() => (mode.value === 'bluetooth' ? BLE_STATE_INTERVAL_MS : 33))

function isBenignFetchFailure(e: unknown): boolean {
  if (e instanceof DOMException && e.name === 'AbortError') return true
  if (e instanceof Error && e.name === 'AbortError') return true
  if (e instanceof TypeError && /Failed to fetch/i.test(e.message)) return true
  return false
}

const session = useControllerSession({
  control: () => deviceControl,
  t,
  autoInitializeOnMount: false,
  liveIntervalMs,
  isLiveViewOpen: () => showSettings.value,
  initialize: loadDeviceConfig,
})

const {
  config,
  connected,
  motorReady,
  showDiagram,
  pauseMode,
  error,
  motorState,
  stateMachine,
  setConfig,
  setPaused,
  setPausedPosition,
  onMacroConfigUpdate,
  fetchConfig,
  initialize,
  restartDevice,
  syncLiveStateSubscription,
} = session

const motorUpdateRate = computed<number | null>(() => {
  const loop = motorState.value?.loop_stats
  if (typeof loop?.ups === 'number') {
    return loop.ups
  }
  if (!loop?.update_history || loop.update_history.length === 0) {
    return null
  }
  return loop.update_history[loop.update_history.length - 1] ?? null
})

const isMotorConnected = computed<boolean>(() => {
  if (typeof motorState.value?.motor_connected === 'boolean') {
    return motorState.value.motor_connected
  }
  return motorUpdateRate.value !== null && motorUpdateRate.value > 0
})

const isRelayMode = computed(
  () => (shellConfig.value.operating_mode ?? 'servo') !== 'servo',
)

async function switchMode(newMode: api.ConnectionMode) {
  if (mode.value === newMode) return

  try {
    await api.unsubscribeState(session.onMotorStateUpdate)
  } catch (e) {
    console.warn('Failed to clean previous state subscription during mode switch:', e)
  }
  api.disconnectBle()

  mode.value = newMode
  api.setConnectionMode(newMode)
  connected.value = false
  motorReady.value = false
  motorState.value = null
  error.value = null

  if (newMode === 'wifi') {
    await initialize()
  }
}

async function connectToBle() {
  bleConnecting.value = true
  error.value = null
  try {
    const ok = await api.connectBle(() => {
      void api.unsubscribeState(session.onMotorStateUpdate)
      connected.value = false
      motorReady.value = false
      motorState.value = null
      error.value = t('errors.bluetoothDisconnected')
    })
    if (ok) {
      await initialize()
    }
  } catch (e: unknown) {
    console.error(e)
    error.value = e instanceof Error ? e.message : t('errors.bleConnectFailed')
    connected.value = false
    motorReady.value = false
  } finally {
    bleConnecting.value = false
  }
}

async function loadDeviceConfig(): Promise<boolean> {
  error.value = null
  shellConfigSaved.value = false

  if (mode.value === 'bluetooth' && !api.isBleConnected()) {
    connected.value = false
    motorReady.value = false
    motorState.value = null
    return false
  }

  try {
    try {
      shellConfig.value = await api.getShellConfig()
    } catch (firstErr) {
      if (mode.value === 'bluetooth') {
        await new Promise((r) => setTimeout(r, 350))
        shellConfig.value = await api.getShellConfig()
      } else {
        throw firstErr
      }
    }
    connected.value = true
  } catch (e) {
    if (!isBenignFetchFailure(e)) {
      console.error(e)
    }
    connected.value = false
    motorReady.value = false
    motorState.value = null
    error.value = t('errors.deviceConnectFailed')
    return false
  }

  try {
    let deviceConfig = await api.getConfig()
    if (mode.value === 'bluetooth') {
      try {
        deviceConfig = await api.getConfig()
      } catch {
        await new Promise((r) => setTimeout(r, 350))
        deviceConfig = await api.getConfig()
      }
    }
    if (deviceConfig) {
      stateMachine.trackAuthoritativeVersion(deviceConfig.version)
    }
    if (stateMachine.shouldAcceptRemoteConfig(deviceConfig)) {
      config.value = wireToDomainConfig(deviceConfig, config.value)
    }
    try {
      motorState.value = await api.getState()
    } catch (e) {
      if (!isBenignFetchFailure(e)) {
        console.warn('Failed to fetch initial state:', e)
      }
    }
    motorReady.value = true
    await syncLiveStateSubscription()
    return true
  } catch (e) {
    if (!isBenignFetchFailure(e)) {
      console.error(e)
    }
    motorReady.value = false
    error.value = t('errors.motorNotInitialized')
    return false
  }
}

async function waitShellOperatingMode(want: string, timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const shell = await api.getShellConfig()
    if ((shell.operating_mode ?? 'servo') === want) {
      shellConfig.value = shell
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`operating_mode did not become ${want}`)
}

async function applyOperatingMode(modeVal?: ShellConfig['operating_mode']) {
  shellConfigSaving.value = true
  shellConfigSaved.value = false
  error.value = null
  try {
    const want = modeVal ?? shellConfig.value.operating_mode ?? 'servo'
    shellConfig.value = { ...shellConfig.value, operating_mode: want }
    shellConfig.value = await api.setShellConfig(shellConfig.value)
    await waitShellOperatingMode(want)
    shellConfigSaved.value = true
  } catch (e) {
    console.error(e)
    error.value = t('errors.saveSettingsFailed')
  } finally {
    shellConfigSaving.value = false
  }
}

async function saveAllSettings() {
  shellConfigSaving.value = true
  shellConfigSaved.value = false
  error.value = null
  try {
    const want = shellConfig.value.operating_mode ?? 'servo'
    shellConfig.value = await api.setShellConfig(shellConfig.value)
    await waitShellOperatingMode(want)
    shellConfigSaved.value = true
  } catch (e) {
    console.error(e)
    error.value = t('errors.saveSettingsFailed')
  } finally {
    shellConfigSaving.value = false
  }
}

async function resetAllSettings() {
  shellConfig.value = { ...defaultShellConfig }
  await saveAllSettings()
}

watch(showSettings, () => {
  void syncLiveStateSubscription()
})

onMounted(() => {
  if (mode.value === 'wifi' && !isWifiModeAvailable.value && isBleModeAvailable.value) {
    void switchMode('bluetooth')
  } else if (mode.value === 'bluetooth' && !isBleModeAvailable.value && isWifiModeAvailable.value) {
    void switchMode('wifi')
  } else if (mode.value === 'wifi') {
    void initialize()
  }
})
</script>

<template>
  <ControllerPageShell
    v-model:config="config"
    v-model:pause-mode="pauseMode"
    v-model:show-diagram="showDiagram"
    :title="t('app.title')"
    :state="motorState"
    :connected="connected"
    :motor-ready="motorReady"
    :motor-ready-label="motorReady ? t('app.connected') : t('app.modbusOnly')"
    :error="error"
    :is-mutating="stateMachine.isMutating.value"
    :hide-motion="isRelayMode"
    :capabilities="{ locale: true, diagram: true, details: false, restart: false }"
    @retry="initialize"
    @locale="setLocale"
    @posted-config="setConfig"
    @macro-config="onMacroConfigUpdate"
    @set-paused="setPaused"
    @set-paused-position="setPausedPosition"
  >
    <template #header-actions>
      <div
        v-if="isWifiModeAvailable && isBleModeAvailable"
        class="flex rounded-lg bg-slate-200/80 p-0.5 text-xs font-semibold"
      >
        <button
          v-if="isWifiModeAvailable"
          type="button"
          class="flex items-center space-x-1 rounded-md px-2.5 py-1 transition-all cursor-pointer"
          :class="mode === 'wifi' ? 'bg-white shadow-xs text-blue-600 font-bold' : 'text-slate-600 hover:text-slate-900'"
          @click="switchMode('wifi')"
        >
          <Wifi :size="14" />
          <span>{{ t('app.wifi') }}</span>
        </button>
        <button
          v-if="isBleModeAvailable"
          type="button"
          class="flex items-center space-x-1 rounded-md px-2.5 py-1 transition-all cursor-pointer"
          :class="mode === 'bluetooth' ? 'bg-white shadow-xs text-blue-600 font-bold' : 'text-slate-600 hover:text-slate-900'"
          @click="switchMode('bluetooth')"
        >
          <Bluetooth :size="14" />
          <span>{{ t('app.ble') }}</span>
        </button>
      </div>
      <button
        v-if="mode === 'bluetooth' && !connected"
        type="button"
        class="flex items-center space-x-1 rounded-xl bg-blue-600 px-3 py-1.5 text-xs font-bold text-white shadow-xs hover:bg-blue-700 active:scale-95 transition-all disabled:opacity-50 cursor-pointer"
        :disabled="bleConnecting"
        @click="connectToBle"
      >
        <Bluetooth :size="15" />
        <span>{{ bleConnecting ? t('app.connecting') : t('app.connectBle') }}</span>
      </button>
      <button
        type="button"
        class="rounded-xl p-2 transition-all cursor-pointer disabled:opacity-50"
        :class="showSettings ? 'bg-blue-600 text-white shadow-xs' : 'bg-slate-100 text-slate-600 hover:bg-slate-200 border border-slate-200/80'"
        :title="showSettings ? t('app.hideSettings') : t('app.showSettings')"
        :aria-label="t('app.toggleSettings')"
        @click="showSettings = !showSettings"
      >
        <Settings :size="17" :stroke-width="2.2" />
      </button>
    </template>

    <template #alerts>
      <div v-if="mode === 'wifi' && isHttps" class="mb-4 rounded bg-amber-500 p-2.5 text-sm text-white shadow">
        <strong>{{ t('app.httpsWifiNoteStrong') }}</strong> {{ t('app.httpsWifiNote') }}
      </div>
      <div
        v-if="connected && isRelayMode"
        id="alert-rtu-relay"
        class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-amber-800 shadow-sm"
      >
        <div class="flex items-center space-x-3">
          <span class="flex h-3 w-3 rounded-full bg-amber-500 animate-ping" />
          <div>
            <p class="font-bold">{{ t('app.rtuRelayTitle') }}</p>
            <p class="text-xs text-amber-700">{{ t('app.rtuRelayBody') }}</p>
          </div>
        </div>
        <button type="button" class="rounded bg-amber-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-700 shadow-sm" @click="showSettings = true">
          {{ t('app.openSettings') }}
        </button>
      </div>
      <div
        v-if="connected && !isRelayMode && (!isMotorConnected || (motorUpdateRate !== null && motorUpdateRate < 50))"
        id="alert-motor-status"
        class="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-amber-800 shadow-sm"
      >
        <div class="flex items-center space-x-3">
          <span class="flex h-3 w-3 rounded-full bg-amber-500 animate-ping" />
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
        <button type="button" class="rounded bg-amber-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-700 shadow-sm" @click="showSettings = true">
          {{ t('app.checkSettings') }}
        </button>
      </div>
    </template>

    <template #settings>
      <UnifiedSettings
        v-if="showSettings"
        v-model:shell-config="shellConfig"
        :state="motorState"
        :connected="connected"
        :saving="shellConfigSaving"
        :saved="shellConfigSaved"
        :mode="mode === 'bluetooth' ? 'bluetooth' : 'wifi'"
        @save="saveAllSettings"
        @reset="resetAllSettings"
        @restart="restartDevice"
        @apply-operating-mode="applyOperatingMode"
      />
    </template>
  </ControllerPageShell>
</template>
