import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'
import tailwindcss from '@tailwindcss/vite'
import { viteSingleFile } from "vite-plugin-singlefile"
import type { Plugin } from 'vite'

/** Drop the plugin's blank `this.info("\\n")` which Vite 7 prints as a warning. */
function viteSingleFileQuiet(): Plugin {
  const plugin = viteSingleFile() as Plugin
  const generateBundle = plugin.generateBundle
  if (typeof generateBundle !== 'function') {
    return plugin
  }
  return {
    ...plugin,
    generateBundle(this, ...args) {
      this.info = () => {}
      return generateBundle.apply(this, args)
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    vueDevTools(),
    viteSingleFileQuiet(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    },
  },
})
