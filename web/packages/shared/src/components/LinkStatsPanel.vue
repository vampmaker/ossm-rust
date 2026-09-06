<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { LinkStats } from '../types'

defineProps<{
  stats: LinkStats | null
}>()

const { t } = useI18n()
</script>

<template>
  <div
    id="link-stats-panel"
    class="rounded-lg border border-gray-200 bg-gray-50 p-4 space-y-2"
  >
    <h3 class="text-sm font-bold text-gray-800">{{ t('linkStats.title') }}</h3>
    <p v-if="!stats" class="text-sm text-gray-500">{{ t('linkStats.loading') }}</p>
    <dl
      v-else
      class="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1 text-xs font-mono text-gray-700"
    >
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.transport') }}</dt>
        <dd class="inline break-all"> {{ stats.transport || '—' }}</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.connected') }}</dt>
        <dd class="inline"> {{ stats.connected }}</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.exchangesPerSec') }}</dt>
        <dd class="inline"> {{ stats.exchanges_per_sec }}</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.exchanges') }}</dt>
        <dd class="inline"> {{ stats.exchanges }}</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.failures') }}</dt>
        <dd class="inline"> {{ stats.failures }}</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.successRate') }}</dt>
        <dd class="inline"> {{ stats.success_rate.toFixed(1) }}%</dd>
      </div>
      <div>
        <dt class="inline text-gray-500">{{ t('linkStats.bytes') }}</dt>
        <dd class="inline"> {{ stats.bytes_tx }} / {{ stats.bytes_rx }}</dd>
      </div>
    </dl>
  </div>
</template>
