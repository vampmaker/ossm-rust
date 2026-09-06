<template>
  <div class="vertical-green-connector">
    <div id="pin-motor-24v" class="green-pin-slot" title="Pin 1 (+V)">
      <div class="green-pin-contact"></div>
    </div>
    <div id="pin-motor-gnd" class="green-pin-slot" title="Pin 2 (GND)">
      <div class="green-pin-contact"></div>
    </div>
    <div class="green-pin-slot" title="Pin 3 (PU+)">
      <div class="green-pin-contact"></div>
    </div>
    <div class="green-pin-slot" title="Pin 4 (PU-)">
      <div class="green-pin-contact"></div>
    </div>
    <div class="green-pin-slot" title="Pin 5 (DIR+)">
      <div class="green-pin-contact"></div>
    </div>
    <div class="green-pin-slot" title="Pin 6 (DIR-)">
      <div class="green-pin-contact"></div>
    </div>
  </div>
</template>

<style scoped>
.vertical-green-connector {
  width: 23px;
  height: 63px;
  background: #16a34a;
  border: 1.5px solid #14532d;
  border-radius: 2px;
  padding: 2px 1.5px;
  position: relative;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  box-sizing: border-box;
}

.green-pin-slot {
  width: 17px;
  height: 8px;
  background: #15803d;
  border: 1px solid #14532d;
  border-radius: 1px;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  box-sizing: border-box;
}

.green-pin-contact {
  width: 4px;
  height: 5px;
  background: #f8fafc;
  border: 0.5px solid #64748b;
  border-radius: 0.5px;
  box-sizing: border-box;
}
</style>
