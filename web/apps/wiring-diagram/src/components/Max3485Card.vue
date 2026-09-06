<template>
  <div id="card-max" class="card">
    <h2 class="card-title">{{ lang === 'zh' ? 'MAX3485 模块' : 'MAX3485 Module' }}</h2>
    <p class="card-subtitle">{{ lang === 'zh' ? 'RS-485 物理收发芯片' : 'RS-485 Bus Transceiver' }}</p>

    <div class="pin-rows">
      <div class="pin-row">
        <span id="pin-max-vcc" class="badge" style="background: #ec4899; color: white;">VCC</span>
        <span id="pin-max-b" class="badge" style="background: #64748b; color: white;">{{ lang === 'zh' ? 'B (总线 B)' : 'B (Bus B)' }}</span>
      </div>
      <div class="pin-row">
        <span id="pin-max-gnd" class="badge" style="background: #10b981; color: white;">GND</span>
        <span id="pin-max-a" class="badge" style="background: #2563eb; color: white;">{{ lang === 'zh' ? 'A (总线 A)' : 'A (Bus A)' }}</span>
      </div>
      <div class="pin-row">
        <span id="pin-max-ro" class="badge" style="background: #8b5cf6; color: white;">
          {{ lang === 'zh' ? 'RO → 接 ESP32 RX' : 'RO → ESP32 RX' }}
        </span>
      </div>
      <div class="pin-row">
        <span id="pin-max-di" class="badge" style="background: #d97706; color: white;">
          {{ lang === 'zh' ? 'DI ← 接 ESP32 TX' : 'DI ← ESP32 TX' }}
        </span>
      </div>
      <div class="pin-row">
        <span id="pin-max-de" class="badge" style="background: #eab308; color: white;">
          {{ lang === 'zh' ? 'DE / RE (短接并连)' : 'DE / RE (tied)' }}
        </span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
defineProps<{
  lang: 'en' | 'zh'
}>()
</script>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  background: #eff6ff;
  border: 2px solid #3b82f6;
  border-radius: 12px;
  padding: 14px 16px 13px 16px;
  box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.04), 0 2px 4px -1px rgba(0, 0, 0, 0.02);
  box-sizing: border-box;
}

.card-title {
  font-size: 15px;
  font-weight: 700;
  color: #0f172a;
  margin: 0 0 3px 0;
}

.card-subtitle {
  font-size: 11.5px;
  color: #64748b;
  margin: 0 0 12px 0;
}

.pin-rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: auto;
}

.pin-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
