<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { DriveState } from '../lib/registers'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

const props = defineProps<{
  state: DriveState | null
}>()

const modeText = computed(() => {
  if (!props.state) return '--'
  const mode = Math.floor(props.state.en / 2) % 2
  return mode === 0 ? t('driver.speedMode') : t('driver.positionMode')
})

const enText = computed(() => {
  if (!props.state) return '--'
  const en = Math.floor(props.state.en / 4) % 2
  return en === 0 ? t('driver.disabled') : t('driver.enabled')
})

const dirText = computed(() => {
  if (!props.state) return '--'
  const dir = Math.floor(props.state.en / 8) % 2
  return dir === 0 ? t('driver.forward') : t('driver.reverse')
})

const modeCode = computed(() => (props.state ? Math.floor(props.state.en / 2) % 2 : '--'))
const enCode = computed(() => (props.state ? Math.floor(props.state.en / 4) % 2 : '--'))
const dirCode = computed(() => (props.state ? Math.floor(props.state.en / 8) % 2 : '--'))
</script>

<template>
  <GroupBox :caption="t('panels.driver')">
    <div class="flex flex-col gap-1.5 text-[12px] pt-1">
      <div class="grid grid-cols-[auto_2rem_1fr] items-baseline gap-2">
        <span class="text-gray-900 select-none">{{ t('driver.mode') }}</span>
        <span class="font-mono text-[#cc0000] text-center">{{ modeCode }}</span>
        <span class="text-gray-900 select-none">{{ modeText }}</span>
      </div>
      <div class="grid grid-cols-[auto_2rem_1fr] items-baseline gap-2">
        <span class="text-gray-900 select-none">{{ t('driver.direction') }}</span>
        <span class="font-mono text-[#cc0000] text-center">{{ dirCode }}</span>
        <span class="text-gray-900 select-none">{{ dirText }}</span>
      </div>
      <div class="grid grid-cols-[auto_2rem_1fr] items-baseline gap-2">
        <span class="text-gray-900 select-none">{{ t('driver.enable') }}</span>
        <span class="font-mono text-[#cc0000] text-center">{{ enCode }}</span>
        <span
          class="select-none"
          :class="enCode === 1 ? 'text-green-700 font-medium' : 'text-gray-900'"
        >{{ enText }}</span>
      </div>
    </div>
  </GroupBox>
</template>
