<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from 'vue'
import { useI18n } from 'vue-i18n'
import { Save, Copy, Pencil, Trash2, Upload, Download, Play, Square, Plus, Code, ArrowUp, ArrowDown, BookOpen, ChevronDown, Check } from '@lucide/vue'
import type { MacroAction, MacroDocument, MacroInstruction, MacroWaveFunc, MotorControllerConfig } from '../types'
import {
  MacroPlayerEngine,
  captureConfigAsSet,
  cloneJson,
  emptyMacro,
  formatMacroTime,
  loadDraft,
  loadMacroLibrary,
  macroDurationMs,
  parseMacroJson,
  parseMacroTime,
  saveDraft,
  saveMacroLibrary,
  MACRO_LAST_SELECTED_KEY,
} from '../macro'

const props = defineProps<{
  modelValue: MotorControllerConfig
  connected: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', config: MotorControllerConfig): void
}>()

const { t } = useI18n()

const library = ref<MacroDocument[]>([])
const selectedName = ref('')
const doc = ref<MacroDocument>(emptyMacro('Warm up'))
const isPlaying = ref(false)
const elapsedMs = ref(0)
const nextIndex = ref(0)
const statusMessage = ref('')
const showRawJson = ref(false)
const rawJsonText = ref('')
const rawJsonError = ref('')
const fileInputRef = ref<HTMLInputElement | null>(null)

let engine: MacroPlayerEngine | null = null

const durationMs = computed(() => macroDurationMs(doc.value))
const editingLocked = computed(() => isPlaying.value)

const libraryNames = computed(() => library.value.map((m) => m.name))

function ensureInstructionIds(d: MacroDocument) {
  d.instructions.forEach((inst, idx) => {
    if (!inst._id) {
      inst._id = `step_${idx}_${Math.random().toString(36).substring(2, 9)}`
    }
  })
}

function syncRawFromDoc() {
  ensureInstructionIds(doc.value)
  rawJsonText.value = JSON.stringify(doc.value, null, 2)
  rawJsonError.value = ''
}

function persistDraft() {
  saveDraft(doc.value)
}

function loadFromLibrary(name: string) {
  const found = library.value.find((m) => m.name === name)
  if (!found) return
  doc.value = cloneJson(found)
  selectedName.value = name
  localStorage.setItem(MACRO_LAST_SELECTED_KEY, name)
  nextIndex.value = 0
  elapsedMs.value = 0
  syncRawFromDoc()
  persistDraft()
}

function saveCurrent() {
  if (editingLocked.value) return
  const idx = library.value.findIndex((m) => m.name === doc.value.name)
  const copy = cloneJson(doc.value)
  if (idx >= 0) {
    library.value[idx] = copy
  } else {
    library.value.push(copy)
  }
  saveMacroLibrary(library.value)
  selectedName.value = copy.name
  localStorage.setItem(MACRO_LAST_SELECTED_KEY, copy.name)
  statusMessage.value = t('macro.saved', { name: copy.name })
}

function saveAs() {
  if (editingLocked.value) return
  const name = prompt(t('macro.promptSaveAs'), doc.value.name)
  if (!name) return
  const trimmed = name.trim()
  if (!trimmed) return
  if (library.value.some((m) => m.name === trimmed && m.name !== doc.value.name)) {
    alert(t('macro.existsError'))
    return
  }
  doc.value = { ...doc.value, name: trimmed }
  library.value.push(cloneJson(doc.value))
  saveMacroLibrary(library.value)
  selectedName.value = trimmed
  localStorage.setItem(MACRO_LAST_SELECTED_KEY, trimmed)
  syncRawFromDoc()
  persistDraft()
  statusMessage.value = t('macro.savedAs', { name: trimmed })
}

function renameCurrent() {
  if (editingLocked.value) return
  const name = prompt(t('macro.promptRename'), doc.value.name)
  if (!name) return
  const trimmed = name.trim()
  if (!trimmed) return
  const old = doc.value.name
  if (trimmed === old) return
  doc.value = { ...doc.value, name: trimmed }
  const idx = library.value.findIndex((m) => m.name === old)
  if (idx >= 0) {
    library.value[idx] = cloneJson(doc.value)
  } else {
    library.value.push(cloneJson(doc.value))
  }
  saveMacroLibrary(library.value)
  selectedName.value = trimmed
  localStorage.setItem(MACRO_LAST_SELECTED_KEY, trimmed)
  syncRawFromDoc()
  persistDraft()
  statusMessage.value = t('macro.renamed', { old, name: trimmed })
}

function deleteCurrent() {
  if (editingLocked.value) return
  if (library.value.length <= 1) {
    alert(t('macro.cannotDeleteLast'))
    return
  }
  if (!confirm(t('macro.confirmDelete', { name: doc.value.name }))) return
  library.value = library.value.filter((m) => m.name !== doc.value.name)
  saveMacroLibrary(library.value)
  loadFromLibrary(library.value[0]!.name)
}

function onSelectChange(e: Event) {
  const name = (e.target as HTMLSelectElement).value
  if (isPlaying.value) return
  loadFromLibrary(name)
}

