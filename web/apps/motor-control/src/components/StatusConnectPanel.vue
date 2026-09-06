<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { getWarningMessage, type DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

const props = defineProps<{
  state: DriveState | null
  connectionType: 'serial' | 'websocket' | 'rs485'
  baudRate: number
  wsUrl: string
  isConnected: boolean
  errorMsg: string
  webSerialSupported: boolean
}>()

defineEmits<{
  (e: 'update:connectionType', value: 'serial' | 'websocket' | 'rs485'): void
  (e: 'update:baudRate', value: number): void
  (e: 'update:wsUrl', value: string): void
  (e: 'toggle'): void
}>()

const baudOptions = [9600, 19200, 38400, 115200]

const warningText = computed(() => {
  if (props.errorMsg) return props.errorMsg
  if (!props.state) return t('status.waitingRead')
  return getWarningMessage(props.state.warningCode, t)
})

const statusKind = computed(() => {
  if (props.errorMsg) return 'error'
  if (!props.state) return 'neutral'
  if (props.state.warningCode !== 0) return 'error'
  return 'ok'
})

const connectCaption = computed(() => {
  if (props.isConnected) {
    return props.connectionType === 'serial' ? t('connect.closeSerial') : t('connect.disconnect')
  }
  return props.connectionType === 'serial' ? t('connect.openSerial') : t('connect.connect')
})
</script>

<template>
  <GroupBox :caption="t('panels.status')" id="motor-control-connection">
    <div class="flex flex-col gap-2 text-[12px] pt-0.5">
      <div
        class="min-h-[2.4rem] border px-1.5 py-1 text-center flex items-center justify-center leading-snug select-none"
        :class="{
          'border-red-400 bg-red-50 text-red-800 font-medium': statusKind === 'error',
          'border-green-500 bg-green-50 text-green-800 font-medium': statusKind === 'ok',
          'border-gray-400 bg-white/80 text-gray-800 font-medium shadow-[inset_1px_1px_2px_rgba(0,0,0,0.1)]': statusKind === 'neutral'
        }"
      >
        {{ warningText }}
      </div>

      <div
        v-if="connectionType === 'serial' && !webSerialSupported"
        class="text-[11px] text-red-700 bg-red-50 border border-red-200 px-1 py-0.5"
      >
        {{ t('connect.webSerialUnsupported') }}
      </div>

      <div class="flex items-center gap-1.5 flex-wrap">
        <label for="mc-connection-type" class="text-gray-900 select-none">{{ t('connect.connectionType') }}</label>
        <select
          id="mc-connection-type"
          class="border border-gray-400 bg-white px-1 py-0.5 disabled:bg-gray-100 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-sans"
          :value="connectionType"
          :disabled="isConnected"
          @input="$emit('update:connectionType', ($event.target as HTMLSelectElement).value as 'serial' | 'websocket' | 'rs485')"
        >
          <option value="serial">{{ t('connect.localSerial') }}</option>
          <option value="websocket">{{ t('connect.remoteWebSocket') }}</option>
          <option value="rs485">{{ t('connect.rs485WebSocket') }}</option>
        </select>
      </div>

      <div v-if="connectionType === 'serial' || connectionType === 'rs485'" class="flex items-center gap-1.5 flex-wrap">
        <label for="mc-baud" class="text-gray-900 select-none">{{ t('connect.baudRate') }}</label>
        <select
          id="mc-baud"
          class="border border-gray-400 bg-white px-1 py-0.5 disabled:bg-gray-100 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-mono"
          :value="baudRate"
          :disabled="isConnected"
          @input="$emit('update:baudRate', Number(($event.target as HTMLSelectElement).value))"
        >
          <option v-for="baud in baudOptions" :key="baud" :value="baud">{{ baud }}</option>
        </select>
      </div>

      <div v-if="connectionType !== 'serial'" class="flex flex-col gap-0.5">
        <label for="mc-ws-url" class="text-gray-900 select-none">{{ t('connect.wsUrl') }}</label>
        <input
          id="mc-ws-url"
          type="text"
          class="border border-gray-400 bg-white px-1 py-0.5 font-mono text-[11px] disabled:bg-gray-100 w-full shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)]"
          :value="wsUrl"
          :disabled="isConnected"
          :placeholder="connectionType === 'rs485' ? t('connect.wsRs485Placeholder') : t('connect.wsUrlPlaceholder')"
          @input="$emit('update:wsUrl', ($event.target as HTMLInputElement).value)"
        />
      </div>

      <button
        type="button"
        class="self-start px-3.5 py-1 border border-gray-500 bg-[#e1e1e1] shadow-sm active:shadow-inner hover:bg-[#d4d0c8] disabled:opacity-50 text-gray-900 font-medium select-none mt-0.5"
        :disabled="connectionType === 'serial' && !webSerialSupported && !isConnected"
        @click="$emit('toggle')"
      >
        {{ connectCaption }}
      </button>
    </div>
  </GroupBox>
</template>
