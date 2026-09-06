import { createI18n } from 'vue-i18n'
import sharedEn from './locales/en.json'
import sharedZh from './locales/zh.json'

export const LOCALE_KEY = 'ossm_locale'
export type AppLocale = 'en' | 'zh'

export function resolveLocale(): AppLocale {
  const saved = localStorage.getItem(LOCALE_KEY)
  if (saved === 'en' || saved === 'zh') return saved
  return navigator.language.toLowerCase().startsWith('zh') ? 'zh' : 'en'
}

export function applyDocumentLang(locale: AppLocale) {
  document.documentElement.lang = locale === 'zh' ? 'zh' : 'en'
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function deepMerge(
  base: Record<string, unknown>,
  over: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...base }
  for (const [k, v] of Object.entries(over)) {
    const prev = out[k]
    if (isPlainObject(prev) && isPlainObject(v)) {
      out[k] = deepMerge(prev, v)
    } else {
      out[k] = v
    }
  }
  return out
}

/** Factory so each app can keep its own locale catalogs (merged over shared motion strings). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function createOssmI18n(messages: { en: object; zh: object }): any {
  const initial = resolveLocale()
  applyDocumentLang(initial)
  return createI18n({
    legacy: false,
    locale: initial,
    fallbackLocale: 'en',
    messages: {
      en: deepMerge(sharedEn as Record<string, unknown>, messages.en as Record<string, unknown>),
      zh: deepMerge(sharedZh as Record<string, unknown>, messages.zh as Record<string, unknown>),
    },
  } as never)
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function bindLocaleHelpers(i18n: any) {
  function setLocale(locale: AppLocale) {
    i18n.global.locale.value = locale
    localStorage.setItem(LOCALE_KEY, locale)
    applyDocumentLang(locale)
  }
  function toggleLocale() {
    setLocale(i18n.global.locale.value === 'zh' ? 'en' : 'zh')
  }
  return { setLocale, toggleLocale }
}
