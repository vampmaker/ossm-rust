import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { viteSingleFileQuiet } from '@ossm/shared/vite-singlefile'

/** Inline `.wasm?arraybuffer` as an ArrayBuffer so the single-file HTML never fetches a sibling file. */
function wasmArrayBuffer(): Plugin {
  return {
    name: 'wasm-arraybuffer',
    enforce: 'pre',
    load(id) {
      const q = id.indexOf('?')
      if (q === -1) return null
      const file = id.slice(0, q)
      const query = new URLSearchParams(id.slice(q + 1))
      if (!file.endsWith('.wasm') || !query.has('arraybuffer')) return null
      const b64 = readFileSync(file).toString('base64')
      return `export default Uint8Array.from(atob(${JSON.stringify(b64)}), (c) => c.charCodeAt(0)).buffer`
    },
  }
}

export default defineConfig(({ mode }) => ({
  plugins: [wasmArrayBuffer(), vue(), viteSingleFileQuiet(), tailwindcss()],
  define: {
    'import.meta.env.VITE_OSSM_ALLOW_MOCK': JSON.stringify(
      process.env.VITE_OSSM_ALLOW_MOCK === 'true' || mode === 'development',
    ),
  },
  build: {
    assetsInlineLimit: 100_000_000,
  },
  optimizeDeps: {
    exclude: ['@ossm/shared', '@ossm/client'],
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
}))
