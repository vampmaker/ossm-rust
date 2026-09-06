import { createApp } from 'vue'
import { setConnectionMode } from '@ossm/client'
import App from './App.vue'
import { i18n } from './i18n'
import './style.css'

setConnectionMode('wifi')
createApp(App).use(i18n).mount('#app')
