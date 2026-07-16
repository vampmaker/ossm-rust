<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { MotorControllerConfig, MotorState } from '../types'

const props = defineProps<{
  config: MotorControllerConfig
  state?: MotorState | null
  connected: boolean
}>()

const { t } = useI18n()

// Calculate Left Limit and Right Limit percentages (0 to 100)
const leftLimitPct = computed(() => {
  if (props.config.depth_top) {
    return 0
  }
  return Math.max(0, Math.min(100, (1 - props.config.depth) * 100))
})

const rightLimitPct = computed(() => {
  if (props.config.depth_top) {
    return Math.max(0, Math.min(100, props.config.depth * 100))
  }
  return 100
})

const strokeWindowWidthPct = computed(() => {
  return Math.max(0, rightLimitPct.value - leftLimitPct.value)
})

// Calculate Current Position percentage along the full 0..100% range
const currentPosPct = computed(() => {
  if (props.state && typeof props.state.shaped_y === 'number') {
    return Math.max(0, Math.min(100, props.state.shaped_y * 100))
  }
  // Fallback when paused or before receiving state
  if (props.config.paused) {
    const shaped = props.config.depth_top
      ? props.config.paused_position * props.config.depth
      : props.config.paused_position * props.config.depth + (1 - props.config.depth)
    return Math.max(0, Math.min(100, shaped * 100))
  }
  return (leftLimitPct.value + rightLimitPct.value) / 2
})

// Physical limits if reported by state
const posMin = computed(() => props.state?.pos_min)
const posMax = computed(() => props.state?.pos_max)
const hasPhysicalRange = computed(() => {
  return typeof posMin.value === 'number' &&
    typeof posMax.value === 'number' &&
    Math.abs(posMax.value! - posMin.value!) > 0.001
})

const physicalPos = computed(() => {
  if (typeof props.state?.position === 'number') {
    return props.state.position
  }
  if (hasPhysicalRange.value) {
    return posMin.value! + (currentPosPct.value / 100) * (posMax.value! - posMin.value!)
  }
  return null
})
</script>

