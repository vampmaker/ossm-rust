import { ref, watch, onMounted, onBeforeUnmount, type ComputedRef, type Ref } from 'vue'
import type { ComposerTranslation } from 'vue-i18n'
import type { MotorControllerConfig, MotorState, PauseMode } from './types'
import { DEFAULT_MOTOR_CONFIG } from './types'
import type { OssmControl } from './control'
import { stateMachine } from './connectionStateMachine'
import {
  wireToDomainConfig,
  domainToWireConfig,
  sanitizeMotorState,
  isConfigEqual,
} from './mapper'

const DEFAULT_DIAGRAM_KEY = 'ossm_show_position_diagram'
const CONTROL_POLL_MS = 1000

export interface ControllerSessionOptions {
  control: () => OssmControl
  t: ComposerTranslation
  diagramStorageKey?: string
  /** Live subscribe interval when diagram or details panel is open (default 33). */
  liveIntervalMs?: Ref<number> | ComputedRef<number>
  /** Extra panels that should keep live WS subscribe active (e.g. ESP32 settings). */
  isLiveViewOpen?: () => boolean
  /** Establish connection and load config; return false when disconnected. */
  initialize?: () => Promise<boolean>
  autoInitializeOnMount?: boolean
}

function isBenignFetchFailure(e: unknown): boolean {
  if (e instanceof DOMException && e.name === 'AbortError') return true
  if (e instanceof Error && e.name === 'AbortError') return true
  if (e instanceof TypeError && /Failed to fetch/i.test(e.message)) return true
  return false
}

