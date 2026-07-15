<script setup lang="ts">
import { computed, ref } from 'vue'
import { ModbusRegister, type DriveState } from '../lib/registers'
import { buildWriteSingleRegister } from '../lib/modbus-rtu'
import type { ModbusTransport } from '../lib/transport'
import GroupBox from './GroupBox.vue'

const props = defineProps<{
  state: DriveState | null
  client: ModbusTransport
  deviceAddress: number
}>()

const isSaving = ref(false)
const saveMsg = ref('')

const saveFlagText = computed(() => {
  if (!props.state) return '未保存'
  switch (props.state.saveFlag) {
    case 0: return '未保存'
    case 1: return '正在保存'
    case 2: return '已保存'
    default: return String(props.state.saveFlag)
  }
})

const dirPolarityText = computed(() => {
  if (!props.state) return '负逻辑'
  return props.state.dirPolarity ? '正逻辑' : '负逻辑'
})

interface ParamRow {
  label: string
  value: string
  hint?: string
  isBlue?: boolean
}

const rows = computed<ParamRow[]>(() => {
  const s = props.state
  if (!s) {
    return [
      { label: 'mosbus使能:', value: '0', hint: '禁止' },
      { label: 'EN(使能):', value: '0', hint: '禁止' },
      { label: 'PU(步数):', value: '0' },
      { label: 'PU(总步数):', value: '0', isBlue: true },
      { label: '目标转速:', value: '0', hint: '( r/min )' },
      { label: '电机加速度:', value: '0', hint: '(r/min)/s' },
      { label: '弱磁角度:', value: '0', hint: '(0.1°)' },
      { label: '速度环Kp:', value: '0' },
      { label: '速度环Ki:', value: '0' },
      { label: '位置环Kp:', value: '0' },
      { label: '速度前馈:', value: '0', hint: 'V/KRPM' },
      { label: 'DIR极性:', value: '0', hint: '负逻辑' },
      { label: '电子齿轮比:', value: '1/1' },
      { label: '参数保存:', value: '0', hint: '未保存' },
      { label: '静态最大输出:', value: '0' },
      { label: '特殊功能:', value: '0' },
    ]
  }
  return [
    { label: 'mosbus使能:', value: String(s.modbusEnable), hint: s.modbusEnable ? '使能' : '禁止' },
    { label: 'EN(使能):', value: String(s.en), hint: s.en ? '使能' : '禁止' },
    { label: 'PU(步数):', value: String(s.pu) },
    { label: 'PU(总步数):', value: String(s.puTotal), isBlue: true },
    { label: '目标转速:', value: String(s.targetSpeed), hint: '( r/min )' },
    { label: '电机加速度:', value: String(s.acceleration), hint: '(r/min)/s' },
    { label: '弱磁角度:', value: String(s.speedStart), hint: '(0.1°)' },
    { label: '速度环Kp:', value: String(s.speedKp) },
    { label: '速度环Ki:', value: String(s.speedKi) },
    { label: '位置环Kp:', value: String(s.positionKp) },
    { label: '速度前馈:', value: String(s.speedFeedforwardRaw), hint: 'V/KRPM' },
    { label: 'DIR极性:', value: String(s.dirPolarity), hint: dirPolarityText.value },
    { label: '电子齿轮比:', value: `${s.eGearNumerator}/${s.eGearDenominator}` },
    { label: '参数保存:', value: String(s.saveFlag), hint: saveFlagText.value },
    { label: '静态最大输出:', value: String(s.speedFilter) },
    { label: '特殊功能:', value: String(s.positionForward) },
  ]
})

async function saveParameters() {
  if (!props.client.isConnected || isSaving.value) return
  isSaving.value = true
  saveMsg.value = ''
  try {
    const frame = buildWriteSingleRegister(props.deviceAddress, ModbusRegister.MODBUS_SAVE_FLAG, 1)
    const response = await props.client.sendRequest(frame, 800)
    if (response.isError) {
      saveMsg.value = `保存失败: 0x${response.errorCode?.toString(16)}`
    } else {
      saveMsg.value = '已触发参数保存'
    }
  } catch (e: any) {
    saveMsg.value = `保存失败: ${e.message}`
  } finally {
    isSaving.value = false
  }
}
</script>

<template>
  <GroupBox caption="modbus控制参数" class="h-full flex flex-col justify-between">
    <div class="overflow-y-auto flex-1 text-[12px] leading-[1.35rem] pr-1 pt-0.5">
      <div
        v-for="(row, i) in rows"
        :key="i"
        class="grid grid-cols-[5.8rem_3rem_1fr] gap-x-2 items-baseline py-[1px]"
      >
        <span class="text-gray-900 select-none">{{ row.label }}</span>
        <span
          class="font-mono text-left font-normal"
          :class="row.isBlue ? 'text-[#0000ff]' : 'text-[#cc0000]'"
        >{{ row.value }}</span>
        <span v-if="row.hint" class="text-gray-900 select-none">{{ row.hint }}</span>
        <span v-else class="text-gray-900" />
      </div>
    </div>

    <div class="mt-2 pt-1 flex items-center justify-between border-t border-gray-300/60">
      <span class="text-[11px] text-gray-700 truncate pr-2">{{ saveMsg }}</span>
      <button
        type="button"
        class="px-3 py-1 text-[12px] border border-gray-500 bg-[#e1e1e1] shadow-sm active:shadow-inner hover:bg-[#d4d0c8] disabled:opacity-50 text-gray-900 font-medium select-none ml-auto"
        :disabled="!client.isConnected || isSaving"
        @click="saveParameters"
      >
        参数保存
      </button>
    </div>
  </GroupBox>
</template>