<template>
  <div class="overflow-hidden rounded-lg bg-white p-5 shadow-md">
    <!-- Header -->
    <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
      <div>
        <h2 class="text-xl font-bold text-gray-800">{{ t('diagram.title') }}</h2>
        <p class="text-xs text-gray-500">
          {{ t('diagram.subtitle') }}
        </p>
      </div>
      <div class="flex items-center space-x-2">
        <span
          class="inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-semibold"
          :class="connected ? 'bg-blue-100 text-blue-800' : 'bg-gray-100 text-gray-600'"
        >
          {{ config.depth_top ? t('diagram.depthFromTop') : t('diagram.depthFromBottom') }}
        </span>
        <span
          v-if="config.reversed"
          class="inline-flex items-center rounded-full bg-amber-100 px-2.5 py-0.5 text-xs font-semibold text-amber-800"
        >
          {{ t('diagram.reversed') }}
        </span>
      </div>
    </div>

    <!-- Main Linear Stage Track -->
    <div class="relative my-8 px-4">
      <!-- Ruler top ticks -->
      <div class="mb-1 flex justify-between text-[11px] font-mono text-gray-400">
        <span>0%</span>
        <span>25%</span>
        <span>50%</span>
        <span>75%</span>
        <span>100%</span>
      </div>

      <!-- Full Range Track -->
      <div class="relative h-12 w-full rounded-lg bg-gray-200 shadow-inner">
        <!-- Tick grid lines -->
        <div class="absolute inset-0 flex justify-between px-2">
          <div class="h-full w-px bg-gray-300/60" />
          <div class="h-full w-px bg-gray-300/60" />
          <div class="h-full w-px bg-gray-300/60" />
          <div class="h-full w-px bg-gray-300/60" />
          <div class="h-full w-px bg-gray-300/60" />
        </div>

        <!-- Active Stroke Window (Left Limit to Right Limit) -->
        <div
          class="absolute top-1.5 bottom-1.5 rounded bg-gradient-to-r from-blue-500 to-indigo-600 shadow transition-all duration-150"
          :style="{
            left: `${leftLimitPct}%`,
            width: `${strokeWindowWidthPct}%`,
          }"
        >
          <!-- Active window texture/pattern line -->
          <div class="h-full w-full opacity-20 bg-[radial-gradient(#fff_1px,transparent_1px)] [background-size:8px_8px]" />
        </div>

        <!-- Left Limit Marker -->
        <div
          class="absolute top-0 bottom-0 z-10 flex flex-col items-center transition-all duration-150"
          :style="{ left: `${leftLimitPct}%` }"
        >
          <div class="h-full w-0.5 bg-blue-700 shadow" />
          <span class="absolute -bottom-6 whitespace-nowrap rounded bg-blue-700 px-1.5 py-0.5 text-[10px] font-bold text-white shadow">
            {{ t('diagram.left', { pct: leftLimitPct.toFixed(0) }) }}
          </span>
        </div>

        <!-- Right Limit Marker -->
        <div
          class="absolute top-0 bottom-0 z-10 flex flex-col items-center transition-all duration-150"
          :style="{ left: `${rightLimitPct}%` }"
        >
          <div class="h-full w-0.5 bg-indigo-700 shadow" />
          <span class="absolute -bottom-6 whitespace-nowrap rounded bg-indigo-700 px-1.5 py-0.5 text-[10px] font-bold text-white shadow">
            {{ t('diagram.right', { pct: rightLimitPct.toFixed(0) }) }}
          </span>
        </div>

        <!-- Current Position Puck -->
        <div
          class="absolute top-1/2 -translate-x-1/2 -translate-y-1/2 z-20 transition-all duration-100 ease-linear"
          :style="{ left: `${currentPosPct}%` }"
        >
          <!-- Floating position value badge above puck -->
          <div class="absolute -top-9 left-1/2 -translate-x-1/2 whitespace-nowrap rounded-full bg-gray-900 px-2 py-0.5 text-[11px] font-mono font-bold text-white shadow-lg">
            {{ currentPosPct.toFixed(1) }}%
          </div>

          <!-- Outer glowing ring -->
          <div class="relative flex h-8 w-8 items-center justify-center rounded-full border-2 border-white bg-cyan-400 shadow-md">
            <div class="h-2.5 w-2.5 rounded-full bg-white shadow-sm" />
          </div>
        </div>
      </div>

      <!-- Rail bottom end labels -->
      <div class="mt-8 flex justify-between text-xs font-semibold text-gray-500">
        <div>
          <span>{{ t('diagram.fullRetraction') }}</span>
          <span v-if="hasPhysicalRange" class="block font-mono text-[11px] text-gray-400">
            {{ t('diagram.units', { value: posMin?.toFixed(1) }) }}
          </span>
        </div>
        <div class="text-right">
          <span>{{ t('diagram.fullExtension') }}</span>
          <span v-if="hasPhysicalRange" class="block font-mono text-[11px] text-gray-400">
            {{ t('diagram.units', { value: posMax?.toFixed(1) }) }}
          </span>
        </div>
      </div>
    </div>

    <!-- Summary Statistics Grid -->
    <div class="mt-4 grid grid-cols-2 gap-3 border-t border-gray-100 pt-4 md:grid-cols-4">
      <div class="rounded-lg bg-gray-50 p-2.5 text-center">
        <div class="text-xs font-medium text-gray-500">{{ t('diagram.fullRange') }}</div>
        <div class="mt-0.5 font-mono text-base font-bold text-gray-800">{{ t('diagram.fullRangeValue') }}</div>
        <div v-if="hasPhysicalRange" class="text-[11px] font-mono text-gray-400">
          {{ posMin?.toFixed(1) }} to {{ posMax?.toFixed(1) }}
        </div>
      </div>

      <div class="rounded-lg bg-blue-50/70 p-2.5 text-center">
        <div class="text-xs font-medium text-blue-700">{{ t('diagram.leftLimit') }}</div>
        <div class="mt-0.5 font-mono text-base font-bold text-blue-900">{{ leftLimitPct.toFixed(1) }}%</div>
        <div class="text-[11px] text-blue-600/80">{{ t('diagram.strokeStart') }}</div>
      </div>

      <div class="rounded-lg bg-indigo-50/70 p-2.5 text-center">
        <div class="text-xs font-medium text-indigo-700">{{ t('diagram.rightLimit') }}</div>
        <div class="mt-0.5 font-mono text-base font-bold text-indigo-900">{{ rightLimitPct.toFixed(1) }}%</div>
        <div class="text-[11px] text-indigo-600/80">{{ t('diagram.strokeEnd') }}</div>
      </div>

      <div class="rounded-lg bg-cyan-50/70 p-2.5 text-center">
        <div class="text-xs font-medium text-cyan-700">{{ t('diagram.currentPosition') }}</div>
        <div class="mt-0.5 font-mono text-base font-bold text-cyan-900">{{ currentPosPct.toFixed(1) }}%</div>
        <div v-if="physicalPos !== null" class="text-[11px] font-mono text-cyan-600/80">
          {{ t('diagram.units', { value: physicalPos.toFixed(1) }) }}
        </div>
        <div v-else class="text-[11px] text-cyan-600/80">{{ t('diagram.livePosition') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped></style>
