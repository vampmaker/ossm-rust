<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

defineProps<{
  deviceAddress: number
  pollIntervalMs: number
  readEnabled: boolean
  addressScanEnabled: boolean
  isConnected: boolean
}>()

defineEmits<{
  (e: 'update:deviceAddress', value: number): void
  (e: 'update:pollIntervalMs', value: number): void
  (e: 'update:readEnabled', value: boolean): void
  (e: 'update:addressScanEnabled', value: boolean): void
}>()

const pollOptions = [
  { label: '0.05s', value: 50 },
  { label: '0.1s', value: 100 },
  { label: '0.2s', value: 200 },
  { label: '0.02s', value: 20 },
]
</script>

<template>
  <GroupBox :caption="t('panels.modbusRead')">
    <div class="flex flex-col gap-2 text-[12px] pt-0.5">
      <div class="flex items-center gap-1.5 flex-wrap">
        <label for="mc-device-addr" class="text-gray-900 select-none">{{ t('modbusRead.deviceAddress') }}</label>
        <input
          id="mc-device-addr"
          type="number"
          min="1"
          max="255"
          class="w-14 border border-gray-400 px-1 py-0.5 bg-white shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-mono"
          :value="deviceAddress"
          @input="$emit('update:deviceAddress', Number(($event.target as HTMLInputElement).value))"
        />
      </div>

      <div class="flex items-center gap-3 pt-0.5">
        <label class="flex items-center gap-1.5 cursor-pointer select-none">
          <input
            type="checkbox"
            class="accent-blue-600"
            :checked="readEnabled"
            :disabled="!isConnected"
            @change="$emit('update:readEnabled', ($event.target as HTMLInputElement).checked)"
          />
          <span>{{ t('modbusRead.startReading') }}</span>
        </label>

        <label class="flex items-center gap-1.5 cursor-pointer select-none">
          <input
            type="checkbox"
            class="accent-blue-600"
            :checked="addressScanEnabled"
            :disabled="!isConnected"
            @change="$emit('update:addressScanEnabled', ($event.target as HTMLInputElement).checked)"
          />
          <span>{{ t('modbusRead.addressScan') }}</span>
        </label>
      </div>

      <fieldset class="border border-[#999999] px-2 pb-1.5 pt-1 mt-1 shadow-[1px_1px_0px_#ffffff]">
        <legend class="px-1 text-[11px] font-medium text-gray-800 select-none">{{ t('modbusRead.pollInterval') }}</legend>
        <div class="grid grid-cols-2 gap-x-2 gap-y-1 mt-0.5">
          <label
            v-for="opt in pollOptions"
            :key="opt.value"
            class="flex items-center gap-1.5 cursor-pointer select-none"
          >
            <input
              type="radio"
              name="mc-poll-interval"
              class="accent-blue-600"
              :value="opt.value"
              :checked="pollIntervalMs === opt.value"
              @change="$emit('update:pollIntervalMs', opt.value)"
            />
            <span>{{ opt.label }}</span>
          </label>
        </div>
      </fieldset>
    </div>
  </GroupBox>
</template>
