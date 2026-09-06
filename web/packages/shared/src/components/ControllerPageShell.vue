<script setup lang="ts">
import { Activity, RotateCcw, Settings } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import type { MotorControllerConfig, MotorState, PauseMode } from '../types'
import MotionWorkbench from './MotionWorkbench.vue'

export interface ControllerShellCapabilities {
  locale?: boolean
  diagram?: boolean
  details?: boolean
  restart?: boolean
}

const config = defineModel<MotorControllerConfig>('config', { required: true })
const pauseMode = defineModel<PauseMode>('pauseMode', { required: true })
const showDiagram = defineModel<boolean>('showDiagram', { default: true })
const showDetails = defineModel<boolean>('showDetails', { default: false })

const props = withDefaults(
  defineProps<{
    title: string
    badge?: string
    state: MotorState | null
    connected: boolean
    motorReady: boolean
    error?: string | null
    isMutating?: boolean
    hideMotion?: boolean
    capabilities?: ControllerShellCapabilities
    connectedLabel?: string
    connectingLabel?: string
    disconnectedLabel?: string
    motorReadyLabel?: string
  }>(),
  {
    capabilities: () => ({
      locale: true,
      diagram: true,
      details: false,
      restart: false,
    }),
  },
)

const emit = defineEmits<{
  retry: []
  restart: []
  locale: [locale: 'en' | 'zh']
  postedConfig: [config: MotorControllerConfig]
  macroConfig: [config: MotorControllerConfig]
  setPaused: [paused: boolean]
  setPausedPosition: [position: number]
}>()

const { t, locale } = useI18n()

const caps = props.capabilities

function statusText(): string {
  if (!props.connected) return props.disconnectedLabel ?? t('shell.disconnected')
  if (props.motorReady) return props.motorReadyLabel ?? props.connectedLabel ?? t('shell.connected')
  return props.connectingLabel ?? t('shell.connecting')
}
</script>

<template>
  <div class="min-h-screen bg-slate-100/70 text-slate-800">
    <div class="container mx-auto max-w-2xl md:max-w-3xl p-4 md:p-6">
      <header class="mb-5 flex flex-wrap items-center justify-between gap-3 bg-white/80 backdrop-blur-md rounded-2xl border border-slate-200/80 shadow-xs px-5 py-3.5">
        <div class="flex items-center gap-2.5">
          <h1 class="text-xl font-extrabold text-slate-900 tracking-tight">
            {{ title }}
          </h1>
          <span
            v-if="badge"
            class="rounded-full bg-blue-50 px-2.5 py-0.5 text-xs font-bold text-blue-700 border border-blue-200 shadow-2xs"
          >
            {{ badge }}
          </span>
        </div>
        <div class="flex items-center space-x-2.5">
          <slot name="header-actions" />
          <div
            v-if="caps.locale !== false"
            class="flex rounded-lg bg-slate-200/80 p-0.5 text-xs font-semibold"
          >
            <button
              type="button"
              class="rounded-md px-2.5 py-1 transition-all cursor-pointer"
              :class="locale === 'en' ? 'bg-white shadow-xs text-blue-600 font-bold' : 'text-slate-600 hover:text-slate-900'"
              @click="emit('locale', 'en')"
            >
              {{ t('locale.en') }}
            </button>
            <button
              type="button"
              class="rounded-md px-2.5 py-1 transition-all cursor-pointer"
              :class="locale === 'zh' ? 'bg-white shadow-xs text-blue-600 font-bold' : 'text-slate-600 hover:text-slate-900'"
              @click="emit('locale', 'zh')"
            >
              {{ t('locale.zh') }}
            </button>
          </div>
          <button
            v-if="caps.diagram !== false"
            type="button"
            class="rounded-xl p-2 transition-all cursor-pointer disabled:opacity-50"
            :class="showDiagram ? 'bg-blue-600 text-white shadow-xs' : 'bg-slate-100 text-slate-600 hover:bg-slate-200 border border-slate-200/80'"
            :title="showDiagram ? t('shell.hideDiagram') : t('shell.showDiagram')"
            :aria-label="t('shell.toggleDiagram')"
            @click="showDiagram = !showDiagram"
          >
            <Activity :size="17" :stroke-width="2.2" />
          </button>
          <button
            v-if="caps.details"
            type="button"
            class="rounded-xl p-2 transition-all cursor-pointer disabled:opacity-50"
            :class="showDetails ? 'bg-blue-600 text-white shadow-xs' : 'bg-slate-100 text-slate-600 hover:bg-slate-200 border border-slate-200/80'"
            :title="showDetails ? t('shell.hideDetails') : t('shell.showDetails')"
            :aria-label="t('shell.toggleDetails')"
            @click="showDetails = !showDetails"
          >
            <Settings :size="17" :stroke-width="2.2" />
          </button>
          <button
            v-if="caps.restart"
            type="button"
            class="rounded-xl p-2 bg-slate-100 text-slate-600 hover:bg-slate-200 border border-slate-200/80 transition-all cursor-pointer disabled:opacity-50"
            :title="t('shell.restart')"
            :aria-label="t('shell.restart')"
            :disabled="!connected"
            @click="emit('restart')"
          >
            <RotateCcw :size="17" :stroke-width="2.2" />
          </button>
          <div class="flex items-center gap-2 pl-1 border-l border-slate-200">
            <span
              class="h-2.5 w-2.5 rounded-full"
              :class="{ 'bg-emerald-500 shadow-xs shadow-emerald-500/50': connected && motorReady, 'bg-rose-500': !connected }"
            />
            <span data-testid="connection-status" class="text-xs font-bold text-slate-700">{{ statusText() }}</span>
          </div>
        </div>
      </header>

      <slot name="connection" />

      <div v-if="error" class="mb-4 bg-red-500 p-2 text-white">
        {{ error }}
        <button type="button" class="ml-4 font-bold" @click="emit('retry')">
          {{ t('shell.retry') }}
        </button>
      </div>

      <slot name="alerts" />

      <div class="space-y-4">
        <slot v-if="showDetails" name="details" />
        <slot name="settings" />
        <MotionWorkbench
          v-model:config="config"
          v-model:pause-mode="pauseMode"
          :state="state"
          :connected="motorReady"
          :show-diagram="showDiagram"
          :is-mutating="isMutating"
          :hide-motion="hideMotion"
          @posted-config="emit('postedConfig', $event)"
          @macro-config="emit('macroConfig', $event)"
          @set-paused="emit('setPaused', $event)"
          @set-paused-position="emit('setPausedPosition', $event)"
        />
      </div>
    </div>
  </div>
</template>
