<script setup lang="ts">
/// <reference types="w3c-web-serial" />
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ESPLoader, Transport } from 'esptool-js'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { setLocale } from './i18n'
import type { AppLocale } from './i18n'

type Status = 'idle' | 'connecting' | 'connected' | 'flashing' | 'flash_done' | 'configuring' | 'config_done' | 'monitoring' | 'error'
type SerialMode = 'idle' | 'bootloader' | 'config' | 'monitor'

const { t, locale } = useI18n()

const status = ref<Status>('idle')
const chipName = ref('')
const errorMessage = ref('')
const terminalEl = ref<HTMLElement | null>(null)
const serialTerminalEl = ref<HTMLElement | null>(null)

const flashProgress = ref(0)
const flashAddress = ref('0x0')
const firmwareFile = ref<File | null>(null)
const firmwareDataStr = ref<string | null>(null)
const isDragOver = ref(false)

const wifiSsid = ref('')
const wifiPassword = ref('')
const pinModbusTx = ref(18)
const pinModbusRx = ref(19)
const pinModbusDeRe = ref(20)
const modbusTimeoutMs = ref(0)
const modbusRxTimeoutUs = ref(0)
const modbusInterFrameDelayUs = ref(0)
const modbusScanDelayUs = ref(0)
const wifiEnabled = ref(true)
const bleEnabled = ref(true)
const modbusDebug = ref(false)
const operatingMode = ref<'servo' | 'rtu_relay' | 'rs485'>('servo')

const DEVICE_CONFIG_STORAGE_KEY = 'ossm-flasher-device-config-v1'
const DEFAULT_DEVICE_CONFIG = {
  wifiEnabled: true,
  wifiSsid: '',
  wifiPassword: '',
  pinModbusTx: 18,
  pinModbusRx: 19,
  pinModbusDeRe: 20,
  modbusTimeoutMs: 0,
  modbusRxTimeoutUs: 0,
  modbusInterFrameDelayUs: 0,
  modbusScanDelayUs: 0,
  bleEnabled: true,
  modbusDebug: false,
  operatingMode: 'servo' as 'servo' | 'rtu_relay' | 'rs485',
  flashAddress: '0x0',
} as const

function applyDeviceConfig(config: Partial<typeof DEFAULT_DEVICE_CONFIG>) {
  wifiEnabled.value = config.wifiEnabled ?? DEFAULT_DEVICE_CONFIG.wifiEnabled
  wifiSsid.value = config.wifiSsid ?? DEFAULT_DEVICE_CONFIG.wifiSsid
  wifiPassword.value = config.wifiPassword ?? DEFAULT_DEVICE_CONFIG.wifiPassword
  pinModbusTx.value = config.pinModbusTx ?? DEFAULT_DEVICE_CONFIG.pinModbusTx
  pinModbusRx.value = config.pinModbusRx ?? DEFAULT_DEVICE_CONFIG.pinModbusRx
  pinModbusDeRe.value = config.pinModbusDeRe ?? DEFAULT_DEVICE_CONFIG.pinModbusDeRe
  modbusTimeoutMs.value = config.modbusTimeoutMs ?? DEFAULT_DEVICE_CONFIG.modbusTimeoutMs
  modbusRxTimeoutUs.value = config.modbusRxTimeoutUs ?? DEFAULT_DEVICE_CONFIG.modbusRxTimeoutUs
  modbusInterFrameDelayUs.value = config.modbusInterFrameDelayUs ?? DEFAULT_DEVICE_CONFIG.modbusInterFrameDelayUs
  modbusScanDelayUs.value = config.modbusScanDelayUs ?? DEFAULT_DEVICE_CONFIG.modbusScanDelayUs
  bleEnabled.value = config.bleEnabled ?? DEFAULT_DEVICE_CONFIG.bleEnabled
  modbusDebug.value = config.modbusDebug ?? DEFAULT_DEVICE_CONFIG.modbusDebug
  operatingMode.value = config.operatingMode ?? DEFAULT_DEVICE_CONFIG.operatingMode
  flashAddress.value = config.flashAddress ?? DEFAULT_DEVICE_CONFIG.flashAddress
}

function loadDeviceConfig() {
  try {
    const raw = localStorage.getItem(DEVICE_CONFIG_STORAGE_KEY)
    if (!raw) {
      return
    }
    const parsed = JSON.parse(raw) as Partial<typeof DEFAULT_DEVICE_CONFIG>
    applyDeviceConfig(parsed)
  } catch (err) {
    console.warn('Failed to load persisted device config:', err)
  }
}

function persistDeviceConfig() {
  try {
    const payload = {
      wifiEnabled: wifiEnabled.value,
      wifiSsid: wifiSsid.value,
      wifiPassword: wifiPassword.value,
      pinModbusTx: pinModbusTx.value,
      pinModbusRx: pinModbusRx.value,
      pinModbusDeRe: pinModbusDeRe.value,
      modbusTimeoutMs: modbusTimeoutMs.value,
      modbusRxTimeoutUs: modbusRxTimeoutUs.value,
      modbusInterFrameDelayUs: modbusInterFrameDelayUs.value,
      modbusScanDelayUs: modbusScanDelayUs.value,
      bleEnabled: bleEnabled.value,
      modbusDebug: modbusDebug.value,
      operatingMode: operatingMode.value,
      flashAddress: flashAddress.value,
    }
    localStorage.setItem(DEVICE_CONFIG_STORAGE_KEY, JSON.stringify(payload))
  } catch (err) {
    console.warn('Failed to persist device config:', err)
  }
}

function resetDeviceConfig() {
  applyDeviceConfig(DEFAULT_DEVICE_CONFIG)
  termLog(`\x1b[33m${t('log.configResetDefaults')}\x1b[0m`)
}

