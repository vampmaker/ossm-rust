<script setup lang="ts">
import type { PinConfiguration } from '../types'

const props = defineProps<{
  modelValue: PinConfiguration
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
          <span class="text-gray-400">0 = baud default</span>
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
