<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from 'vue'
import type { ModbusTransport, CommLogEntry } from '../lib/transport'

const props = defineProps<{
  client: ModbusTransport
}>()

const isExpanded = ref(false)
const logs = ref<CommLogEntry[]>([])
const MAX_LOGS = 100

function formatHex(data: Uint8Array): string {
  return Array.from(data).map((b) => b.toString(16).padStart(2, '0').toUpperCase()).join(' ')
}

function formatTime(timestamp: number): string {
  const d = new Date(timestamp)
  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}.${d.getMilliseconds().toString().padStart(3, '0')}`
}

let cleanup: (() => void) | null = null

function bindClient(client: ModbusTransport) {
  if (cleanup) {
    cleanup()
    cleanup = null
  }
  cleanup = client.onLog((entry) => {
    logs.value.unshift(entry)
    if (logs.value.length > MAX_LOGS) {
      logs.value.pop()
    }
  })
}

onMounted(() => bindClient(props.client))
onUnmounted(() => {
  if (cleanup) cleanup()
})
watch(
  () => props.client,
  (next) => bindClient(next),
)

function clearLogs() {
  logs.value = []
}
</script>

<template>
  <div class="border border-[#999999] bg-[#ece9d8] shadow-[1px_1px_0px_#ffffff]">
    <div
      class="px-2 py-1 flex justify-between items-center cursor-pointer hover:bg-[#d4d0c8] text-[12px] select-none"
      @click="isExpanded = !isExpanded"
    >
      <div class="flex items-center gap-2">
        <span class="font-medium text-gray-900">通信日志</span>
        <span class="text-gray-600">({{ logs.length }})</span>
      </div>
      <div class="flex items-center gap-3">
        <button
          v-if="isExpanded"
          type="button"
          class="text-gray-600 hover:text-gray-900 select-none"
          @click.stop="clearLogs"
        >
          清空
        </button>
        <span class="text-gray-600 select-none">{{ isExpanded ? '▲' : '▼' }}</span>
      </div>
    </div>

    <div
      v-show="isExpanded"
      class="p-2 border-t border-gray-400 bg-white h-40 overflow-y-auto font-mono text-[11px] shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)]"
    >
      <div v-if="logs.length === 0" class="text-gray-400 text-center py-4 italic select-none">
        暂无通信
      </div>
      <div
        v-for="(log, i) in logs"
        :key="i"
        class="flex gap-2 py-0.5 border-b border-gray-100 last:border-0"
      >
        <span class="text-gray-400 w-24 shrink-0">{{ formatTime(log.timestamp) }}</span>
        <span
          class="font-bold w-6 shrink-0"
          :class="log.direction === 'TX' ? 'text-blue-600' : 'text-green-600'"
        >
          {{ log.direction }}
        </span>
        <span class="text-gray-700 break-all select-all">{{ formatHex(log.data) }}</span>
      </div>
    </div>
  </div>
</template>

