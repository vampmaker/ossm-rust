import en from './locales/en.json'
import zh from './locales/zh.json'
import { createOssmI18n, bindLocaleHelpers, type AppLocale } from '@ossm/shared'

export type { AppLocale }
export const i18n = createOssmI18n({ en, zh })
const helpers = bindLocaleHelpers(i18n)
export const setLocale = helpers.setLocale
export const toggleLocale = helpers.toggleLocale
