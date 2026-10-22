// Focus sessions: for a set time, distracting sites wait behind a calm gate instead
// of loading. The user can still open one for five minutes after a short pause.
import { Notification } from 'electron'
import { hostMatches } from '@shared/settings'
import { hostOf } from '@shared/url'
import type { FocusState } from '@shared/types'
import { bus } from './events'
import { resourcePath, userPath } from './paths'
import { settings, t } from './settings'
import { addFocusMinutes } from './stats'
import { JsonFile } from './store'

const PASS_MINUTES = 5
const file = new JsonFile<FocusState>(userPath('focus.json'), 0)
let state: FocusState = { active: false, startedAt: 0, endsAt: 0, minutes: 0 }
let timer: NodeJS.Timeout | null = null
const passes = new Map<string, number>()

export function loadFocus(): void {
  const saved = file.read()
  if (saved?.active && saved.endsAt > Date.now()) {
    state = saved
    schedule()
  } else if (saved?.active) {
    // The session ended while the browser was closed.
    addFocusMinutes((saved.endsAt - saved.startedAt) / 60_000)
    file.writeNow({ active: false, startedAt: 0, endsAt: 0, minutes: 0 })
  }
}

export function focusState(): FocusState {
  return state
}

function schedule(): void {
  if (timer) clearTimeout(timer)
  timer = setTimeout(() => finish(true), Math.max(0, state.endsAt - Date.now()))
}

export function startFocus(minutes: number): void {
  const length = Math.min(240, Math.max(5, Math.round(minutes)))
  if (state.active) finish(false)
  const now = Date.now()
  state = { active: true, startedAt: now, endsAt: now + length * 60_000, minutes: length }
  passes.clear()
  file.writeNow(state)
  schedule()
  bus.emit('global')
}

export function stopFocus(): void {
  if (state.active) finish(false)
}

function finish(completed: boolean): void {
  if (timer) clearTimeout(timer)
  timer = null
  const end = Math.min(Date.now(), state.endsAt)
  const minutes = Math.max(0, Math.round((end - state.startedAt) / 60_000))
  addFocusMinutes(minutes)
  state = { active: false, startedAt: 0, endsAt: 0, minutes: 0 }
  passes.clear()
  file.writeNow(state)
  bus.emit('global')
  if (completed && Notification.isSupported()) {
    new Notification({
      title: t('focus.done.title'),
      body: t('focus.done.body', { n: minutes }),
      icon: resourcePath('icon.png'),
      silent: true,
    }).show()
  }
}

function matchingSite(url: string): string | null {
  const host = hostOf(url)
  if (!host) return null
  return settings().focusSites.find((site) => hostMatches(host, site)) ?? null
}

/** Whether a top-level navigation to `url` should wait behind the focus gate. */
export function shouldGate(url: string): boolean {
  if (!state.active || !/^https?:/.test(url)) return false
  const site = matchingSite(url)
  if (!site) return false
  const pass = passes.get(site)
  return !pass || pass < Date.now()
}

export function grantPass(url: string): void {
  const site = matchingSite(url)
  if (site) passes.set(site, Date.now() + PASS_MINUTES * 60_000)
}

export function silencingNotifications(): boolean {
  return state.active && settings().focusSilenceNotifications
}
