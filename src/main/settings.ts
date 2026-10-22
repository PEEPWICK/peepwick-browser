import { app } from 'electron'
import { defaultSettings, sanitizeSettingsPatch } from '@shared/settings'
import { systemLang, translate, type MessageKey, type Vars } from '@shared/i18n'
import type { Settings } from '@shared/types'
import { bus } from './events'
import { userPath } from './paths'
import { JsonFile } from './store'

const file = new JsonFile<Partial<Settings>>(userPath('settings.json'), 300)
let current: Settings

export function loadSettings(): Settings {
  const defaults = defaultSettings(systemLang(app.getLocale()))
  current = { ...defaults, ...sanitizeSettingsPatch(file.read()) }
  return current
}

export function settings(): Settings {
  return current
}

export function updateSettings(patch: unknown): Settings {
  const clean = sanitizeSettingsPatch(patch)
  if (Object.keys(clean).length === 0) return current
  current = { ...current, ...clean }
  file.schedule(current)
  bus.emit('settings')
  return current
}

export function flushSettings(): void {
  file.flush()
}

/** Translate with the user's current language. */
export function t(key: MessageKey, vars?: Vars): string {
  return translate(current.language, key, vars)
}
