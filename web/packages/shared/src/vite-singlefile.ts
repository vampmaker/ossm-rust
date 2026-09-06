import { viteSingleFile } from 'vite-plugin-singlefile'
import type { Plugin } from 'vite'

/** Drop the plugin's blank `this.info("\\n")` which Vite 7 prints as a warning. */
export function viteSingleFileQuiet(): Plugin {
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
