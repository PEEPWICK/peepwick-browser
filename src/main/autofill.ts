// Autofill: messages from the page script (src/preload/page.ts) about form fields,
// the suggestion popup under the field, filling what the person picks, and the
// "save password?" prompt after a sign-in. The origin always comes from the frame
// that sent the message, never from the message, and only the main frame counts.
import { ipcMain, WebContentsView, type IpcMainEvent, type Session, type WebFrameMain } from 'electron'
import { randomBytes } from 'node:crypto'
import { generatePassword } from '@shared/passwordcsv'
import { IPC_EVENT } from '@shared/ipc'
import type { Address, AutofillItem, AutofillPopupState, SaveLoginPrompt } from '@shared/types'
import { hostOf } from '@shared/url'
import { openPage } from './actions'
import { listAddresses, rememberEntries, suggestValues } from './formdata'
import { afterSubmit, credentialsFor, addLogin, loginFor, neverSave } from './passwords'
import { devServerURL, pagePreloadPath, preloadPath } from './paths'
import { tabForContents } from './registry'
import { uiSession } from './sessions'
import { settings, t } from './settings'
import type { Tab } from './tab'
import type { BrowserWin } from './window'

const CHANNEL = 'peepwick:autofill'
const ROW = 40
const MANAGE_ROW = 37
const PADDING = 10

type FieldKind = 'username' | 'password' | 'new-password' | 'text'

interface FocusMessage {
  kind: FieldKind
  rect: { x: number; y: number; width: number; height: number }
  zoom: number
  key: string
  value: string
  address: keyof Omit<Address, 'id'> | null
}

interface Suggestion {
  item: AutofillItem
  pick: () => void
}

/** The field a window's popup belongs to. */
interface Active {
  tab: Tab
  frame: WebFrameMain
  origin: string
  focus: FocusMessage
  suggestions: Suggestion[]
  selected: number
}

const active = new Map<number, Active>()
const popups = new Map<number, WebContentsView>()
const hideTimers = new Map<number, NodeJS.Timeout>()

export function installAutofill(session: Session): void {
  session.registerPreloadScript({ type: 'frame', id: 'peepwick-autofill', filePath: pagePreloadPath() })
}

// ── The popup view ───────────────────────────────────────────────────────────

function popupFor(win: BrowserWin): WebContentsView {
  let view = popups.get(win.id)
  if (view && !view.webContents.isDestroyed()) return view
  view = new WebContentsView({
    webPreferences: { session: uiSession(), preload: preloadPath(), sandbox: true, contextIsolation: true, nodeIntegration: false, spellcheck: false },
  })
  view.setBackgroundColor('#00000000')
  const dev = devServerURL()
  void view.webContents.loadURL(dev ? `${dev}/popup.html` : 'peepwick://ui/popup.html')
  // A new view takes focus as it first loads; give it back to the page being filled.
  view.webContents.once('did-finish-load', () => active.get(win.id)?.tab.webContents?.focus())
  view.webContents.on('will-navigate', (event) => event.preventDefault())
  popups.set(win.id, view)
  win.base.once('closed', () => {
    popups.delete(win.id)
    active.delete(win.id)
  })
  return view
}

/** Messages from a popup view: only picking and hovering. */
export function popupMessage(senderId: number, method: string, args: unknown[]): boolean {
  for (const [windowId, view] of popups) {
    if (view.webContents.id !== senderId) continue
    const state = active.get(windowId)
    const index = typeof args[0] === 'number' ? args[0] : -1
    if (state && method === 'autofillPick') pick(state, index)
    else if (state && method === 'autofillHover' && index >= 0 && index < state.suggestions.length) {
      state.selected = index
      render(state)
    }
    return true
  }
  return false
}

function render(state: Active): void {
  const view = popups.get(state.tab.win.id)
  if (!view) return
  const payload: AutofillPopupState = {
    items: state.suggestions.map((s) => s.item),
    selected: state.selected,
    dark: state.tab.win.isDark(),
    lang: settings().language,
  }
  const send = () => view.webContents.send(IPC_EVENT, 'autofill', payload)
  if (view.webContents.isLoading()) view.webContents.once('did-finish-load', send)
  else send()
  state.frame.send(CHANNEL, { type: 'popup', open: true, selected: state.selected >= 0 })
}

