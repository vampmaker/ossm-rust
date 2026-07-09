<script setup lang="ts">
import type { NetworkConfiguration } from '../types'

const props = defineProps<{
  modelValue: NetworkConfiguration
  connected: boolean
  saving: boolean
  saved: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: NetworkConfiguration): void
  (e: 'save'): void
  (e: 'reset'): void
}>()

function updateField<K extends keyof NetworkConfiguration>(
  key: K,
  value: NetworkConfiguration[K],
) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}
</script>

<template>
  <div class="space-y-4 bg-white p-4 shadow-md">
    <div class="flex items-center justify-between">
      <h2 class="text-xl font-bold">Network &amp; mDNS Settings</h2>
      <button
        class="px-4 py-2 font-bold text-white bg-blue-500 hover:bg-blue-600 disabled:bg-gray-300 disabled:text-gray-500 rounded"
        :disabled="!connected || saving"
        @click="emit('save')"
      >
        {{ saving ? 'Saving...' : 'Save' }}
      </button>
    </div>

    <p class="text-sm text-gray-500">
      Configure device hostname (mDNS responder) and IP address assignment. Changes require a device restart.
    </p>

    <p v-if="saved" class="text-sm text-green-600">
      Network settings saved. Restart the device to apply.
    </p>

    <div>
      <label for="net-hostname" class="mb-1 block text-sm font-medium">
        Hostname (mDNS)
        <span class="text-gray-400">Accessible at http://[hostname].local or .lan</span>
      </label>
      <input
        id="net-hostname"
        type="text"
        :value="modelValue.hostname"
        class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
        :disabled="!connected || saving"
        @input="updateField('hostname', ($event.target as HTMLInputElement).value)"
      >
    </div>

    <div class="flex items-center space-x-2 py-2">
      <input
        id="dhcp-enabled"
        type="checkbox"
        :checked="modelValue.dhcp_enabled"
        class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
        :disabled="!connected || saving"
        @change="updateField('dhcp_enabled', ($event.target as HTMLInputElement).checked)"
      >
      <label for="dhcp-enabled" class="text-sm font-medium text-gray-700">
        Use DHCP (Automatic IP Address Assignment)
      </label>
    </div>

    <div v-if="!modelValue.dhcp_enabled" class="grid grid-cols-1 gap-4 md:grid-cols-2 rounded bg-gray-50 p-3 border border-gray-200">
      <div>
        <label for="static-ip" class="mb-1 block text-sm">Static IP Address</label>
        <input
          id="static-ip"
          type="text"
          :value="modelValue.static_ip"
          placeholder="192.168.1.100"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('static_ip', ($event.target as HTMLInputElement).value)"
        >
      </div>
      <div>
        <label for="static-mask" class="mb-1 block text-sm">Subnet Mask</label>
        <input
          id="static-mask"
          type="text"
          :value="modelValue.static_mask"
          placeholder="255.255.255.0"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('static_mask', ($event.target as HTMLInputElement).value)"
        >
      </div>
      <div>
        <label for="static-gateway" class="mb-1 block text-sm">Gateway</label>
        <input
          id="static-gateway"
          type="text"
          :value="modelValue.static_gateway"
          placeholder="192.168.1.1"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('static_gateway', ($event.target as HTMLInputElement).value)"
        >
      </div>
      <div>
        <label for="static-dns" class="mb-1 block text-sm">DNS Server</label>
        <input
          id="static-dns"
          type="text"
          :value="modelValue.static_dns"
          placeholder="8.8.8.8"
          class="w-full rounded border border-gray-300 px-3 py-2 font-mono text-sm"
          :disabled="!connected || saving"
          @input="updateField('static_dns', ($event.target as HTMLInputElement).value)"
        >
      </div>
    </div>

    <div class="flex justify-start pt-2">
      <button
        class="rounded bg-gray-200 px-4 py-2 font-medium text-gray-800 hover:bg-gray-300 disabled:opacity-50"
        :disabled="!connected || saving"
        @click="emit('reset')"
      >
        Reset Network Defaults
      </button>
    </div>
  </div>
</template>
