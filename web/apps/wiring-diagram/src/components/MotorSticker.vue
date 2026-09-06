<template>
  <div class="motor-sticker">
    <div class="sticker-header">57AIM30</div>
    <div class="sticker-body">
      <!-- Left specs column -->
      <div v-if="type === 'front'" class="specs-col">
        <div>Pn: 100w</div>
        <div>Un: 24~36v</div>
        <div>In: 4.4A</div>
        <div>Nn: 1500</div>
        <div>Pn: 0.96</div>
      </div>
      <div v-else class="specs-col">
        <div>RS-485</div>
        <div>Comm 5V</div>
        <div>Modbus</div>
        <div>115200</div>
        <div>2×5P</div>
      </div>

      <!-- Right pinout column -->
      <div v-if="type === 'front'" class="pinout-col-front">
        <div>+V</div>
        <div>GND</div>
        <div>PU+</div>
        <div>PU-</div>
        <div>DIR+</div>
        <div>DIR-</div>
      </div>
      <div v-else class="pinout-grid-back">
        <span class="p-left">5V</span><span class="p-sep">|</span><span class="p-right">EN-</span>
        <span class="p-left">ZO</span><span class="p-sep">|</span><span class="p-right">EN+</span>
        <span class="p-left">RDY</span><span class="p-sep">|</span><span class="p-right">485B</span>
        <span class="p-left">WR</span><span class="p-sep">|</span><span class="p-right">485A</span>
        <span class="p-left">COM</span><span class="p-sep">|</span><span class="p-right">NC</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
defineProps<{
  type: 'front' | 'back'
}>()
</script>

<style scoped>
.motor-sticker {
  background: #ffffff;
  border: 1px solid #94a3b8;
  border-radius: 2px;
  padding: 5px 6px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, monospace;
  position: absolute;
  left: 44px;
  width: 114px;
  height: 86px;
  z-index: 3;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  box-sizing: border-box;
}

.sticker-header {
  font-weight: 800;
  font-size: 10px;
  color: #0f172a;
  border-bottom: 1.5px solid #0f172a;
  padding-bottom: 2px;
  margin-bottom: 3px;
  letter-spacing: 0.4px;
  line-height: 1;
}

.sticker-body {
  display: flex;
  justify-content: space-between;
  font-size: 8px;
  line-height: 1.25;
  color: #1e293b;
  font-weight: 600;
  flex: 1;
}

.specs-col {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  font-size: 8px;
  line-height: 1.25;
}

.pinout-col-front {
  border-left: 1px solid #cbd5e1;
  padding-left: 6px;
  text-align: right;
  color: #0f172a;
  font-weight: 700;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.pinout-grid-back {
  border-left: 1px solid #cbd5e1;
  padding-left: 5px;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  row-gap: 2px;
  column-gap: 2px;
  font-weight: 700;
  color: #0f172a;
}

.p-left {
  text-align: right;
  white-space: nowrap;
}

.p-sep {
  text-align: center;
  color: #94a3b8;
  font-weight: 400;
  padding: 0 1px;
}

.p-right {
  text-align: left;
  white-space: nowrap;
}
</style>
