<script setup lang="ts">
/// <reference types="w3c-web-serial" />
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { ESPLoader, Transport } from 'esptool-js'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'

type Status = 'idle' | 'connecting' | 'connected' | 'flashing' | 'flash_done' | 'configuring' | 'config_done' | 'monitoring' | 'error'
type SerialMode = 'idle' | 'bootloader' | 'config' | 'monitor'

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

function serialLog(line: string) {
  serialXterm?.writeln(line)
}

onMounted(() => {
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
    xterm.writeln('\x1b[90mWaiting for activity...\x1b[0m')
  }

  if (serialTerminalEl.value) {
    serialXterm = new Terminal({
      cursorBlink: false,
      disableStdin: true,
      scrollback: 5000,
      fontSize: 13,
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      theme,
    })
    serialFitAddon = new FitAddon()
    serialXterm.loadAddon(serialFitAddon)
    serialXterm.open(serialTerminalEl.value)
    serialFitAddon.fit()
    serialXterm.writeln('\x1b[90mWaiting for serial data...\x1b[0m')
  }

  resizeObserver = new ResizeObserver(() => {
    fitAddon?.fit()
    serialFitAddon?.fit()
  })
  if (terminalEl.value) resizeObserver.observe(terminalEl.value)
  if (serialTerminalEl.value) resizeObserver.observe(serialTerminalEl.value)
})

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
  if (!serialPort) throw new Error('No serial port available.')
  if (!isPortOpenNoLock()) {
    await serialPort.open({ baudRate })
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
    try { await transport.disconnect() } catch { /* ignore */ }
    transport = null
    esploader = null
  }
  if (serialMode.value === 'bootloader') {
    serialMode.value = 'idle'
  }
}

async function ensureNormalModeFromBootloader() {
  if (esploader) {
    termLog('\x1b[33mDevice is in bootloader mode. Resetting to normal mode...\x1b[0m')
    try {
      await esploader.after('hard_reset')
    } catch {
      termLog('\x1b[90mHard reset via esptool failed. Continuing...\\x1b[0m')
    }
    await cleanupEsptool()
    await sleep(800)
  }
}

async function enterConfigMode() {
  await withSerialLock(async () => {
    monitorRequested = false
    monitorToken += 1
    await releaseIoNoLock()
    await closePortNoLock()
    await openPortNoLock(115200)
    if (!serialPort?.readable || !serialPort?.writable) throw new Error('Serial port streams unavailable.')
    sessionReader = serialPort.readable.getReader()
    sessionWriter = serialPort.writable.getWriter()
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
    termLog('\x1b[1;36m--- Monitor started (115200 baud) ---\x1b[0m')
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
        if (!serialPort?.readable) throw new Error('Readable stream unavailable.')
        const reader = serialPort.readable.getReader()
        sessionReader = reader
        return reader
      })
      if (!activeReader) throw new Error('Failed to acquire monitor reader.')

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
        termLog('\x1b[33mMonitor stream ended. Reconnecting...\x1b[0m')
      } else {
        termLog(`\x1b[31mMonitor error:\x1b[0m ${err?.message || err}`)
      }
      reconnectAttempts += 1
      if (reconnectAttempts > maxReconnectAttempts) {
        termLog('\x1b[31mMonitor stopped after repeated reconnect failures.\x1b[0m')
        break
      }
      termLog(`\x1b[33mReconnecting monitor (${reconnectAttempts}/${maxReconnectAttempts})...\x1b[0m`)
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
    termLog('\x1b[1;36m--- Monitor stopped ---\x1b[0m')
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
  termLog(`\x1b[32mLoaded firmware:\x1b[0m ${file.name} \x1b[90m(${(file.size / 1024).toFixed(1)} KB)\x1b[0m`)
}

const espTerminal = {
  clean() { serialXterm?.clear() },
  writeLine(data: string) { serialXterm?.writeln(data) },
  write(data: string) { serialXterm?.write(data) },
}

