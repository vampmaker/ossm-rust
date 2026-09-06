<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { MotorTelemetryPanel } from '@ossm/shared'
import type { ShellConfig, MotorState } from '../types'

const props = defineProps<{
  shellConfig: ShellConfig
  state?: MotorState | null
  connected: boolean
  saving: boolean
  saved: boolean
  mode?: 'wifi' | 'bluetooth'
}>()

const emit = defineEmits<{
  (e: 'update:shellConfig', config: ShellConfig): void
  (e: 'save'): void
  (e: 'reset'): void
  (e: 'restart'): void
  (e: 'apply-operating-mode', mode: ShellConfig['operating_mode']): void
}>()

const { t } = useI18n()

function updateField<K extends keyof ShellConfig>(key: K, value: ShellConfig[K]) {
  emit('update:shellConfig', { ...props.shellConfig, [key]: value })
}

function onOperatingModeChange(event: Event) {
  const value = (event.target as HTMLSelectElement).value as ShellConfig['operating_mode']
  emit('update:shellConfig', { ...props.shellConfig, operating_mode: value })
  emit('apply-operating-mode', value)
}

const motorTelemetryAvailable = computed(
  () => (props.shellConfig.operating_mode ?? 'servo') === 'servo',
)
</script>

<template>
  <div class="space-y-6 bg-white p-6 shadow-lg rounded-lg border border-gray-200">
    <!-- Header -->
    <div class="flex items-center justify-between border-b border-gray-200 pb-4">
      <div>
        <h2 class="text-2xl font-bold text-gray-800">{{ t('settings.title') }}</h2>
        <p class="text-sm text-gray-500 mt-1">
          {{ t('settings.subtitle') }}
        </p>
      </div>
      <button
        class="px-5 py-2.5 font-bold text-white bg-blue-600 hover:bg-blue-700 disabled:bg-gray-300 disabled:text-gray-500 rounded shadow transition-colors"
        :disabled="!connected || saving"
        @click="emit('save')"
      >
        {{ saving ? t('settings.saving') : t('settings.save') }}
      </button>
    </div>

    <p v-if="saved" class="text-sm font-semibold text-green-600 bg-green-50 p-3 rounded border border-green-200">
      {{ t('settings.saved') }}
    </p>

    <!-- SECTION 1: MOTOR CONNECTION & MODBUS CONFIG -->
    <div class="space-y-4">
      <div class="flex items-center space-x-2 border-b border-gray-100 pb-2">
        <h3 class="text-lg font-bold text-gray-800">{{ t('settings.motorSection') }}</h3>
      </div>

      <!-- Device operating mode -->
      <div class="rounded-lg border border-gray-200 bg-white p-4 space-y-2">
        <label class="block text-sm font-semibold text-gray-700">{{ t('settings.operatingMode') }}</label>
        <select
          id="pin-operating-mode"
          class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
          :value="shellConfig.operating_mode ?? 'servo'"
          :disabled="!connected"
          @change="onOperatingModeChange"
        >
          <option value="servo">{{ t('settings.modeServo') }}</option>
          <option value="rtu_relay">{{ t('settings.modeRtuRelay') }}</option>
          <option value="rs485">{{ t('settings.modeRs485') }}</option>
        </select>
        <p class="text-xs text-gray-500">
          {{ t('settings.modeHint') }}
        </p>
      </div>

      <!-- Motor Telemetry Card -->
      <MotorTelemetryPanel :state="state" :motor-telemetry-available="motorTelemetryAvailable" />

      <div class="grid grid-cols-1 gap-4 md:grid-cols-3">
        <div>
          <label for="modbus-tx-pin" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.modbusTx') }}
          </label>
          <input
            id="modbus-tx-pin"
            type="number"
            min="0"
            max="48"
            :value="shellConfig.modbus_tx"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_tx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
        <div>
          <label for="modbus-rx-pin" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.modbusRx') }}
          </label>
          <input
            id="modbus-rx-pin"
            type="number"
            min="0"
            max="48"
            :value="shellConfig.modbus_rx"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_rx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
        <div>
          <label for="modbus-de-re-pin" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.modbusDeRe') }}
          </label>
          <input
            id="modbus-de-re-pin"
            type="number"
            min="0"
            max="48"
            :value="shellConfig.modbus_de_re"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_de_re', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
      </div>

      <div class="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-4">
        <div>
          <label for="modbus-timeout-ms" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.readTimeout') }}
            <span class="text-xs text-gray-400 block font-normal">{{ t('settings.readTimeoutHint') }}</span>
          </label>
          <input
            id="modbus-timeout-ms"
            type="number"
            min="0"
            max="1000"
            :value="shellConfig.modbus_timeout_ms"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_timeout_ms', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
        <div>
          <label for="modbus-rx-timeout-us" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.rxInterByte') }}
            <span class="text-xs text-gray-400 block font-normal">{{ t('settings.rxInterByteHint') }}</span>
          </label>
          <input
            id="modbus-rx-timeout-us"
            type="number"
            min="0"
            max="200000"
            :value="shellConfig.modbus_rx_timeout_us || 0"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_rx_timeout_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
        <div>
          <label for="modbus-inter-frame-delay-us" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.interFrameQuiet') }}
            <span class="text-xs text-gray-400 block font-normal">{{ t('settings.interFrameQuietHint') }}</span>
          </label>
          <input
            id="modbus-inter-frame-delay-us"
            type="number"
            min="0"
            max="200000"
            :value="shellConfig.modbus_inter_frame_delay_us"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_inter_frame_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
        <div>
          <label for="modbus-scan-delay-us" class="mb-1 block text-sm font-medium text-gray-700">
            {{ t('settings.scanDelay') }}
            <span class="text-xs text-gray-400 block font-normal">{{ t('settings.scanDelayHint') }}</span>
          </label>
          <input
            id="modbus-scan-delay-us"
            type="number"
            min="0"
            max="200000"
            :value="shellConfig.modbus_scan_delay_us"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updateField('modbus_scan_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
          >
        </div>
      </div>
    </div>

    <!-- SECTION 2: WIFI & BLE SECTION -->
    <div class="space-y-4 pt-4 border-t border-gray-200">
      <div class="flex items-center space-x-2 border-b border-gray-100 pb-2">
        <h3 class="text-lg font-bold text-gray-800">{{ t('settings.wifiBleSection') }}</h3>
      </div>

      <!-- Independent Interface Toggles -->
      <div class="grid grid-cols-1 md:grid-cols-2 gap-4 bg-gray-50 p-4 rounded-lg border border-gray-200">
        <div class="flex items-center space-x-3">
          <input
            id="ble-enabled"
            type="checkbox"
            :checked="shellConfig.ble_enabled ?? true"
            class="h-5 w-5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
            :disabled="!connected || saving || props.mode === 'bluetooth'"
            :title="props.mode === 'bluetooth' ? t('settings.cannotDisableBle') : ''"
            @change="updateField('ble_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="ble-enabled" class="text-sm font-semibold text-gray-800">
            {{ t('settings.enableBle') }}
          </label>
        </div>
        <div class="flex flex-col gap-1">
          <div class="flex items-center space-x-3">
            <input
              id="modbus-debug"
              type="checkbox"
              :checked="shellConfig.modbus_debug ?? false"
              class="h-5 w-5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
              :disabled="!connected || saving"
              @change="updateField('modbus_debug', ($event.target as HTMLInputElement).checked)"
            >
            <label for="modbus-debug" class="text-sm font-semibold text-gray-800">
              {{ t('settings.modbusDebug') }}
            </label>
          </div>
          <p class="pl-8 text-xs text-amber-700">
            {{ t('settings.modbusDebugHint') }}
          </p>
        </div>

        <div class="flex items-center space-x-3">
          <input
            id="wifi-enabled"
            type="checkbox"
            :checked="shellConfig.wifi_enabled ?? true"
            class="h-5 w-5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
            :disabled="!connected || saving || props.mode === 'wifi'"
            :title="props.mode === 'wifi' ? t('settings.cannotDisableWifi') : ''"
            @change="updateField('wifi_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="wifi-enabled" class="text-sm font-semibold text-gray-800">
            {{ t('settings.enableWifi') }}
          </label>
        </div>
      </div>

      <!-- Conditional WiFi & Network Configuration Items -->
      <div
        v-if="shellConfig.wifi_enabled"
        id="wifi-config-panel"
        class="space-y-4 rounded-lg border border-blue-200 bg-blue-50/50 p-4"
      >
        <h4 class="text-sm font-bold text-blue-900">
          {{ t('settings.wifiCredentials') }}
        </h4>

        <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div>
            <label for="wifi-ssid" class="mb-1 block text-sm font-medium text-gray-700">
              {{ t('settings.wifiSsid') }}
            </label>
            <input
              id="wifi-ssid"
              type="text"
              maxlength="32"
              :placeholder="t('settings.wifiSsidPlaceholder')"
              :value="shellConfig.ssid"
              class="w-full rounded border border-gray-300 px-3 py-2 text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateField('ssid', ($event.target as HTMLInputElement).value)"
            >
          </div>

          <div>
            <label for="wifi-password" class="mb-1 block text-sm font-medium text-gray-700">
              {{ t('settings.wifiPassword') }}
            </label>
            <input
              id="wifi-password"
              type="password"
              maxlength="64"
              :placeholder="t('settings.wifiPasswordPlaceholder')"
              :value="shellConfig.password"
              class="w-full rounded border border-gray-300 px-3 py-2 text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateField('password', ($event.target as HTMLInputElement).value)"
            >
          </div>

          <div>
            <label for="net-hostname" class="mb-1 block text-sm font-medium text-gray-700">
              {{ t('settings.hostname') }}
              <span class="text-xs text-gray-400 block font-normal">{{ t('settings.hostnameHint') }}</span>
            </label>
            <input
              id="net-hostname"
              type="text"
              :value="shellConfig.hostname"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateField('hostname', ($event.target as HTMLInputElement).value)"
            >
          </div>
        </div>

        <div class="flex items-center space-x-2 pt-2">
          <input
            id="dhcp-enabled"
            type="checkbox"
            :checked="shellConfig.dhcp_enabled"
            class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
            :disabled="!connected || saving"
            @change="updateField('dhcp_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="dhcp-enabled" class="text-sm font-medium text-gray-700">
            {{ t('settings.useDhcp') }}
          </label>
        </div>

        <div
          v-if="!shellConfig.dhcp_enabled"
          class="grid grid-cols-1 gap-4 md:grid-cols-2 rounded bg-white p-3 border border-gray-200"
        >
          <div>
            <label for="static-ip" class="mb-1 block text-sm font-medium">{{ t('settings.staticIp') }}</label>
            <input
              id="static-ip"
              type="text"
              :value="shellConfig.static_ip"
              placeholder="192.168.1.100"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateField('static_ip', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-mask" class="mb-1 block text-sm font-medium">{{ t('settings.subnetMask') }}</label>
            <input
              id="static-mask"
              type="text"
              :value="shellConfig.static_mask"
              placeholder="255.255.255.0"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateField('static_mask', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-gateway" class="mb-1 block text-sm font-medium">{{ t('settings.defaultGateway') }}</label>
            <input
              id="static-gateway"
              type="text"
              :value="shellConfig.static_gateway"
              placeholder="192.168.1.1"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateField('static_gateway', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-dns" class="mb-1 block text-sm font-medium">{{ t('settings.dnsServer') }}</label>
            <input
              id="static-dns"
              type="text"
              :value="shellConfig.static_dns"
              placeholder="8.8.8.8"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateField('static_dns', ($event.target as HTMLInputElement).value)"
            >
          </div>
        </div>
      </div>
    </div>

    <!-- Action Buttons -->
    <div class="flex flex-wrap items-center justify-end gap-3 pt-4 border-t border-gray-200">
      <button
        class="rounded bg-gray-200 px-4 py-2 font-medium text-gray-800 hover:bg-gray-300 disabled:opacity-50"
        :disabled="!connected || saving"
        @click="emit('reset')"
      >
        {{ t('settings.resetDefaults') }}
      </button>
      <button
        class="rounded bg-amber-500 px-4 py-2 font-medium text-white hover:bg-amber-600 disabled:opacity-50"
        :disabled="!connected || saving"
        @click="emit('restart')"
      >
        {{ t('settings.restartDevice') }}
      </button>
    </div>
  </div>
</template>
