<template>
  <div id="card-esp" class="card">
    <h2 class="card-title">{{ lang === 'zh' ? 'ESP32-C6 / S3 开发板' : 'ESP32-C6 / S3 Dev Board' }}</h2>
    <p class="card-subtitle">{{ lang === 'zh' ? '主控制器 & WiFi/BLE 运行核心' : 'Main Controller & WiFi/BLE Brain' }}</p>

    <div class="pin-rows">
      <div class="pin-row">
        <span class="pin-label" style="color: #ea580c;">{{ lang === 'zh' ? '5V (USB供电输出)' : '5V (VBUS)' }}</span>
        <span id="pin-esp-5v" class="badge" style="background: #f97316; color: white;">Pin 5V</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #059669;">{{ lang === 'zh' ? 'GND (485 接口地)' : 'GND (RS-485)' }}</span>
        <span id="pin-esp-gnd1" class="badge" style="background: #10b981; color: white;">Pin GND</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #db2777;">{{ lang === 'zh' ? '3.3V (MAX供电)' : '3.3V (3V3)' }}</span>
        <span id="pin-esp-3v3" class="badge" style="background: #ec4899; color: white;">Pin 3V3</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #059669;">{{ lang === 'zh' ? 'GND (MAX地线)' : 'GND (MAX)' }}</span>
        <span id="pin-esp-gnd2" class="badge" style="background: #10b981; color: white;">Pin GND</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #7c3aed;">GPIO 19 UART RX</span>
        <span id="pin-esp-rx" class="badge" style="background: #8b5cf6; color: white;">GPIO 19</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #b45309;">GPIO 18 UART TX</span>
        <span id="pin-esp-tx" class="badge" style="background: #d97706; color: white;">GPIO 18</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #ca8a04;">GPIO 20 DE/RE</span>
        <span id="pin-esp-de" class="badge" style="background: #eab308; color: white;">GPIO 20</span>
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
  background: #ffffff;
  border: 1px solid #cbd5e1;
  border-radius: 12px;
  padding: 14px 16px;
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
}

.pin-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.pin-label {
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
</style>