async function connect() {
  if (!webSerialSupported.value) return
  status.value = 'connecting'
  errorMessage.value = ''
  termLog('\x1b[90mRequesting serial port...\x1b[0m')
  try {
    const port = await navigator.serial.requestPort()
    setPort(port)
    transport = new Transport(port, true)
    esploader = new ESPLoader({
      transport,
      baudrate: 115200,
      romBaudrate: 115200,
      terminal: espTerminal,
    })
    termLog('\x1b[33mConnecting to ESP device...\x1b[0m')
    const chip = await esploader.main()
    chipName.value = chip
    serialMode.value = 'bootloader'
    status.value = 'connected'
    termLog(`\x1b[1;32mConnected: ${chip}\x1b[0m`)
  } catch (err: any) {
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[1;31mConnection failed:\x1b[0m ${errorMessage.value}`)
  }
}

async function disconnect() {
  await leaveCurrentMode()
  await cleanupEsptool()
  setPort(null)
  chipName.value = ''
  status.value = 'idle'
  termLog('\x1b[90mDisconnected.\x1b[0m')
}

async function flash() {
  if (!esploader || !firmwareDataStr.value) return
  const address = parseInt(flashAddress.value, 16)
  if (isNaN(address)) {
    termLog('\x1b[31mInvalid flash address.\x1b[0m')
    return
  }
  status.value = 'flashing'
  flashProgress.value = 0
  termLog(`\x1b[33mFlashing\x1b[0m ${firmwareFile.value?.name} at \x1b[36m0x${address.toString(16)}\x1b[0m...`)
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
    termLog('\x1b[1;32mFlash complete.\x1b[0m Resetting device...')
    await esploader.after('hard_reset')
    await cleanupEsptool()
    serialMode.value = 'idle'
    status.value = 'flash_done'
    termLog('\x1b[32mDevice reset. Ready for configuration.\x1b[0m')
  } catch (err: any) {
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[1;31mFlash failed:\x1b[0m ${errorMessage.value}`)
  }
}

async function waitForBoot(reader: ReadableStreamDefaultReader<Uint8Array>, timeoutMs: number): Promise<boolean> {
  const decoder = new TextDecoder()
  let buffer = ''
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const remaining = Math.max(0, deadline - Date.now())
    const result = await Promise.race([
      reader.read(),
      sleep(Math.min(remaining, 500)).then(() => null),
    ])
    if (result === null) continue
    if (result.done) break
    if (result.value) {
      const text = decoder.decode(result.value, { stream: true })
      buffer += text
      const lines = text.split('\n')
      for (const line of lines) {
        const trimmed = line.trim()
        if (trimmed) serialLog(`\x1b[36m[device]\x1b[0m ${trimmed}`)
      }
      if (buffer.includes('Hello, world!') || buffer.includes('ossm')) {
        termLog('\x1b[1;32mDevice boot detected.\x1b[0m')
        return true
      }
    }
  }
  const tail = decoder.decode()
  if (tail) serialXterm?.write(tail)
  return false
}

async function waitForAck(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  expectedSubstrings: string[],
  timeoutMs: number,
): Promise<boolean> {
  const decoder = new TextDecoder()
  const deadline = Date.now() + timeoutMs
  let buffer = ''

  while (Date.now() < deadline) {
    const remaining = Math.max(0, deadline - Date.now())
    const result = await Promise.race([
      reader.read(),
      sleep(Math.min(remaining, 500)).then(() => null),
    ])

    if (result === null) continue
    if (result.done) {
      throw new Error(`Stream ended while waiting for ACK: ${expectedSubstrings.join(' | ')}`)
    }

    if (result.value && result.value.length > 0) {
      const text = decoder.decode(result.value, { stream: true })
      buffer += text

      const lines = text.split('\n')
      for (const line of lines) {
        const trimmed = line.trim()
        if (trimmed) serialLog(`\x1b[36m[device]\x1b[0m ${trimmed}`)
      }

      if (expectedSubstrings.some((s) => buffer.includes(s))) {
        return true
      }
    }
  }

  return false
}

async function sendConfig() {
  status.value = 'configuring'
  errorMessage.value = ''
  termLog('\x1b[33mPreparing to send configuration...\x1b[0m')
  try {
    if (!serialPort) throw new Error('No serial port available. Please use Connect first.')
    await ensureNormalModeFromBootloader()
    await enterConfigMode()
    if (!sessionReader || !sessionWriter) throw new Error('Failed to open serial streams.')

    termLog('\x1b[90mToggling DTR/RTS to reset device...\x1b[0m')
    await pulseResetSignalsNoLock(serialPort!)
    termLog('\x1b[33mWaiting for device to boot (up to 15s)...\x1b[0m')
    const booted = await waitForBoot(sessionReader, 15000)
    if (!booted) {
      termLog('\x1b[33mWarning: Did not detect boot message, sending config anyway...\x1b[0m')
    }

    const commands: Array<{ cmd: string; ack: string[] }> = []
    if (wifiSsid.value) {
      commands.push({
        cmd: `set_wifi_ssid ${wifiSsid.value}`,
        ack: ['SSID saved:'],
      })
    }
    if (wifiPassword.value) {
      commands.push({
        cmd: `set_wifi_password ${wifiPassword.value}`,
        ack: ['Password saved:'],
      })
    }
    commands.push({
      cmd: `set_pin_modbus_tx ${pinModbusTx.value}`,
      ack: ['Modbus TX pin set to'],
    })
    commands.push({
      cmd: `set_pin_modbus_rx ${pinModbusRx.value}`,
      ack: ['Modbus RX pin set to'],
    })
    commands.push({
      cmd: `set_pin_modbus_de_re ${pinModbusDeRe.value}`,
      ack: ['Modbus DE/RE pin set to'],
    })

    const encoder = new TextEncoder()
    for (const item of commands) {
      termLog(`\x1b[32m>\x1b[0m ${item.cmd}`)
      await sessionWriter.write(encoder.encode(item.cmd + '\r\n'))
      const acked = await waitForAck(sessionReader, item.ack, 5000)
      if (!acked) {
        throw new Error(`Timeout waiting for ACK: ${item.ack.join(' | ')}`)
      }
      termLog('\x1b[32mACK received.\x1b[0m')
    }

    termLog('\x1b[32mConfiguration sent.\x1b[0m Resetting device to apply...')
    await sessionWriter.write(encoder.encode('reset\r\n'))
    const resetAcked = await waitForAck(sessionReader, ['Restarting device...'], 4000)
    if (!resetAcked) {
      termLog('\x1b[33mReset ACK not seen; continuing with monitor attach.\x1b[0m')
    }

    await leaveCurrentMode()
    status.value = 'config_done'
    termLog('\x1b[1;32mConfiguration complete. Device is restarting with new settings.\x1b[0m')
    await sleep(500)
    await startMonitor(true)
  } catch (err: any) {
    await leaveCurrentMode()
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[1;31mConfiguration failed:\x1b[0m ${errorMessage.value}`)
  }
}

async function startMonitor(resetFirst: boolean = true) {
  if (!serialPort) {
    termLog('\x1b[31mNo serial port available. Please use Connect first.\x1b[0m')
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
    termLog('\x1b[31mNo serial port available.\x1b[0m')
    return
  }
  termLog('\x1b[33mResetting device...\x1b[0m')
  try {
    await ensureNormalModeFromBootloader()
    await performReset(true)
    status.value = hasPort.value ? 'flash_done' : 'idle'
    termLog('\x1b[32mDevice reset.\x1b[0m')
  } catch (err: any) {
    status.value = 'error'
    errorMessage.value = err?.message || String(err)
    termLog(`\x1b[31mReset failed:\x1b[0m ${errorMessage.value}`)
  }
}

function sleep(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms))
}

const statusLabel = computed(() => {
  switch (status.value) {
    case 'idle': return 'Not connected'
    case 'connecting': return 'Connecting...'
    case 'connected': return `Connected to ${chipName.value}`
    case 'flashing': return `Flashing... ${flashProgress.value}%`
    case 'flash_done': return 'Flash complete'
    case 'configuring': return 'Sending configuration...'
    case 'config_done': return 'Configuration complete'
    case 'monitoring': return 'Monitoring serial output'
    case 'error': return 'Error'
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
</script>

<template>
  <div class="min-h-screen p-4 md:p-8 max-w-4xl mx-auto">
    <header class="mb-8">
      <h1 class="text-3xl font-bold tracking-tight text-gray-900">OSSM Flasher</h1>
      <p class="text-gray-500 mt-1">Flash firmware and configure your OSSM device over USB</p>
    </header>

    <!-- Browser check -->
    <div v-if="!webSerialSupported" class="bg-red-50 border border-red-200 rounded-lg p-4 mb-6">
      <p class="font-semibold text-red-700">Web Serial API not supported</p>
      <p class="text-red-600 text-sm mt-1">Please use Google Chrome or Microsoft Edge (version 89+).</p>
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
          Connect
        </button>
        <button
          v-if="canMonitor"
          @click="startMonitor(true)"
          class="px-4 py-1.5 bg-cyan-600 hover:bg-cyan-700 text-white rounded text-sm font-medium transition-colors"
        >
          Monitor
        </button>
        <button
          v-if="status === 'monitoring'"
          @click="stopMonitor"
          class="px-4 py-1.5 bg-orange-500 hover:bg-orange-600 text-white rounded text-sm font-medium transition-colors"
        >
          Stop
        </button>
        <button
          v-if="canReset"
          @click="resetDevice"
          class="px-4 py-1.5 bg-yellow-500 hover:bg-yellow-600 text-white rounded text-sm font-medium transition-colors"
        >
          Reset
        </button>
        <button
          v-if="status === 'connected' || status === 'flash_done'"
          @click="disconnect"
          class="px-4 py-1.5 bg-gray-200 hover:bg-gray-300 text-gray-700 rounded text-sm font-medium transition-colors"
        >
          Disconnect
        </button>
      </div>
    </div>

    <div class="grid gap-6 lg:grid-cols-2">
      <!-- Left column: Flash -->
      <section class="space-y-4">
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2">Flash Firmware</h2>

        <p class="text-sm text-gray-500">
          Download the latest firmware from
          <a href="https://github.com/vampmaker/ossm-rust/releases/" target="_blank" rel="noopener" class="text-blue-600 hover:text-blue-800 underline">GitHub Releases</a>,
          then upload the <code class="text-xs bg-gray-100 px-1 py-0.5 rounded">.bin</code> file below.
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
            <p class="font-medium">Drop .bin file here or click to browse</p>
          </div>
        </div>

        <!-- Flash address -->
        <div class="flex items-center gap-3">
          <label class="text-sm text-gray-600 whitespace-nowrap">Flash address:</label>
          <input
            v-model="flashAddress"
            class="bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono w-28 focus:border-blue-500 focus:outline-none"
            placeholder="0x0"
          />
        </div>

        <!-- Flash button + progress -->
        <button
          @click="flash"
          :disabled="!canFlash"
          class="w-full py-2.5 bg-green-600 hover:bg-green-700 text-white disabled:bg-gray-300 disabled:text-gray-500 rounded font-semibold transition-colors"
        >
          {{ status === 'flashing' ? `Flashing... ${flashProgress}%` : 'Flash Firmware' }}
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
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2">Device Configuration</h2>

        <div class="space-y-3">
          <!-- WiFi -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">WiFi</legend>
            <div>
              <label class="text-xs text-gray-500">SSID</label>
              <input
                v-model="wifiSsid"
                maxlength="32"
                placeholder="Your WiFi network"
                class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
              />
            </div>
            <div>
              <label class="text-xs text-gray-500">Password</label>
              <input
                v-model="wifiPassword"
                type="password"
                maxlength="64"
                placeholder="WiFi password"
                class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm focus:border-blue-500 focus:outline-none"
              />
            </div>
          </fieldset>

          <!-- GPIO Pins -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">GPIO Pins</legend>
            <div class="grid grid-cols-3 gap-2">
              <div>
                <label class="text-xs text-gray-500">Modbus TX</label>
                <input
                  v-model.number="pinModbusTx"
                  type="number"
                  min="0"
                  max="48"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">Modbus RX</label>
                <input
                  v-model.number="pinModbusRx"
                  type="number"
                  min="0"
                  max="48"
                  class="w-full bg-white border border-gray-300 rounded px-3 py-1.5 text-sm font-mono focus:border-blue-500 focus:outline-none"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">Modbus DE/RE</label>
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

          <!-- Motor / Modbus (read-only) -->
          <fieldset class="space-y-2">
            <legend class="text-sm font-medium text-gray-700">Motor / Modbus <span class="text-xs text-gray-400">(read-only)</span></legend>
            <div class="grid grid-cols-2 gap-2">
              <div>
                <label class="text-xs text-gray-500">Baud Rate</label>
                <input
                  value="115200"
                  disabled
                  class="w-full bg-gray-100 border border-gray-200 rounded px-3 py-1.5 text-sm font-mono text-gray-400 cursor-not-allowed"
                />
              </div>
              <div>
                <label class="text-xs text-gray-500">Device ID</label>
                <input
                  value="1"
                  disabled
                  class="w-full bg-gray-100 border border-gray-200 rounded px-3 py-1.5 text-sm font-mono text-gray-400 cursor-not-allowed"
                />
              </div>
            </div>
          </fieldset>
        </div>

        <button
          @click="sendConfig"
          :disabled="!canConfigure"
          class="w-full py-2.5 bg-indigo-600 hover:bg-indigo-700 text-white disabled:bg-gray-300 disabled:text-gray-500 rounded font-semibold transition-colors"
        >
          {{ status === 'configuring' ? 'Sending...' : 'Send Configuration' }}
        </button>

        <p v-if="status === 'config_done' || status === 'monitoring'" class="text-green-600 text-sm text-center">
          Configuration sent. Monitoring device output.
        </p>
      </section>
    </div>

    <!-- Logs -->
    <section class="mt-6 grid gap-4 lg:grid-cols-2">
      <div>
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2 mb-2">Console</h2>
        <div
          ref="terminalEl"
          class="border border-gray-200 rounded-lg overflow-hidden h-72 shadow-inner"
        />
      </div>
      <div>
        <h2 class="text-lg font-semibold border-b border-gray-200 pb-2 mb-2">Serial</h2>
        <div
          ref="serialTerminalEl"
          class="border border-gray-200 rounded-lg overflow-hidden h-72 shadow-inner"
        />
      </div>
    </section>
  </div>
</template>