function exportMacro() {
  const blob = new Blob([JSON.stringify(doc.value, null, 2)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `${doc.value.name.replace(/[^\w.-]+/g, '_') || 'macro'}.json`
  a.click()
  URL.revokeObjectURL(url)
}

function importMacro() {
  fileInputRef.value?.click()
}

function onFileSelected(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  const reader = new FileReader()
  reader.onload = () => {
    const text = String(reader.result ?? '')
    const res = parseMacroJson(text)
    if (!res.ok || !res.document) {
      statusMessage.value = res.errors[0] || t('macro.invalidFormat')
      return
    }
    doc.value = res.document
    const existing = library.value.findIndex((m) => m.name === doc.value.name)
    if (existing >= 0) library.value[existing] = cloneJson(doc.value)
    else library.value.push(cloneJson(doc.value))
    saveMacroLibrary(library.value)
    selectedName.value = doc.value.name
    localStorage.setItem(MACRO_LAST_SELECTED_KEY, doc.value.name)
    syncRawFromDoc()
    persistDraft()
    statusMessage.value = t('macro.imported', { name: doc.value.name })
  }
  reader.readAsText(file)
  input.value = ''
}

function sortInstructions() {
  doc.value.instructions = [...doc.value.instructions].sort((a, b) => a.at - b.at)
  persistDraft()
}

function updateInstruction(idx: number, patch: Partial<MacroInstruction>, sort = true) {
  if (editingLocked.value) return
  const list = [...doc.value.instructions]
  const cur = list[idx]
  if (!cur) return
  list[idx] = {
    ...cur,
    ...patch,
    params: patch.params !== undefined ? patch.params : cur.params,
    _id: cur._id || `step_${idx}_${Math.random().toString(36).substring(2, 9)}`,
  }
  doc.value = { ...doc.value, instructions: list }
  if (sort && patch.at !== undefined) {
    sortInstructions()
  } else {
    persistDraft()
  }
  syncRawFromDoc()
}

function setAction(idx: number, action: MacroAction) {
  if (editingLocked.value) return
  const cur = doc.value.instructions[idx]
  if (!cur) return
  const next: MacroInstruction = { at: cur.at, action }
  if (action === 'set') next.params = cur.params ?? { bpm: 60, depth: 1, wave_func: 'sine' }
  if (action === 'stop' && cur.params?.position !== undefined) next.params = { position: cur.params.position }
  updateInstruction(idx, next, true)
}

function setTimeFromText(idx: number, text: string, inputEl?: HTMLInputElement, sort = true) {
  if (editingLocked.value) return
  const ms = parseMacroTime(text)
  if (ms === null) {
    if (inputEl) inputEl.value = formatMacroTime(doc.value.instructions[idx]?.at ?? 0)
    return
  }
  updateInstruction(idx, { at: ms }, sort)
  if (inputEl && sort) inputEl.value = formatMacroTime(ms)
}

function setParam(idx: number, key: string, value: unknown) {
  if (editingLocked.value) return
  const cur = doc.value.instructions[idx]
  if (!cur) return
  const params = { ...cur.params } as Record<string, unknown>
  params[key] = value
  updateInstruction(idx, { params: params as MacroInstruction['params'] }, false)
}

function insertBelow(idx: number) {
  if (editingLocked.value) return
  const baseAt = doc.value.instructions[idx]?.at ?? 0
  const list = [...doc.value.instructions]
  list.splice(idx + 1, 0, { at: baseAt, action: 'set', params: { bpm: 60, depth: 1, wave_func: 'sine' } })
  doc.value = { ...doc.value, instructions: list }
  persistDraft()
  syncRawFromDoc()
}

function duplicateRow(idx: number) {
  if (editingLocked.value) return
  const cur = doc.value.instructions[idx]
  if (!cur) return
  const list = [...doc.value.instructions]
  list.splice(idx + 1, 0, cloneJson(cur))
  doc.value = { ...doc.value, instructions: list }
  persistDraft()
  syncRawFromDoc()
}

function deleteRow(idx: number) {
  if (editingLocked.value) return
  const list = [...doc.value.instructions]
  list.splice(idx, 1)
  doc.value = { ...doc.value, instructions: list }
  persistDraft()
  syncRawFromDoc()
}

function moveRow(idx: number, dir: -1 | 1) {
  if (editingLocked.value) return
  const j = idx + dir
  if (j < 0 || j >= doc.value.instructions.length) return
  const list = [...doc.value.instructions]
  const a = list[idx]!
  const b = list[j]!
  // Swap timestamps so order survives sort-by-at
  const atA = a.at
  list[idx] = { ...b, at: atA }
  list[j] = { ...a, at: b.at }
  doc.value = { ...doc.value, instructions: list }
  sortInstructions()
  syncRawFromDoc()
}

function captureCurrent() {
  if (editingLocked.value) return
  const inst = captureConfigAsSet(props.modelValue)
  const lastAt = doc.value.instructions.length
    ? doc.value.instructions[doc.value.instructions.length - 1]!.at
    : 0
  inst.at = lastAt
  doc.value = {
    ...doc.value,
    instructions: [...doc.value.instructions, inst],
  }
  persistDraft()
  syncRawFromDoc()
  statusMessage.value = t('macro.captured')
}

function applyRawJson() {
  if (editingLocked.value) return
  const res = parseMacroJson(rawJsonText.value)
  if (!res.ok || !res.document) {
    rawJsonError.value = res.errors.join('; ')
    return
  }
  rawJsonError.value = ''
  doc.value = res.document
  persistDraft()
  statusMessage.value = t('macro.rawApplied')
}

function deltaLabel(idx: number): string {
  const cur = doc.value.instructions[idx]
  if (!cur) return ''
  const prev = idx > 0 ? doc.value.instructions[idx - 1]!.at : 0
  const d = (cur.at - prev) / 1000
  return `+${d.toFixed(1)}s`
}

function seekToMs(ms: number, exactIndex?: number) {
  const target = Math.max(0, Math.min(ms, durationMs.value))
  if (engine && isPlaying.value) {
    engine.seek(target, exactIndex)
  } else {
    elapsedMs.value = target
    if (exactIndex !== undefined && exactIndex >= 0 && exactIndex <= doc.value.instructions.length) {
      nextIndex.value = exactIndex
    } else if (target <= 0) {
      nextIndex.value = 0
    } else if (target >= durationMs.value && durationMs.value > 0) {
      nextIndex.value = doc.value.instructions.length
    } else {
      let idx = 0
      while (idx < doc.value.instructions.length && doc.value.instructions[idx]!.at <= target) {
        idx++
      }
      nextIndex.value = idx
    }
  }
}

function onElapsedChange(text: string, inputEl?: HTMLInputElement) {
  if (editingLocked.value) return
  const ms = parseMacroTime(text)
  if (ms === null) {
    if (inputEl) inputEl.value = formatMacroTime(elapsedMs.value)
    return
  }
  seekToMs(ms)
  if (inputEl) inputEl.value = formatMacroTime(elapsedMs.value)
}

function onSliderInput(val: string) {
  if (editingLocked.value) return
  const ms = Number(val)
  if (!Number.isFinite(ms)) return
  if (engine && isPlaying.value) {
    engine.seek(ms, ms <= 0 ? 0 : (ms >= durationMs.value && durationMs.value > 0 ? doc.value.instructions.length : undefined))
  } else {
    elapsedMs.value = ms
    if (ms <= 0) {
      nextIndex.value = 0
    } else if (ms >= durationMs.value && durationMs.value > 0) {
      nextIndex.value = doc.value.instructions.length
    } else {
      let idx = 0
      while (idx < doc.value.instructions.length && doc.value.instructions[idx]!.at <= ms) {
        idx++
      }
      nextIndex.value = idx
    }
  }
}

function onSliderChange(val: string) {
  if (editingLocked.value) return
  const ms = Number(val)
  if (!Number.isFinite(ms)) return
  seekToMs(ms)
}

function seekGap(gapIndex: number) {
  if (showRawJson.value) return
  const seekAt =
    gapIndex <= 0
      ? 0
      : gapIndex >= doc.value.instructions.length
        ? macroDurationMs(doc.value)
        : Math.max(0, doc.value.instructions[gapIndex - 1]!.at)
  seekToMs(seekAt, gapIndex)
}

async function startPlayback() {
  if (!props.connected || doc.value.instructions.length === 0) return
  if (engine) {
    await engine.stop()
    engine = null
  }
  if (elapsedMs.value >= durationMs.value && durationMs.value > 0 && !doc.value.loop) {
    elapsedMs.value = 0
    nextIndex.value = 0
  }
  // Seed live config from current UI, but leave wave_func as concrete waves during playback
  const base = { ...props.modelValue, spline_points: [...props.modelValue.spline_points] }
  engine = new MacroPlayerEngine(doc.value, base, {
    onConfig: (cfg) => {
      emit('update:modelValue', { ...cfg, wave_func: 'macro' })
    },
    onTick: (elapsed, next) => {
      elapsedMs.value = elapsed
      nextIndex.value = next
    },
    onStatus: (msg) => {
      if (msg === 'error') statusMessage.value = t('macro.playError')
      else if (msg === 'playing' || msg === 'stopped') statusMessage.value = ''
    },
    onDone: () => {
      isPlaying.value = false
      engine = null
    },
  })
  isPlaying.value = true
  await engine.start(elapsedMs.value)
}

async function stopPlayback() {
  isPlaying.value = false
  if (engine) {
    await engine.stop()
    engine = null
  }
  statusMessage.value = ''
}

function toggleLoop() {
  if (editingLocked.value) return
  doc.value = { ...doc.value, loop: !doc.value.loop }
  persistDraft()
  syncRawFromDoc()
}

watch(
  () => doc.value,
  () => {
    persistDraft()
  },
  { deep: true },
)

onMounted(() => {
  library.value = loadMacroLibrary()
  const draft = loadDraft()
  const last = localStorage.getItem(MACRO_LAST_SELECTED_KEY)
  if (draft) {
    doc.value = draft
    selectedName.value = draft.name
    if (!library.value.some((m) => m.name === draft.name)) {
      library.value.push(cloneJson(draft))
      saveMacroLibrary(library.value)
    }
  } else if (last && library.value.some((m) => m.name === last)) {
    loadFromLibrary(last)
  } else if (library.value.length > 0) {
    loadFromLibrary(library.value[0]!.name)
  }
  syncRawFromDoc()
  statusMessage.value = t('macro.loadPrompt')
})

onBeforeUnmount(() => {
  void stopPlayback()
})

const waveOptions: MacroWaveFunc[] = ['sine', 'thrust', 'spline']
</script>

<template>
  <div id="macro-player" class="space-y-6">
    <!-- Header & Library Bar -->
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-md transition-all space-y-5">
      <!-- Title Row -->
      <div class="flex items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <h2 class="text-xl font-extrabold text-gray-900 tracking-tight">{{ t('macro.title') }}</h2>
          <span class="inline-flex items-center rounded-full bg-blue-50 px-2.5 py-0.5 text-xs font-bold text-blue-700 border border-blue-200 shadow-2xs">
            {{ t('macro.suite') }}
          </span>
        </div>
      </div>

      <!-- Active Macro & Actions Box -->
      <div class="bg-slate-50/90 p-4 rounded-xl border border-slate-200/80 space-y-4 shadow-inner">
        <!-- First Line: Active Macro Selection -->
        <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div class="text-xs font-bold uppercase tracking-wider text-indigo-950 flex items-center gap-1.5 shrink-0">
            <BookOpen :size="15" class="text-indigo-600 shrink-0" />
            <span>Active Macro</span>
          </div>
          <div class="relative inline-flex items-center group w-full sm:w-auto flex-1 max-w-xl">
            <select
              id="macro-library-select"
              :value="selectedName"
              :disabled="editingLocked"
              class="appearance-none w-full bg-white hover:bg-indigo-50/60 border border-indigo-200/90 text-indigo-950 font-extrabold text-sm rounded-xl pl-4 pr-9 py-2.5 shadow-2xs hover:shadow-sm focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-indigo-500 transition-all cursor-pointer disabled:opacity-50"
              @change="onSelectChange"
            >
              <option v-for="name in libraryNames" :key="name" :value="name">{{ name }}</option>
            </select>
            <div class="absolute right-3 pointer-events-none flex items-center text-indigo-500 transition-transform group-hover:translate-y-0.5">
              <ChevronDown :size="15" />
            </div>
          </div>
        </div>

        <!-- Second Line: Management Actions operating on the selected macro -->
        <div class="pt-3.5 border-t border-slate-200/80 flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div class="text-xs font-bold uppercase tracking-wider text-gray-500 flex items-center gap-1.5">
            <span>{{ t('macro.macroActions') }}</span>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <button
              id="macro-btn-save"
              type="button"
              class="inline-flex items-center gap-1.5 px-4 py-2 text-xs font-extrabold rounded-xl bg-emerald-600 text-white hover:bg-emerald-700 shadow-sm hover:shadow active:scale-95 transition-all disabled:opacity-40 disabled:pointer-events-none cursor-pointer"
              :disabled="editingLocked"
              @click="saveCurrent"
            >
              <Save :size="14" class="text-emerald-100 shrink-0" />
              <span>{{ t('macro.save') }}</span>
            </button>
            <button
              id="macro-btn-save-as"
              type="button"
              class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-white text-slate-800 border border-slate-300 hover:bg-slate-100 hover:border-slate-400 shadow-2xs hover:shadow-sm active:scale-95 transition-all disabled:opacity-40 disabled:pointer-events-none cursor-pointer"
              :disabled="editingLocked"
              @click="saveAs"
            >
              <Copy :size="14" class="text-slate-600 shrink-0" />
              <span>{{ t('macro.saveAs') }}</span>
            </button>
            <button
              id="macro-btn-rename"
              type="button"
              class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-blue-50 text-blue-700 border border-blue-200 hover:bg-blue-100 hover:border-blue-300 shadow-2xs hover:shadow-sm active:scale-95 transition-all disabled:opacity-40 disabled:pointer-events-none cursor-pointer"
              :disabled="editingLocked"
              @click="renameCurrent"
            >
              <Pencil :size="14" class="text-blue-600 shrink-0" />
              <span>{{ t('macro.rename') }}</span>
            </button>
            <button
              id="macro-btn-delete"
              type="button"
              class="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-bold rounded-xl bg-rose-50 text-rose-700 border border-rose-200 hover:bg-rose-100 hover:border-rose-300 shadow-2xs hover:shadow-sm active:scale-95 transition-all disabled:opacity-40 disabled:pointer-events-none cursor-pointer"
              :disabled="editingLocked"
              @click="deleteCurrent"
            >
              <Trash2 :size="14" class="text-rose-600 shrink-0" />
              <span>{{ t('macro.delete') }}</span>
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Playback Controls -->
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-md transition-all space-y-4">
      <div class="flex flex-wrap items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <button
            v-if="!isPlaying"
            id="macro-play-btn"
            type="button"
            class="inline-flex items-center justify-center gap-2 bg-gradient-to-r from-emerald-500 to-teal-600 hover:from-emerald-600 hover:to-teal-700 disabled:from-gray-300 disabled:to-gray-300 disabled:text-gray-500 text-white font-extrabold text-sm px-6 py-3 rounded-xl transition-all shadow-md hover:shadow-lg active:scale-95 cursor-pointer"
            :disabled="!connected || doc.instructions.length === 0"
            @click="startPlayback"
          >
            <Play :size="16" class="fill-white" />
            <span>{{ t('macro.play') }}</span>
          </button>
          <button
            v-else
            id="macro-stop-btn"
            type="button"
            class="inline-flex items-center justify-center gap-2 bg-gradient-to-r from-amber-500 to-orange-600 hover:from-amber-600 hover:to-orange-700 text-white font-extrabold text-sm px-6 py-3 rounded-xl transition-all shadow-md hover:shadow-lg active:scale-95 cursor-pointer"
            @click="stopPlayback"
          >
            <Square :size="16" class="fill-white" />
            <span>{{ t('macro.stop') }}</span>
          </button>
          <button
            v-if="isPlaying"
            type="button"
            class="inline-flex items-center justify-center gap-2 bg-gray-100 hover:bg-gray-200 text-gray-700 font-bold text-sm px-4 py-3 rounded-xl transition-all border border-gray-300 shadow-2xs active:scale-95 cursor-pointer"
            @click="stopPlayback"
          >
            <span>{{ t('macro.stop') }}</span>
          </button>
          <label class="inline-flex items-center gap-2 text-sm font-bold text-gray-700 ml-3 select-none cursor-pointer">
            <input
              id="macro-loop-checkbox"
              type="checkbox"
              class="rounded border-gray-300 text-emerald-600 focus:ring-emerald-500 h-4.5 w-4.5 cursor-pointer"
              :checked="doc.loop"
              :disabled="editingLocked"
              @change="toggleLoop"
            >
            <span>{{ t('macro.loop') }}</span>
          </label>
        </div>
        <div class="text-right">
          <div id="macro-elapsed" class="flex items-baseline justify-end gap-1.5 text-3xl font-mono font-extrabold text-gray-900 tracking-tight">
            <span class="sr-only">{{ formatMacroTime(elapsedMs) }}</span>
            <input
              id="macro-elapsed-input"
              type="text"
              :value="formatMacroTime(elapsedMs)"
              :disabled="editingLocked"
              :title="t('macro.editTimestampToSeek')"
              class="w-28 bg-gray-50/90 hover:bg-gray-100 focus:bg-white text-gray-900 font-mono text-3xl font-extrabold border border-gray-300 focus:border-blue-500 rounded-xl px-2 py-0.5 text-right outline-none focus:ring-2 focus:ring-blue-200 transition-all cursor-text shadow-inner"
              @change="onElapsedChange(($event.target as HTMLInputElement).value, $event.target as HTMLInputElement)"
              @blur="onElapsedChange(($event.target as HTMLInputElement).value, $event.target as HTMLInputElement)"
              @keyup.enter="onElapsedChange(($event.target as HTMLInputElement).value, $event.target as HTMLInputElement); ($event.target as HTMLInputElement).blur()"
            >
            <span class="text-gray-400 text-xl font-semibold select-none">/ {{ formatMacroTime(durationMs) }}</span>
          </div>
          <div
            id="macro-status"
            class="text-xs font-bold uppercase tracking-wider mt-1"
            :class="isPlaying ? 'text-emerald-600 animate-pulse flex items-center justify-end gap-1.5' : 'text-gray-500'"
          >
            <span v-if="isPlaying" class="h-2 w-2 rounded-full bg-emerald-500 inline-block"></span>
            {{ statusMessage }}
          </div>
        </div>
      </div>

      <!-- Draggable Progress Bar for quick positioning -->
      <div class="pt-2 border-t border-gray-100">
        <div class="flex justify-between items-center text-xs font-bold text-gray-500 mb-1.5">
          <span class="inline-flex items-center gap-1.5">
            <span class="h-2 w-2 rounded-full" :class="isPlaying ? 'bg-emerald-500 animate-pulse' : 'bg-blue-500'"></span>
            <span>Progress: <strong class="text-gray-700 font-mono">{{ formatMacroTime(elapsedMs) }}</strong></span>
          </span>
          <span class="font-mono text-gray-500">{{ formatMacroTime(durationMs) }}</span>
        </div>
        <div class="relative flex items-center group/slider py-1">
          <input
            id="macro-progress-slider"
            type="range"
            :min="0"
            :max="durationMs"
            :step="10"
            :value="elapsedMs"
            :disabled="editingLocked"
            class="w-full h-3 bg-gray-200 rounded-lg appearance-none cursor-pointer accent-blue-600 focus:outline-none focus:ring-2 focus:ring-blue-400 focus:ring-offset-1 transition-all"
            @input="onSliderInput(($event.target as HTMLInputElement).value)"
            @change="onSliderChange(($event.target as HTMLInputElement).value)"
          >
        </div>
      </div>
    </div>

    <!-- Editor Section -->
    <div class="bg-white p-6 rounded-2xl border border-gray-200 shadow-lg relative transition-all">
      <div class="flex justify-between items-center mb-5 pb-4 border-b border-gray-100">
        <button
          id="macro-btn-capture"
          type="button"
          class="inline-flex items-center gap-1.5 px-4 py-2 rounded-xl bg-blue-50 text-blue-700 border border-blue-200 hover:bg-blue-100 font-bold text-xs shadow-2xs hover:shadow-sm transition-all active:scale-95 cursor-pointer disabled:opacity-40"
          :disabled="editingLocked || !connected"
          @click="captureCurrent"
        >
          <Plus :size="14" class="text-blue-600" />
          <span>{{ t('macro.capture') }}</span>
        </button>
        <button
          id="macro-btn-raw-toggle"
          type="button"
          class="inline-flex items-center gap-1.5 px-3.5 py-2 rounded-xl bg-gray-100 hover:bg-gray-200 text-gray-700 font-semibold text-xs border border-gray-300 shadow-2xs transition-all active:scale-95 cursor-pointer"
          @click="showRawJson = !showRawJson; if (showRawJson) syncRawFromDoc()"
        >
          <Code :size="14" class="text-gray-500" />
          <span>{{ showRawJson ? t('macro.closeRaw') : t('macro.rawJson') }}</span>
        </button>
      </div>

      <!-- Raw JSON Editor View -->
      <div v-if="showRawJson" class="space-y-4 animate-fade-in">
        <div>
          <label class="block text-xs font-bold text-gray-600 mb-2 uppercase tracking-wider">{{ t('macro.rawJsonLabel') }}</label>
          <textarea
            id="macro-raw-textarea"
            v-model="rawJsonText"
            rows="14"
            class="w-full bg-gray-50 border border-gray-300 rounded-xl p-4 font-mono text-xs text-gray-900 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:bg-white shadow-inner leading-relaxed transition-all"
            :disabled="editingLocked"
          ></textarea>
        </div>
        <div class="flex flex-wrap items-center justify-between gap-3 pt-3 border-t border-gray-100">
          <div class="flex items-center gap-2.5">
            <button
              id="macro-btn-apply-raw"
              type="button"
              class="inline-flex items-center gap-1.5 bg-blue-600 hover:bg-blue-700 text-white font-extrabold text-xs px-5 py-2 rounded-xl shadow-md hover:shadow-lg transition-all active:scale-95 disabled:opacity-40 cursor-pointer"
              :disabled="editingLocked"
              @click="applyRawJson"
            >
              <Check :size="15" class="stroke-[3]" />
              <span>{{ t('macro.applyRaw') }}</span>
            </button>
            <span
              v-if="rawJsonError"
              id="macro-raw-error"
              class="text-xs font-semibold text-red-600 bg-red-50 px-3 py-1 rounded-lg border border-red-200 animate-pulse"
            >{{ rawJsonError }}</span>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-xs font-bold text-gray-400 uppercase tracking-wider mr-1">{{ t('macro.fileActions') }}</span>
            <button
              id="macro-btn-import"
              type="button"
              class="inline-flex items-center gap-1.5 bg-gray-100 hover:bg-gray-200 text-gray-800 font-bold text-xs px-3.5 py-2 rounded-xl border border-gray-300 shadow-2xs hover:shadow-sm transition-all active:scale-95 disabled:opacity-40 cursor-pointer"
              :disabled="editingLocked"
              @click="importMacro"
            >
              <Upload :size="14" class="text-gray-600" />
              <span>{{ t('macro.import') }}</span>
            </button>
            <button
              id="macro-btn-export"
              type="button"
              class="inline-flex items-center gap-1.5 bg-gray-100 hover:bg-gray-200 text-gray-800 font-bold text-xs px-3.5 py-2 rounded-xl border border-gray-300 shadow-2xs hover:shadow-sm transition-all active:scale-95 cursor-pointer"
              @click="exportMacro"
            >
              <Download :size="14" class="text-gray-600" />
              <span>{{ t('macro.export') }}</span>
            </button>
            <input
              id="macro-file-input"
              ref="fileInputRef"
              type="file"
              accept=".json,application/json"
              class="hidden"
              @change="onFileSelected"
            >
          </div>
        </div>
      </div>

      <div v-else id="macro-instruction-list" class="space-y-1.5">
        <!-- Gap 0 -->
        <div
          :id="`macro-gap-0`"
          class="group/gap relative cursor-pointer transition-all my-1 py-3 -my-1.5 flex items-center justify-center select-none"
          :title="t('macro.seekGap')"
          @click="seekGap(0)"
        >
          <template v-if="nextIndex === 0">
            <div class="w-full flex items-center justify-between gap-2.5 px-3 py-1 bg-gradient-to-r from-emerald-50 via-teal-50 to-emerald-50 border border-emerald-300/80 rounded-xl text-emerald-700 font-extrabold text-sm shadow-sm transition-all scale-101">
              <span class="font-mono text-base tracking-tighter drop-shadow-2xs text-emerald-600">&gt;&gt;</span>
              <div class="h-2 flex-1 bg-gradient-to-r from-emerald-500 via-teal-400 to-emerald-500 rounded-full relative overflow-hidden shadow-inner">
                <div class="absolute inset-0 bg-white/40 animate-pulse"></div>
              </div>
              <span class="font-mono text-base tracking-tighter drop-shadow-2xs text-emerald-600">&lt;&lt;</span>
            </div>
          </template>
          <template v-else>
            <div class="w-full h-2 bg-gray-100 group-hover/gap:bg-blue-300 group-hover/gap:h-3 rounded-full transition-all shadow-inner"></div>
          </template>
        </div>

        <template v-for="(inst, idx) in doc.instructions" :key="inst._id || idx">
          <div
            :id="`macro-row-${idx}`"
            class="p-4 rounded-xl border transition-all shadow-2xs space-y-3"
            :class="idx < nextIndex
              ? 'bg-gray-50/80 border-gray-200 opacity-65'
              : 'bg-white border-gray-200/90 hover:border-blue-300 hover:shadow-md'"
          >
            <!-- Top Bar: Time Pill + Action Dropdown + Row Toolbar -->
            <div class="flex flex-wrap items-center justify-between gap-3 pb-2 border-b border-gray-100">
              <div class="flex items-center gap-2.5">
                <div class="flex items-center gap-1.5 bg-gray-50 border border-gray-200 rounded-lg px-2 py-1 shadow-2xs">
                  <span class="text-xs font-bold text-gray-400 uppercase tracking-wider">At</span>
                  <input
                    :id="`macro-row-${idx}-time`"
                    type="text"
                    :value="formatMacroTime(inst.at)"
                    :readonly="editingLocked"
                    class="w-20 bg-white text-gray-900 font-mono text-sm font-bold border border-gray-300 rounded px-1.5 py-0.5 focus:border-blue-500 focus:ring-2 focus:ring-blue-100 outline-none transition-all text-center"
                    @input="setTimeFromText(idx, ($event.target as HTMLInputElement).value, undefined, false)"
                    @change="setTimeFromText(idx, ($event.target as HTMLInputElement).value, $event.target as HTMLInputElement, true)"
                    @blur="setTimeFromText(idx, ($event.target as HTMLInputElement).value, $event.target as HTMLInputElement, true)"
                    @keyup.enter="setTimeFromText(idx, ($event.target as HTMLInputElement).value, $event.target as HTMLInputElement, true); ($event.target as HTMLInputElement).blur()"
                  >
                  <span class="text-[11px] font-bold text-gray-500 ml-0.5">{{ deltaLabel(idx) }}</span>
                </div>

                <select
                  :id="`macro-row-${idx}-action`"
                  :value="inst.action"
                  :disabled="editingLocked"
                  class="font-extrabold text-sm rounded-lg px-3 py-1.5 shadow-2xs focus:ring-2 cursor-pointer transition-all border"
                  :class="{
                    'bg-blue-50 border-blue-200 text-blue-700 focus:border-blue-500 focus:ring-blue-100': inst.action === 'set',
                    'bg-emerald-50 border-emerald-200 text-emerald-700 focus:border-emerald-500 focus:ring-emerald-100': inst.action === 'start',
                    'bg-amber-50 border-amber-200 text-amber-700 focus:border-amber-500 focus:ring-amber-100': inst.action === 'stop'
                  }"
                  @change="setAction(idx, ($event.target as HTMLSelectElement).value as MacroAction)"
                >
                  <option value="set">set</option>
                  <option value="start">start</option>
                  <option value="stop">stop</option>
                </select>
              </div>

              <!-- Row Tools Toolbar -->
              <div class="flex items-center gap-1 bg-gray-50 border border-gray-200/80 p-1 rounded-xl shadow-2xs">
                <button :id="`macro-row-${idx}-up`" type="button" class="p-1.5 rounded-lg text-gray-600 hover:text-gray-900 hover:bg-white hover:shadow-2xs transition-all disabled:opacity-30 cursor-pointer" :disabled="editingLocked || idx === 0" :title="t('macro.moveUp')" @click="moveRow(idx, -1)"><ArrowUp :size="14" /></button>
                <button :id="`macro-row-${idx}-down`" type="button" class="p-1.5 rounded-lg text-gray-600 hover:text-gray-900 hover:bg-white hover:shadow-2xs transition-all disabled:opacity-30 cursor-pointer" :disabled="editingLocked || idx === doc.instructions.length - 1" :title="t('macro.moveDown')" @click="moveRow(idx, 1)"><ArrowDown :size="14" /></button>
                <button :id="`macro-row-${idx}-dup`" type="button" class="p-1.5 rounded-lg text-gray-600 hover:text-blue-600 hover:bg-blue-50 hover:shadow-2xs transition-all disabled:opacity-30 cursor-pointer" :disabled="editingLocked" :title="t('macro.duplicate')" @click="duplicateRow(idx)"><Copy :size="14" /></button>
                <button :id="`macro-row-${idx}-insert`" type="button" class="p-1.5 rounded-lg text-gray-600 hover:text-emerald-600 hover:bg-emerald-50 hover:shadow-2xs transition-all disabled:opacity-30 cursor-pointer" :disabled="editingLocked" :title="t('macro.insertBelow')" @click="insertBelow(idx)"><Plus :size="14" /></button>
                <button :id="`macro-row-${idx}-delete`" type="button" class="p-1.5 rounded-lg text-gray-600 hover:text-red-600 hover:bg-red-50 hover:shadow-2xs transition-all disabled:opacity-30 cursor-pointer" :disabled="editingLocked" :title="t('macro.deleteRow')" @click="deleteRow(idx)"><Trash2 :size="14" /></button>
              </div>
            </div>

            <!-- Parameters Area -->
            <div class="flex flex-wrap items-center gap-3 text-sm pt-1">
              <template v-if="inst.action === 'start'">
                <span class="text-gray-600 font-semibold italic bg-emerald-50/50 px-3 py-1.5 rounded-lg border border-emerald-200/60 flex items-center gap-2">
                  <Play :size="14" class="text-emerald-600 fill-emerald-600" />
                  <span>{{ t('macro.resumeMotion') }}</span>
                </span>
              </template>
              <template v-else-if="inst.action === 'stop'">
                <div class="inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden">
                  <span class="text-xs font-bold text-gray-600 px-3 py-1.5 bg-gray-100 border-r border-gray-200">{{ t('macro.position') }}</span>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.01"
                    :value="inst.params?.position ?? ''"
                    :disabled="editingLocked"
                    class="w-24 bg-white px-3 py-1.5 text-gray-900 font-semibold text-sm focus:bg-blue-50/20 focus:outline-none transition-all"
                    :placeholder="t('macro.optional')"
                    @input="setParam(idx, 'position', Number(($event.target as HTMLInputElement).value))"
                    @change="setParam(idx, 'position', Number(($event.target as HTMLInputElement).value))"
                  >
                </div>
              </template>
              <template v-else>
                <!-- BPM (First input[type=number] inside row for tests) -->
                <div class="inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden">
                  <span class="text-xs font-bold text-gray-600 px-2.5 py-1.5 bg-gray-100 border-r border-gray-200">BPM</span>
                  <input
                    type="number"
                    min="10"
                    max="300"
                    :value="inst.params?.bpm ?? ''"
                    :disabled="editingLocked"
                    class="w-18 bg-white px-2.5 py-1.5 text-gray-900 font-semibold text-sm focus:bg-blue-50/20 focus:outline-none transition-all"
                    @input="setParam(idx, 'bpm', Number(($event.target as HTMLInputElement).value))"
                    @change="setParam(idx, 'bpm', Number(($event.target as HTMLInputElement).value))"
                  >
                </div>

                <!-- Depth -->
                <div class="inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden">
                  <span class="text-xs font-bold text-gray-600 px-2.5 py-1.5 bg-gray-100 border-r border-gray-200">{{ t('macro.depth') }}</span>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.01"
                    :value="inst.params?.depth ?? ''"
                    :disabled="editingLocked"
                    class="w-18 bg-white px-2.5 py-1.5 text-gray-900 font-semibold text-sm focus:bg-blue-50/20 focus:outline-none transition-all"
                    @input="setParam(idx, 'depth', Number(($event.target as HTMLInputElement).value))"
                    @change="setParam(idx, 'depth', Number(($event.target as HTMLInputElement).value))"
                  >
                </div>

                <!-- Wave -->
                <div class="inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden">
                  <span class="text-xs font-bold text-gray-600 px-2.5 py-1.5 bg-gray-100 border-r border-gray-200">{{ t('macro.wave') }}</span>
                  <select
                    :value="inst.params?.wave_func ?? 'sine'"
                    :disabled="editingLocked"
                    class="bg-white px-2.5 py-1.5 text-gray-900 font-semibold text-sm focus:bg-blue-50/20 focus:outline-none transition-all cursor-pointer border-0"
                    @change="setParam(idx, 'wave_func', ($event.target as HTMLSelectElement).value)"
                  >
                    <option v-for="w in waveOptions" :key="w" :value="w">{{ w }}</option>
                  </select>
                </div>

                <!-- Sharpness -->
                <div class="inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden">
                  <span class="text-xs font-bold text-gray-600 px-2.5 py-1.5 bg-gray-100 border-r border-gray-200">{{ t('macro.sharpness') }}</span>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.01"
                    :value="inst.params?.sharpness ?? ''"
                    :disabled="editingLocked"
                    class="w-18 bg-white px-2.5 py-1.5 text-gray-900 font-semibold text-sm focus:bg-blue-50/20 focus:outline-none transition-all"
                    @input="setParam(idx, 'sharpness', Number(($event.target as HTMLInputElement).value))"
                    @change="setParam(idx, 'sharpness', Number(($event.target as HTMLInputElement).value))"
                  >
                </div>

                <!-- Depth Top Checkbox -->
                <label
                  class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg border text-xs font-bold select-none cursor-pointer transition-all shadow-2xs"
                  :class="inst.params?.depth_top ? 'bg-blue-50 border-blue-300 text-blue-700' : 'bg-gray-50 border-gray-200 text-gray-700 hover:bg-gray-100'"
                >
                  <input
                    type="checkbox"
                    class="rounded border-gray-300 text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
                    :checked="!!inst.params?.depth_top"
                    :disabled="editingLocked"
                    @change="setParam(idx, 'depth_top', ($event.target as HTMLInputElement).checked)"
                  >
                  <span>{{ t('macro.depthTop') }}</span>
                </label>

                <!-- Reversed Checkbox -->
                <label
                  class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg border text-xs font-bold select-none cursor-pointer transition-all shadow-2xs"
                  :class="inst.params?.reversed ? 'bg-blue-50 border-blue-300 text-blue-700' : 'bg-gray-50 border-gray-200 text-gray-700 hover:bg-gray-100'"
                >
                  <input
                    type="checkbox"
                    class="rounded border-gray-300 text-blue-600 focus:ring-blue-500 h-4 w-4 cursor-pointer"
                    :checked="!!inst.params?.reversed"
                    :disabled="editingLocked"
                    @change="setParam(idx, 'reversed', ($event.target as HTMLInputElement).checked)"
                  >
                  <span>{{ t('macro.reversed') }}</span>
                </label>

                <!-- Spline Points (full width if spline) -->
                <div
                  v-if="inst.params?.wave_func === 'spline'"
                  class="w-full flex items-center rounded-lg border border-gray-200 bg-gray-50 shadow-2xs overflow-hidden mt-1"
                >
                  <span class="text-xs font-bold text-gray-600 px-3 py-1.5 bg-gray-100 border-r border-gray-200 shrink-0">{{ t('macro.splinePoints') }}</span>
                  <input
                    type="text"
                    :value="(inst.params?.spline_points ?? []).join(' ')"
                    :disabled="editingLocked"
                    class="flex-1 bg-white px-3 py-1.5 text-gray-900 font-mono text-xs focus:bg-blue-50/20 focus:outline-none transition-all"
                    @input="setParam(idx, 'spline_points', ($event.target as HTMLInputElement).value.trim().split(/\s+/).map(Number).filter(n => !Number.isNaN(n)))"
                    @change="setParam(idx, 'spline_points', ($event.target as HTMLInputElement).value.trim().split(/\s+/).map(Number).filter(n => !Number.isNaN(n)))"
                  >
                </div>
              </template>
            </div>
          </div>

          <!-- Gap after row -->
          <div
            :id="`macro-gap-${idx + 1}`"
            class="group/gap relative cursor-pointer transition-all my-1 py-3 -my-1.5 flex items-center justify-center select-none"
            :title="t('macro.seekGap')"
            @click="seekGap(idx + 1)"
          >
            <template v-if="nextIndex === idx + 1">
              <div class="w-full flex items-center justify-between gap-2.5 px-3 py-1 bg-gradient-to-r from-emerald-50 via-teal-50 to-emerald-50 border border-emerald-300/80 rounded-xl text-emerald-700 font-extrabold text-sm shadow-sm transition-all scale-101">
                <span class="font-mono text-base tracking-tighter drop-shadow-2xs text-emerald-600">&gt;&gt;</span>
                <div class="h-2 flex-1 bg-gradient-to-r from-emerald-500 via-teal-400 to-emerald-500 rounded-full relative overflow-hidden shadow-inner">
                  <div class="absolute inset-0 bg-white/40 animate-pulse"></div>
                </div>
                <span class="font-mono text-base tracking-tighter drop-shadow-2xs text-emerald-600">&lt;&lt;</span>
              </div>
            </template>
            <template v-else>
              <div class="w-full h-2 bg-gray-100 group-hover/gap:bg-blue-300 group-hover/gap:h-3 rounded-full transition-all shadow-inner"></div>
            </template>
          </div>
        </template>

        <button
          v-if="doc.instructions.length === 0"
          type="button"
          class="w-full py-8 text-sm font-bold text-gray-500 border-2 border-dashed border-gray-300 rounded-xl hover:border-blue-500 hover:text-blue-600 hover:bg-blue-50/50 transition-all cursor-pointer disabled:opacity-40"
          :disabled="editingLocked"
          @click="insertBelow(-1)"
        >
          {{ t('macro.addFirst') }}
        </button>
      </div>
    </div>
  </div>
</template>
