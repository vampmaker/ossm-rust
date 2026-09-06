<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { LoopStats, MotorState, TimingWindowStats } from '../types'

const props = defineProps<{
  state?: MotorState | null
  /** When false, show relay/unavailable hint instead of motor metrics. */
  motorTelemetryAvailable?: boolean
}>()

const { t } = useI18n()

const loop = computed(() => props.state?.loop_stats)

const motorUpdateRate = computed<number | null>(() => {
  if (typeof loop.value?.ups === 'number') {
    return loop.value.ups
  }
  const hist = loop.value?.update_history
  if (!hist || hist.length === 0) {
    return null
  }
  return hist[hist.length - 1] ?? null
})

const averageUpdateRate = computed(() => {
  const hist = loop.value?.update_history
  if (!hist || hist.length === 0) {
    return null
  }
  const sum = hist.reduce((a, b) => a + b, 0)
  return Math.round(sum / hist.length)
})

const linkStatsAvailable = computed(() => {
  const stats = props.state?.link_stats
  if (!stats) return false
  return stats.exchanges + stats.failures > 0
})

function timingRowVisible(w: TimingWindowStats | undefined): boolean {
  if (!w) return false
  return w.min > 0 || w.max > 0 || w.mean > 0
}
</script>

<template>
  <div class="rounded-lg border border-gray-200 bg-gray-50 p-4">
    <div
      v-if="motorTelemetryAvailable === false"
      class="text-sm text-gray-600"
    >
      {{ t('telemetry.relayUnavailable') }}
    </div>
    <template v-else>
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center space-x-2">
          <span
            class="inline-block h-3 w-3 rounded-full"
            :class="motorUpdateRate !== null && motorUpdateRate > 0 ? 'bg-green-500 animate-pulse' : 'bg-red-400'"
          />
          <span class="text-sm font-semibold text-gray-700">{{ t('telemetry.updateRateTitle') }}</span>
        </div>
        <div>
          <span v-if="motorUpdateRate !== null" class="font-mono text-xl font-bold text-blue-600">
            {{ motorUpdateRate }}
            <span class="text-xs font-normal text-gray-500">{{ t('telemetry.updatesPerSec') }}</span>
          </span>
          <span v-else class="text-xs font-medium text-gray-400">
            {{ t('telemetry.notAvailable') }}
          </span>
        </div>
      </div>
      <div
        v-if="loop && typeof loop.dt_avg_ms === 'number'"
        class="mt-2 flex flex-wrap items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-600"
      >
        <span>{{ t('telemetry.loopDt') }} <strong class="font-mono text-gray-800">{{ loop.dt_min_ms?.toFixed(1) }} / {{ loop.dt_avg_ms?.toFixed(1) }} / {{ loop.dt_max_ms?.toFixed(1) }} ms</strong></span>
        <span>{{ t('telemetry.mdev') }} <strong class="font-mono text-gray-800">{{ loop.dt_mdev_ms?.toFixed(2) }} ms</strong></span>
      </div>
      <div
        v-else-if="motorUpdateRate !== null && loop?.update_history"
        class="mt-2 flex items-center justify-between border-t border-gray-200 pt-2 text-xs text-gray-500"
      >
        <span>{{ t('telemetry.tenSecAvg') }} <strong class="font-mono text-gray-700">{{ averageUpdateRate }} Hz</strong></span>
      </div>

      <div
        v-if="linkStatsAvailable && state?.link_stats"
        class="mt-3 border-t border-gray-200 pt-3"
      >
        <div class="flex flex-wrap items-center justify-between text-xs mb-2">
          <span class="font-semibold text-gray-700">{{ t('telemetry.modbusTelemetry') }}</span>
          <span
            class="rounded-full px-2 py-0.5 font-bold"
            :class="state.link_stats.success_rate >= 98 ? 'bg-green-100 text-green-800' : 'bg-amber-100 text-amber-800'"
          >
            {{ t('telemetry.successRate', {
              rate: state.link_stats.success_rate.toFixed(1),
              ok: state.link_stats.exchanges - state.link_stats.failures,
              fail: state.link_stats.failures,
            }) }}
          </span>
        </div>
        <div class="overflow-x-auto">
          <table class="w-full text-left font-mono text-[11px] text-gray-700 border-collapse">
            <thead>
              <tr class="border-b border-gray-200 text-gray-500">
                <th class="py-1 pr-2">{{ t('telemetry.metricUs') }}</th>
                <th class="py-1 px-1">{{ t('telemetry.min') }}</th>
                <th class="py-1 px-1">{{ t('telemetry.p5') }}</th>
                <th class="py-1 px-1">{{ t('telemetry.p50') }}</th>
                <th class="py-1 px-1">{{ t('telemetry.p95') }}</th>
                <th class="py-1 px-1">{{ t('telemetry.max') }}</th>
                <th class="py-1 pl-1">{{ t('telemetry.meanMdev') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-gray-100">
              <tr v-if="timingRowVisible(state.link_stats.round_trip)">
                <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('telemetry.roundTrip') }}</td>
                <td class="py-1 px-1">{{ state.link_stats.round_trip.min }}</td>
                <td class="py-1 px-1">{{ state.link_stats.round_trip.pct5 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.round_trip.pct50 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.round_trip.pct95 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.round_trip.max }}</td>
                <td class="py-1 pl-1">{{ state.link_stats.round_trip.mean }}±{{ state.link_stats.round_trip.mdev }}</td>
              </tr>
              <tr v-if="timingRowVisible(state.link_stats.slave_latency)">
                <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('telemetry.slaveLatency') }}</td>
                <td class="py-1 px-1">{{ state.link_stats.slave_latency.min }}</td>
                <td class="py-1 px-1">{{ state.link_stats.slave_latency.pct5 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.slave_latency.pct50 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.slave_latency.pct95 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.slave_latency.max }}</td>
                <td class="py-1 pl-1">{{ state.link_stats.slave_latency.mean }}±{{ state.link_stats.slave_latency.mdev }}</td>
              </tr>
              <tr v-if="timingRowVisible(state.link_stats.rx_duration)">
                <td class="py-1 pr-2 font-semibold text-gray-800">{{ t('telemetry.rxDuration') }}</td>
                <td class="py-1 px-1">{{ state.link_stats.rx_duration.min }}</td>
                <td class="py-1 px-1">{{ state.link_stats.rx_duration.pct5 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.rx_duration.pct50 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.rx_duration.pct95 }}</td>
                <td class="py-1 px-1">{{ state.link_stats.rx_duration.max }}</td>
                <td class="py-1 pl-1">{{ state.link_stats.rx_duration.mean }}±{{ state.link_stats.rx_duration.mdev }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </template>
  </div>
</template>
