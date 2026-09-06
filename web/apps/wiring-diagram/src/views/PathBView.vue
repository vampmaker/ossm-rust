<template>
  <div id="diagram" class="container">
    <WireOverlay mode="esp" />

    <div class="header">
      <h1 class="title">
        {{ lang === 'zh' ? '方案 B — ossm-esp32 (ESP32 独立固件)' : 'PATH B — ossm-esp32 (ESP32 firmware)' }}
      </h1>
      <p class="subtitle">
        {{ lang === 'zh'
          ? '57AIM30 + ESP32-C6/S3 + MAX3485。MCU TX → DI, MCU RX → RO (NVS 默认 GPIO 18 / 19 / 20)。'
          : '57AIM30 + ESP32-C6/S3 + MAX3485. MCU TX → DI, MCU RX → RO (NVS defaults GPIO 18 / 19 / 20).' }}
      </p>
    </div>

    <div class="grid-container">
      <!-- Column 1: DC Power & ESP32 -->
      <div class="col-1">
        <DcPowerCard :lang="lang" />
        <Esp32Card :lang="lang" />
      </div>

      <!-- Column 2: MAX3485 -->
      <div class="col-2">
        <Max3485Card :lang="lang" />
      </div>

      <!-- Column 3: 57AIM30 Motor (Front & Back Terminals) -->
      <div class="col-3">
        <!-- Card 1: Front Terminal (Power) -->
        <div id="card-front" class="card">
          <div class="card-header-row">
            <div class="card-title-group">
              <h2 class="card-title">{{ lang === 'zh' ? '正面端子 (动力输入)' : 'Front Terminal (Power)' }}</h2>
              <span class="badge badge-sm badge-green">KF2EDGK-3.81 6P</span>
            </div>
            <p class="card-subtitle">{{ lang === 'zh' ? '2D 侧面结构 · 绿色直立拔插端子' : '2D Side View · Vertical Green Pluggable Terminal' }}</p>
          </div>

          <MotorUnit type="front" />
        </div>

        <!-- Card 2: Back Terminal (Comm) -->
        <div id="card-back" class="card card-back-pad">
          <div class="card-header-row">
            <div class="card-title-group">
              <h2 class="card-title">{{ lang === 'zh' ? '背面端子 (通讯)' : 'Back Terminal (Comm)' }}</h2>
              <span class="badge badge-sm badge-slate">PHB 2.0 2×5P</span>
            </div>
            <p class="card-subtitle">{{ lang === 'zh' ? '2D 侧面结构 · 白色直立带锁母座' : '2D Side View · Vertical White Shrouded Connector' }}</p>
          </div>

          <MotorUnit type="back" />

          <div class="note-warning">
            ⚠ {{ lang === 'zh' ? '警告：切勿将 24V 动力电源接入后侧 10-pin 插座！' : 'Note: Do NOT apply 24V to the rear 10-pin connector!' }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import WireOverlay from '../components/WireOverlay.vue'
import DcPowerCard from '../components/DcPowerCard.vue'
import Esp32Card from '../components/Esp32Card.vue'
import Max3485Card from '../components/Max3485Card.vue'
import MotorUnit from '../components/MotorUnit.vue'

defineProps<{
  lang: 'en' | 'zh'
}>()
</script>

<style scoped>
.container {
  width: 780px;
  background: #f8fafc;
  border: 2px solid #e2e8f0;
  border-radius: 16px;
  padding: 20px 22px;
  position: relative;
  box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.05);
  box-sizing: border-box;
}

.header {
  margin-bottom: 50px;
}

.title {
  font-size: 20px;
  font-weight: 800;
  margin: 0 0 5px 0;
  color: #0f172a;
  letter-spacing: 0.3px;
}

.subtitle {
  font-size: 13px;
  color: #64748b;
  margin: 0;
}

.grid-container {
  display: grid;
  grid-template-columns: 224px 156px 326px;
  gap: 15px;
  position: relative;
  z-index: 2;
}

.col-1 {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.col-2 {
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
}

.col-3 {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.card {
  background: #ffffff;
  border: 1px solid #cbd5e1;
  border-radius: 12px;
  padding: 14px 16px;
  box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.04), 0 2px 4px -1px rgba(0, 0, 0, 0.02);
  box-sizing: border-box;
}

.card-back-pad {
  padding-bottom: 16px;
}

.card-header-row {
  margin-bottom: 10px;
}

.card-title-group {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 3px;
}

.card-title {
  font-size: 14.5px;
  font-weight: 700;
  color: #0f172a;
  margin: 0;
  white-space: nowrap;
  flex-shrink: 0;
}

.card-subtitle {
  font-size: 11px;
  color: #64748b;
  margin: 0;
}

.badge-green {
  background: #15803d;
  color: white;
}

.badge-slate {
  background: #475569;
  color: white;
}

.note-warning {
  font-size: 10px;
  color: #dc2626;
  font-weight: 600;
  margin-top: 44px;
  white-space: nowrap;
}
</style>
