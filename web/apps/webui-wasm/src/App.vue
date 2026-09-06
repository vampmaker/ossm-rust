<script setup lang="ts">
import { ref, computed, provide, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  ControllerPageShell,
  LinkStatsPanel,
  MotorTelemetryPanel,
  DelegatingOssmControl,
  OSSM_CONTROL_KEY,
  useControllerSession,
} from '@ossm/shared'
import type { LinkStats } from '@ossm/shared'
import { getLinkStats } from '@ossm/client'
import { setLocale } from './i18n'
import { WasmClient } from './wasm-client'
import { RemoteApiControl } from './remote-api-control'

declare global {
  interface Window {
    __ossmGetState: () => Promise<unknown>
    __ossmRpc: (method: string, params?: unknown) => Promise<unknown>
    __ossmPaused: (payload: unknown) => Promise<unknown>
    __ossmStateCount: number
    __ossmControlMethods: string[]
  }
}

const { t } = useI18n()

const allowMock = Boolean(import.meta.env.DEV || import.meta.env.VITE_OSSM_ALLOW_MOCK)

type ConnectionKind = 'serial' | 'rs485' | 'modbus-ws' | 'remote-api' | 'mock'

const localClient = new WasmClient()
const remoteClient = new RemoteApiControl(
  typeof window !== 'undefined' && window.location.hostname
    ? `http://${window.location.hostname}:8080`
    : 'http://127.0.0.1:8080',
)
const controlTarget = ref(localClient)
const delegating = new DelegatingOssmControl(controlTarget)
provide(OSSM_CONTROL_KEY, delegating)

const connectionType = ref<ConnectionKind>(allowMock ? 'mock' : 'serial')
const baud = ref(115200)
const wsUrl = ref(
  typeof window !== 'undefined' && window.location.hostname
    ? `ws://${window.location.hostname}/ws/rs485`
    : 'ws://ossm.lan/ws/rs485',
)
const modbusWsUrl = ref(
  typeof window !== 'undefined' && window.location.hostname
    ? `ws://${window.location.hostname}/ws/modbus`
    : 'ws://ossm.lan/ws/modbus',
)
const apiUrl = ref(
  typeof window !== 'undefined' && window.location.hostname
    ? `http://${window.location.hostname}:8080`
    : 'http://127.0.0.1:8080',
)
const connecting = ref(false)
const homing = ref(false)
const linkStats = ref<LinkStats | null>(null)
let linkTimer: ReturnType<typeof setInterval> | null = null

async function refreshLinkStats() {
  if (!connected.value) {
    linkStats.value = null
    return
  }
  if (connectionType.value === 'remote-api') {
    try {
      linkStats.value = await getLinkStats()
    } catch {
      /* ignore */
    }
    return
  }
  try {
    linkStats.value = localClient.getLinkStats()
  } catch {
    linkStats.value = null
  }
}

onMounted(() => {
  linkTimer = setInterval(() => void refreshLinkStats(), 1000)
})

onUnmounted(() => {
  if (linkTimer) clearInterval(linkTimer)
})

const session = useControllerSession({
  control: () => delegating,
  t,
  autoInitializeOnMount: false,
})

const {
  config,
  connected,
  motorReady,
  showDiagram,
  showDetails,
  pauseMode,
  error,
  motorState,
  stateMachine,
  setConfig,
  setPaused,
  setPausedPosition,
  onMacroConfigUpdate,
  fetchConfig,
  onMotorStateUpdate,
} = session

function bumpStateCount() {
  window.__ossmStateCount = (window.__ossmStateCount ?? 0) + 1
}

function wireTestHooks() {
  window.__ossmStateCount = 0
  window.__ossmGetState = () => delegating.getState()
  window.__ossmRpc = (method, params) =>
    localClient.request('rpc', { jsonrpc: '2.0', method, params }) as Promise<unknown>
  window.__ossmPaused = (payload) => delegating.setPaused(payload as never)
  window.__ossmControlMethods = [
    'connect',
    'disconnect',
    'mock',
    'set-config',
    'get-state',
    'paused',
    'subscribe-state',
    'unsubscribe-state',
    'set-waypoints',
    'append-waypoints',
    'reset-timestamp',
    'restart',
    'remote-api',
    'modbus-ws',
  ]
}

