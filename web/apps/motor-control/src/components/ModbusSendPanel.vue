<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { toUnsigned16, fromSigned32 } from '../lib/registers'
import { buildCustomFunctionWrite, buildWriteMultipleRegisters, buildWriteSingleRegister, calculateCRC } from '../lib/modbus-rtu'
import type { ModbusTransport } from '../lib/transport'
import { getSendTypeOptions } from '../lib/send-options'
import GroupBox from './GroupBox.vue'

const { t } = useI18n()

const props = defineProps<{
  client: ModbusTransport
  deviceAddress: number
}>()

const sendTypeId = ref(0)
const sendTypeValue = ref(0)
const isWriting = ref(false)
const writeError = ref('')
const writeInfo = ref('')
const showAdvanced = ref(false)
const manualRawHex = ref('')
const manualAppendCrc = ref(false)
const manualWaitResponse = ref(true)
const manualTimeoutMs = ref(800)

const sendTypeOptions = computed(() => getSendTypeOptions(t))

const selectedSendType = computed(
  () => sendTypeOptions.value.find((item) => item.id === sendTypeId.value) ?? sendTypeOptions.value[0]!,
)

async function sendFrame(frame: Uint8Array, successMessage: string) {
  if (!props.client.isConnected) return
  isWriting.value = true
  writeError.value = ''
  writeInfo.value = ''
  try {
    const response = await props.client.sendRequest(frame, 800)
    if (response.isError) {
      writeError.value = t('toasts.modbusError', {
        code: response.errorCode?.toString(16) ?? '?',
      })
      return
    }
    writeInfo.value = successMessage
  } catch (e: any) {
    writeError.value = t('send.sendFailed', { message: e.message })
  } finally {
    isWriting.value = false
  }
}

async function sendBySelectedType() {
  const type = selectedSendType.value
  const value = Math.trunc(sendTypeValue.value)
  const success = t('send.sent', { label: type.label })
  if (type.kind === 'u16' && type.register !== undefined) {
    await sendFrame(
      buildWriteSingleRegister(props.deviceAddress, type.register, value & 0xFFFF),
      success,
    )
    return
  }
  if (type.kind === 's16' && type.register !== undefined) {
    await sendFrame(
      buildWriteSingleRegister(props.deviceAddress, type.register, toUnsigned16(value)),
      success,
    )
    return
  }
  if (type.kind === 's32' && type.startRegister !== undefined) {
    const [lo, hi] = fromSigned32(value)
    await sendFrame(
      buildWriteMultipleRegisters(props.deviceAddress, type.startRegister, [lo, hi]),
      success,
    )
    return
  }
  if (type.kind === 'custom79') {
    await sendFrame(
      buildCustomFunctionWrite(props.deviceAddress, 0x79, 0, value & 0xFFFF),
      success,
    )
  }
}

function parseHexByteTokens(hex: string): number[] {
  const normalized = hex
    .replace(/0x/gi, '')
    .replace(/[\r\n\t,;]+/g, ' ')
    .trim()
  if (!normalized) throw new Error(t('errors.emptyInput'))

  let tokens: string[]
  if (normalized.includes(' ')) {
    tokens = normalized.split(/\s+/).filter(Boolean)
  } else {
    if (normalized.length % 2 !== 0) throw new Error(t('errors.hexOddLength'))
    tokens = normalized.match(/.{1,2}/g) ?? []
  }

  const bytes = tokens.map((token) => {
    if (!/^[0-9a-fA-F]{1,2}$/.test(token)) throw new Error(t('send.invalidHex', { token }))
    return parseInt(token, 16)
  })
  if (bytes.length === 0) throw new Error(t('errors.noBytesParsed'))
  return bytes
}

