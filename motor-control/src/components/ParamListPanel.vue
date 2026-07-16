<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { ModbusRegister, type DriveState } from '../lib/registers'
import { buildWriteSingleRegister } from '../lib/modbus-rtu'
import type { ModbusTransport } from '../lib/transport'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

const props = defineProps<{
  state: DriveState | null
  client: ModbusTransport
  deviceAddress: number
}>()

const isSaving = ref(false)
const saveMsg = ref('')

const saveFlagText = computed(() => {
  if (!props.state) return t('paramList.saveNotSaved')
  switch (props.state.saveFlag) {
    case 0: return t('paramList.saveNotSaved')
    case 1: return t('paramList.saveSaving')
    case 2: return t('paramList.saveSaved')
    default: return String(props.state.saveFlag)
  }
})

const dirPolarityText = computed(() => {
  if (!props.state) return t('paramList.hintNegativeLogic')
  return props.state.dirPolarity ? t('paramList.hintPositiveLogic') : t('paramList.hintNegativeLogic')
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
      { label: t('paramList.modbusEnable'), value: '0', hint: t('paramList.hintDisable') },
      { label: t('paramList.en'), value: '0', hint: t('paramList.hintDisable') },
      { label: t('paramList.pu'), value: '0' },
      { label: t('paramList.puTotal'), value: '0', isBlue: true },
      { label: t('paramList.targetSpeed'), value: '0', hint: t('paramList.hintRmin') },
      { label: t('paramList.acceleration'), value: '0', hint: t('paramList.hintAccel') },
      { label: t('paramList.weakField'), value: '0', hint: t('paramList.hintAngle') },
      { label: t('paramList.speedKp'), value: '0' },
      { label: t('paramList.speedKi'), value: '0' },
      { label: t('paramList.positionKp'), value: '0' },
      { label: t('paramList.speedFeedforward'), value: '0', hint: t('paramList.hintFeedforward') },
      { label: t('paramList.dirPolarity'), value: '0', hint: t('paramList.hintNegativeLogic') },
      { label: t('paramList.eGearRatio'), value: '1/1' },
      { label: t('paramList.saveFlag'), value: '0', hint: t('paramList.saveNotSaved') },
      { label: t('paramList.staticMaxOutput'), value: '0' },
      { label: t('paramList.specialFunction'), value: '0' },
    ]
  }
  return [
    { label: t('paramList.modbusEnable'), value: String(s.modbusEnable), hint: s.modbusEnable ? t('paramList.hintEnable') : t('paramList.hintDisable') },
    { label: t('paramList.en'), value: String(s.en), hint: s.en ? t('paramList.hintEnable') : t('paramList.hintDisable') },
    { label: t('paramList.pu'), value: String(s.pu) },
    { label: t('paramList.puTotal'), value: String(s.puTotal), isBlue: true },
    { label: t('paramList.targetSpeed'), value: String(s.targetSpeed), hint: t('paramList.hintRmin') },
    { label: t('paramList.acceleration'), value: String(s.acceleration), hint: t('paramList.hintAccel') },
    { label: t('paramList.weakField'), value: String(s.speedStart), hint: t('paramList.hintAngle') },
    { label: t('paramList.speedKp'), value: String(s.speedKp) },
    { label: t('paramList.speedKi'), value: String(s.speedKi) },
    { label: t('paramList.positionKp'), value: String(s.positionKp) },
    { label: t('paramList.speedFeedforward'), value: String(s.speedFeedforwardRaw), hint: t('paramList.hintFeedforward') },
    { label: t('paramList.dirPolarity'), value: String(s.dirPolarity), hint: dirPolarityText.value },
    { label: t('paramList.eGearRatio'), value: `${s.eGearNumerator}/${s.eGearDenominator}` },
    { label: t('paramList.saveFlag'), value: String(s.saveFlag), hint: saveFlagText.value },
    { label: t('paramList.staticMaxOutput'), value: String(s.speedFilter) },
    { label: t('paramList.specialFunction'), value: String(s.positionForward) },
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
      saveMsg.value = t('paramList.saveFailed', {
        message: `0x${response.errorCode?.toString(16) ?? '?'}`,
      })
    } else {
      saveMsg.value = t('paramList.saveTriggered')
    }
  } catch (e: any) {
    saveMsg.value = t('paramList.saveFailed', { message: e.message })
  } finally {
    isSaving.value = false
  }
}
</script>

<template>
  <GroupBox :caption="t('panels.paramList')" class="h-full flex flex-col justify-between">
    <div class="overflow-y-auto flex-1 text-[12px] leading-[1.35rem] pr-1 pt-0.5">
      <div
        v-for="(row, i) in rows"
        :key="i"
        class="grid grid-cols-[auto_3rem_1fr] gap-x-2 items-baseline py-[1px]"
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
        {{ t('paramList.saveButton') }}
      </button>
    </div>
  </GroupBox>
</template>
