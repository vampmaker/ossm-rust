<template>
  <div id="card-rs485" class="card">
    <div style="margin-bottom: 12px;">
      <h2 class="card-title">{{ lang === 'zh' ? 'USB 转 RS-485 适配器' : 'USB-RS485 adapter' }}</h2>
      <p class="card-subtitle">
        {{ lang === 'zh' ? '自动收发 DE/RE · 5V 供电输出' : 'Auto DE/RE · 5V power output' }}
      </p>
    </div>

    <div class="pin-list">
      <div class="pin-row">
        <span class="pin-label" style="color: #ea580c;">{{ lang === 'zh' ? '5V / VCC (供电输出)' : '5V / VCC' }}</span>
        <span id="pin-rs485-5v" class="badge" style="background: #f97316; color: white;">5V</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #059669;">{{ lang === 'zh' ? 'GND (信号地)' : 'GND' }}</span>
        <span id="pin-rs485-gnd" class="badge" style="background: #10b981; color: white;">GND</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #2563eb;">{{ lang === 'zh' ? 'A (总线 A / D+)' : 'A (Bus A)' }}</span>
        <span id="pin-rs485-a" class="badge" style="background: #2563eb; color: white;">A</span>
      </div>
      <div class="pin-row">
        <span class="pin-label" style="color: #64748b;">{{ lang === 'zh' ? 'B (总线 B / D-)' : 'B (Bus B)' }}</span>
        <span id="pin-rs485-b" class="badge" style="background: #64748b; color: white;">B</span>
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
  border: 2px solid #3b82f6;
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
  font-size: 11px;
  color: #64748b;
  margin: 0 0 12px 0;
  white-space: nowrap;
}

.pin-list {
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
  font-size: 12px;
  font-weight: 700;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
</style>