let transport: Transport | null = null
let esploader: ESPLoader | null = null
const serialMode = ref<SerialMode>('idle')
const hasPort = ref(false)
let serialPort: SerialPort | null = null
let sessionReader: ReadableStreamDefaultReader<Uint8Array> | null = null
let sessionWriter: WritableStreamDefaultWriter<Uint8Array> | null = null
let monitorRequested = false
let monitorToken = 0
let opQueue: Promise<void> = Promise.resolve()

let xterm: Terminal | null = null
let fitAddon: FitAddon | null = null
let serialXterm: Terminal | null = null
let serialFitAddon: FitAddon | null = null
let resizeObserver: ResizeObserver | null = null

const webSerialSupported = computed(() => 'serial' in navigator)
const canFlash = computed(() => status.value === 'connected' && firmwareDataStr.value !== null)
const canConfigure = computed(() =>
  status.value === 'flash_done' || status.value === 'connected' || status.value === 'config_done'
)
const canMonitor = computed(() =>
  hasPort.value && !['idle', 'connecting', 'flashing', 'configuring', 'monitoring'].includes(status.value)
)
const canReset = computed(() =>
  hasPort.value && !['idle', 'connecting', 'flashing', 'configuring'].includes(status.value)
)

function setPort(port: SerialPort | null) {
  serialPort = port
  hasPort.value = port !== null
}

function termLog(line: string) {
  xterm?.writeln(line)
}

watch(
  locale,
  () => {
    document.title = t('meta.title')
  },
  { immediate: true },
)

onMounted(() => {
  loadDeviceConfig()

  const theme = {
    background: '#1a1a2e',
    foreground: '#e0e0e0',
    cursor: '#e0e0e0',
    black: '#1a1a2e',
    red: '#ff6b6b',
    green: '#51cf66',
    yellow: '#fcc419',
    blue: '#339af0',
    magenta: '#cc5de8',
    cyan: '#22b8cf',
    white: '#e0e0e0',
    brightBlack: '#6c757d',
    brightRed: '#ff8787',
    brightGreen: '#69db7c',
    brightYellow: '#ffd43b',
    brightBlue: '#5c7cfa',
    brightMagenta: '#da77f2',
    brightCyan: '#3bc9db',
    brightWhite: '#f8f9fa',
  } as const

  if (terminalEl.value) {
    xterm = new Terminal({
      cursorBlink: false,
      disableStdin: true,
      scrollback: 5000,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      theme,
    })
    fitAddon = new FitAddon()
    xterm.loadAddon(fitAddon)
    xterm.open(terminalEl.value)
    fitAddon.fit()
    xterm.writeln(`\x1b[90m${t('log.waitingActivity')}\x1b[0m`)
  }

  if (serialTerminalEl.value) {
    serialXterm = new Terminal({
      cursorBlink: true,
      disableStdin: false,
      scrollback: 5000,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      theme,
    })
    serialFitAddon = new FitAddon()
    serialXterm.loadAddon(serialFitAddon)
    serialXterm.open(serialTerminalEl.value)
    serialFitAddon.fit()
    serialXterm.writeln(`\x1b[90m${t('log.waitingSerial')}\x1b[0m`)

    serialXterm.onData(async (data) => {
      const encoder = new TextEncoder()
      const payload = encoder.encode(data)
      if (sessionWriter) {
        try {
          await sessionWriter.write(payload)
        } catch (err) {
          console.error('Serial terminal write error:', err)
        }
      } else if (serialPort?.writable && !serialPort.writable.locked) {
        try {
          const writer = serialPort.writable.getWriter()
          await writer.write(payload)
          writer.releaseLock()
        } catch (err) {
          console.error('Serial terminal write error:', err)
        }
      }
    })
  }

  resizeObserver = new ResizeObserver(() => {
    fitAddon?.fit()
    serialFitAddon?.fit()
  })
  if (terminalEl.value) resizeObserver.observe(terminalEl.value)
  if (serialTerminalEl.value) resizeObserver.observe(serialTerminalEl.value)
})

watch(
  [wifiSsid, wifiPassword, pinModbusTx, pinModbusRx, pinModbusDeRe, modbusTimeoutMs, modbusRxTimeoutUs, modbusInterFrameDelayUs, modbusScanDelayUs, bleEnabled, modbusDebug, operatingMode, flashAddress],
  () => {
    persistDeviceConfig()
  },
)

onUnmounted(() => {
  resizeObserver?.disconnect()
  xterm?.dispose()
  serialXterm?.dispose()
})

function withSerialLock<T>(op: () => Promise<T>): Promise<T> {
  const run = opQueue.then(op, op)
  opQueue = run.then(() => undefined, () => undefined)
  return run
}

function isPortOpenNoLock(): boolean {
  if (!serialPort) return false
  return serialPort.readable !== null || serialPort.writable !== null
}

async function releaseIoNoLock() {
  if (sessionReader) {
    try { await sessionReader.cancel() } catch { /* ignore */ }
    try { sessionReader.releaseLock() } catch { /* ignore */ }
    sessionReader = null
  }
  if (sessionWriter) {
    try { sessionWriter.releaseLock() } catch { /* ignore */ }
    sessionWriter = null
  }
}

async function closePortNoLock() {
  if (!serialPort) return
  try { await serialPort.close() } catch { /* ignore */ }
}

