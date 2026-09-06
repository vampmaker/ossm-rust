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
  <div class="overflow-hidden rounded-2xl bg-white p-6 shadow-sm border border-slate-200/80 hover:shadow-md transition-all">
    <!-- Header -->
    <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
      <h2 class="text-xl font-extrabold text-slate-900 tracking-tight">{{ t('diagram.title') }}</h2>
      <div class="flex items-center space-x-2">
        <span
          class="inline-flex items-center rounded-full px-3 py-1 text-xs font-bold shadow-2xs border"
          :class="connected ? 'bg-blue-50 text-blue-700 border-blue-200' : 'bg-slate-100 text-slate-600 border-slate-200'"
        >
          {{ config.depth_top ? t('diagram.depthFromTop') : t('diagram.depthFromBottom') }}
        </span>
        <span
          v-if="config.reversed"
          class="inline-flex items-center rounded-full bg-amber-50 px-3 py-1 text-xs font-bold text-amber-800 border border-amber-200 shadow-2xs"
        >
          {{ t('diagram.reversed') }}
        </span>
      </div>
    </div>

    <!-- Main Linear Stage Track -->
    <div class="relative my-8">
      <!-- Ruler top ticks -->
      <div class="mb-1.5 flex justify-between text-[11px] font-mono font-bold text-slate-400">
        <span>0%</span>
        <span>25%</span>
        <span>50%</span>
        <span>75%</span>
        <span>100%</span>
      </div>

      <!-- Full Range Track (Light theme track) -->
      <div class="relative h-12 w-full rounded-xl bg-slate-100 shadow-inner border border-slate-200">
        <!-- Tick grid lines -->
        <div class="absolute inset-y-0 left-1/4 w-px bg-slate-200 pointer-events-none" />
        <div class="absolute inset-y-0 left-1/2 -translate-x-1/2 w-px bg-slate-300 pointer-events-none" />
        <div class="absolute inset-y-0 left-3/4 w-px bg-slate-200 pointer-events-none" />

        <!-- Active Stroke Window (Left Limit to Right Limit) -->
        <div
          class="absolute top-1.5 bottom-1.5 rounded-lg bg-gradient-to-r from-blue-500 via-indigo-500 to-violet-500 shadow-xs transition-all duration-150 border border-indigo-400/40"
          :style="{
            left: `${leftLimitPct}%`,
            width: `${strokeWindowWidthPct}%`,
          }"
        >
          <!-- Active window subtle pattern line -->
          <div class="h-full w-full opacity-20 bg-[radial-gradient(#fff_1px,transparent_1px)] [background-size:8px_8px]" />
        </div>

        <!-- Left Limit Marker -->
        <div
          class="absolute top-0 bottom-0 z-10 flex flex-col items-center transition-all duration-150 pointer-events-none"
          :style="{ left: `${leftLimitPct}%`, transform: 'translateX(-50%)' }"
        >
          <div class="h-full w-0.5 bg-blue-600 shadow-xs" />
          <span
            class="absolute -bottom-7 whitespace-nowrap inline-flex items-center justify-center h-5 px-2 text-[10px] font-mono font-bold text-white rounded-md bg-blue-600 border border-blue-500 shadow-xs leading-none transition-all duration-150"
            :style="{
              left: leftLimitPct < 20 ? '1px' : leftLimitPct > 80 ? '-1px' : '50%',
              transform: leftLimitPct < 20 ? 'translateX(0)' : leftLimitPct > 80 ? 'translateX(-100%)' : 'translateX(-50%)'
            }"
          >
            <span class="inline-block translate-y-[1.5px]">{{ t('diagram.left', { pct: leftLimitPct.toFixed(0) }) }}</span>
          </span>
        </div>

        <!-- Right Limit Marker -->
        <div
          class="absolute top-0 bottom-0 z-10 flex flex-col items-center transition-all duration-150 pointer-events-none"
          :style="{ left: `${rightLimitPct}%`, transform: 'translateX(-50%)' }"
        >
          <div class="h-full w-0.5 bg-indigo-600 shadow-xs" />
          <span
            class="absolute -bottom-7 whitespace-nowrap inline-flex items-center justify-center h-5 px-2 text-[10px] font-mono font-bold text-white rounded-md bg-indigo-600 border border-indigo-500 shadow-xs leading-none transition-all duration-150"
            :style="{
              left: rightLimitPct > 80 ? '-1px' : rightLimitPct < 20 ? '1px' : '50%',
              transform: rightLimitPct > 80 ? 'translateX(-100%)' : rightLimitPct < 20 ? 'translateX(0)' : 'translateX(-50%)'
            }"
          >
            <span class="inline-block translate-y-[1.5px]">{{ t('diagram.right', { pct: rightLimitPct.toFixed(0) }) }}</span>
          </span>
        </div>

        <!-- Current Position Puck -->
        <div
          class="absolute top-1/2 -translate-x-1/2 -translate-y-1/2 z-20 transition-all duration-100 ease-linear pointer-events-none"
          :style="{ left: `${currentPosPct}%` }"
        >
          <!-- Floating position value badge above puck -->
          <div
            class="absolute -top-9 whitespace-nowrap inline-flex items-center justify-center h-6 px-2.5 text-[11px] font-mono font-extrabold text-white rounded-full bg-slate-800 border border-slate-700 shadow-md leading-none transition-all duration-150"
            :style="{
              left: '50%',
              transform: currentPosPct < 15 ? 'translateX(0)' : currentPosPct > 85 ? 'translateX(-100%)' : 'translateX(-50%)'
            }"
          >
            <span class="inline-block translate-y-[1.5px]">{{ currentPosPct.toFixed(1) }}%</span>
          </div>

          <!-- Outer glowing ring -->
          <div class="relative flex h-8 w-8 items-center justify-center rounded-full border-2 border-white bg-blue-600 shadow-md shadow-blue-500/40">
            <div class="h-2.5 w-2.5 rounded-full bg-white shadow-xs" />
          </div>
        </div>
      </div>

      <!-- Rail bottom end labels -->
      <div class="mt-8 flex justify-between text-xs font-semibold text-slate-500">
        <div>
          <span>{{ t('diagram.fullRetraction') }}</span>
          <span v-if="hasPhysicalRange" class="block font-mono text-[11px] text-slate-400">
            {{ t('diagram.units', { value: posMin?.toFixed(1) }) }}
          </span>
        </div>
        <div class="text-right">
          <span>{{ t('diagram.fullExtension') }}</span>
          <span v-if="hasPhysicalRange" class="block font-mono text-[11px] text-slate-400">
            {{ t('diagram.units', { value: posMax?.toFixed(1) }) }}
          </span>
        </div>
      </div>
    </div>

    <!-- Summary Statistics Grid -->
    <div class="mt-4 grid grid-cols-2 gap-3 border-t border-slate-100 pt-5 md:grid-cols-4">
      <div class="rounded-xl bg-slate-50/80 border border-slate-200/80 p-3 text-center shadow-2xs flex flex-col items-center justify-center gap-1 min-h-[80px]">
        <div class="text-xs font-bold text-slate-600 leading-none">{{ t('diagram.fullRange') }}</div>
        <div class="font-mono text-base font-extrabold text-slate-800 leading-none translate-y-[1.5px]">{{ t('diagram.fullRangeValue') }}</div>
        <div v-if="hasPhysicalRange" class="text-[11px] font-mono text-slate-400 leading-none translate-y-[1px]">
          {{ posMin?.toFixed(1) }} to {{ posMax?.toFixed(1) }}
        </div>
      </div>

      <div class="rounded-xl bg-blue-50/70 border border-blue-200/80 p-3 text-center shadow-2xs flex flex-col items-center justify-center gap-1 min-h-[80px]">
        <div class="text-xs font-bold text-blue-700 leading-none">{{ t('diagram.leftLimit') }}</div>
        <div class="font-mono text-base font-extrabold text-blue-900 leading-none translate-y-[1.5px]">{{ leftLimitPct.toFixed(1) }}%</div>
        <div class="text-[11px] font-semibold text-blue-600/90 leading-none">{{ t('diagram.strokeStart') }}</div>
      </div>

      <div class="rounded-xl bg-indigo-50/70 border border-indigo-200/80 p-3 text-center shadow-2xs flex flex-col items-center justify-center gap-1 min-h-[80px]">
        <div class="text-xs font-bold text-indigo-700 leading-none">{{ t('diagram.rightLimit') }}</div>
        <div class="font-mono text-base font-extrabold text-indigo-900 leading-none translate-y-[1.5px]">{{ rightLimitPct.toFixed(1) }}%</div>
        <div class="text-[11px] font-semibold text-indigo-600/90 leading-none">{{ t('diagram.strokeEnd') }}</div>
      </div>

      <div class="rounded-xl bg-cyan-50/70 border border-cyan-200/80 p-3 text-center shadow-2xs flex flex-col items-center justify-center gap-1 min-h-[80px]">
        <div class="text-xs font-bold text-cyan-700 leading-none">{{ t('diagram.currentPosition') }}</div>
        <div class="font-mono text-base font-extrabold text-cyan-900 leading-none translate-y-[1.5px]">{{ currentPosPct.toFixed(1) }}%</div>
        <div v-if="physicalPos !== null" class="text-[11px] font-mono text-cyan-600/90 leading-none translate-y-[1px]">
          {{ t('diagram.units', { value: physicalPos.toFixed(1) }) }}
        </div>
        <div v-else class="text-[11px] font-semibold text-cyan-600/90 leading-none">{{ t('diagram.livePosition') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped></style>
