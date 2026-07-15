<script setup lang="ts">
import { computed, ref } from 'vue'
import { toUnsigned16, fromSigned32 } from '../lib/registers'
import { buildCustomFunctionWrite, buildWriteMultipleRegisters, buildWriteSingleRegister, calculateCRC } from '../lib/modbus-rtu'
import type { ModbusTransport } from '../lib/transport'
import { sendTypeOptions } from '../lib/send-options'
import GroupBox from './GroupBox.vue'

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

const selectedSendType = computed(
  () => sendTypeOptions.find((item) => item.id === sendTypeId.value) ?? sendTypeOptions[0]!,
)

async function sendFrame(frame: Uint8Array, successMessage: string) {
  if (!props.client.isConnected) return
  isWriting.value = true
  writeError.value = ''
  writeInfo.value = ''
  try {
    const response = await props.client.sendRequest(frame, 800)
    if (response.isError) {
      writeError.value = `发送失败: 0x${response.errorCode?.toString(16)}`
      return
    }
    writeInfo.value = successMessage
  } catch (e: any) {
    writeError.value = `发送失败: ${e.message}`
  } finally {
    isWriting.value = false
  }
}

async function sendBySelectedType() {
  const type = selectedSendType.value
  const value = Math.trunc(sendTypeValue.value)
  if (type.kind === 'u16' && type.register !== undefined) {
    await sendFrame(
      buildWriteSingleRegister(props.deviceAddress, type.register, value & 0xFFFF),
      `已发送 ${type.label}`,
    )
    return
  }
  if (type.kind === 's16' && type.register !== undefined) {
    await sendFrame(
      buildWriteSingleRegister(props.deviceAddress, type.register, toUnsigned16(value)),
      `已发送 ${type.label}`,
    )
    return
  }
  if (type.kind === 's32' && type.startRegister !== undefined) {
    const [lo, hi] = fromSigned32(value)
    await sendFrame(
      buildWriteMultipleRegisters(props.deviceAddress, type.startRegister, [lo, hi]),
      `已发送 ${type.label}`,
    )
    return
  }
  if (type.kind === 'custom79') {
    await sendFrame(
      buildCustomFunctionWrite(props.deviceAddress, 0x79, 0, value & 0xFFFF),
      `已发送 ${type.label}`,
    )
  }
}

function parseHexByteTokens(hex: string): number[] {
  const normalized = hex
    .replace(/0x/gi, '')
    .replace(/[\r\n\t,;]+/g, ' ')
    .trim()
  if (!normalized) throw new Error('输入为空')

  let tokens: string[]
  if (normalized.includes(' ')) {
    tokens = normalized.split(/\s+/).filter(Boolean)
  } else {
    if (normalized.length % 2 !== 0) throw new Error('无空格时十六进制长度须为偶数')
    tokens = normalized.match(/.{1,2}/g) ?? []
  }

  const bytes = tokens.map((token) => {
    if (!/^[0-9a-fA-F]{1,2}$/.test(token)) throw new Error(`无效十六进制: ${token}`)
    return parseInt(token, 16)
  })
  if (bytes.length === 0) throw new Error('未解析到字节')
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
      writeError.value = `异常响应: 0x${response.errorCode?.toString(16)}`
      return
    }

    if (manualWaitResponse.value) {
      writeInfo.value = `已发送 ${frame.length} 字节, 功能码=0x${response?.functionCode.toString(16)}`
    } else {
      writeInfo.value = `已发送 ${frame.length} 字节 (不等待响应)`
    }
  } catch (e: any) {
    writeError.value = `发送失败: ${e.message}`
  } finally {
    isWriting.value = false
  }
}
</script>

<template>
  <GroupBox caption="modbus发送">
    <div class="flex flex-col gap-2 text-[12px] pt-0.5">
      <div class="flex items-center gap-1.5">
        <label class="shrink-0 w-16 text-gray-900 select-none">参数类型:</label>
        <select v-model.number="sendTypeId" class="flex-1 border border-gray-400 bg-white px-1 py-0.5 min-w-0 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-sans">
          <option v-for="option in sendTypeOptions" :key="option.id" :value="option.id">
            {{ option.label }}
          </option>
        </select>
      </div>

      <div class="flex items-center gap-1.5">
        <label class="shrink-0 w-16 text-gray-900 select-none">参数数据:</label>
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
          发送
        </button>
      </div>

      <div class="text-[11px] text-gray-700 leading-snug pt-0.5">
        <span class="text-gray-900 font-medium select-none">参数含义:</span> {{ selectedSendType.description }}
      </div>

      <div v-if="writeError" class="text-[11px] text-red-700 font-medium">{{ writeError }}</div>
      <div v-else-if="writeInfo" class="text-[11px] text-green-700 font-medium">{{ writeInfo }}</div>

      <button
        type="button"
        class="self-start text-[11px] text-gray-600 underline hover:text-gray-900 select-none pt-1"
        @click="showAdvanced = !showAdvanced"
      >
        {{ showAdvanced ? '隐藏高级' : '高级 (原始帧发送)' }}
      </button>

      <div v-if="showAdvanced" class="border-t border-gray-300 pt-1.5 flex flex-col gap-1">
        <textarea
          v-model="manualRawHex"
          class="border border-gray-400 bg-white px-1 py-0.5 font-mono text-[11px] min-h-14 shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)]"
          placeholder="例: 01 03 00 00 00 1A C4 0B"
        />
        <div class="flex flex-wrap items-center gap-2 text-[11px]">
          <label class="flex items-center gap-1 select-none">
            <input v-model="manualAppendCrc" type="checkbox" class="accent-blue-600" />
            追加 CRC16
          </label>
          <label class="flex items-center gap-1 select-none">
            <input v-model="manualWaitResponse" type="checkbox" class="accent-blue-600" />
            等待响应
          </label>
          <label class="flex items-center gap-1 select-none">
            超时(ms):
            <input v-model.number="manualTimeoutMs" type="number" min="20" class="w-14 border border-gray-400 px-0.5 bg-white shadow-[inset_1px_1px_2px_rgba(0,0,0,0.15)] font-mono" />
          </label>
          <button
            type="button"
            class="px-2 py-0.5 border border-gray-500 bg-[#e1e1e1] shadow-sm active:shadow-inner hover:bg-[#d4d0c8] disabled:opacity-50 font-medium select-none ml-auto"
            :disabled="!client.isConnected || isWriting"
            @click="sendManualRawHex"
          >
            发送原始帧
          </button>
        </div>
      </div>
    </div>
  </GroupBox>
</template>