async function openPortNoLock(baudRate: number = 115200) {
  if (!serialPort) throw new Error(t('errors.noPort'))
  if (!isPortOpenNoLock()) {
    try {
      await serialPort.open({ baudRate })
    } catch (e: any) {
      if (String(e).includes('NetworkError') || String(e).includes('lost') || String(e).includes('NotFoundError') || String(e).includes('disconnected') || String(e).includes('Failed to connect')) {
        // Recover lost port (Native USB disconnect/reconnect)
        termLog(`\x1b[90m${t('log.portLostRecovery')}\x1b[0m`)
        let recovered = false
        for (let i = 0; i < 20; i++) {
          await sleep(250)
          const ports = await navigator.serial.getPorts()
          if (ports.length > 0) {
            setPort(ports[0] || null)
            try {
              await serialPort!.open({ baudRate })
              recovered = true
              termLog(`\x1b[32m${t('log.portRecovered')}\x1b[0m`)
              break
            } catch (openErr) {
              // Port might not be fully ready yet
            }
          }
        }
        if (!recovered) throw e
      } else {
        throw e
      }
    }
  }
}

async function pulseResetSignalsNoLock(port: SerialPort) {
  await port.setSignals({ dataTerminalReady: false, requestToSend: true })
  await sleep(100)
  await port.setSignals({ dataTerminalReady: true, requestToSend: false })
  await sleep(100)
  await port.setSignals({ dataTerminalReady: false, requestToSend: false })
}

async function leaveCurrentMode() {
  await withSerialLock(async () => {
    monitorRequested = false
    monitorToken += 1
    await releaseIoNoLock()
    await closePortNoLock()
    serialMode.value = 'idle'
  })
}

async function cleanupEsptool() {
  if (transport) {
    try {
      await Promise.race([
        transport.disconnect(),
        sleep(3000),
      ])
    } catch { /* ignore */ }
    transport = null
    esploader = null
  }
  if (serialMode.value === 'bootloader') {
    serialMode.value = 'idle'
  }
}

async function ensureNormalModeFromBootloader() {
  if (esploader) {
    termLog(`\x1b[33m${t('log.bootloaderResetting')}\x1b[0m`)
    try {
      await esploader.after('hard_reset')
    } catch {
      termLog(`\x1b[90m${t('log.hardResetFailed')}\x1b[0m`)
    }
    await cleanupEsptool()
    await sleep(800)
  }
}

async function enterConfigMode() {
  await withSerialLock(async () => {
    monitorRequested = false
    monitorToken += 1
    // e2e: keep the already-open ACM. close+open pulses kernel DTR and resets the chip.
    const keepOpen = isE2eSerialMode() && !!serialPort
    if (!keepOpen) {
      await releaseIoNoLock()
      await closePortNoLock()
      await openPortNoLock(115200)
    } else if (!sessionReader || !sessionWriter) {
      await releaseIoNoLock()
      if (!serialPort?.readable || !serialPort?.writable) throw new Error(t('errors.streamsUnavailable'))
      sessionReader = serialPort.readable.getReader()
      sessionWriter = serialPort.writable.getWriter()
    }
    if (!serialPort?.readable || !serialPort?.writable) throw new Error(t('errors.streamsUnavailable'))
    if (!sessionReader) sessionReader = serialPort.readable.getReader()
    if (!sessionWriter) sessionWriter = serialPort.writable.getWriter()
    serialMode.value = 'config'
  })
}

async function performReset(resumeMonitor: boolean) {
  await withSerialLock(async () => {
    monitorRequested = false
    monitorToken += 1
    await releaseIoNoLock()
    await closePortNoLock()
    await openPortNoLock(115200)
    await pulseResetSignalsNoLock(serialPort!)
    await closePortNoLock()
    serialMode.value = 'idle'
    if (resumeMonitor) {
      monitorRequested = true
      monitorToken += 1
      const token = monitorToken
      queueMicrotask(() => {
        void monitorLoop(token, false)
      })
    }
  })
}

async function enterMonitorMode(resetFirst: boolean) {
  await withSerialLock(async () => {
    monitorRequested = false
    monitorToken += 1
    await releaseIoNoLock()
    await closePortNoLock()

    if (resetFirst) {
      await openPortNoLock(115200)
      await pulseResetSignalsNoLock(serialPort!)
      await closePortNoLock()
      await sleep(250)
    }

    monitorRequested = true
    monitorToken += 1
    const token = monitorToken
    serialMode.value = 'monitor'
    queueMicrotask(() => {
      void monitorLoop(token, true)
    })
  })
}