export function useControllerSession(opts: ControllerSessionOptions) {
  const config = ref<MotorControllerConfig>({ ...DEFAULT_MOTOR_CONFIG })
  const connected = ref(false)
  const motorReady = ref(false)
  const diagramKey = opts.diagramStorageKey ?? DEFAULT_DIAGRAM_KEY
  const showDiagram = ref(localStorage.getItem(diagramKey) !== 'false')
  const showDetails = ref(false)
  const pauseMode = ref<PauseMode>('fixed-position')
  const error = ref<string | null>(null)
  const isInitialized = ref(false)
  const motorState = ref<MotorState | null>(null)

  let controlPollTimer: ReturnType<typeof setInterval> | undefined
  let stateListenerBound = false

  watch(showDiagram, (val) => {
    localStorage.setItem(diagramKey, String(val))
  })

  function applyRemoteState(state: MotorState) {
    const cleanState = sanitizeMotorState(state)
    motorState.value = cleanState
    if (cleanState.config) {
      stateMachine.trackAuthoritativeVersion(cleanState.config.version)
    }
    if (stateMachine.shouldAcceptRemoteConfig(cleanState.config)) {
      const normalized = wireToDomainConfig(cleanState.config, config.value)
      if (!isConfigEqual(config.value, normalized)) {
        config.value = normalized
      }
    }
  }

  function onMotorStateUpdate(state: MotorState) {
    applyRemoteState(state)
  }

  function stopControlPoll() {
    if (controlPollTimer !== undefined) {
      clearInterval(controlPollTimer)
      controlPollTimer = undefined
    }
  }

  async function pollControlState() {
    if (!isInitialized.value || !connected.value) return
    if (showDiagram.value || showDetails.value || opts.isLiveViewOpen?.()) return
    try {
      const state = await opts.control().getState()
      applyRemoteState(state)
    } catch (e) {
      if (!isBenignFetchFailure(e)) {
        console.warn('Control-state poll failed:', e)
      }
    }
  }

  function startControlPoll() {
    if (controlPollTimer !== undefined) return
    controlPollTimer = setInterval(() => {
      void pollControlState()
    }, CONTROL_POLL_MS)
    void pollControlState()
  }

  function liveInterval(): number {
    return opts.liveIntervalMs?.value ?? 33
  }

  async function syncLiveStateSubscription() {
    if (!isInitialized.value) {
      stopControlPoll()
      return
    }

    const wantLive = showDiagram.value || showDetails.value || (opts.isLiveViewOpen?.() ?? false)
    if (wantLive) {
      stopControlPoll()
      try {
        await opts.control().subscribeState(onMotorStateUpdate, liveInterval())
        stateListenerBound = true
      } catch (e) {
        console.warn('Could not subscribe to live motor state:', e)
      }
      return
    }

    if (stateListenerBound) {
      try {
        await opts.control().unsubscribeState(onMotorStateUpdate)
      } catch (e) {
        console.warn('Failed to unsubscribe live motor state:', e)
      }
      stateListenerBound = false
    }
    startControlPoll()
  }

  watch([showDiagram, showDetails], () => {
    void syncLiveStateSubscription()
  })

  function setConfig(newConfig: MotorControllerConfig) {
    if (stateMachine.authoritativeVersion.value > 0 || newConfig.version !== undefined) {
      newConfig.version = stateMachine.getNextVersion()
    }
    config.value = newConfig
    error.value = null

    stateMachine.scheduleOptimisticMutation(
      'config',
      200,
      () => opts.control().setConfig(domainToWireConfig(config.value)),
      (posted) => {
        if (posted) {
          stateMachine.trackAuthoritativeVersion(posted.version)
        }
        config.value = wireToDomainConfig(posted, config.value)
      },
      (err) => {
        console.error(err)
        error.value = opts.t('errors.setConfigFailed')
        motorReady.value = false
      },
    )
  }

  async function setPaused(paused: boolean) {
    stateMachine.clearOptimisticMutation('config')
    stateMachine.clearOptimisticMutation('pausedPosition')
    config.value = { ...config.value, paused }
    error.value = null

    stateMachine.beginLocalEdit()
    try {
      let updatedConfig: MotorControllerConfig
      if (paused && pauseMode.value === 'in-place') {
        const state = await stateMachine.executeControl(() => opts.control().getState())
        const pausePos =
          typeof state.y === 'number'
            ? state.y
            : typeof state.shaped_y === 'number'
              ? state.shaped_y
              : config.value.paused_position
        updatedConfig = await stateMachine.executeControl(() =>
          opts.control().setPaused({ paused: true, position: pausePos }),
        )
      } else {
        updatedConfig = await stateMachine.executeControl(() =>
          opts.control().setPaused({ paused }),
        )
      }
      if (updatedConfig) {
        stateMachine.trackAuthoritativeVersion(updatedConfig.version)
      }
      config.value = wireToDomainConfig(updatedConfig, config.value)
    } catch (e) {
      console.error(e)
      error.value = opts.t('errors.setPausedFailed')
      motorReady.value = false
    } finally {
      stateMachine.endLocalEdit()
    }
  }

  function setPausedPosition(position: number) {
    config.value = { ...config.value, paused_position: position }
    error.value = null

    stateMachine.scheduleOptimisticMutation(
      'pausedPosition',
      100,
      () => opts.control().setPaused({ position }),
      (updated) => {
        if (updated) {
          stateMachine.trackAuthoritativeVersion(updated.version)
        }
        config.value = wireToDomainConfig(updated, config.value)
      },
      (err) => {
        console.error(err)
        error.value = opts.t('errors.setPausedPositionFailed')
        motorReady.value = false
      },
    )
  }

  function onMacroConfigUpdate(newConfig: MotorControllerConfig) {
    config.value = newConfig
  }

  async function fetchConfig() {
    error.value = null
    try {
      const deviceConfig = await opts.control().getConfig()
      connected.value = true
      if (deviceConfig) {
        stateMachine.trackAuthoritativeVersion(deviceConfig.version)
      }
      if (stateMachine.shouldAcceptRemoteConfig(deviceConfig)) {
        config.value = wireToDomainConfig(deviceConfig, config.value)
      }
      try {
        motorState.value = sanitizeMotorState(await opts.control().getState())
      } catch (e) {
        if (!isBenignFetchFailure(e)) {
          console.warn('Failed to fetch initial state:', e)
        }
      }
      motorReady.value = true
      isInitialized.value = true
      await syncLiveStateSubscription()
      return true
    } catch (e) {
      if (!isBenignFetchFailure(e)) {
        console.error(e)
      }
      connected.value = false
      motorReady.value = false
      isInitialized.value = false
      motorState.value = null
      error.value = opts.t('errors.deviceConnectFailed')
      return false
    }
  }

  async function initialize() {
    if (opts.initialize) {
      const ok = await opts.initialize()
      if (ok) {
        isInitialized.value = true
      }
      return ok
    }
    return fetchConfig()
  }

  async function restartDevice() {
    const restart = opts.control().restart
    if (!restart) return
    try {
      error.value = null
      await restart()
      connected.value = false
      motorReady.value = false
      error.value = opts.t('errors.restartSent')
      setTimeout(() => {
        void initialize()
      }, 3000)
    } catch (e) {
      console.error(e)
      error.value = opts.t('errors.restartFailed')
    }
  }

  stateMachine.onReconnect(async () => {
    await initialize()
  })

  watch(connected, (isConn, wasConn) => {
    if (isConn && !wasConn) {
      stateMachine.setConnectionStatus('CONNECTED')
    } else if (!isConn) {
      stateMachine.setConnectionStatus('DISCONNECTED')
    }
  })

  watch(
    () => config.value.wave_func,
    (newWaveFunc, oldWaveFunc) => {
      if (isInitialized.value && newWaveFunc === 'thrust' && oldWaveFunc !== 'thrust') {
        setConfig({ ...config.value, sharpness: 0.1 })
      }
    },
  )

  onMounted(() => {
    if (opts.autoInitializeOnMount !== false && opts.initialize === undefined) {
      void fetchConfig()
    } else if (opts.autoInitializeOnMount !== false && opts.initialize) {
      void initialize()
    }
  })

  onBeforeUnmount(() => {
    stopControlPoll()
    if (stateListenerBound) {
      void opts.control().unsubscribeState(onMotorStateUpdate)
    }
  })

  return {
    config,
    connected,
    motorReady,
    showDiagram,
    showDetails,
    pauseMode,
    error,
    isInitialized,
    motorState,
    stateMachine,
    applyRemoteState,
    onMotorStateUpdate,
    syncLiveStateSubscription,
    setConfig,
    setPaused,
    setPausedPosition,
    onMacroConfigUpdate,
    fetchConfig,
    initialize,
    restartDevice,
  }
}
