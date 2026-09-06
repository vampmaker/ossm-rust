import { createApp } from 'vue'
import App from './App.vue'
import { i18n } from './i18n'
import { bindBleI18n } from '@ossm/client'

bindBleI18n((key, params) => {
  const g = i18n.global as unknown as { t: (k: string, p?: Record<string, unknown>) => string }
  return String(g.t(key, params))
})
createApp(App).use(i18n).mount('#app')