async function monitorLoop(token: number, logStart: boolean) {
  if (logStart) {
    status.value = 'monitoring'
    termLog(`\x1b[1;36m${t('log.monitorStarted')}\x1b[0m`)
  }

  let reconnectAttempts = 0
  const maxReconnectAttempts = 10
  const decoder = new TextDecoder()

  while (monitorRequested && token === monitorToken && serialMode.value === 'monitor') {
    let activeReader: ReadableStreamDefaultReader<Uint8Array> | null = null
    try {
      activeReader = await withSerialLock(async () => {
        await releaseIoNoLock()
        await closePortNoLock()
        await openPortNoLock(115200)
        if (!serialPort?.readable || !serialPort?.writable) throw new Error(t('errors.serialStreamsUnavailable'))
        const reader = serialPort.readable.getReader()
        sessionReader = reader
        sessionWriter = serialPort.writable.getWriter()
        return reader
      })
      if (!activeReader) throw new Error(t('errors.acquireMonitorReader'))

      reconnectAttempts = 0
      while (monitorRequested && token === monitorToken && serialMode.value === 'monitor') {
        const result = await activeReader.read()
        if (result.done) {
          throw new Error('__STREAM_DONE__')
        }
        if (result.value && result.value.length > 0) {
          serialXterm?.write(decoder.decode(result.value, { stream: true }))
        }
      }
    } catch (err: any) {
      if (!monitorRequested || token !== monitorToken || serialMode.value !== 'monitor') break
      if (String(err?.message || err) === '__STREAM_DONE__') {
        termLog(`\x1b[33m${t('log.monitorStreamEnded')}\x1b[0m`)
      } else {
        termLog(`\x1b[31m${t('log.monitorError', { message: err?.message || err })}\x1b[0m`)
      }
      reconnectAttempts += 1
      if (reconnectAttempts > maxReconnectAttempts) {
        termLog(`\x1b[31m${t('log.monitorStoppedFailures')}\x1b[0m`)
        break
      }
      termLog(`\x1b[33m${t('log.monitorReconnecting', { attempt: reconnectAttempts, max: maxReconnectAttempts })}\x1b[0m`)
      await sleep(1000)
    } finally {
      const tail = decoder.decode()
      if (tail) serialXterm?.write(tail)
      await withSerialLock(async () => {
        if (activeReader) {
          try { activeReader.releaseLock() } catch { /* ignore */ }
        }
        if (sessionReader === activeReader) sessionReader = null
        await closePortNoLock()
      })
    }
  }

  if (token === monitorToken) {
    monitorRequested = false
    serialMode.value = 'idle'
    status.value = hasPort.value ? 'flash_done' : 'idle'
    termLog(`\x1b[1;36m${t('log.monitorStopped')}\x1b[0m`)
  }
}

function arrayBufferToBinaryString(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer)
  return Array.from(bytes, (b) => String.fromCharCode(b)).join('')
}

function handleFileSelect(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (file) void loadFile(file)
}

function handleDrop(event: DragEvent) {
  isDragOver.value = false
  const file = event.dataTransfer?.files[0]
  if (file) void loadFile(file)
}

async function loadFile(file: File) {
  firmwareFile.value = file
  const buffer = await file.arrayBuffer()
  firmwareDataStr.value = arrayBufferToBinaryString(buffer)
  termLog(`\x1b[32m${t('log.loadedFirmware', { name: file.name, size: (file.size / 1024).toFixed(1) })}\x1b[0m`)
}

const espTerminal = {
  clean() { serialXterm?.clear() },
  writeLine(data: string) { serialXterm?.writeln(data) },
  write(data: string) { serialXterm?.write(data) },
}

function isE2eSerialMode(): boolean {
  return new URLSearchParams(window.location.search).get('e2e') === '1'
}

async function connect() {
  if (!webSerialSupported.value) return
  status.value = 'connecting'
  errorMessage.value = ''
  termLog(`\x1b[90m${t('log.requestingPort')}\x1b[0m`)
  try {
    const port = await navigator.serial.requestPort()
    setPort(port)
    if (isE2eSerialMode()) {
      termLog(`\x1b[33m${t('log.e2eMode')}\x1b[0m`)
      await withSerialLock(async () => {
        await closePortNoLock()
        await openPortNoLock(115200)
      })
      chipName.value = 'E2E'
      serialMode.value = 'idle'
      status.value = 'flash_done'
      termLog(`\x1b[1;32m${t('log.serialReadyConfig')}\x1b[0m`)
      return
    }
    transport = new Transport(port, true)
    esploader = new ESPLoader({
      transport,
      baudrate: 115200,
      romBaudrate: 115200,
      terminal: espTerminal,
    })
    termLog(`\x1b[33m${t('log.connectingEsp')}\x1b[0m`)
    const chip = await esploader.main()
    chipName.value = chip
    serialMode.value = 'bootloader'
    status.value = 'connected'
    termLog(`\x1b[1;32m${t('log.connectedChip', { chip })}\x1b[0m`)
  } catch (err: any) {
    if (String(err).includes('NetworkError') || String(err).includes('lost') || String(err).includes('Timeout') || String(err).includes('Failed to connect')) {
      termLog(`\x1b[33m${t('log.fallbackNormalMode')}\x1b[0m`)
      await cleanupEsptool()
      esploader = null
      serialMode.value = 'idle'
      status.value = 'connected'
    } else {
      status.value = 'error'
      errorMessage.value = err?.message || String(err)
      termLog(`\x1b[1;31m${t('log.connectionFailed', { message: errorMessage.value })}\x1b[0m`)
    }
  }
}

async function disconnect() {
  await leaveCurrentMode()
  await cleanupEsptool()
  setPort(null)
  chipName.value = ''
  status.value = 'idle'
  termLog(`\x1b[90m${t('log.disconnected')}\x1b[0m`)
}

