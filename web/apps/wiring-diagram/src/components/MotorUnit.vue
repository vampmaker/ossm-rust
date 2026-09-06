<template>
  <div class="motor-unit">
    <div class="motor-shaft"></div>
    <div class="motor-flange"></div>
    <div class="motor-body">
      <div class="body-seam"></div>
      <MotorSticker :type="type" />
      <div class="connector-bay" :class="{ 'connector-bay--back': type === 'back' }">
        <div v-if="type === 'front'" class="chassis-screw-hole"></div>
        <VerticalGreenConnector v-if="type === 'front'" />
        <VerticalWhiteConnector v-else />
        <div v-if="type === 'front'" class="chassis-screw-hole"></div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import MotorSticker from './MotorSticker.vue'
import VerticalGreenConnector from './VerticalGreenConnector.vue'
import VerticalWhiteConnector from './VerticalWhiteConnector.vue'

defineProps<{
  type: 'front' | 'back'
}>()
</script>

<style scoped>
.motor-unit {
  display: flex;
  align-items: center;
  position: relative;
  height: 160px;
  box-sizing: border-box;
}

.motor-shaft {
  width: 56px;
  height: 22px;
  background: #94a3b8;
  border-radius: 2px 0 0 2px;
  border: 1.5px solid #475569;
  border-right: none;
  position: relative;
  flex-shrink: 0;
  box-sizing: border-box;
}

.motor-flange {
  width: 9px;
  height: 156px;
  background: #d8851a;
  border: 1.5px solid #782d06;
  border-radius: 1px;
  position: relative;
  z-index: 2;
  flex-shrink: 0;
  box-sizing: border-box;
}

.motor-body {
  width: 216px;
  height: 144px;
  background: #d8851a;
  border: 1.5px solid #782d06;
  border-left: none;
  border-radius: 0 2px 2px 0;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding: 3px 6px;
  flex-shrink: 0;
  box-sizing: border-box;
}

.body-seam {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 36px;
  width: 1.5px;
  background: #782d06;
}

.connector-bay {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: space-between;
  height: 86px;
  z-index: 4;
  position: relative;
  margin-right: 6px;
}

.connector-bay--back {
  justify-content: center;
}

.chassis-screw-hole {
  width: 5px;
  height: 5px;
  background: #270e04;
  border: 1px solid #782d06;
  border-radius: 50%;
  margin: 1px 0;
}
</style>
