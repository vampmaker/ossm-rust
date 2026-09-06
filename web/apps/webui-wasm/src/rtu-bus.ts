/** Modbus RTU CRC-16 and expected response length (host USB / WS pipe). */

export function crc16(buf: Uint8Array, length = buf.length): number {
  let crc = 0xffff
  for (let pos = 0; pos < length; pos++) {
    crc ^= buf[pos]!
    for (let i = 0; i < 8; i++) {
      if (crc & 1) crc = (crc >> 1) ^ 0xa001
      else crc >>= 1
    }
  }
  return crc & 0xffff
}

export function expectedResponseLen(req: Uint8Array): number {
  if (req.length >= 6) {
    const fc = req[1]!
    if (fc === 0x03 || fc === 0x04) {
      const qty = (req[4]! << 8) | req[5]!
      return 5 + qty * 2
    }
    return 8
  }
  return 8
}

export function crcOk(frame: Uint8Array): boolean {
  if (frame.length < 4) return false
  const crc = crc16(frame, frame.length - 2)
  return frame[frame.length - 2] === (crc & 0xff) && frame[frame.length - 1] === (crc >> 8)
}

export interface ByteBus {
  exchange(req: Uint8Array, timeoutMs: number): Promise<Uint8Array>
  close(): Promise<void>
}

function extractFrame(buf: Uint8Array, slave: number, expected: number, fc: number): Uint8Array | null {
  for (let off = 0; off + 4 <= buf.length; off++) {
    const len = buf[off + 1]! & 0x80 ? 5 : expected
    if (off + len > buf.length) continue
    const slice = buf.subarray(off, off + len)
    if (slice[0] !== slave) continue
    const gotFc = slice[1]!
    if (gotFc !== fc && gotFc !== (fc | 0x80)) continue
    if (crcOk(slice)) return new Uint8Array(slice)
  }
  return null
}

export async function openSerialPort(port: SerialPort, baud: number): Promise<ByteBus> {
  try {
    await port.setSignals?.({ dataTerminalReady: false, requestToSend: false })
  } catch {
    /* ignore */
  }
  if (!port.readable) {
    await port.open({ baudRate: baud })
    try {
      await port.setSignals?.({ dataTerminalReady: false, requestToSend: false })
    } catch {
      /* ignore */
    }
  }
  const reader = port.readable!.getReader()
  const writer = port.writable!.getWriter()
  let rx = new Uint8Array(0)
  let closed = false
  const pump = (async () => {
    try {
      while (!closed) {
        const { value, done } = await reader.read()
        if (done) break
        if (value && value.length) {
          const next = new Uint8Array(rx.length + value.length)
          next.set(rx)
          next.set(value, rx.length)
          rx = next
        }
      }
    } catch {
      /* stream closed */
    }
  })()
  return {
    async exchange(req, timeoutMs) {
      rx = new Uint8Array(0)
      await writer.write(req)
      const slave = req[0] ?? 1
      const fc = (req[1] ?? 0x10) & 0x7f
      const expected = expectedResponseLen(req)
      const deadline = performance.now() + timeoutMs
      while (performance.now() < deadline) {
        const hit = extractFrame(rx, slave, expected, fc)
        if (hit) return hit
        await new Promise((r) => setTimeout(r, 2))
      }
      throw new Error(`no CRC-valid response (${rx.length} bytes)`)
    },
    async close() {
      closed = true
      try {
        await reader.cancel()
      } catch {
        /* ignore */
      }
      try {
        writer.releaseLock()
      } catch {
        /* ignore */
      }
      try {
        await port.close()
      } catch {
        /* ignore */
      }
      await pump
    },
  }
}

const PKT_TX = 0
const PKT_RX = 1
const PKT_CFG = 2

function packRs485(typ: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(3 + payload.length)
  out[0] = typ
  out[1] = payload.length & 0xff
  out[2] = (payload.length >> 8) & 0xff
  out.set(payload, 3)
  return out
}

function unpackRs485(frame: Uint8Array): { typ: number; payload: Uint8Array } | null {
  if (frame.length < 3) return null
  const typ = frame[0]!
  const len = frame[1]! | (frame[2]! << 8)
  if (frame.length < 3 + len) return null
  return { typ, payload: frame.subarray(3, 3 + len) }
}

export async function openModbusWs(url: string): Promise<ByteBus> {
  const ws = new WebSocket(url)
  ws.binaryType = 'arraybuffer'
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve()
    ws.onerror = () => reject(new Error(`ws connect failed: ${url}`))
  })
  let pending: Uint8Array | null = null
  let waiters: Array<(frame: Uint8Array) => void> = []
  ws.onmessage = (ev) => {
    if (!(ev.data instanceof ArrayBuffer)) return
    const frame = new Uint8Array(ev.data)
    if (waiters.length) {
      waiters.shift()!(frame)
    } else {
      pending = frame
    }
  }
  return {
    async exchange(req, timeoutMs) {
      pending = null
      waiters = []
      if (!crcOk(req)) {
        throw new Error('request CRC invalid')
      }
      const frame = await new Promise<Uint8Array>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('modbus ws timeout')), timeoutMs)
        const onFrame = (resp: Uint8Array) => {
          clearTimeout(timer)
          resolve(resp)
        }
        if (pending) {
          const p = pending
          pending = null
          onFrame(p)
        } else {
          waiters.push(onFrame)
        }
        ws.send(req)
      })
      if (!crcOk(frame)) {
        throw new Error('response CRC invalid')
      }
      return frame
    },
    async close() {
      ws.close()
    },
  }
}

export async function openRs485Ws(url: string, baud: number): Promise<ByteBus> {
  const ws = new WebSocket(url)
  ws.binaryType = 'arraybuffer'
  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve()
    ws.onerror = () => reject(new Error(`ws connect failed: ${url}`))
  })
  ws.send(packRs485(PKT_CFG, new TextEncoder().encode(JSON.stringify({ baud }))))
  let rx = new Uint8Array(0)
  ws.onmessage = (ev) => {
    if (!(ev.data instanceof ArrayBuffer)) return
    const pkt = unpackRs485(new Uint8Array(ev.data))
    if (!pkt || pkt.typ !== PKT_RX) return
    const next = new Uint8Array(rx.length + pkt.payload.length)
    next.set(rx)
    next.set(pkt.payload, rx.length)
    rx = next
  }
  return {
    async exchange(req, timeoutMs) {
      rx = new Uint8Array(0)
      ws.send(packRs485(PKT_TX, req))
      const slave = req[0] ?? 1
      const fc = (req[1] ?? 0x10) & 0x7f
      const expected = expectedResponseLen(req)
      const deadline = performance.now() + timeoutMs
      while (performance.now() < deadline) {
        const hit = extractFrame(rx, slave, expected, fc)
        if (hit) return hit
        await new Promise((r) => setTimeout(r, 2))
      }
      throw new Error(`no CRC-valid response (${rx.length} bytes)`)
    },
    async close() {
      ws.close()
    },
  }
}
