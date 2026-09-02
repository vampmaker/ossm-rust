<script setup lang="ts">
import { ref, onUnmounted, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { setLocale, type AppLocale } from './i18n'
import { SerialModbusClient } from './lib/serial-port'
import { WebSocketModbusClient } from './lib/websocket-modbus'
import { WebSocketRs485Client } from './lib/websocket-rs485'
import type { ModbusTransport } from './lib/transport'
import { buildReadHoldingRegisters } from './lib/modbus-rtu'
import { parseRegistersToState, type DriveState } from './lib/registers'

import WaveformChart from './components/WaveformChart.vue'
import ParamListPanel from './components/ParamListPanel.vue'
import TelemetryPanel from './components/TelemetryPanel.vue'
import DriverSettingsPanel from './components/DriverSettingsPanel.vue'
import ModbusReadPanel from './components/ModbusReadPanel.vue'
import StatusConnectPanel from './components/StatusConnectPanel.vue'
import ModbusSendPanel from './components/ModbusSendPanel.vue'
import SendEchoStrip from './components/SendEchoStrip.vue'
import CommLog from './components/CommLog.vue'

const { t, locale } = useI18n()

const serialClient = new SerialModbusClient()
const wsClient = new WebSocketModbusClient()
const rs485Client = new WebSocketRs485Client()

const connectionType = ref<'serial' | 'websocket' | 'rs485'>('serial')
const client = ref<ModbusTransport>(serialClient)

const isConnected = ref(false)
const baudRate = ref(115200)
const wsUrl = ref('ws://ossm.lan/ws/modbus')
const deviceAddress = ref(1)
const pollIntervalMs = ref(50)
const readEnabled = ref(true)
const addressScanEnabled = ref(false)
const isPolling = ref(false)
const scanTickCounter = ref(0)

const driveState = ref<DriveState | null>(null)
const errorMsg = ref('')

let pollTimer: number | null = null

const webSerialSupported = computed(() => 'serial' in navigator)

watch(connectionType, (next) => {
  if (isConnected.value) return
  if (next === 'websocket') {
    client.value = wsClient
    if (wsUrl.value.includes('/ws/rs485')) {
      wsUrl.value = 'ws://ossm.lan/ws/modbus'
    }
  } else if (next === 'rs485') {
    client.value = rs485Client
    if (wsUrl.value.includes('/ws/modbus')) {
      wsUrl.value = 'ws://ossm.lan/ws/rs485'
    }
  } else {
    client.value = serialClient
  }
})

watch(baudRate, (baud) => {
  if (isConnected.value && connectionType.value === 'rs485') {
    rs485Client.setBaudRate(baud)
  }
})

function switchLocale(next: AppLocale) {
  setLocale(next)
}

function clampDeviceAddress() {
  const safe = Math.trunc(Number(deviceAddress.value))
  if (!Number.isFinite(safe)) {
    deviceAddress.value = 1
    return
  }
  deviceAddress.value = Math.min(255, Math.max(1, safe))
}

function advanceScanAddressIfNeeded() {
  if (!addressScanEnabled.value) return
  scanTickCounter.value += 1
  if (scanTickCounter.value >= 3) {
    if (deviceAddress.value < 255) {
      deviceAddress.value += 1
    }
    scanTickCounter.value = 0
  }
}

async function toggleConnection() {
  if (isConnected.value) {
    stopPolling()
    await client.value.disconnect()
    isConnected.value = false
    driveState.value = null
  } else {
    try {
      errorMsg.value = ''
      clampDeviceAddress()
      scanTickCounter.value = 0
      client.value =
        connectionType.value === 'websocket'
          ? wsClient
          : connectionType.value === 'rs485'
            ? rs485Client
            : serialClient
      if (connectionType.value === 'websocket' || connectionType.value === 'rs485') {
        await client.value.connect({ url: wsUrl.value, baudRate: baudRate.value })
      } else {
        await client.value.connect({ baudRate: baudRate.value })
      }
      isConnected.value = true
      startPolling()
    } catch (e: any) {
      errorMsg.value = t('toasts.connectFailed', { message: e.message })
    }
  }
}

function startPolling() {
  if (isPolling.value || !isConnected.value) return
  isPolling.value = true
  pollLoop()
}

function stopPolling() {
  isPolling.value = false
  if (pollTimer) {
    clearTimeout(pollTimer)
    pollTimer = null
  }
}

async function pollLoop() {
  if (!isPolling.value || !isConnected.value) return

  if (readEnabled.value) {
    try {
      clampDeviceAddress()
      const frame = buildReadHoldingRegisters(deviceAddress.value, 0, 26)
      const response = await client.value.sendRequest(frame, 300)

      if (!response.isError && response.data.length === 52) {
        const registers = new Uint16Array(26)
        for (let i = 0; i < 26; i++) {
          registers[i] = (response.data[i * 2]! << 8) | response.data[i * 2 + 1]!
        }
        const nextState = parseRegistersToState(registers)
        driveState.value = nextState
        errorMsg.value = ''

        if (addressScanEnabled.value && nextState.warningCode === 0) {
          addressScanEnabled.value = false
          scanTickCounter.value = 0
          errorMsg.value = t('toasts.scanFound', { address: deviceAddress.value })
        }
      } else if (response.isError) {
        errorMsg.value = t('toasts.modbusError', {
          code: response.errorCode?.toString(16) ?? '?',
        })
      }
    } catch (e: any) {
      errorMsg.value = t('toasts.pollFailed', { message: e.message })
    }

    advanceScanAddressIfNeeded()
  }

  if (isPolling.value) {
    pollTimer = window.setTimeout(pollLoop, pollIntervalMs.value)
  }
}

onUnmounted(() => {
  stopPolling()
  if (isConnected.value) {
    void client.value.disconnect()
  }
})
</script>

<template>
  <div class="min-h-screen bg-[#ece9d8] text-gray-900 flex flex-col font-sans text-[12px]">
    <header class="bg-[#000080] text-white px-2.5 py-1 flex items-center justify-between shadow-sm select-none border-b border-gray-400">
      <div class="flex items-center gap-2">
        <div class="w-3.5 h-3.5 bg-white/20 border border-white/40 flex items-center justify-center text-[9px] font-bold">YZ</div>
        <h1 class="text-[13px] font-bold tracking-wide">{{ t('meta.title') }}</h1>
        <span class="text-[11px] text-blue-200">{{ t('meta.subtitle') }}</span>
      </div>
      <div class="flex items-center gap-2">
        <span class="text-[11px] text-blue-200 hidden sm:inline mr-2">{{ t('meta.headerHint') }}</span>
        <div class="flex items-center gap-0 border border-white/40 overflow-hidden text-[11px]">
          <button
            type="button"
            class="px-1.5 py-0.5 leading-none"
            :class="locale === 'en' ? 'bg-white text-[#000080] font-bold' : 'bg-transparent text-blue-100 hover:bg-white/10'"
            @click="switchLocale('en')"
          >
            {{ t('locale.en') }}
          </button>
          <button
            type="button"
            class="px-1.5 py-0.5 leading-none border-l border-white/40"
            :class="locale === 'zh' ? 'bg-white text-[#000080] font-bold' : 'bg-transparent text-blue-100 hover:bg-white/10'"
            @click="switchLocale('zh')"
          >
            {{ t('locale.zh') }}
          </button>
        </div>
        <div class="flex gap-0.5">
          <div class="w-4 h-3.5 bg-[#d4d0c8] border border-white border-r-gray-600 border-b-gray-600 flex items-center justify-center text-black text-[9px] font-bold leading-none">_</div>
          <div class="w-4 h-3.5 bg-[#d4d0c8] border border-white border-r-gray-600 border-b-gray-600 flex items-center justify-center text-black text-[9px] font-bold leading-none">□</div>
          <div class="w-4 h-3.5 bg-[#d4d0c8] border border-white border-r-gray-600 border-b-gray-600 flex items-center justify-center text-black text-[9px] font-bold leading-none">×</div>
        </div>
      </div>
    </header>

    <main class="flex-1 p-2 grid vb-layout gap-2 min-h-0 overflow-auto">
      <!-- Left side: chart, 4 bottom boxes, send data strip & log -->
      <div class="flex flex-col gap-2 min-w-0">
        <WaveformChart class="min-h-[320px] flex-1" :state="driveState" />
        
        <div class="grid vb-left-boxes gap-2">
          <TelemetryPanel :state="driveState" />
          <DriverSettingsPanel :state="driveState" />
          <ModbusReadPanel
            v-model:deviceAddress="deviceAddress"
            v-model:pollIntervalMs="pollIntervalMs"
            v-model:readEnabled="readEnabled"
            v-model:addressScanEnabled="addressScanEnabled"
            :isConnected="isConnected"
          />
          <StatusConnectPanel
            v-model:connectionType="connectionType"
            v-model:baudRate="baudRate"
            v-model:wsUrl="wsUrl"
            :state="driveState"
            :isConnected="isConnected"
            :errorMsg="errorMsg"
            :webSerialSupported="webSerialSupported"
            @toggle="toggleConnection"
          />
        </div>

        <SendEchoStrip :client="client" />
        <CommLog :client="client" />
      </div>

      <!-- Right side: parameter list and modbus send -->
      <div class="flex flex-col gap-2 min-w-0">
        <ParamListPanel
          class="flex-1 min-h-[360px]"
          :state="driveState"
          :client="client"
          :deviceAddress="deviceAddress"
        />
        <ModbusSendPanel :client="client" :deviceAddress="deviceAddress" />
      </div>
    </main>
  </div>
</template>

<style>
html, body {
  margin: 0;
  padding: 0;
  height: 100%;
  background-color: #ece9d8;
}
#app {
  height: 100%;
}

.vb-layout {
  grid-template-columns: 1fr;
}
.vb-left-boxes {
  grid-template-columns: 1fr;
}

@media (min-width: 1100px) {
  .vb-layout {
    grid-template-columns: minmax(0, 1fr) 320px;
  }
  .vb-left-boxes {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}

@media (min-width: 640px) and (max-width: 1099px) {
  .vb-left-boxes {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
