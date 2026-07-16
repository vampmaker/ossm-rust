import { createI18n } from 'vue-i18n'
import en from './locales/en.json'
import zh from './locales/zh.json'

export const LOCALE_KEY = 'ossm_locale'
export type AppLocale = 'en' | 'zh'

function resolveLocale(): AppLocale {
  const saved = localStorage.getItem(LOCALE_KEY)
  if (saved === 'en' || saved === 'zh') return saved
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en'
}

function applyDocumentLang(locale: AppLocale) {
  document.documentElement.lang = locale === 'zh' ? 'zh' : 'en'
}

const initial = resolveLocale()
applyDocumentLang(initial)

export const i18n = createI18n({
  legacy: false,
  locale: initial,
  fallbackLocale: 'en',
  messages: { en, zh },
})

export function setLocale(locale: AppLocale) {
  i18n.global.locale.value = locale
  localStorage.setItem(LOCALE_KEY, locale)
  applyDocumentLang(locale)
}

export function toggleLocale() {
  setLocale(i18n.global.locale.value === 'zh' ? 'en' : 'zh')
}
