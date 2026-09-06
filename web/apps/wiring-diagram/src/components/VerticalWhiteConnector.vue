<template>
  <div class="vertical-white-connector">
    <div class="white-shroud-notch"></div>
    <div class="white-socket-cavity">
      <!-- Row 1: Pin 10 (5V) & Pin 5 (EN-) -->
      <div class="pin-row-2x5">
        <div id="pin-motor-5v" class="metal-pin" title="Pin 10 (5V)"></div>
        <div class="metal-pin" title="Pin 5 (EN-)"></div>
      </div>
      <!-- Row 2: Pin 9 (ZO) & Pin 4 (EN+) -->
      <div class="pin-row-2x5">
        <div class="metal-pin" title="Pin 9 (ZO)"></div>
        <div class="metal-pin" title="Pin 4 (EN+)"></div>
      </div>
      <!-- Row 3: Pin 8 (RDY) & Pin 3 (485B) -->
      <div class="pin-row-2x5">
        <div class="metal-pin" title="Pin 8 (RDY)"></div>
        <div id="pin-motor-b" class="metal-pin" title="Pin 3 (485B)"></div>
      </div>
      <!-- Row 4: Pin 7 (WR) & Pin 2 (485A) -->
      <div class="pin-row-2x5">
        <div class="metal-pin" title="Pin 7 (WR)"></div>
        <div id="pin-motor-a" class="metal-pin" title="Pin 2 (485A)"></div>
      </div>
      <!-- Row 5: Pin 6 (COM) & Pin 1 (NC) -->
      <div class="pin-row-2x5">
        <div id="pin-motor-com" class="metal-pin" title="Pin 6 (COM)"></div>
        <div class="metal-pin" title="Pin 1 (NC)"></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.vertical-white-connector {
  width: 24px;
  height: 44px;
  background: #f8fafc;
  border: 1.5px solid #94a3b8;
  border-radius: 2px;
  padding: 2px 1.5px;
  position: relative;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
}

.white-shroud-notch {
  width: 6px;
  height: 2px;
  background: #cbd5e1;
  border-radius: 0 0 1.5px 1.5px;
  margin: 0 auto 1.5px auto;
}

.white-socket-cavity {
  flex: 1;
  background: #e2e8f0;
  border: 1px solid #cbd5e1;
  border-radius: 1.5px;
  padding: 1.5px 1.5px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  box-sizing: border-box;
}

.pin-row-2x5 {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0 1.5px;
  height: 5.5px;
  box-sizing: border-box;
}

.metal-pin {
  width: 4px;
  height: 4px;
  background: #f59e0b;
  border: 0.6px solid #92400e;
  border-radius: 0.5px;
  position: relative;
  box-sizing: border-box;
}
</style>
