<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { PinConfiguration, NetworkConfiguration, MotorState } from '../types'

const props = defineProps<{
  pinConfig: PinConfiguration
  netConfig: NetworkConfiguration
  state?: MotorState | null
  connected: boolean
  saving: boolean
  saved: boolean
  mode?: 'wifi' | 'bluetooth'
}>()

const emit = defineEmits<{
  (e: 'update:pinConfig', config: PinConfiguration): void
  (e: 'update:netConfig', config: NetworkConfiguration): void
  (e: 'save'): void
  (e: 'reset'): void
  (e: 'restart'): void
}>()

const { t } = useI18n()

function updatePinField<K extends keyof PinConfiguration>(
  key: K,
  value: PinConfiguration[K],
) {
  emit('update:pinConfig', { ...props.pinConfig, [key]: value })
}

function updateNetField<K extends keyof NetworkConfiguration>(
  key: K,
  value: NetworkConfiguration[K],
) {
  emit('update:netConfig', { ...props.netConfig, [key]: value })
}

const motorUpdateRate = computed<number | null>(() => {
  if (typeof props.state?.ups === 'number') {
    return props.state.ups
  }
  if (!props.state?.update_history || props.state.update_history.length === 0) {
    return null
  }
  return props.state.update_history[props.state.update_history.length - 1] ?? null
})

