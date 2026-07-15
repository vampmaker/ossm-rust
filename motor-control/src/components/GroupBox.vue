<script setup lang="ts">
defineProps<{
  caption: string
}>()
</script>

<template>
  <fieldset class="group-box relative m-0 min-w-0 border border-[#999999] bg-[#ece9d8] px-2.5 pb-2 pt-1.5 shadow-[1px_1px_0px_#ffffff]">
    <legend class="px-1 text-[12px] font-medium text-gray-900 leading-none select-none">{{ caption }}</legend>
    <div class="min-h-0 h-full">
      <slot />
    </div>
  </fieldset>
</template>