async function flash() {
  if (!esploader || !firmwareDataStr.value) return
  const address = parseInt(flashAddress.value, 16)
  if (isNaN(address)) {
    termLog(`\x1b[31m${t('log.invalidFlashAddress')}\x1b[0m`)
    return
  }
  status.value = 'flashing'
  flashProgress.value = 0
  termLog(`\x1b[33m${t('log.flashingFile', { name: firmwareFile.value?.name ?? '', address: `0x${address.toString(16)}` })}\x1b[0m`)
  try {
    await esploader.writeFlash({
      fileArray: [{ data: firmwareDataStr.value, address }],
      flashMode: 'dio',
      flashFreq: '40m',
      flashSize: '4MB',
      eraseAll: false,
      compress: true,
      reportProgress: (_fileIndex: number, written: number, total: number) => {
        flashProgress.value = Math.round((written / total) * 100)
      },
    })
    flashProgress.value = 100
    termLog(`\x1b[1;32m${t('log.flashCompleteResetting')}\x1b[0m`)
    await esploader.after('hard_reset')
    await cleanupEsptool()
    serialMode.value = 'idle'
    status.value = 'flash_done'
    termLog(`\x1b[32m${t('log.deviceResetReady')}\x1b[0m`)
  } catch (err: any) {
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[1;31m${t('log.flashFailed', { message: errorMessage.value })}\x1b[0m`)
  }
}

type SerialReadResult = ReadableStreamReadResult<Uint8Array>

/** One outstanding `reader.read()` at a time. Racing a new read against sleep
 *  abandons the first promise; USB bytes then complete the dangling read and
 *  never reach the waiter (ACK timeout after `[console] ready`). */
async function readSerialUntil(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  pred: (buffer: string) => boolean,
  timeoutMs: number,
  options?: { throwOnDone?: boolean; doneMessage?: string },
): Promise<boolean> {
  const decoder = new TextDecoder()
  let buffer = ''
  const deadline = Date.now() + timeoutMs
  let pending: Promise<SerialReadResult> | null = null

  while (Date.now() < deadline) {
    if (!pending) pending = reader.read()
    const remaining = Math.max(0, deadline - Date.now())
    const result = await Promise.race([
      pending.then((r) => ({ kind: 'read' as const, r })),
      sleep(Math.min(remaining, 500)).then(() => null),
    ])
    if (result === null) continue
    pending = null
    if (result.r.done) {
      const tail = decoder.decode()
      if (tail) {
        buffer += tail
        serialXterm?.write(tail)
      }
      if (pred(buffer)) return true
      if (options?.throwOnDone) {
        throw new Error(options.doneMessage ?? 'serial stream ended')
      }
      break
    }
    if (result.r.value && result.r.value.length > 0) {
      const text = decoder.decode(result.r.value, { stream: true })
      buffer += text
      serialXterm?.write(text)
      if (pred(buffer)) return true
    }
  }

  const tail = decoder.decode()
  if (tail) {
    buffer += tail
    serialXterm?.write(tail)
  }
  return pred(buffer)
}

async function waitForBoot(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  timeoutMs: number,
): Promise<boolean> {
  const ok = await readSerialUntil(
    reader,
    (buffer) => buffer.includes('[console] ready'),
    timeoutMs,
  )
  if (ok) termLog(`\x1b[1;32m${t('log.bootDetected')}\x1b[0m`)
  return ok
}

async function waitForAck(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  expectedSubstrings: string[],
  timeoutMs: number,
): Promise<boolean> {
  return readSerialUntil(
    reader,
    (buffer) => expectedSubstrings.some((s) => buffer.includes(s)),
    timeoutMs,
    {
      throwOnDone: true,
      doneMessage: t('errors.streamEndedWaitingAck', { expected: expectedSubstrings.join(' | ') }),
    },
  )
}

async function sendConfig() {
  status.value = 'configuring'
  errorMessage.value = ''
  termLog(`\x1b[33m${t('log.preparingConfig')}\x1b[0m`)
  try {
    if (!serialPort) throw new Error(t('errors.noPortConnectFirst'))
    
    const neededReset = !esploader && !isE2eSerialMode()
    await ensureNormalModeFromBootloader()

    if (neededReset) {
      termLog(`\x1b[90m${t('log.togglingReset')}\x1b[0m`)
      try {
        await withSerialLock(async () => {
          await releaseIoNoLock()
          await closePortNoLock()
          await openPortNoLock(115200)
          await pulseResetSignalsNoLock(serialPort!)
        })
      } catch {
        // Ignore disconnect errors from native USB CDC resetting
      }
      await withSerialLock(async () => {
        await closePortNoLock()
      })
      await sleep(1000)
    }

    await enterConfigMode()
    if (!sessionReader || !sessionWriter) throw new Error(t('errors.openStreams'))

    termLog(`\x1b[33m${t('log.waitingBoot')}\x1b[0m`)
    const booted = await waitForBoot(sessionReader, 15000)
    if (!booted) {
      termLog(`\x1b[33m${t('log.bootNotDetected')}\x1b[0m`)
    } else {
      await sleep(750)
    }

    // ACK substrings below are firmware protocol English — do not translate
    const commands: Array<{ cmd: string; ack: string[] }> = []
    commands.push({
      cmd: `set net.wifi_enabled ${wifiEnabled.value}`,
      ack: ['shell.wifi_enabled set to', 'wifi_enabled set to'],
    })
    if (wifiEnabled.value && wifiSsid.value) {
      commands.push({
        cmd: `set net.ssid ${escapeCliArg(wifiSsid.value)}`,
        ack: ['shell.ssid set to'],
      })
    }
    if (wifiEnabled.value && wifiPassword.value) {
      commands.push({
        cmd: `set net.password ${escapeCliArg(wifiPassword.value)}`,
        ack: ['shell.password set to'],
      })
    }
    commands.push({
      cmd: `set pin.modbus_tx ${pinModbusTx.value}`,
      ack: ['shell.modbus_tx set to'],
    })
    commands.push({
      cmd: `set pin.modbus_rx ${pinModbusRx.value}`,
      ack: ['shell.modbus_rx set to'],
    })
    commands.push({
      cmd: `set pin.modbus_de_re ${pinModbusDeRe.value}`,
      ack: ['shell.modbus_de_re set to'],
    })
    commands.push({
      cmd: `set pin.modbus_timeout_ms ${modbusTimeoutMs.value}`,
      ack: ['shell.modbus_timeout_ms set to'],
    })
    commands.push({
      cmd: `set pin.modbus_rx_timeout_us ${modbusRxTimeoutUs.value}`,
      ack: ['shell.modbus_rx_timeout_us set to'],
    })
    commands.push({
      cmd: `set pin.modbus_inter_frame_delay_us ${modbusInterFrameDelayUs.value}`,
      ack: ['shell.modbus_inter_frame_delay_us set to'],
    })
    commands.push({
      cmd: `set pin.modbus_scan_delay_us ${modbusScanDelayUs.value}`,
      ack: ['shell.modbus_scan_delay_us set to'],
    })
    commands.push({
      cmd: `set pin.ble_enabled ${bleEnabled.value}`,
      ack: ['shell.ble_enabled set to'],
    })
    commands.push({
      cmd: `set pin.modbus_debug ${modbusDebug.value}`,
      ack: ['shell.modbus_debug set to'],
    })
    commands.push({
      cmd: `set pin.operating_mode ${operatingMode.value}`,
      ack: ['shell.operating_mode set to'],
    })

    const encoder = new TextEncoder()
    for (const item of commands) {
      termLog(`\x1b[32m>\x1b[0m ${item.cmd}`)
      await sessionWriter.write(encoder.encode(item.cmd + '\r\n'))
      const acked = await waitForAck(sessionReader, item.ack, 20000)
      if (!acked) {
        throw new Error(t('errors.timeoutWaitingAck', { expected: item.ack.join(' | ') }))
      }
      termLog(`\x1b[32m${t('log.ackReceived')}\x1b[0m`)
    }

    await leaveCurrentMode()
    status.value = 'config_done'
    termLog(`\x1b[1;32m${t('log.configCompleteRestarting')}\x1b[0m`)
    // Allow the firmware's storage task to persist the final config before hardware reset
    await sleep(500)
    await startMonitor(true)
  } catch (err: any) {
    await leaveCurrentMode()
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[1;31m${t('log.configFailed', { message: errorMessage.value })}\x1b[0m`)
  }
}

async function startMonitor(resetFirst: boolean = true) {
  if (!serialPort) {
    termLog(`\x1b[31m${t('log.noPortConnectFirst')}\x1b[0m`)
    return
  }
  await ensureNormalModeFromBootloader()
  await enterMonitorMode(resetFirst)
}

async function stopMonitor() {
  await leaveCurrentMode()
  if (status.value === 'monitoring') {
    status.value = hasPort.value ? 'flash_done' : 'idle'
  }
}

async function resetDevice() {
  if (!serialPort) {
    termLog(`\x1b[31m${t('log.noPort')}\x1b[0m`)
    return
  }
  termLog(`\x1b[33m${t('log.resettingDevice')}\x1b[0m`)
  try {
    await ensureNormalModeFromBootloader()
    await performReset(true)
    status.value = hasPort.value ? 'flash_done' : 'idle'
    termLog(`\x1b[32m${t('log.deviceReset')}\x1b[0m`)
  } catch (err: any) {
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[31m${t('log.resetFailed', { message: errorMessage.value })}\x1b[0m`)
  }
}

function escapeCliArg(value: string): string {
  const escaped = value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
  return `"${escaped}"`
}

function sleep(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms))
}

const statusLabel = computed(() => {
  switch (status.value) {
    case 'idle': return t('status.idle')
    case 'connecting': return t('status.connecting')
    case 'connected': return t('status.connected', { chip: chipName.value })
    case 'flashing': return t('status.flashing', { progress: flashProgress.value })
    case 'flash_done': return t('status.flashComplete')
    case 'configuring': return t('status.sendingConfig')
    case 'config_done': return t('status.configComplete')
    case 'monitoring': return t('status.monitoring')
    case 'error': return t('status.error')
  }
})

const statusColor = computed(() => {
  switch (status.value) {
    case 'idle': return 'text-gray-500'
    case 'connecting':
    case 'flashing':
    case 'configuring': return 'text-yellow-600'
    case 'connected':
    case 'flash_done':
    case 'config_done': return 'text-green-600'
    case 'monitoring': return 'text-cyan-600'
    case 'error': return 'text-red-600'
  }
})

function switchLocale(next: AppLocale) {
  setLocale(next)
}
</script>

<template>
  <div class="min-h-screen p-4 md:p-8 max-w-4xl mx-auto">
    <header class="mb-8">
      <div class="flex items-center justify-between gap-4">
        <h1 class="text-3xl font-bold tracking-tight text-gray-900">{{ t('meta.title') }}</h1>
        <div class="flex items-center space-x-3">
          <div class="flex rounded bg-gray-200 p-0.5 text-sm font-medium">
            <button
              type="button"
              class="rounded px-2.5 py-1 transition"
              :class="locale === 'en' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchLocale('en')"
            >
              {{ t('locale.en') }}
            </button>
            <button
              type="button"
              class="rounded px-2.5 py-1 transition"
              :class="locale === 'zh' ? 'bg-white shadow text-blue-600 font-bold' : 'text-gray-600 hover:text-gray-900'"
              @click="switchLocale('zh')"
            >
              {{ t('locale.zh') }}
            </button>
          </div>
        </div>
      </div>
      <p class="text-gray-500 mt-1">{{ t('meta.subtitle') }}</p>
    </header>

    <!-- Browser check -->
    <div v-if="!webSerialSupported" class="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
      <p class="font-semibold text-red-700">{{ t('browser.unsupportedTitle') }}</p>
      <p class="text-red-600 text-sm mt-1">{{ t('browser.unsupportedHint') }}</p>
    </div>

    <!-- Status bar -->
    <div class="bg-white border border-gray-200 rounded-lg p-3 mb-6 flex items-center justify-between shadow-sm">
      <div class="flex items-center gap-3">
        <div
          class="w-2.5 h-2.5 rounded-full"
          :class="{
            'bg-gray-400': status === 'idle',
            'bg-yellow-500 animate-pulse': status === 'connecting' || status === 'flashing' || status === 'configuring',
            'bg-green-500': status === 'connected' || status === 'flash_done' || status === 'config_done',
            'bg-cyan-500 animate-pulse': status === 'monitoring',
            'bg-red-500': status === 'error',
          }"
        />
        <span :class="statusColor" class="text-sm font-medium">{{ statusLabel }}</span>
      </div>
      <div class="flex gap-2">
        <button
          v-if="status === 'idle' || status === 'error' || status === 'config_done'"
          @click="connect"
          :disabled="!webSerialSupported"
          class="px-4 py-1.5 bg-blue-600 hover:bg-blue-700 text-white disabled:bg-gray-300 disabled:text-gray-500 rounded text-sm font-medium transition-colors"
        >
          {{ t('actions.connect') }}
        </button>
        <button
          v-if="canMonitor"
          @click="startMonitor(true)"
          class="px-4 py-1.5 bg-cyan-600 hover:bg-cyan-700 text-white rounded text-sm font-medium transition-colors"
        >
          {{ t('actions.monitor') }}
        </button>
        <button
          v-if="status === 'monitoring'"
          @click="stopMonitor"
          class="px-4 py-1.5 bg-orange-500 hover:bg-orange-600 text-white rounded text-sm font-medium transition-colors"
        >
          {{ t('actions.stop') }}
        </button>
        <button
          v-if="canReset"
          @click="resetDevice"
          class="px-4 py-1.5 bg-yellow-500 hover:bg-yellow-600 text-white rounded text-sm font-medium transition-colors"
        >
          {{ t('actions.reset') }}
        </button>
        <button
          v-if="status === 'connected' || status === 'flash_done'"
          @click="disconnect"
          class="px-4 py-1.5 bg-gray-200 hover:bg-gray-300 text-gray-700 rounded text-sm font-medium transition-colors"
        >
          {{ t('actions.disconnect') }}
        </button>
      </div>
    </div>

    <div class="grid gap-6 lg:grid-cols-2">
      <!-- Left column: Flash -->
      <section class="space-y-4">
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2">{{ t('flash.title') }}</h2>

        <p class="text-sm text-gray-500">
          {{ t('flash.githubBefore') }}
          <a href="https://github.com/vampmaker/ossm-rust/releases/" target="_blank" rel="noopener" class="text-blue-600 hover:text-blue-800 underline">{{ t('flash.githubLink') }}</a>{{ t('flash.githubAfter') }} <span class="font-mono font-medium text-gray-700">{{ t('flash.binExt') }}</span> {{ t('flash.githubAfterFile') }}
        </p>

        <!-- File upload -->
        <div
          @dragover.prevent="isDragOver = true"
          @dragleave="isDragOver = false"
          @drop.prevent="handleDrop"
          :class="[
            'border-2 border-dashed rounded-lg p-6 text-center transition-colors cursor-pointer',
            isDragOver ? 'border-blue-500 bg-blue-50' : 'border-gray-300 hover:border-gray-400',
          ]"
          @click="($refs.fileInput as HTMLInputElement).click()"
        >
          <input ref="fileInput" type="file" accept=".bin" class="hidden" @change="handleFileSelect" />
          <div v-if="firmwareFile" class="text-sm">
            <p class="font-medium text-green-700">{{ firmwareFile.name }}</p>
            <p class="text-gray-500 mt-1">{{ (firmwareFile.size / 1024).toFixed(1) }} KB</p>
          </div>
          <div v-else class="text-gray-400 text-sm">
            <p class="font-medium">{{ t('flash.dropHint') }}</p>
          </div>
        </div>

        <!-- Flash address -->
        <div class="flex items-center gap-3">
          <label class="text-sm text-gray-600 whitespace-nowrap">{{ t('flash.addressLabel') }}</label>
          <input
            v-model="flashAddress"
            class="bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono w-28 focus:border-blue-500 focus:outline-none"
            :placeholder="t('flash.addressPlaceholder')"
          />
        </div>

        <!-- Flash button + progress -->
        <button
          @click="flash"
          :disabled="!canFlash"
          class="w-full py-2.5 bg-green-600 hover:bg-green-700 text-white disabled:bg-gray-300 disabled:text-gray-500 rounded font-medium transition-colors"
        >
          {{ status === 'flashing' ? t('actions.flashing', { progress: flashProgress }) : t('actions.flash') }}
        </button>

        <div v-if="status === 'flashing' || flashProgress > 0" class="w-full bg-gray-200 rounded-full h-2.5 overflow-hidden">
          <div
            class="bg-green-500 h-full transition-all duration-200"
            :style="{ width: `${flashProgress}%` }"
          />
        </div>

        <!-- Error -->
        <div v-if="status === 'error' && errorMessage" class="bg-red-50 border border-red-200 rounded p-3 text-sm text-red-700">
          {{ errorMessage }}
        </div>
      </section>

      <!-- Right column: Configure -->
      <section class="space-y-4">
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2">{{ t('config.title') }}</h2>

        <div class="space-y-3">
          <!-- WiFi -->
          <fieldset class="space-y-2">
            <div class="flex items-center justify-between">
              <legend class="text-sm font-medium text-gray-700">{{ t('config.wifi.legend') }}</legend>
              <div class="flex items-center space-x-2">
                <input
                  id="flasher-wifi-enabled"
                  type="checkbox"
                  v-model="wifiEnabled"
                  class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                <label for="flasher-wifi-enabled" class="text-xs font-medium text-gray-700">
                  {{ t('config.wifi.enable') }}
                </label>
              </div>
            </div>
            <div v-if="wifiEnabled" class="space-y-2">
              <div>
                <label class="text-xs text-gray-500">{{ t('config.wifi.ssid') }}</label>
                <input
                  v-model="wifiSsid"
                  maxlength="32"
                  :placeholder="t('config.wifi.ssidPlaceholder')"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.wifi.password') }}</label>
                <input
                  v-model="wifiPassword"
                  type="password"
                  maxlength="64"
                  :placeholder="t('config.wifi.passwordPlaceholder')"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
                />
              </div>
            </div>
          </fieldset>

          <!-- Operating mode -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">{{ t('config.operatingMode.legend') }}</legend>
            <select
              v-model="operatingMode"
              class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
            >
              <option value="servo">{{ t('config.operatingMode.servo') }}</option>
              <option value="rtu_relay">{{ t('config.operatingMode.rtuRelay') }}</option>
              <option value="rs485">{{ t('config.operatingMode.rs485') }}</option>
            </select>
            <p class="text-xs text-gray-500">{{ t('config.operatingMode.hint') }}</p>
          </fieldset>

          <!-- GPIO Pins -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">{{ t('config.gpio.legend') }}</legend>
            <div class="grid grid-cols-3 gap-2">
              <div>
                <label class="text-xs text-gray-500">{{ t('config.gpio.modbusTx') }}</label>
                <input
                  v-model.number="pinModbusTx"
                  type="number"
                  min="0"
                  max="48"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.gpio.modbusRx') }}</label>
                <input
                  v-model.number="pinModbusRx"
                  type="number"
                  min="0"
                  max="48"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.gpio.modbusDeRe') }}</label>
                <input
                  v-model.number="pinModbusDeRe"
                  type="number"
                  min="0"
                  max="48"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
            </div>
          </fieldset>

          <!-- Motor / Modbus -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">{{ t('config.modbus.legend') }}</legend>
            <div class="grid grid-cols-2 gap-2">
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.baudRate') }}</label>
                <input
                  value="115200"
                  disabled
                  class="w-full bg-gray-100 border border-gray-200 rounded px-3 py-1.5 text-sm font-mono text-gray-400 cursor-not-allowed"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.deviceId') }}</label>
                <input
                  value="1"
                  disabled
                  class="w-full bg-gray-100 border border-gray-200 rounded px-3 py-1.5 text-sm font-mono text-gray-400 cursor-not-allowed"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.readTimeout') }} <span class="text-gray-400">{{ t('config.modbus.readTimeoutHint') }}</span></label>
                <input
                  v-model.number="modbusTimeoutMs"
                  type="number"
                  min="0"
                  max="1000"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.rxInterByte') }} <span class="text-gray-400">{{ t('config.modbus.rxInterByteHint') }}</span></label>
                <input
                  v-model.number="modbusRxTimeoutUs"
                  type="number"
                  min="0"
                  max="200000"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.interFrameQuiet') }} <span class="text-gray-400">{{ t('config.modbus.interFrameQuietHint') }}</span></label>
                <input
                  v-model.number="modbusInterFrameDelayUs"
                  type="number"
                  min="0"
                  max="200000"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">{{ t('config.modbus.scanDelay') }} <span class="text-gray-400">{{ t('config.modbus.scanDelayHint') }}</span></label>
                <input
                  v-model.number="modbusScanDelayUs"
                  type="number"
                  min="0"
                  max="200000"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
            </div>
            <div class="mt-3 flex flex-col gap-2 pt-2 border-t border-gray-100">
              <div class="flex items-center space-x-2">
                <input
                  id="flasher-ble-enabled"
                  type="checkbox"
                  v-model="bleEnabled"
                  class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                />
                <label for="flasher-ble-enabled" class="text-xs font-medium text-gray-700">
                  {{ t('config.ble.enable') }}
                </label>
              </div>
              <div class="flex flex-col gap-1">
                <div class="flex items-center space-x-2">
                  <input
                    id="flasher-modbus-debug"
                    type="checkbox"
                    v-model="modbusDebug"
                    class="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                  />
                  <label for="flasher-modbus-debug" class="text-xs font-medium text-gray-700">
                    {{ t('config.modbus.debug') }}
                  </label>
                </div>
                <p class="pl-6 text-[11px] text-amber-700">
                  {{ t('config.modbus.debugHint') }}
                </p>
              </div>
            </div>
          </fieldset>
        </div>

        <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <button
            @click="sendConfig"
            :disabled="!canConfigure"
            class="w-full py-2.5 bg-indigo-600 hover:bg-indigo-700 text-white disabled:bg-gray-300 disabled:text-gray-500 rounded font-medium transition-colors"
          >
            {{ status === 'configuring' ? t('actions.sending') : t('actions.sendConfig') }}
          </button>
          <button
            @click="resetDeviceConfig"
            class="w-full py-2.5 bg-orange-500 hover:bg-orange-600 text-white rounded font-medium transition-colors"
          >
            {{ t('actions.resetDefaults') }}
          </button>
        </div>

        <p v-if="status === 'config_done' || status === 'monitoring'" class="text-green-600 text-sm text-center">
          {{ t('config.sentMonitoring') }}
        </p>
      </section>
    </div>

    <!-- Logs -->
    <section class="mt-6 grid gap-4 lg:grid-cols-2">
      <div>
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2 mb-2">{{ t('terminals.console') }}</h2>
        <div
          id="flasher-terminal"
          ref="terminalEl"
          class="border border-gray-200 rounded-lg overflow-hidden h-72 shadow-inner"
        />
      </div>
      <div>
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2 mb-2">{{ t('terminals.serial') }}</h2>
        <div
          ref="serialTerminalEl"
          class="border border-gray-200 rounded-lg overflow-hidden h-72 shadow-inner"
        />
      </div>
    </section>
  </div>
</template>
