<script setup lang="ts">
import { provide } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  ControllerPageShell,
  OSSM_CONTROL_KEY,
  useControllerSession,
} from '@ossm/shared'
import { deviceControl } from '@ossm/client'
import { setLocale } from './i18n'
import StdDetailsPanel from './components/StdDetailsPanel.vue'

provide(OSSM_CONTROL_KEY, deviceControl)

const { t } = useI18n()

const session = useControllerSession({
  control: () => deviceControl,
  t,
  autoInitializeOnMount: true,
})

const {
  config,
  connected,
  motorReady,
  showDiagram,
  showDetails,
  pauseMode,
  error,
  motorState,
  stateMachine,
  setConfig,
  setPaused,
  setPausedPosition,
  onMacroConfigUpdate,
  fetchConfig,
  restartDevice,
} = session
</script>

<template>
  <ControllerPageShell
    v-model:config="config"
    v-model:pause-mode="pauseMode"
    v-model:show-diagram="showDiagram"
    v-model:show-details="showDetails"
    :title="t('app.title')"
    :state="motorState"
    :connected="connected"
    :motor-ready="motorReady"
    :error="error"
    :is-mutating="stateMachine.isMutating.value"
    :capabilities="{ locale: true, diagram: true, details: true, restart: true }"
    @retry="fetchConfig"
    @restart="restartDevice"
    @locale="setLocale"
    @posted-config="setConfig"
    @macro-config="onMacroConfigUpdate"
    @set-paused="setPaused"
    @set-paused-position="setPausedPosition"
  >
    <template #details>
      <StdDetailsPanel :state="motorState" :config="config" />
    </template>
  </ControllerPageShell>
</template>
