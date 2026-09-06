<template>
  <nav class="nav-bar">
    <div class="nav-brand">
      <span class="brand-title">OSSM Wiring Diagrams</span>
    </div>

    <div class="nav-tabs">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="nav-tab"
        :class="{ active: currentRoute === tab.id }"
        @click="setRoute(tab.id)"
      >
        <span class="tab-label">{{ tab.label }}</span>
        <span class="tab-sublabel">{{ tab.sublabel }}</span>
      </button>
    </div>

    <div class="nav-actions">
      <button class="btn-download" @click="downloadSvg">
        Download SVG
      </button>
    </div>
  </nav>
</template>

<script setup lang="ts">
import { currentRoute, setRoute } from '../router'
import type { DiagramRoute, RouteOption } from '../types'
import * as htmlToImage from 'html-to-image'

const tabs: RouteOption[] = [
  { id: 'std', label: 'Path A (std)', sublabel: 'USB-RS485 / EN' },
  { id: 'std-zh', label: '方案 A (std)', sublabel: 'USB-RS485 / 中文' },
  { id: 'esp', label: 'Path B (esp)', sublabel: 'ESP32 / EN' },
  { id: 'esp-zh', label: '方案 B (esp)', sublabel: 'ESP32 / 中文' },
]

async function downloadSvg() {
  const el = document.getElementById('diagram')
  if (!el) return
  try {
    const dataUrl = await htmlToImage.toSvg(el)
    const link = document.createElement('a')
    link.download = `wiring_diagram_${currentRoute.value}.svg`
    link.href = dataUrl
    link.click()
  } catch (err) {
    console.error('Failed to export SVG:', err)
  }
}
</script>

<style scoped>
.nav-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 12px;
  padding: 10px 16px;
  margin-bottom: 24px;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.04);
  width: 100%;
  max-width: 820px;
  box-sizing: border-box;
}

.brand-title {
  font-size: 15px;
  font-weight: 700;
  color: #0f172a;
}

.nav-tabs {
  display: flex;
  gap: 8px;
}

.nav-tab {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 6px 12px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  cursor: pointer;
  transition: all 0.15s ease;
}

.nav-tab:hover {
  background: #f1f5f9;
}

.nav-tab.active {
  background: #eff6ff;
  border-color: #3b82f6;
}

.tab-label {
  font-size: 12.5px;
  font-weight: 600;
  color: #0f172a;
}

.nav-tab.active .tab-label {
  color: #2563eb;
}

.tab-sublabel {
  font-size: 10px;
  color: #64748b;
}

.btn-download {
  background: #2563eb;
  color: #ffffff;
  border: none;
  border-radius: 6px;
  padding: 8px 14px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s ease;
}

.btn-download:hover {
  background: #1d4ed8;
}
</style>
