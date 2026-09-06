<template>
  <div class="app-layout">
    <NavBar />

    <main class="diagram-wrapper">
      <PathAView v-if="currentRoute === 'std'" lang="en" />
      <PathAView v-else-if="currentRoute === 'std-zh'" lang="zh" />
      <PathBView v-else-if="currentRoute === 'esp'" lang="en" />
      <PathBView v-else-if="currentRoute === 'esp-zh'" lang="zh" />
    </main>
  </div>
</template>

<script setup lang="ts">
import { currentRoute } from './router'
import NavBar from './components/NavBar.vue'
import PathAView from './views/PathAView.vue'
import PathBView from './views/PathBView.vue'
</script>

<style>
* {
  box-sizing: border-box;
}

body {
  margin: 0;
  padding: 24px;
  background: #ffffff;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
  color: #0f172a;
  display: flex;
  justify-content: center;
}

.app-layout {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
}

.diagram-wrapper {
  display: flex;
  justify-content: center;
  width: 100%;
}
</style>
