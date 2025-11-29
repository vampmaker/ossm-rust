<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import type { MotorControllerConfig } from '../types'

const props = defineProps<{
  modelValue: MotorControllerConfig
  connected: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: MotorControllerConfig): void
}>()

interface SplinePreset {
  name: string
  points: number[]
}

const presets = ref<SplinePreset[]>([
  { name: 'Default Linear', points: [0, 1] },
  { name: 'Thrust', points: [0, 0, 1, 0.8, 0.5, 0.2] },
  { name: 'Triangle', points: [0, 0.2, 0.4, 0.6, 0.8, 1.0, 0.8, 0.6, 0.4, 0.2] },
])
const selectedPreset = ref<string>('')
const pointsStr = ref<string>('')

onMounted(() => {
  const savedPresets = localStorage.getItem('splinePresets')
  if (savedPresets) {
    presets.value = JSON.parse(savedPresets)
  }
  const lastSelected = localStorage.getItem('lastSelectedSpline')
  if (lastSelected && presets.value.some(p => p.name === lastSelected)) {
    selectedPreset.value = lastSelected
  }
  else if (presets.value.length > 0) {
    const firstPreset = presets.value[0]
    if (firstPreset) {
      selectedPreset.value = firstPreset.name
    }
  }
  updatePointsFromPreset()
})

watch(presets, (newPresets) => {
  localStorage.setItem('splinePresets', JSON.stringify(newPresets))
}, { deep: true })

watch(selectedPreset, (newName) => {
  localStorage.setItem('lastSelectedSpline', newName)
  updatePointsFromPreset()
})

watch(() => props.modelValue.spline_points, (newPoints) => {
  pointsStr.value = newPoints.join(' ')
}, { immediate: true })

function updatePointsFromPreset() {
  const preset = presets.value.find(p => p.name === selectedPreset.value)
  if (preset) {
    const newConfig = { ...props.modelValue, spline_points: preset.points }
    emit('update:modelValue', newConfig)
  }
}

function applyPoints() {
  const points = pointsStr.value.trim().split(/\s+/).map(Number.parseFloat).filter(n => !Number.isNaN(n) && n >= 0 && n <= 1)
  if (points.length > 1) {
    const newConfig = { ...props.modelValue, spline_points: points }
    emit('update:modelValue', newConfig)

    // also update current preset
    const preset = presets.value.find(p => p.name === selectedPreset.value)
    if (preset) {
      preset.points = points
    }
  }
}

function addPreset() {
  const name = prompt('Enter new preset name:')
  if (name && !presets.value.some(p => p.name === name)) {
    presets.value.push({ name, points: [0, 1] })
    selectedPreset.value = name
  }
}

function renamePreset() {
  const oldName = selectedPreset.value
  const newName = prompt('Enter new name for preset:', oldName)
  if (newName && newName !== oldName && !presets.value.some(p => p.name === newName)) {
    const preset = presets.value.find(p => p.name === oldName)
    if (preset) {
      preset.name = newName
      selectedPreset.value = newName
    }
  }
}

function deletePreset() {
  if (confirm(`Are you sure you want to delete preset "${selectedPreset.value}"?`)) {
    presets.value = presets.value.filter(p => p.name !== selectedPreset.value)
    if (presets.value.length > 0) {
      const firstPreset = presets.value[0]
      if (firstPreset) {
        selectedPreset.value = firstPreset.name
      }
    }
    else {
      presets.value.push({ name: 'Default', points: [0, 1] })
      selectedPreset.value = 'Default'
    }
  }
}

function exportPresets() {
  const json = JSON.stringify(presets.value, null, 2)
  const blob = new Blob([json], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = 'spline_presets.json'
  a.click()
  URL.revokeObjectURL(url)
}

function importPresets() {
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = 'application/json'
  input.onchange = async (e) => {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (file) {
      try {
        const text = await file.text()
        const importedPresets: SplinePreset[] = JSON.parse(text)
        // basic validation
        if (Array.isArray(importedPresets) && importedPresets.every(p => typeof p.name === 'string' && Array.isArray(p.points))) {
          presets.value = importedPresets
          if (presets.value.length > 0) {
            const firstPreset = presets.value[0]
            if (firstPreset) {
              selectedPreset.value = firstPreset.name
            }
          }
        }
        else {
          alert('Invalid preset file format.')
        }
      }
      catch (err) {
        alert('Failed to import presets.')
        console.error(err)
      }
    }
  }
  input.click()
}
</script>

<template>
  <div class="space-y-4 bg-white p-4 shadow-md">
    <h2 class="text-xl font-bold">Spline Editor</h2>

    <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
      <div>
        <label :for="`preset-select`" class="mb-1 block">Preset</label>
        <select :id="`preset-select`" v-model="selectedPreset" class="w-full border border-gray-300 bg-gray-50 p-2" :disabled="!connected">
          <option v-for="preset in presets" :key="preset.name" :value="preset.name">
            {{ preset.name }}
          </option>
        </select>
      </div>
      <div class="flex items-end space-x-2">
        <button class="flex-1 bg-green-500 px-3 py-2 text-white hover:bg-green-600" :disabled="!connected" @click="addPreset">
          New
        </button>
        <button class="flex-1 bg-yellow-500 px-3 py-2 text-white hover:bg-yellow-600" :disabled="!connected" @click="renamePreset">
          Rename
        </button>
        <button class="flex-1 bg-red-500 px-3 py-2 text-white hover:bg-red-600" :disabled="!connected" @click="deletePreset">
          Delete
        </button>
      </div>
    </div>
    <div>
      <label :for="`points-textarea`" class="mb-1 block">Points (space-separated, 0.0 to 1.0)</label>
      <textarea
        :id="`points-textarea`" v-model="pointsStr" class="w-full border border-gray-300 bg-gray-50 p-2" rows="3"
        :disabled="!connected" @blur="applyPoints"
      />
    </div>

    <div class="flex space-x-2">
      <button class="flex-1 bg-blue-500 px-4 py-2 text-white hover:bg-blue-600" @click="importPresets">
        Import
      </button>
      <button class="flex-1 bg-blue-500 px-4 py-2 text-white hover:bg-blue-600" @click="exportPresets">
        Export
      </button>
    </div>
  </div>
</template>
