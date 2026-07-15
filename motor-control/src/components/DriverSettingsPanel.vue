<script setup lang="ts">
import { computed } from 'vue'
import type { DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const props = defineProps<{
  state: DriveState | null
}>()

const modeText = computed(() => {
  if (!props.state) return '--'
  const mode = Math.floor(props.state.en / 2) % 2
  return mode === 0 ? '速度模式' : '位置模式'
})

const enText = computed(() => {
  if (!props.state) return '--'
  const en = Math.floor(props.state.en / 4) % 2
  return en === 0 ? '禁止' : '使能'
})

const dirText = computed(() => {
  if (!props.state) return '--'
  const dir = Math.floor(props.state.en / 8) % 2
  return dir === 0 ? '正转' : '反转'
})

const modeCode = computed(() => (props.state ? Math.floor(props.state.en / 2) % 2 : '--'))
const enCode = computed(() => (props.state ? Math.floor(props.state.en / 4) % 2 : '--'))
const dirCode = computed(() => (props.state ? Math.floor(props.state.en / 8) % 2 : '--'))
</script>

<template>
  <GroupBox caption="驱动器设置参数">
    <div class="flex flex-col gap-1.5 text-[12px] pt-1">
      <div class="grid grid-cols-[2.8rem_2rem_1fr] items-baseline gap-1">
        <span class="text-gray-900 select-none">模式:</span>
        <span class="font-mono text-[#cc0000] text-center">{{ modeCode }}</span>
        <span class="text-gray-900 select-none">{{ modeText }}</span>
      </div>
      <div class="grid grid-cols-[2.8rem_2rem_1fr] items-baseline gap-1">
        <span class="text-gray-900 select-none">转向:</span>
        <span class="font-mono text-[#cc0000] text-center">{{ dirCode }}</span>
        <span class="text-gray-900 select-none">{{ dirText }}</span>
      </div>
      <div class="grid grid-cols-[2.8rem_2rem_1fr] items-baseline gap-1">
        <span class="text-gray-900 select-none">使能:</span>
        <span class="font-mono text-[#cc0000] text-center">{{ enCode }}</span>
        <span
          class="select-none"
          :class="enCode === 1 ? 'text-green-700 font-medium' : 'text-gray-900'"
        >{{ enText }}</span>
      </div>
    </div>
  </GroupBox>
</template>

