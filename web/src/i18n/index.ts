import { createInstance, type i18n } from 'i18next'
import { initReactI18next } from 'react-i18next'

import { en } from './en'
import { zh } from './zh'

export const languages = ['en', 'zh'] as const
export type Language = (typeof languages)[number]

/// Where each language lives. The path picks the language, so the server and the browser
/// always render the same one.
export const paths: Record<Language, '/' | '/zh'> = { en: '/', zh: '/zh' }
export const names: Record<Language, string> = { en: 'English', zh: '中文' }
export const htmlLang: Record<Language, string> = { en: 'en', zh: 'zh-Hans' }

const resources = { en: { translation: en }, zh: { translation: zh } }

declare module 'i18next' {
  interface CustomTypeOptions {
    resources: (typeof resources)['en']
  }
}

export function languageOf(pathname: string): Language {
  return pathname === '/zh' || pathname.startsWith('/zh/') ? 'zh' : 'en'
}

const instances = new Map<Language, i18n>()

/// One fixed-language instance per language: requests for different languages share a Worker,
/// so nothing may switch a shared instance under another render.
export function i18nFor(lng: Language): i18n {
  let instance = instances.get(lng)
  if (!instance) {
    instance = createInstance()
    instance.use(initReactI18next).init({
      lng,
      fallbackLng: 'en',
      resources,
      initAsync: false,
      interpolation: { escapeValue: false },
    })
    instances.set(lng, instance)
  }
  return instance
}
