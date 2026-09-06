<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { LinkStatsPanel, MotorTelemetryPanel } from '@ossm/shared'
import type { LinkStats, MotorControllerConfig, MotorState, StdRuntimeConfig } from '@ossm/shared'
import { getLinkStats, getRuntimeConfig } from '@ossm/client'

defineProps<{
  state: MotorState | null
  config: MotorControllerConfig
}>()

const { t } = useI18n()
const runtime = ref<StdRuntimeConfig | null>(null)
const runtimeError = ref<string | null>(null)
const linkStats = ref<LinkStats | null>(null)
let linkTimer: ReturnType<typeof setInterval> | null = null

async function refreshLinkStats() {
  try {
    linkStats.value = await getLinkStats()
  } catch {
    /* ignore transient errors */
  }
}

onMounted(() => {
  void (async () => {
    try {
      runtime.value = await getRuntimeConfig()
    } catch (e) {
      runtimeError.value = e instanceof Error ? e.message : String(e)
    }
  })()
  void refreshLinkStats()
  linkTimer = setInterval(() => void refreshLinkStats(), 1000)
})

onUnmounted(() => {
  if (linkTimer) clearInterval(linkTimer)
})

function row(label: string, value: string | number | boolean | null | undefined) {
  if (value === null || value === undefined || value === '') return null
  return { label, value: String(value) }
}
</script>

<template>
  <section
    id="std-details-panel"
    class="space-y-4 bg-white p-6 shadow-lg rounded-lg border border-gray-200"
  >
    <div class="border-b border-gray-200 pb-3">
      <h2 class="text-xl font-bold text-gray-800">{{ t('details.title') }}</h2>
      <p class="text-sm text-gray-500 mt-1">{{ t('details.subtitle') }}</p>
    </div>

    <MotorTelemetryPanel :state="state" />
    <LinkStatsPanel :stats="linkStats" />

    <div class="rounded-lg border border-gray-200 bg-gray-50 p-4 space-y-2">
      <h3 class="text-sm font-bold text-gray-800">{{ t('details.motorConfig') }}</h3>
      <dl class="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1 text-xs font-mono text-gray-700">
        <div><dt class="inline text-gray-500">bpm</dt> <dd class="inline">{{ config.bpm }}</dd></div>
        <div><dt class="inline text-gray-500">depth</dt> <dd class="inline">{{ config.depth }}</dd></div>
        <div><dt class="inline text-gray-500">wave</dt> <dd class="inline">{{ config.wave_func }}</dd></div>
        <div><dt class="inline text-gray-500">version</dt> <dd class="inline">{{ config.version ?? 0 }}</dd></div>
        <div><dt class="inline text-gray-500">paused</dt> <dd class="inline">{{ config.paused }}</dd></div>
      </dl>
    </div>

    <div class="rounded-lg border border-gray-200 bg-gray-50 p-4 space-y-2">
      <h3 class="text-sm font-bold text-gray-800">{{ t('details.runtimeConfig') }}</h3>
      <p v-if="runtimeError" class="text-sm text-rose-600">{{ runtimeError }}</p>
      <dl
        v-else-if="runtime"
        class="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1 text-xs font-mono text-gray-700"
      >
        <template
          v-for="item in [
            row(t('details.mode'), runtime.mode),
            row(t('details.mock'), runtime.mock),
            row(t('details.transport'), runtime.transport),
            row(t('details.serial'), runtime.serial),
            row(t('details.baud'), runtime.baud),
            row(t('details.slaveId'), runtime.slave_id),
            row(t('details.bind'), runtime.bind),
            row(t('details.modbusBind'), runtime.modbus_bind),
            row(t('details.relayTcp'), runtime.relay_tcp),
            row(t('details.relayWs'), runtime.relay_ws),
            row(t('details.rs485Ws'), runtime.rs485_ws),
            row(t('details.configPath'), runtime.config_path),
            row(t('details.staticDir'), runtime.static_dir),
            row(t('details.repl'), runtime.repl),
            row(t('details.noRepl'), runtime.no_repl),
          ].filter(Boolean)"
          :key="item!.label"
        >
          <div>
            <dt class="inline text-gray-500">{{ item!.label }}</dt>
            <dd class="inline break-all"> {{ item!.value }}</dd>
          </div>
        </template>
      </dl>
      <p v-else class="text-sm text-gray-500">{{ t('details.loading') }}</p>
      <p class="text-xs text-amber-700 pt-2">{{ t('details.readOnlyHint') }}</p>
    </div>
  </section>
</template>
