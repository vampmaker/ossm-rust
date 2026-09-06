<template>
  <svg id="wires" ref="svgRef"></svg>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch } from 'vue'

const props = defineProps<{
  mode: 'std' | 'esp'
}>()

const svgRef = ref<SVGSVGElement | null>(null)

interface Point {
  x: number
  y: number
}

function updateWires() {
  const containerEl = document.getElementById('diagram')
  const svg = svgRef.value
  if (!containerEl || !svg) return

  const svgEl = svg
  const svgRect = svgEl.getBoundingClientRect()
  svgEl.setAttribute('width', svgRect.width.toString())
  svgEl.setAttribute('height', svgRect.height.toString())
  svgEl.innerHTML = ''

  function getPt(id: string, side: 'right' | 'left' | 'top' | 'bottom' | 'center'): Point {
    const el = document.getElementById(id)
    if (!el) return { x: 0, y: 0 }
    const r = el.getBoundingClientRect()
    let x = r.left - svgRect.left + r.width / 2
    let y = r.top - svgRect.top + r.height / 2
    if (side === 'right') x = r.right - svgRect.left
    if (side === 'left') x = r.left - svgRect.left
    if (side === 'top') y = r.top - svgRect.top
    if (side === 'bottom') y = r.bottom - svgRect.top
    return { x, y }
  }

  function addPath(d: string, color: string, width = 3.5, dash?: string) {
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
    path.setAttribute('d', d)
    path.setAttribute('fill', 'none')
    path.setAttribute('stroke', color)
    path.setAttribute('stroke-width', width.toString())
    path.setAttribute('stroke-linecap', 'round')
    path.setAttribute('stroke-linejoin', 'round')
    if (dash) path.setAttribute('stroke-dasharray', dash)
    svgEl.appendChild(path)
  }

  function addTerminalDot(pt: Point, color: string, r = 2.0) {
    const circle = document.createElementNS('http://www.w3.org/2000/svg', 'circle')
    circle.setAttribute('cx', pt.x.toString())
    circle.setAttribute('cy', pt.y.toString())
    circle.setAttribute('r', r.toString())
    circle.setAttribute('fill', color)
    circle.setAttribute('stroke', '#ffffff')
    circle.setAttribute('stroke-width', '0.75')
    svgEl.appendChild(circle)
  }

  const cardAdapter = document.getElementById('card-adapter')
  if (!cardAdapter) return
  const adapterRect = cardAdapter.getBoundingClientRect()

  // 1. Top sky corridor ABOVE top card border
  // Lowered red 24V line (inner/lower track) and raised black GND line (outer/upper track)
  // to completely eliminate crossover between 24V and GND.
  const topCardY = adapterRect.top - svgRect.top
  const redY = props.mode === 'std' ? topCardY - 16 : topCardY - 18
  const blackY = props.mode === 'std' ? topCardY - 30 : topCardY - 32

  // Wire 1: +24V from DC Jack to Motor Front Pin 1 (+V)
  const p1 = getPt('pin-dc-24v', 'right')
  const p2 = getPt('pin-motor-24v', 'center')
  addPath(`M ${p1.x} ${p1.y} L ${p1.x + 12} ${p1.y} L ${p1.x + 12} ${redY} L ${p2.x + 18} ${redY} L ${p2.x + 18} ${p2.y} L ${p2.x} ${p2.y}`, '#ef4444', 3.5)
  addTerminalDot(p2, '#ef4444', 2.0)

  // Wire 2: GND from DC Jack to Motor Front Pin 2 (GND)
  const g1 = getPt('pin-dc-gnd', 'right')
  const g2 = getPt('pin-motor-gnd', 'center')
  addPath(`M ${g1.x} ${g1.y} L ${g1.x + 20} ${g1.y} L ${g1.x + 20} ${blackY} L ${g2.x + 26} ${blackY} L ${g2.x + 26} ${g2.y} L ${g2.x} ${g2.y}`, '#475569', 3.5)
  addTerminalDot(g2, '#475569', 2.0)

  // Motor 2 / Rear Motor element
  const motor2El = document.querySelector('#card-back .motor-unit')
  if (!motor2El) return
  const motor2Rect = motor2El.getBoundingClientRect()

  if (props.mode === 'std') {
    // ==========================================
    // PATH A WIRING (std: Desktop / USB-RS485)
    // ==========================================
    const subtitle2El = document.querySelector('#card-back .card-subtitle')
    const subtitle2Rect = subtitle2El ? subtitle2El.getBoundingClientRect() : motor2Rect
    const gap2TopY = (subtitle2Rect.bottom - svgRect.top + motor2Rect.top - svgRect.top) / 2

    // Wire 3: 5V RS-485 transceiver power from USB-RS485 to Motor Back Pin 10 (5V)
    // 5V is topmost pin, uses outermost track (+40) so it never crosses GND/A/B on its way down
    const e5 = getPt('pin-rs485-5v', 'right')
    const m5 = getPt('pin-motor-5v', 'center')
    addPath(`M ${e5.x} ${e5.y} L ${e5.x + 40} ${e5.y} L ${e5.x + 40} ${gap2TopY} L ${m5.x} ${gap2TopY} L ${m5.x} ${m5.y}`, '#f97316', 3)
    addTerminalDot(m5, '#f97316', 1.8)

    // Open corridor below Motor 2
    const midTerminalGapY = (motor2Rect.bottom - svgRect.top) + 22

    // Wire 4: GND/COM to Motor Back Pin 6 (COM)
    const eg1 = getPt('pin-rs485-gnd', 'right')
    const mc = getPt('pin-motor-com', 'center')
    addPath(`M ${eg1.x} ${eg1.y} L ${eg1.x + 32} ${eg1.y} L ${eg1.x + 32} ${midTerminalGapY - 14} L ${mc.x} ${midTerminalGapY - 14} L ${mc.x} ${mc.y}`, '#10b981', 3)
    addTerminalDot(mc, '#10b981', 1.8)

    // Wire 5: 485A to Motor Back Pin 2 (485A)
    const ma = getPt('pin-rs485-a', 'right')
    const moa = getPt('pin-motor-a', 'center')
    addPath(`M ${ma.x} ${ma.y} L ${ma.x + 24} ${ma.y} L ${ma.x + 24} ${midTerminalGapY - 4} L ${moa.x + 18} ${midTerminalGapY - 4} L ${moa.x + 18} ${moa.y} L ${moa.x} ${moa.y}`, '#2563eb', 3)
    addTerminalDot(moa, '#2563eb', 1.8)

    // Wire 6: 485B to Motor Back Pin 3 (485B)
    const mb = getPt('pin-rs485-b', 'right')
    const mob = getPt('pin-motor-b', 'center')
    addPath(`M ${mb.x} ${mb.y} L ${mb.x + 16} ${mb.y} L ${mb.x + 16} ${midTerminalGapY + 6} L ${mob.x + 25} ${midTerminalGapY + 6} L ${mob.x + 25} ${mob.y} L ${mob.x} ${mob.y}`, '#64748b', 3)
    addTerminalDot(mob, '#64748b', 1.8)
  } else {
    // ==========================================
    // PATH B WIRING (esp: ESP32 + MAX3485)
    // ZERO OVERLAPPING ORTHOGONAL ROUTING
    // ==========================================
    const cardEspEl = document.getElementById('card-esp')
    const cardMaxEl = document.getElementById('card-max')
    const frontMotorEl = document.querySelector('#card-front .motor-unit')
    const subtitle2El = document.querySelector('#card-back .card-subtitle')

    const espRight = cardEspEl ? cardEspEl.getBoundingClientRect().right - svgRect.left : 541
    const maxRight = cardMaxEl ? cardMaxEl.getBoundingClientRect().right - svgRect.left : 729

    // Middle corridor between front motor card and back card subtitle
    const frontBottom = frontMotorEl ? frontMotorEl.getBoundingClientRect().bottom - svgRect.top : 475
    const subtitle2Rect = subtitle2El ? subtitle2El.getBoundingClientRect() : motor2Rect
    const gap2MidY = (frontBottom + subtitle2Rect.top - svgRect.top) / 2

    // 1. Two separate vertical lanes in the 15px gap between ESP32 and MAX3485 (going UP)
    const track5v = espRight + 4
    const trackGnd = espRight + 11

    // Wire 3: 5V RS-485 transceiver power from ESP32 Pin 5V (Row 1) -> Motor Back Pin 10 (5V)
    const e5 = getPt('pin-esp-5v', 'right')
    const m5 = getPt('pin-motor-5v', 'center')
    const y5v = gap2MidY - 5
    addPath(`M ${e5.x} ${e5.y} L ${track5v} ${e5.y} L ${track5v} ${y5v} L ${m5.x} ${y5v} L ${m5.x} ${m5.y}`, '#f97316', 3)
    addTerminalDot(m5, '#f97316', 1.8)

    // Wire 4: GND RS-485 ground from ESP32 Pin GND1 (Row 2) -> Motor Back Pin 6 (COM)
    // Goes UP into middle corridor, runs parallel to 5V, drops down left of socket, enters Pin 6 from left
    const eg1 = getPt('pin-esp-gnd1', 'right')
    const mc = getPt('pin-motor-com', 'center')
    const yGnd = gap2MidY + 5
    const dropGndX = mc.x - 12
    addPath(`M ${eg1.x} ${eg1.y} L ${trackGnd} ${eg1.y} L ${trackGnd} ${yGnd} L ${dropGndX} ${yGnd} L ${dropGndX} ${mc.y} L ${mc.x} ${mc.y}`, '#10b981', 3)
    addTerminalDot(mc, '#10b981', 1.8)

    // 2. Direct 5 horizontal connections between ESP32 and MAX3485 (ZERO crossings)
    const connectDirect = (id1: string, id2: string, color: string, idx: number) => {
      const a = getPt(id1, 'right')
      const b = getPt(id2, 'left')
      if (Math.abs(a.y - b.y) < 1.5) {
        addPath(`M ${a.x} ${a.y} L ${b.x} ${a.y}`, color, 3)
      } else {
        const laneX = a.x + (b.x - a.x) * (0.2 + 0.15 * idx)
        addPath(`M ${a.x} ${a.y} L ${laneX} ${a.y} L ${laneX} ${b.y} L ${b.x} ${b.y}`, color, 3)
      }
    }
    connectDirect('pin-esp-3v3', 'pin-max-vcc', '#ec4899', 0)
    connectDirect('pin-esp-gnd2', 'pin-max-gnd', '#10b981', 1)
    connectDirect('pin-esp-rx', 'pin-max-ro', '#8b5cf6', 2)
    connectDirect('pin-esp-tx', 'pin-max-di', '#d97706', 3)
    connectDirect('pin-esp-de', 'pin-max-de', '#eab308', 4)

    // 3. Wires A & B from MAX3485 drop down in the gap between MAX3485 and Motor Card (BEFORE the motor shaft!)
    const midTerminalGapY = (motor2Rect.bottom - svgRect.top) + 22
    const dropB = maxRight + 11
    const dropA = maxRight + 5

    // Wire 5: 485A from MAX3485 Pin A (Row 2, inner) -> Motor Back Pin 2 (485A, Row 4, inner)
    const ma = getPt('pin-max-a', 'right')
    const moa = getPt('pin-motor-a', 'center')
    addPath(`M ${ma.x} ${ma.y} L ${dropA} ${ma.y} L ${dropA} ${midTerminalGapY - 4} L ${moa.x + 16} ${midTerminalGapY - 4} L ${moa.x + 16} ${moa.y} L ${moa.x} ${moa.y}`, '#2563eb', 3)
    addTerminalDot(moa, '#2563eb', 1.8)

    // Wire 6: 485B from MAX3485 Pin B (Row 1, outer) -> Motor Back Pin 3 (485B, Row 3, outer)
    const mb = getPt('pin-max-b', 'right')
    const mob = getPt('pin-motor-b', 'center')
    addPath(`M ${mb.x} ${mb.y} L ${dropB} ${mb.y} L ${dropB} ${midTerminalGapY + 6} L ${mob.x + 24} ${midTerminalGapY + 6} L ${mob.x + 24} ${mob.y} L ${mob.x} ${mob.y}`, '#64748b', 3)
    addTerminalDot(mob, '#64748b', 1.8)
  }
}

let resizeObserver: ResizeObserver | null = null

function triggerRedraw() {
  nextTick(() => {
    updateWires()
    setTimeout(updateWires, 60)
  })
}

onMounted(() => {
  triggerRedraw()
  window.addEventListener('resize', updateWires)
  const containerEl = document.getElementById('diagram')
  if (containerEl && typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(() => updateWires())
    resizeObserver.observe(containerEl)
  }
})

onUnmounted(() => {
  window.removeEventListener('resize', updateWires)
  if (resizeObserver) {
    resizeObserver.disconnect()
    resizeObserver = null
  }
})

watch(() => props.mode, () => {
  triggerRedraw()
})

defineExpose({
  updateWires
})
</script>

<style scoped>
#wires {
  position: absolute;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: 10;
}
</style>