function show(state: Active): void {
  const win = state.tab.win
  const view = popupFor(win)
  const page = state.tab.view?.getBounds()
  if (!page) return
  const { rect, zoom } = state.focus
  const rows = state.suggestions.filter((s) => s.item.kind !== 'manage').length
  const height = rows * ROW + (state.suggestions.length > rows ? MANAGE_ROW : 0) + PADDING
  const width = Math.round(Math.min(440, Math.max(300, rect.width * zoom)))
  const { width: winWidth, height: winHeight } = win.base.getContentBounds()
  let x = Math.round(page.x + rect.x * zoom)
  let y = Math.round(page.y + (rect.y + rect.height) * zoom + 2)
  // No room below: above the field instead.
  if (y + height > winHeight - 8) y = Math.round(page.y + rect.y * zoom - height - 2)
  x = Math.max(8, Math.min(x, winWidth - width - 8))
  win.showPopup(view, { x, y: Math.max(8, y), width, height })
  render(state)
}

export function hideAutofill(win: BrowserWin): void {
  const state = active.get(win.id)
  active.delete(win.id)
  clearTimeout(hideTimers.get(win.id))
  win.hidePopup()
  if (state && !state.frame.isDestroyed()) state.frame.send(CHANNEL, { type: 'popup', open: false })
}

// ── Suggestions ──────────────────────────────────────────────────────────────

function fill(state: Active, message: Record<string, unknown>): void {
  const win = state.tab.win
  // The page may have navigated since: fill only the same origin.
  if (state.frame.isDestroyed() || originOf(state.frame.url) !== state.origin) return hideAutofill(win)
  state.frame.send(CHANNEL, message)
  hideAutofill(win)
  state.tab.webContents?.focus()
}

function pick(state: Active, index: number): void {
  const suggestion = state.suggestions[index]
  if (suggestion) suggestion.pick()
}

function suggestionsFor(state: Omit<Active, 'suggestions' | 'selected'>, incognito: boolean): Suggestion[] {
  const out: Suggestion[] = []
  const { focus, origin } = state
  const s = settings()
  const self = () => active.get(state.tab.win.id)!
  if (focus.kind === 'username' || focus.kind === 'password') {
    if (!s.passwordFill) return []
    for (const login of credentialsFor(origin)) {
      out.push({
        item: { kind: 'login', label: login.username || t('autofill.noUsername'), detail: `${login.exact ? '' : `${hostOf(login.origin)} · `}••••••••` },
        pick: () => {
          const values = loginFor(login.id, origin)
          if (values) fill(self(), { type: 'login', ...values })
        },
      })
    }
    if (out.length > 0) out.push({ item: { kind: 'manage', label: t('autofill.managePasswords') }, pick: () => manage(self(), 'passwords') })
    return out
  }
  if (focus.kind === 'new-password') {
    if (!s.passwordFill || incognito) return []
    const password = generatePassword((n) => randomBytes(n))
    out.push({
      item: { kind: 'generate', label: t('autofill.generate'), detail: password },
      pick: () => fill(self(), { type: 'generated', password }),
    })
    out.push({ item: { kind: 'manage', label: t('autofill.managePasswords') }, pick: () => manage(self(), 'passwords') })
    return out
  }
  if (!s.formFill) return []
  if (focus.address) {
    for (const address of listAddresses()) {
      const value = address[focus.address]
      if (!value || !value.toLocaleLowerCase().startsWith(focus.value.trim().toLocaleLowerCase())) continue
      out.push({
        item: { kind: 'address', label: value, detail: [address.name, address.street, address.city].filter((v) => v && v !== value).join(', ') },
        pick: () => fill(self(), { type: 'address', address }),
      })
    }
  }
  if (focus.key) {
    for (const value of suggestValues(focus.key, focus.value)) {
      if (out.some((o) => o.item.label === value)) continue
      out.push({ item: { kind: 'value', label: value }, pick: () => fill(self(), { type: 'value', value }) })
    }
  }
  if (out.some((o) => o.item.kind === 'address')) out.push({ item: { kind: 'manage', label: t('autofill.manageAddresses') }, pick: () => manage(self(), 'addresses') })
  return out.slice(0, 8)
}

function manage(state: Active, what: 'passwords' | 'addresses'): void {
  const win = state.tab.win
  hideAutofill(win)
  if (what === 'passwords') openPage(win, 'passwords')
  else openPage(win, 'settings', 'autofill')
}

function originOf(url: string): string | null {
  try {
    const u = new URL(url)
    return u.protocol === 'https:' || u.protocol === 'http:' ? u.origin : null
  } catch {
    return null
  }
}

// ── Saving after sign-in ─────────────────────────────────────────────────────

const pending = new Map<number, { win: BrowserWin; origin: string; username: string; password: string; timer: NodeJS.Timeout }>()
let nextPrompt = 1

