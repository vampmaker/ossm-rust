<template>
  <div id="card-adapter" class="card">
    <div style="margin-bottom: 12px;">
      <h2 class="card-title">{{ lang === 'zh' ? '24V 直流电源适配器' : '24V DC Power Adapter' }}</h2>
      <p class="card-subtitle">
        {{ lang === 'zh' ? '电机动力供电 · 5.5mm DC 插头' : 'Motor shaft power · 5.5mm DC jack' }}
      </p>
    </div>

    <div class="dc-jack-box">
      <div class="dc-row" style="margin-bottom: 8px;">
        <span class="dc-label">
          <span style="color: #ef4444;">●</span>
          {{ lang === 'zh' ? 'DC 圆孔母头' : 'DC Female Jack' }}
        </span>
        <span id="pin-dc-24v" class="badge badge-red">+24V</span>
      </div>
      <div class="dc-row">
        <span class="dc-label" style="color: #64748b;">
          {{ lang === 'zh' ? 'GND (0V 地线)' : 'GND (0V)' }}
        </span>
        <span id="pin-dc-gnd" class="badge badge-dark">GND</span>
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
  font-size: 11px;
  color: #64748b;
  margin: 0;
  white-space: nowrap;
}

.dc-jack-box {
  background: #f8fafc;
  border: 1px solid #e2e8f0;
  border-radius: 8px;
  padding: 10px;
}

.dc-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.dc-label {
  font-size: 12px;
  font-weight: 600;
  color: #334155;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.badge-red {
  background: #ef4444;
  color: white;
}

.badge-dark {
  background: #475569;
  color: white;
}
</style>
