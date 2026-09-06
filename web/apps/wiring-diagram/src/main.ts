import { createApp } from 'vue'
import App from './App.vue'
import * as htmlToImage from 'html-to-image'
import './badge.css'

;(window as any).htmlToImage = htmlToImage

createApp(App).mount('#app')

