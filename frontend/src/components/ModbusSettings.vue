<script setup lang="ts">
import { computed } from 'vue'
import type { PinConfiguration, MotorState } from '../types'

const props = defineProps<{
  modelValue: PinConfiguration
  state?: MotorState | null
  connected: boolean
  saving: boolean
  saved: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: PinConfiguration): void
  (e: 'save'): void
  (e: 'reset'): void
  (e: 'restart'): void
}>()

function updateField<K extends keyof PinConfiguration>(
  key: K,
  value: PinConfiguration[K],
) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}

const motorUpdateRate = computed(() => {
  if (typeof props.state?.ups === 'number') {
    return props.state.ups
  }
  if (!props.state?.update_history || props.state.update_history.length === 0) {
    return null
  }
  return props.state.update_history[props.state.update_history.length - 1]
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
  <div class="space-y-4 bg-white p-4 shadow-md">
    <div class="flex items-center justify-between">
      <h2 class="text-xl font-bold">Device Settings</h2>
      <button
        class="px-4 py-2 font-bold text-white bg-blue-500 hover:bg-blue-600 disabled:bg-gray-300 disabled:text-gray-500"
        :disabled="!connected || saving"
        @click="emit('save')"
      >
        {{ saving ? 'Saving...' : 'Save' }}
      </button>
    </div>

    <!-- Motor Telemetry Card (Motor Update Per Second & Loop Stats) -->
    <div class="rounded-lg border border-gray-200 bg-gray-50 p-3.5">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center space-x-2">
          <span
            class="inline-block h-2.5 w-2.5 rounded-full"
            :class="motorUpdateRate !== null ? 'bg-green-500 animate-pulse' : 'bg-gray-300'"
          />
          <span class="text-sm font-semibold text-gray-700">Motor Update Rate</span>
        </div>
        <div>
          <span v-if="motorUpdateRate !== null" class="font-mono text-xl font-bold text-blue-600">
            {{ motorUpdateRate }}
            <span class="text-xs font-normal text-gray-500">updates/sec (Hz)</span>
          </span>
          <span v-else class="text-xs font-medium text-gray-400">
            Not available
          </span>
        </div>
      </div>
      <div
        v-if="state && typeof state.dt_avg_ms === 'number'"
        class="mt-2 flex flex-wrap items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-600"
      >
        <span>Loop dt (min/avg/max): <strong class="font-mono text-gray-800">{{ state.dt_min_ms?.toFixed(1) }} / {{ state.dt_avg_ms?.toFixed(1) }} / {{ state.dt_max_ms?.toFixed(1) }} ms</strong></span>
        <span>mdev: <strong class="font-mono text-gray-800">{{ state.dt_mdev_ms?.toFixed(2) }} ms</strong></span>
      </div>
      <div
        v-else-if="motorUpdateRate !== null && state?.update_history"
        class="mt-2 flex items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-500"
      >
        <span>10s Avg: <strong class="font-mono text-gray-700">{{ averageUpdateRate }} Hz</strong></span>
        <span class="font-mono">History: [{{ state.update_history.slice(-5).join(', ') }}]</span>
      </div>
    </div>

    <p class="text-sm text-gray-500">
      Changes require a device restart to take effect.
      Use 0 for default behavior.
    </p>

    <p v-if="saved" class="text-sm text-green-600">
      Settings saved. Restart the device to apply.
    </p>

    <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <div>
        <label for="modbus-timeout-ms" class="mb-1 block text-sm">
          Read timeout (ms)
          <span class="text-gray-400">0 = Auto (15ms @ 115200)</span>
        </label>
        <input
          id="modbus-timeout-ms"
          type="number"
          min="0"
          max="1000"
          :value="modelValue.modbus_timeout_ms"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_timeout_ms', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
      <div>
        <label for="modbus-rx-timeout-us" class="mb-1 block text-sm">
          RX inter-byte timeout (µs)
          <span class="text-gray-400">0 = Auto (1750µs @ 115200)</span>
        </label>
        <input
          id="modbus-rx-timeout-us"
          type="number"
          min="0"
          max="200000"
          :value="modelValue.modbus_rx_timeout_us || 0"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_rx_timeout_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
      <div>
        <label for="modbus-inter-frame-delay-us" class="mb-1 block text-sm">
          Inter-frame quiet interval (µs)
          <span class="text-gray-400">0 = Auto (350µs @ 115200)</span>
        </label>
        <input
          id="modbus-inter-frame-delay-us"
          type="number"
          min="0"
          max="200000"
          :value="modelValue.modbus_inter_frame_delay_us"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_inter_frame_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
      <div>
        <label for="modbus-scan-delay-us" class="mb-1 block text-sm">
          Scan delay (µs)
          <span class="text-gray-400">0 = Modbus t3.5 only</span>
        </label>
        <input
          id="modbus-scan-delay-us"
          type="number"
          min="0"
          max="200000"
          :value="modelValue.modbus_scan_delay_us"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_scan_delay_us', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
    </div>

    <div class="grid grid-cols-1 gap-4 md:grid-cols-3">
      <div>
        <label for="modbus-tx-pin" class="mb-1 block text-sm">
          Modbus TX pin
        </label>
        <input
          id="modbus-tx-pin"
          type="number"
          min="0"
          max="48"
          :value="modelValue.modbus_tx"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_tx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
      <div>
        <label for="modbus-rx-pin" class="mb-1 block text-sm">
          Modbus RX pin
        </label>
        <input
          id="modbus-rx-pin"
          type="number"
          min="0"
          max="48"
          :value="modelValue.modbus_rx"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_rx', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
      <div>
        <label for="modbus-de-re-pin" class="mb-1 block text-sm">
          Modbus DE/RE pin
        </label>
        <input
          id="modbus-de-re-pin"
          type="number"
          min="0"
          max="48"
          :value="modelValue.modbus_de_re"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('modbus_de_re', Number.parseInt(($event.target as HTMLInputElement).value) || 0)"
        >
      </div>
    </div>

    <div class="flex items-center space-x-2 py-2">
      <input
        id="ble-enabled"
        type="checkbox"
        :checked="modelValue.ble_enabled ?? true"
        class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
        :disabled="!connected || saving"
        @change="updateField('ble_enabled', ($event.target as HTMLInputElement).checked)"
      >
      <label for="ble-enabled" class="text-sm font-medium text-gray-700">
        Enable Bluetooth Low Energy (BLE)
      </label>
    </div>

    <div class="grid grid-cols-1 gap-2 md:grid-cols-2">
      <button
        class="rounded bg-gray-200 px-4 py-2 font-medium text-gray-800 hover:bg-gray-300 disabled:opacity-50"
        :disabled="!connected || saving"
        @click="emit('reset')"
      >
        Reset to defaults
      </button>
      <button
        class="rounded bg-amber-500 px-4 py-2 font-medium text-white hover:bg-amber-600 disabled:opacity-50"
        :disabled="!connected || saving"
        @click="emit('restart')"
      >
        Restart device
      </button>
    </div>
  </div>
</template>
