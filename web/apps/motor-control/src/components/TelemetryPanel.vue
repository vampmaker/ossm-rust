<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

defineProps<{
  state: DriveState | null
}>()

function formatNumber(val: number | undefined, decimals: number = 1): string {
  if (val === undefined) return '0'
  return val.toFixed(decimals)
}
</script>

<template>
  <GroupBox :caption="t('panels.telemetry')">
    <div class="flex flex-col gap-1.5 text-[12px] pt-1">
      <div class="grid grid-cols-[auto_1fr_max-content] items-baseline gap-2">
        <span class="text-[#0000ff] font-medium select-none">{{ t('telemetry.current') }}</span>
        <span class="font-mono text-[#0000ff] text-right">{{ formatNumber(state?.currentA, 1) }}</span>
        <span class="text-gray-900 select-none">{{ t('telemetry.unitA') }}</span>
      </div>
      <div class="grid grid-cols-[auto_1fr_max-content] items-baseline gap-2">
        <span class="text-[#008000] font-medium select-none">{{ t('telemetry.pwm') }}</span>
        <span class="font-mono text-[#008000] text-right">{{ formatNumber(state?.pwmPercent, 1) }}</span>
        <span class="text-gray-900 select-none">{{ t('telemetry.unitPercent') }}</span>
      </div>
      <div class="grid grid-cols-[auto_1fr_max-content] items-baseline gap-2">
        <span class="text-[#cc0000] font-medium select-none">{{ t('telemetry.speed') }}</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.speedRmin, 1) }}</span>
        <span class="text-gray-900 select-none">{{ t('telemetry.unitRmin') }}</span>
      </div>
      <div class="grid grid-cols-[auto_1fr_max-content] items-baseline gap-2">
        <span class="text-gray-900 font-medium select-none">{{ t('telemetry.temperature') }}</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.temperatureC, 0) }}</span>
        <span class="text-gray-900 select-none">{{ t('telemetry.unitC') }}</span>
      </div>
      <div class="grid grid-cols-[auto_1fr_max-content] items-baseline gap-2">
        <span class="text-gray-900 font-medium select-none">{{ t('telemetry.voltage') }}</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.voltageV, 1) }}</span>
        <span class="text-gray-900 select-none">{{ t('telemetry.unitV') }}</span>
      </div>
    </div>
  </GroupBox>
</template>
