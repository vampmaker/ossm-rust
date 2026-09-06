import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { viteSingleFileQuiet } from '@ossm/shared/vite-singlefile'

export default defineConfig({
  plugins: [
    vue(),
    viteSingleFileQuiet(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
})
