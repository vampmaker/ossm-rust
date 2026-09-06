import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'
import tailwindcss from '@tailwindcss/vite'
import { viteSingleFileQuiet } from '@ossm/shared/vite-singlefile'

export default defineConfig({
  plugins: [
    vue(),
    vueDevTools(),
    viteSingleFileQuiet(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  optimizeDeps: {
    exclude: ['@ossm/shared', '@ossm/client'],
  },
})