async function sendManualRawHex() {
  if (!props.client.isConnected) return
  isWriting.value = true
  writeError.value = ''
  writeInfo.value = ''
  try {
    const bytes = parseHexByteTokens(manualRawHex.value)
    let frame = Uint8Array.from(bytes)

    if (manualAppendCrc.value) {
      const crc = calculateCRC(frame)
      const extended = new Uint8Array(frame.length + 2)
      extended.set(frame)
      extended[frame.length] = crc & 0xFF
      extended[frame.length + 1] = (crc >> 8) & 0xFF
      frame = extended
    }

    const timeoutMs = Math.max(20, Math.trunc(manualTimeoutMs.value || 800))
    const response = await props.client.sendRawFrame(frame, timeoutMs, manualWaitResponse.value)

    if (response?.isError) {
      writeError.value = t('errors.unexpectedResponse')
      return
    }

    if (manualWaitResponse.value) {
      writeInfo.value = t('send.rawSentWithResponse', {
        bytes: frame.length,
        code: response?.functionCode.toString(16) ?? '?',
      })
    } else {
      writeInfo.value = t('send.rawSentNoWait', { bytes: frame.length })
    }
  } catch (e: any) {
    writeError.value = t('send.sendFailed', { message: e.message })
  } finally {
    isWriting.value = false
  }
}
</script>

<template>
  <GroupBox :caption="t('panels.send')">
    <div class="flex flex-col gap-2 text-[12px] pt-0.5">
      <div class="flex items-center gap-1.5">
        <label class="shrink-0 w-16 text-gray-900 select-none">{{ t('send.paramType') }}</label>
        <select v-model.number="sendTypeId" class="flex-1 border border-gray-400 bg-white px-1 py-0.5 min-w-0 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-sans">
          <option v-for="option in sendTypeOptions" :key="option.id" :value="option.id">
            {{ option.label }}
          </option>
        </select>
      </div>

      <div class="flex items-center gap-1.5">
        <label class="shrink-0 w-16 text-gray-900 select-none">{{ t('send.paramData') }}</label>
        <input
          v-model.number="sendTypeValue"
          type="number"
          class="flex-1 border border-gray-400 bg-white px-1 py-0.5 min-w-0 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-mono"
        />
        <button
          type="button"
          class="px-3.5 py-0.5 border border-gray-500 bg-[#e1e1e1] shadow-sm active:shadow-inner hover:bg-[#d4d0c8] disabled:opacity-50 text-gray-900 font-medium select-none ml-1"
          :disabled="!client.isConnected || isWriting"
          @click="sendBySelectedType"
        >
          {{ t('send.send') }}
        </button>
      </div>

      <div class="text-[11px] text-gray-700 leading-snug pt-0.5">
        <span class="text-gray-900 font-medium select-none">{{ t('send.paramMeaning') }}</span> {{ selectedSendType.description }}
      </div>

      <div v-if="writeError" class="text-[11px] text-red-700 font-medium">{{ writeError }}</div>
      <div v-else-if="writeInfo" class="text-[11px] text-green-700 font-medium">{{ writeInfo }}</div>

      <button
        type="button"
        class="self-start text-[11px] text-gray-600 underline hover:text-gray-900 select-none pt-1"
        @click="showAdvanced = !showAdvanced"
      >
        {{ showAdvanced ? t('send.hideAdvanced') : t('send.showAdvanced') }}
      </button>

      <div v-if="showAdvanced" class="border-t border-gray-300 pt-1.5 flex flex-col gap-1">
        <textarea
          v-model="manualRawHex"
          class="border border-gray-400 bg-white px-1 py-0.5 font-mono text-[11px] min-h-14 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)]"
          :placeholder="t('send.rawPlaceholder')"
        />
        <div class="flex flex-wrap items-center gap-2 text-[11px]">
          <label class="flex items-center gap-1 select-none">
            <input v-model="manualAppendCrc" type="checkbox" class="accent-blue-600" />
            {{ t('send.appendCrc') }}
          </label>
          <label class="flex items-center gap-1 select-none">
            <input v-model="manualWaitResponse" type="checkbox" class="accent-blue-600" />
            {{ t('send.waitResponse') }}
          </label>
          <label class="flex items-center gap-1 select-none">
            {{ t('send.timeoutMs') }}
            <input v-model.number="manualTimeoutMs" type="number" min="20" class="w-14 border border-gray-400 px-0.5 bg-white shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-mono" />
          </label>
          <button
            type="button"
            class="px-2 py-0.5 border border-gray-500 bg-[#e1e1e1] shadow-sm active:shadow-inner hover:bg-[#d4d0c8] disabled:opacity-50 font-medium select-none ml-auto"
            :disabled="!client.isConnected || isWriting"
            @click="sendManualRawHex"
          >
            {{ t('send.sendRaw') }}
          </button>
        </div>
      </div>
    </div>
  </GroupBox>
</template>
