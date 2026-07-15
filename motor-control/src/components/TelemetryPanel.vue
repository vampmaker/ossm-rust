<script setup lang="ts">
import type { DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const props = defineProps<{
  state: DriveState | null
}>()

function formatNumber(val: number | undefined, decimals: number = 1): string {
  if (val === undefined) return '0'
  return val.toFixed(decimals)
}
</script>

<template>
  <GroupBox caption="电机运行参数">
    <div class="flex flex-col gap-1.5 text-[12px] pt-1">
      <div class="grid grid-cols-[4.5rem_1fr_3rem] items-baseline gap-1">
        <span class="text-[#0000ff] font-medium select-none">电流:</span>
        <span class="font-mono text-[#0000ff] text-right">{{ formatNumber(state?.currentA, 1) }}</span>
        <span class="text-gray-900 select-none">(A)</span>
      </div>
      <div class="grid grid-cols-[4.5rem_1fr_3rem] items-baseline gap-1">
        <span class="text-[#008000] font-medium select-none">输出脉宽:</span>
        <span class="font-mono text-[#008000] text-right">{{ formatNumber(state?.pwmPercent, 1) }}</span>
        <span class="text-gray-900 select-none">(%)</span>
      </div>
      <div class="grid grid-cols-[4.5rem_1fr_3rem] items-baseline gap-1">
        <span class="text-[#cc0000] font-medium select-none">当前转速:</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.speedRmin, 1) }}</span>
        <span class="text-gray-900 select-none">(r/min)</span>
      </div>
      <div class="grid grid-cols-[4.5rem_1fr_3rem] items-baseline gap-1">
        <span class="text-gray-900 font-medium select-none">温度:</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.temperatureC, 0) }}</span>
        <span class="text-gray-900 select-none">(℃)</span>
      </div>
      <div class="grid grid-cols-[4.5rem_1fr_3rem] items-baseline gap-1">
        <span class="text-gray-900 font-medium select-none">电压:</span>
        <span class="font-mono text-[#cc0000] text-right">{{ formatNumber(state?.voltageV, 1) }}</span>
        <span class="text-gray-900 select-none">(V)</span>
      </div>
    </div>
  </GroupBox>
</template>