wireTestHooks()

localClient.onEvent((ev) => {
  if (connectionType.value === 'remote-api') return
  if (ev.type === 'homing') {
    homing.value = true
    motorReady.value = false
  }
  if (ev.type === 'ready') {
    homing.value = false
    motorReady.value = true
    connected.value = true
    error.value = null
    void afterLocalReady()
  }
  if (ev.type === 'error') {
    homing.value = false
    motorReady.value = false
    error.value = String((ev.payload as { message?: string })?.message ?? 'error')
  }
  if (ev.type === 'disconnected') {
    connected.value = false
    motorReady.value = false
    homing.value = false
  }
  if (ev.type === 'state') {
    bumpStateCount()
    onMotorStateUpdate(ev.payload as never)
  }
})

async function afterLocalReady() {
  window.__ossmStateCount = 0
  await delegating.subscribeState(() => {
    /* events already handled via client 'state' messages */
  }, 33)
  await fetchConfig()
  stateMachine.setConnectionStatus('CONNECTED')
}

async function onConnect() {
  error.value = null
  connecting.value = true
  homing.value = connectionType.value !== 'mock' && connectionType.value !== 'remote-api'
  motorReady.value = false
  try {
    if (connectionType.value === 'remote-api') {
      controlTarget.value = remoteClient
      remoteClient.setBaseUrl(apiUrl.value)
      await fetchConfig()
      connected.value = true
      motorReady.value = true
      homing.value = false
      stateMachine.setConnectionStatus('CONNECTED')
      return
    }
    controlTarget.value = localClient
    if (connectionType.value === 'mock') {
      await localClient.connectMock()
      return
    }
    if (connectionType.value === 'modbus-ws') {
      await localClient.connectModbusWs(modbusWsUrl.value)
      return
    }
    if (connectionType.value === 'rs485') {
      await localClient.connectRs485(wsUrl.value, baud.value)
      return
    }
    const port = await navigator.serial.requestPort()
    await localClient.connectSerial(port, baud.value)
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
    homing.value = false
    connected.value = false
    motorReady.value = false
  } finally {
    connecting.value = false
  }
}

async function onDisconnect() {
  if (connectionType.value === 'remote-api') {
    remoteClient.close()
  } else {
    await localClient.disconnect()
  }
  connected.value = false
  motorReady.value = false
  stateMachine.setConnectionStatus('DISCONNECTED')
}

const needsBaud = computed(
  () => connectionType.value === 'serial' || connectionType.value === 'rs485',
)
</script>

