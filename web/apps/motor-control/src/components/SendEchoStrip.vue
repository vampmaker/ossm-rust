<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { ModbusTransport } from '../lib/transport'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

const props = defineProps<{
  client: ModbusTransport
}>()

const lastTx = ref('')
let cleanup: (() => void) | null = null

function formatHex(data: Uint8Array): string {
  return Array.from(data)
    .map((b) => b.toString(16).padStart(2, '0').toUpperCase())
    .join(' ')
}

function bindClient(client: ModbusTransport) {
  if (cleanup) {
    cleanup()
    cleanup = null
  }
  cleanup = client.onLog((entry) => {
    if (entry.direction === 'TX') {
      lastTx.value = formatHex(entry.data)
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
</script>

<template>
  <GroupBox :caption="t('panels.sendEcho')">
    <div
      class="font-mono text-[12px] text-[#800000] min-h-[1.6rem] px-1.5 py-0.5 bg-white border border-gray-400 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] truncate select-all"
      :title="lastTx"
    >
      {{ lastTx || '—' }}
    </div>
  </GroupBox>
</template>