function offerSave(win: BrowserWin, origin: string, username: string, password: string): void {
  const offer = afterSubmit(origin, username, password)
  if (offer === 'none') return
  // One prompt per window: a new sign-in replaces the old question.
  for (const [id, entry] of pending) if (entry.win === win) dropPrompt(id)
  const id = nextPrompt++
  const timer = setTimeout(() => answerSaveLogin(id, 'dismiss'), 3 * 60_000)
  pending.set(id, { win, origin, username, password, timer })
  const prompt: SaveLoginPrompt = { id, host: hostOf(origin).replace(/^www\./, ''), username, update: offer === 'update' }
  win.send('command', { type: 'save-login', prompt })
}

function dropPrompt(id: number): void {
  const entry = pending.get(id)
  if (!entry) return
  clearTimeout(entry.timer)
  pending.delete(id)
  if (!entry.win.base.isDestroyed()) entry.win.send('command', { type: 'save-login', prompt: null })
}

export function answerSaveLogin(id: number, answer: 'save' | 'never' | 'dismiss'): void {
  const entry = pending.get(id)
  if (!entry) return
  if (answer === 'save') addLogin({ origin: entry.origin, username: entry.username, password: entry.password })
  else if (answer === 'never') neverSave(entry.origin)
  dropPrompt(id)
}

// ── Messages from pages ──────────────────────────────────────────────────────

export function startAutofill(): void {
  ipcMain.on(CHANNEL, (event: IpcMainEvent, type: unknown, payload: unknown) => {
    const tab = tabForContents(event.sender.id)
    const frame = event.senderFrame
    if (!tab || !frame || frame !== event.sender.mainFrame) return
    const origin = originOf(frame.url)
    if (!origin) return
    const win = tab.win
    // Only the tab that's on screen has a popup, and not while it's leaving its page:
    // messages from the old page can arrive after the new one started loading.
    if (type !== 'submit' && (win.activeTab() !== tab || tab.navigating)) return
    switch (type) {
      case 'focus':
      case 'input': {
        clearTimeout(hideTimers.get(win.id))
        const previous = active.get(win.id)
        const message = payload as Partial<FocusMessage>
        let focus: FocusMessage
        if (type === 'focus') {
          if (!message || typeof message.kind !== 'string' || !message.rect) return
          focus = {
            kind: message.kind,
            rect: message.rect,
            zoom: typeof message.zoom === 'number' ? message.zoom : 1,
            key: String(message.key ?? '').slice(0, 100),
            value: String(message.value ?? '').slice(0, 200),
            address: message.address ?? null,
          }
        } else {
          if (!previous || previous.tab !== tab) return
          focus = { ...previous.focus, value: String(message?.value ?? '').slice(0, 200), rect: message?.rect ?? previous.focus.rect }
          // Logins and generated passwords don't narrow as you type.
          if (focus.kind !== 'text') return
        }
        const base = { tab, frame, origin, focus }
        const suggestions = suggestionsFor(base, win.incognito)
        if (suggestions.length === 0) return hideAutofill(win)
        const state: Active = { ...base, suggestions, selected: -1 }
        active.set(win.id, state)
        show(state)
        return
      }
      case 'blur': {
        // A click in the popup blurs the page first; give the pick a moment.
        clearTimeout(hideTimers.get(win.id))
        hideTimers.set(
          win.id,
          setTimeout(() => hideAutofill(win), 250),
        )
        return
      }
      case 'key': {
        const state = active.get(win.id)
        if (!state) return
        const count = state.suggestions.length
        if (payload === 'ArrowDown') state.selected = (state.selected + 1) % count
        else if (payload === 'ArrowUp') state.selected = state.selected <= 0 ? count - 1 : state.selected - 1
        else if (payload === 'Enter' && state.selected >= 0) return pick(state, state.selected)
        else if (payload === 'Escape' || payload === 'Tab') return hideAutofill(win)
        render(state)
        return
      }
      case 'submit': {
        const message = (payload ?? {}) as { username?: unknown; password?: unknown; entries?: unknown }
        if (win.incognito) return
        const s = settings()
        if (s.passwordSave && typeof message.password === 'string' && message.password) {
          offerSave(win, origin, typeof message.username === 'string' ? message.username.slice(0, 500) : '', message.password.slice(0, 1000))
        }
        if (s.formFill && Array.isArray(message.entries)) {
          rememberEntries(
            message.entries
              .filter((e): e is { key: string; value: string } => typeof e?.key === 'string' && typeof e?.value === 'string')
              .slice(0, 50)
              .map((e) => ({ key: e.key.slice(0, 100), value: e.value })),
          )
        }
        return
      }
    }
  })
}