<template>
  <ControllerPageShell
    v-model:config="config"
    v-model:pause-mode="pauseMode"
    v-model:show-diagram="showDiagram"
    v-model:show-details="showDetails"
    :title="t('app.title')"
    badge="WebAssembly"
    :state="motorState"
    :connected="connected"
    :motor-ready="motorReady"
    :error="error"
    :is-mutating="stateMachine.isMutating.value"
    :capabilities="{ locale: true, diagram: true, details: true, restart: false }"
    @locale="setLocale"
    @posted-config="setConfig"
    @macro-config="onMacroConfigUpdate"
    @set-paused="setPaused"
    @set-paused-position="setPausedPosition"
  >
    <template #connection>
      <section id="wasm-connection" class="mb-4 bg-white rounded-2xl border border-slate-200/80 shadow-sm p-5 space-y-3">
        <div class="flex flex-wrap gap-3 items-end">
          <label class="text-xs font-bold text-slate-700">
            <span class="block mb-1">{{ t('connection.mode') }}</span>
            <select
              id="wasm-connection-type"
              v-model="connectionType"
              class="bg-slate-50 border border-slate-300 text-slate-900 font-medium rounded-xl px-3 py-1.5 focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all cursor-pointer"
            >
              <option value="serial">{{ t('connection.serial') }}</option>
              <option value="rs485">{{ t('connection.rs485') }}</option>
              <option value="modbus-ws">{{ t('connection.modbusWs') }}</option>
              <option value="remote-api">{{ t('connection.remoteApi') }}</option>
              <option v-if="allowMock" value="mock">{{ t('connection.mock') }}</option>
            </select>
          </label>
          <label v-if="needsBaud" class="text-xs font-bold text-slate-700">
            <span class="block mb-1">{{ t('connection.baud') }}</span>
            <input
              id="wasm-baud"
              v-model.number="baud"
              type="number"
              class="bg-slate-50 border border-slate-300 text-slate-900 font-medium rounded-xl px-3 py-1.5 w-28 focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all"
            >
          </label>
          <label v-if="connectionType === 'rs485'" class="text-xs font-bold text-slate-700 flex-1 min-w-48">
            <span class="block mb-1">{{ t('connection.wsUrl') }}</span>
            <input
              id="wasm-ws-url"
              v-model="wsUrl"
              class="bg-slate-50 border border-slate-300 text-slate-900 font-medium rounded-xl px-3 py-1.5 w-full focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all"
            >
          </label>
          <label v-if="connectionType === 'modbus-ws'" class="text-xs font-bold text-slate-700 flex-1 min-w-48">
            <span class="block mb-1">{{ t('connection.modbusWsUrl') }}</span>
            <input
              id="wasm-modbus-ws-url"
              v-model="modbusWsUrl"
              class="bg-slate-50 border border-slate-300 text-slate-900 font-medium rounded-xl px-3 py-1.5 w-full focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all"
            >
          </label>
          <label v-if="connectionType === 'remote-api'" class="text-xs font-bold text-slate-700 flex-1 min-w-48">
            <span class="block mb-1">{{ t('connection.apiUrl') }}</span>
            <input
              id="wasm-api-url"
              v-model="apiUrl"
              class="bg-slate-50 border border-slate-300 text-slate-900 font-medium rounded-xl px-3 py-1.5 w-full focus:ring-2 focus:ring-blue-500 focus:border-blue-500 transition-all"
            >
          </label>
          <button
            id="wasm-connect"
            type="button"
            class="inline-flex items-center justify-center font-bold text-sm px-4 py-2 rounded-xl bg-blue-600 hover:bg-blue-700 text-white shadow-xs hover:shadow active:scale-95 transition-all disabled:opacity-50 cursor-pointer"
            :disabled="connecting || connected"
            @click="onConnect"
          >
            {{ t('connection.connect') }}
          </button>
          <button
            id="wasm-disconnect"
            type="button"
            class="inline-flex items-center justify-center font-bold text-sm px-4 py-2 rounded-xl bg-slate-100 hover:bg-slate-200 text-slate-700 border border-slate-300 shadow-2xs active:scale-95 transition-all disabled:opacity-50 cursor-pointer"
            :disabled="!connected"
            @click="onDisconnect"
          >
            {{ t('connection.disconnect') }}
          </button>
        </div>
        <p v-if="error" class="text-rose-600 text-sm font-semibold bg-rose-50 px-3 py-1.5 rounded-lg border border-rose-200">{{ error }}</p>
        <p v-show="homing" id="wasm-homing" class="text-amber-700 text-sm font-semibold flex items-center gap-1.5">
          <span class="h-2 w-2 rounded-full bg-amber-500 animate-ping" />{{ t('connection.homing') }}
        </p>
        <p v-show="motorReady" id="wasm-motor-ready" class="text-emerald-700 text-sm font-semibold flex items-center gap-1.5">
          <span class="h-2 w-2 rounded-full bg-emerald-500" />{{ t('connection.motorReady') }}
        </p>
      </section>
    </template>

    <template #details>
      <section id="wasm-telemetry" class="bg-white p-6 shadow-lg rounded-lg border border-gray-200 space-y-3">
        <h2 class="text-xl font-bold text-gray-800">{{ t('telemetry.title') }}</h2>
        <MotorTelemetryPanel :state="motorState" />
        <LinkStatsPanel :stats="linkStats" />
      </section>
    </template>
  </ControllerPageShell>
</template>