const averageUpdateRate = computed(() => {
  if (!props.state?.update_history || props.state.update_history.length === 0) {
    return null
  }
  const sum = props.state.update_history.reduce((a, b) => a + b, 0)
  return Math.round(sum / props.state.update_history.length)
})
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
          class="w-full border border-gray-300 rounded px-3 py-2 text-sm"
          :value="pinConfig.operating_mode ?? 'servo'"
          :disabled="!connected"
          @change="updatePinField('operating_mode', ($event.target as HTMLSelectElement).value as PinConfiguration['operating_mode'])"
        >
          <option value="servo">{{ t('settings.modeServo') }}</option>
          <option value="rtu_relay">{{ t('settings.modeRtuRelay') }}</option>
        </select>
        <p class="text-xs text-gray-500">
          {{ t('settings.modeHint') }}
        </p>
      </div>

      <!-- Motor Telemetry Card -->
      <div class="rounded-lg border border-gray-200 bg-gray-50 p-4">
        <div
          v-if="(pinConfig.operating_mode ?? 'servo') === 'rtu_relay'"
          class="text-sm text-gray-600"
        >
          {{ t('settings.relayTelemetryUnavailable') }}
        </div>
        <template v-else>
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center space-x-2">
              <span
                class="inline-block h-3 w-3 rounded-full"
                :class="motorUpdateRate !== null && motorUpdateRate > 0 ? 'bg-green-500 animate-pulse' : 'bg-red-400'"
              />
              <span class="text-sm font-semibold text-gray-700">{{ t('settings.updateRateTitle') }}</span>
            </div>
            <div>
              <span v-if="motorUpdateRate !== null" class="font-mono text-xl font-bold text-blue-600">
                {{ motorUpdateRate }}
                <span class="text-xs font-normal text-gray-500">{{ t('settings.updatesPerSec') }}</span>
              </span>
              <span v-else class="text-xs font-medium text-gray-400">
                {{ t('settings.notAvailable') }}
              </span>
            </div>
          </div>
          <div
            v-if="state && typeof state.dt_avg_ms === 'number'"
            class="mt-2 flex flex-wrap items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-600"
          >
            <span>{{ t('settings.loopDt') }} <strong class="font-mono text-gray-800">{{ state.dt_min_ms?.toFixed(1) }} / {{ state.dt_avg_ms?.toFixed(1) }} / {{ state.dt_max_ms?.toFixed(1) }} ms</strong></span>
            <span>{{ t('settings.mdev') }} <strong class="font-mono text-gray-800">{{ state.dt_mdev_ms?.toFixed(2) }} ms</strong></span>
          </div>
          <div
            v-else-if="motorUpdateRate !== null && state?.update_history"
            class="mt-2 flex items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-500"
          >
            <span>{{ t('settings.tenSecAvg') }} <strong class="font-mono text-gray-700">{{ averageUpdateRate }} Hz</strong></span>
          </div>

          <!-- Modbus Timing & Percentiles Table (1s Rolling Window) -->
          <div
            v-if="state && state.modbus_stats"
            class="mt-3 border-t border-gray-200 pt-3"
          >
            <div class="flex flex-wrap items-center justify-between text-xs mb-2">
              <span class="font-semibold text-gray-700">{{ t('settings.modbusTelemetry') }}</span>
              <span
                class="rounded-full px-2 py-0.5 font-bold"
                :class="state.modbus_stats.success_rate >= 98 ? 'bg-green-100 text-green-800' : 'bg-amber-100 text-amber-800'"
              >
                {{ t('settings.successRate', {
                  rate: state.modbus_stats.success_rate.toFixed(1),
                  ok: state.modbus_stats.successful_requests,
                  fail: state.modbus_stats.failed_requests,
                }) }}
              </span>
            </div>
            <div class="overflow-x-auto">
              <table class="w-full text-left font-mono text-[11px] text-gray-700 border-collapse">
                <thead>
                  <tr class="border-b border-gray-200 text-gray-500">
                    <th class="py-1 pr-2">{{ t('settings.metricUs') }}</th>
                    <th class="py-1 px-1">{{ t('settings.min') }}</th>
                    <th class="py-1 px-1">{{ t('settings.p5') }}</th>
                    <th class="py-1 px-1">{{ t('settings.p50') }}</th>
                    <th class="py-1 px-1">{{ t('settings.p95') }}</th>
                    <th class="py-1 px-1">{{ t('settings.max') }}</th>
                    <th class="py-1 pl-1">{{ t('settings.meanMdev') }}</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-gray-100">
                  <tr>
                    <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('settings.roundTrip') }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.round_trip.min }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.round_trip.pct5 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.round_trip.pct50 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.round_trip.pct95 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.round_trip.max }}</td>
                    <td class="py-1 pl-1">{{ state.modbus_stats.round_trip.mean }}±{{ state.modbus_stats.round_trip.mdev }}</td>
                  </tr>
                  <tr>
                    <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('settings.slaveLatency') }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.slave_latency.min }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.slave_latency.pct5 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.slave_latency.pct50 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.slave_latency.pct95 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.slave_latency.max }}</td>
                    <td class="py-1 pl-1">{{ state.modbus_stats.slave_latency.mean }}±{{ state.modbus_stats.slave_latency.mdev }}</td>
                  </tr>
                  <tr>
                    <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('settings.rxDuration') }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.rx_duration.min }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.rx_duration.pct5 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.rx_duration.pct50 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.rx_duration.pct95 }}</td>
                    <td class="py-1 px-1">{{ state.modbus_stats.rx_duration.max }}</td>
                    <td class="py-1 pl-1">{{ state.modbus_stats.rx_duration.mean }}±{{ state.modbus_stats.rx_duration.mdev }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </template>
      </div>

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
            :value="pinConfig.modbus_tx"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_tx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_rx"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_rx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_de_re"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_de_re', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_timeout_ms"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_timeout_ms', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_rx_timeout_us || 0"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_rx_timeout_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_inter_frame_delay_us"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_inter_frame_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :value="pinConfig.modbus_scan_delay_us"
            class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
            :disabled="!connected || saving"
            @input="updatePinField('modbus_scan_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
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
            :checked="pinConfig.ble_enabled ?? true"
            class="h-5 w-5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
            :disabled="!connected || saving || props.mode === 'bluetooth'"
            :title="props.mode === 'bluetooth' ? t('settings.cannotDisableBle') : ''"
            @change="updatePinField('ble_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="ble-enabled" class="text-sm font-semibold text-gray-800">
            {{ t('settings.enableBle') }}
          </label>
        </div>

        <div class="flex items-center space-x-3">
          <input
            id="wifi-enabled"
            type="checkbox"
            :checked="netConfig.wifi_enabled ?? true"
            class="h-5 w-5 rounded border-gray-300 text-blue-600 focus:ring-blue-500 disabled:opacity-50"
            :disabled="!connected || saving || props.mode === 'wifi'"
            :title="props.mode === 'wifi' ? t('settings.cannotDisableWifi') : ''"
            @change="updateNetField('wifi_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="wifi-enabled" class="text-sm font-semibold text-gray-800">
            {{ t('settings.enableWifi') }}
          </label>
        </div>
      </div>

      <!-- Conditional WiFi & Network Configuration Items -->
      <div
        v-if="netConfig.wifi_enabled"
        id="wifi-config-panel"
        class="space-y-4 rounded-lg border border-blue-200 bg-blue-50/50 p-4"
      >
        <h4 class="text-sm font-bold uppercase tracking-wider text-blue-800">
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
              :value="netConfig.ssid"
              class="w-full rounded border border-gray-300 px-3 py-2 text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateNetField('ssid', ($event.target as HTMLInputElement).value)"
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
              :value="netConfig.password"
              class="w-full rounded border border-gray-300 px-3 py-2 text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateNetField('password', ($event.target as HTMLInputElement).value)"
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
              :value="netConfig.hostname"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm bg-white"
              :disabled="!connected || saving"
              @input="updateNetField('hostname', ($event.target as HTMLInputElement).value)"
            >
          </div>
        </div>

        <div class="flex items-center space-x-2 pt-2">
          <input
            id="dhcp-enabled"
            type="checkbox"
            :checked="netConfig.dhcp_enabled"
            class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
            :disabled="!connected || saving"
            @change="updateNetField('dhcp_enabled', ($event.target as HTMLInputElement).checked)"
          >
          <label for="dhcp-enabled" class="text-sm font-medium text-gray-700">
            {{ t('settings.useDhcp') }}
          </label>
        </div>

        <div
          v-if="!netConfig.dhcp_enabled"
          class="grid grid-cols-1 gap-4 md:grid-cols-2 rounded bg-white p-3 border border-gray-200"
        >
          <div>
            <label for="static-ip" class="mb-1 block text-sm font-medium">{{ t('settings.staticIp') }}</label>
            <input
              id="static-ip"
              type="text"
              :value="netConfig.static_ip"
              placeholder="192.168.1.100"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateNetField('static_ip', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-mask" class="mb-1 block text-sm font-medium">{{ t('settings.subnetMask') }}</label>
            <input
              id="static-mask"
              type="text"
              :value="netConfig.static_mask"
              placeholder="255.255.255.0"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateNetField('static_mask', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-gateway" class="mb-1 block text-sm font-medium">{{ t('settings.defaultGateway') }}</label>
            <input
              id="static-gateway"
              type="text"
              :value="netConfig.static_gateway"
              placeholder="192.168.1.1"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateNetField('static_gateway', ($event.target as HTMLInputElement).value)"
            >
          </div>
          <div>
            <label for="static-dns" class="mb-1 block text-sm font-medium">{{ t('settings.dnsServer') }}</label>
            <input
              id="static-dns"
              type="text"
              :value="netConfig.static_dns"
              placeholder="8.8.8.8"
              class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
              :disabled="!connected || saving"
              @input="updateNetField('static_dns', ($event.target as HTMLInputElement).value)"
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
