// A tab owns at most one WebContentsView. Built-in pages (new tab, settings…),
// error screens and the focus gate are drawn by the UI instead, and sleeping tabs
// have no view at all until they are shown again.
import { dialog, WebContentsView, type WebContents, type WebContentsViewConstructorOptions } from 'electron'
import { NEW_TAB_URL, hostOf, internalPage, internalURL } from '@shared/url'
import type { MessageKey } from '@shared/i18n'
import type { FindState, InternalPage, Interstitial, TabState, TranslationState } from '@shared/types'
import { getExtensions } from './extensions'
import { resolveFavicon, storedFavicon } from './favicons'
import { grantPass, shouldGate } from './focus'
import { addVisit, updateTitle } from './history'
import { showPageMenu } from './menus'
import { hideAutofill } from './autofill'
import { recordLoadError } from './diagnostics'
import { denyPendingFor } from './permissions'
import { ReaderView, extractArticle, isReaderable, renderArticle, type Article } from './reader'
import { registerContents, unregisterContents } from './registry'
import { allowHttp, httpsFallback, httpsSettled } from './httpsonly'
import { allowCertificate, securityFor } from './security'
import { savedSource } from './readinglist'
import { passDangerous, type Threat } from './safebrowsing'
import { canTranslate, defaultTarget, downloadSize, ensureModels, pageLanguage, restorePage, translatePage } from './translation'
import { rememberZoom, zoomFor } from './zoom'
import { settings, t } from './settings'
import { handleShortcut } from './shortcuts'
import type { BrowserWin } from './window'

export interface SavedEntry {
  url: string
  title: string
  pageState?: string
}

export interface SavedHistory {
  entries: SavedEntry[]
  index: number
}

export interface SavedTab {
  url: string
  title: string
  pinned: boolean
  muted: boolean
  history?: SavedHistory
}

export interface TabOptions {
  url?: string
  title?: string
  pinned?: boolean
  muted?: boolean
  history?: SavedHistory
  /** Options handed over by setWindowOpenHandler, to adopt a page opened by script. */
  adopt?: WebContentsViewConstructorOptions
}

const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5]
const PAGE_TITLES: Record<InternalPage, MessageKey> = {
  newtab: 'newtab.title',
  settings: 'settings.title',
  history: 'history.title',
  extensions: 'ext.title',
  bookmarks: 'bookmarks.title',
  downloads: 'downloads.title',
  passwords: 'passwords.title',
  reading: 'reading.title',
}
/** Views we closed on purpose (sleep, close), as opposed to pages closing themselves. */
const closedByUs = new WeakSet<WebContents>()
let nextId = 1

export class Tab {
  readonly id = nextId++
  view: WebContentsView | null = null
  url: string
  title: string
  favicon: string | null = null
  pinned: boolean
  muted: boolean
  loading = false
  audible = false
  blocked = 0
  readonly blockedHosts = new Map<string, number>()
  page: InternalPage | null
  interstitial: Interstitial | null = null
  readerAvailable = false
  reader: ReaderView | null = null
  find: FindState | null = null
  lastActive = Date.now()
  openerId: number | null = null
  /** Between a new page starting to load and it replacing the old one. */
  navigating = false
  media: 'playing' | 'paused' | null = null
  /** The article reader view shows, for reading it aloud. */
  readerArticle: Article | null = null
  pageLang: string | null = null
  translation: TranslationState | null = null
  /** Bumped whenever a translation should stop: a new one, a new page, closing the bar. */
  private translationRun = 0
  private savedHistory: SavedHistory | null
  private readerToken = 0

  constructor(
    readonly win: BrowserWin,
    options: TabOptions = {},
  ) {
    this.url = options.url ?? NEW_TAB_URL
    this.title = options.title ?? ''
    this.pinned = options.pinned ?? false
    this.muted = options.muted ?? false
    this.savedHistory = options.history ?? null
    this.page = internalPage(this.url)
    if (!this.page) this.favicon = storedFavicon(this.url)
    if (options.adopt) this.createView(options.adopt)
  }

  get webContents(): WebContents | null {
    return this.view && !this.view.webContents.isDestroyed() ? this.view.webContents : null
  }

  get sleeping(): boolean {
    return !this.view && !this.page && !this.interstitial
  }

  /** URL of the page currently loaded in the view, for first/third-party checks. */
  pageURL(): string | null {
    return this.webContents?.getURL() || (this.page ? null : this.url)
  }

  // ── View lifecycle ──────────────────────────────────────────────────────────

  /** Creates the view if needed, restoring saved history or loading the tab's URL. */
  ensureView(): WebContentsView {
    if (this.view && !this.view.webContents.isDestroyed()) return this.view
    const view = this.createView()
    const wc = view.webContents
    if (this.savedHistory && this.savedHistory.entries.length > 0) {
      const { entries, index } = this.savedHistory
      this.savedHistory = null
      void wc.navigationHistory.restore({ entries, index }).catch(() => this.loadInView(this.url))
    } else if (!this.page && !this.interstitial) {
      this.loadInView(this.url)
    }
    return view
  }

  private createView(adopt?: WebContentsViewConstructorOptions): WebContentsView {
    const view = adopt
      ? new WebContentsView(adopt)
      : new WebContentsView({
          webPreferences: {
            session: this.win.session,
            sandbox: true,
            contextIsolation: true,
            nodeIntegration: false,
            spellcheck: true,
            navigateOnDragDrop: false,
            safeDialogs: true,
            // Settings › Appearance; like Chrome, monospace text keeps its smaller ratio.
            defaultFontSize: settings().fontSize,
            defaultMonospaceFontSize: Math.round((settings().fontSize * 13) / 16),
          },
        })
    view.setBackgroundColor('#FFFFFF')
    view.setBorderRadius(4)
    this.view = view
    const wc = view.webContents
    // A busy page briefly holds many listeners (pending loads, cosmetic filter scripts,
    // extension observers); they go away when loading stops, so this is not a leak.
    wc.setMaxListeners(50)
    registerContents(wc.id, this)
    wc.setAudioMuted(this.muted)
    this.wire(wc)
    if (!this.win.incognito) getExtensions()?.addTab(wc, this.win.base)
    return view
  }

  private wire(wc: WebContents): void {
    wc.on('did-start-loading', () => {
      this.loading = true
      this.changed()
    })
    wc.on('did-stop-loading', () => {
      this.navigating = false
      this.loading = false
      this.changed()
      void this.checkReader(wc)
      void this.detectLanguage(wc)
    })
    wc.on('did-start-navigation', (details) => {
      if (!details.isMainFrame || details.isSameDocument) return
      this.navigating = true
      if (this.win.activeTab() === this) hideAutofill(this.win)
      if (this.interstitial && this.interstitial.kind !== 'focus') this.interstitial = null
      if (this.find) this.stopFind()
      this.closeReader()
      this.readerAvailable = false
      this.win.layout()
      this.changed()
    })
    wc.on('did-navigate', (_event, url) => {
      httpsSettled(wc.id)
      this.navigating = false
      this.media = null
      this.pageLang = null
      this.translation = null
      this.translationRun++
      wc.setZoomFactor(zoomFor(url, this.win.incognito))
      if (this.win.activeTab() === this) hideAutofill(this.win)
      if (this.interstitial) return
      const hostChanged = hostOf(url) !== hostOf(this.url)
      this.url = url
      this.page = null
      this.blocked = 0
      this.blockedHosts.clear()
      if (hostChanged) this.favicon = storedFavicon(savedSource(url) ?? url)
      if (!this.win.incognito) addVisit(url, wc.getTitle())
      this.changed()
    })
    wc.on('did-navigate-in-page', (_event, url, isMainFrame) => {
      if (!isMainFrame) return
      this.url = url
      if (!this.win.incognito) addVisit(url, wc.getTitle())
      this.changed()
    })
    wc.on('page-title-updated', (_event, title) => {
      this.title = title
      if (!this.win.incognito) updateTitle(wc.getURL(), title)
      this.changed()
      this.win.updateTitle()
    })
    wc.on('page-favicon-updated', (_event, favicons) => {
      const pageURL = wc.getURL()
      void resolveFavicon(this.win.session, favicons, pageURL, !this.win.incognito).then((data) => {
        if (data && !wc.isDestroyed() && wc.getURL() === pageURL) {
          this.favicon = data
          this.changed()
        }
      })
    })
    wc.on('did-fail-load', (_event, code, description, url, isMainFrame) => {
      if (!isMainFrame || code === -3) return
      // Our own warnings cancel the request they replace.
      if (this.interstitial && ['focus', 'https-only', 'dangerous'].includes(this.interstitial.kind)) return
      const http = httpsFallback(wc.id, url, code)
      if (http) {
        this.httpsOnlyFailed(http)
        return
      }
      // The shield's own blocks (-20) aren't failures worth reporting.
      if (code !== -20) recordLoadError(description)
      this.interstitial = { kind: 'error', code, description, url, certificate: code <= -200 && code > -300 }
      this.loading = false
      this.win.layout()
      this.changed()
    })
    wc.on('render-process-gone', (_event, details) => {
      if (details.reason === 'clean-exit') return
      this.interstitial = { kind: 'crashed' }
      this.media = null
      this.loading = false
      this.win.layout()
      this.changed()
    })
    wc.on('media-started-playing', () => {
      this.media = 'playing'
      this.changed()
    })
    wc.on('media-paused', () => {
      if (this.media === 'playing') this.media = 'paused'
      this.changed()
    })
    wc.on('audio-state-changed', (event) => {
      this.audible = event.audible
      this.changed()
    })
    wc.on('found-in-page', (_event, result) => {
      if (!this.find) return
      this.find = { ...this.find, matches: result.matches, active: result.activeMatchOrdinal }
      this.changed()
    })
    wc.on('zoom-changed', (_event, direction) => this.zoom(direction))
    wc.on('enter-html-full-screen', () => this.win.setHtmlFullscreen(this, true))
    wc.on('leave-html-full-screen', () => this.win.setHtmlFullscreen(this, false))
    wc.on('context-menu', (_event, params) => showPageMenu(this, wc, params))
    wc.on('before-input-event', (event, input) => {
      if (handleShortcut(this.win, input)) event.preventDefault()
    })
    wc.on('focus', () => this.win.paneFocused(this))
    // The answer is needed synchronously, so this stays a system dialog (themed
    // through GTK_THEME, see early.ts) rather than one drawn by the UI.
    wc.on('will-prevent-unload', (event) => {
      const choice = dialog.showMessageBoxSync(this.win.base, {
        type: 'question',
        buttons: [t('dialog.leave'), t('common.cancel')],
        defaultId: 1,
        cancelId: 1,
        message: wc.getTitle() || hostOf(wc.getURL()),
        detail: t('dialog.leaveBody'),
      })
      if (choice === 0) event.preventDefault()
    })
    wc.setWindowOpenHandler((details) => {
      if (!/^(https?|about|blob|data):/.test(details.url) && details.url !== '') return { action: 'deny' }
      const background = details.disposition === 'background-tab'
      return {
        action: 'allow',
        outlivesOpener: true,
        createWindow: (options) => {
          const tab = this.win.openTab({ adopt: options as WebContentsViewConstructorOptions, background, opener: this })
          return tab.view!.webContents
        },
      }
    })
    const contentsId = wc.id
    wc.on('destroyed', () => {
      unregisterContents(contentsId)
      if (this.view?.webContents === wc) this.view = null
      // A page closed itself (window.close() after a sign-in popup, for example).
      if (!closedByUs.has(wc)) this.win.closeTab(this, { remember: false })
    })
  }

  private destroyView(): void {
    const wc = this.webContents
    this.closeReader()
    if (wc) {
      closedByUs.add(wc)
      getExtensions()?.removeTab(wc)
      unregisterContents(wc.id)
      wc.close()
    }
    this.view = null
  }

  /** Frees the view of a background tab; it reloads from saved history when shown. */
  sleep(): boolean {
    const wc = this.webContents
    if (!wc || this.audible || this.page || wc.isCurrentlyAudible()) return false
    this.savedHistory = this.captureHistory() ?? null
    this.destroyView()
    this.loading = false
    this.find = null
    this.changed()
    return true
  }

  destroy(): void {
    denyPendingFor(this.id)
    this.destroyView()
  }

  // ── Navigation ──────────────────────────────────────────────────────────────

  private loadInView(url: string): void {
    const wc = this.webContents ?? this.createView().webContents
    void wc.loadURL(url).catch(() => {})
  }

  /** Loads a URL, showing a built-in page or the focus gate when appropriate. */
  load(url: string): void {
    const page = internalPage(url)
    this.interstitial = null
    if (page) {
      this.showPage(page)
      return
    }
    this.page = null
    if (shouldGate(url)) {
      this.url = url
      this.holdForFocus(url)
      return
    }
    this.url = url
    if (this.sleeping && !this.view) this.savedHistory = null
    this.loadInView(url)
    this.win.layout()
    this.changed()
  }

  showPage(page: InternalPage, hash = ''): void {
    this.page = page
    this.url = internalURL(page, hash)
    this.title = ''
    this.favicon = null
    this.interstitial = null
    this.closeReader()
    this.win.layout()
    this.changed()
  }

  /** Called from the request pipeline when a top-level load hits the focus gate. */
  holdForFocus(url: string): void {
    this.interstitial = { kind: 'focus', url, host: hostOf(url).replace(/^www\./, '') }
    this.loading = false
    this.win.layout()
    this.changed()
  }

  canGoBack(): boolean {
    if (this.page && this.webContents?.getURL()) return true
    if (this.interstitial) return true
    return this.webContents?.navigationHistory.canGoBack() ?? false
  }

  canGoForward(): boolean {
    if (this.page || this.interstitial?.kind === 'focus') return false
    return this.webContents?.navigationHistory.canGoForward() ?? false
  }

  back(): void {
    const wc = this.webContents
    if (this.page && wc?.getURL()) {
      this.page = null
      this.url = wc.getURL()
      this.title = wc.getTitle()
      this.favicon = storedFavicon(this.url)
      this.win.layout()
      this.changed()
      return
    }
    if (this.interstitial) {
      const wasFocus = this.interstitial.kind === 'focus'
      this.interstitial = null
      if (wasFocus && wc && wc.getURL() && !shouldGate(wc.getURL())) {
        // The gated link was clicked on a page that is still loaded underneath.
        this.url = wc.getURL()
        this.title = wc.getTitle()
      } else if (wc?.navigationHistory.canGoBack()) {
        wc.navigationHistory.goBack()
      } else {
        this.showPage('newtab')
        return
      }
      this.win.layout()
      this.changed()
      return
    }
    if (wc?.navigationHistory.canGoBack()) wc.navigationHistory.goBack()
  }

  forward(): void {
    const wc = this.webContents
    if (wc?.navigationHistory.canGoForward()) wc.navigationHistory.goForward()
  }

  reload(hard = false): void {
    if (this.page) return
    if (this.interstitial) {
      this.retry()
      return
    }
    const wc = this.webContents
    if (!wc) {
      this.ensureView()
      return
    }
    if (hard) wc.reloadIgnoringCache()
    else wc.reload()
  }

  stop(): void {
    this.webContents?.stop()
  }

  retry(): void {
    const target = this.interstitial && 'url' in this.interstitial ? this.interstitial.url : this.url
    this.interstitial = null
    if (target && !internalPage(target)) this.load(target)
    else this.reload()
  }

  openAnyway(): void {
    if (this.interstitial?.kind !== 'focus') return
    const url = this.interstitial.url
    grantPass(url)
    this.load(url)
  }

  /** The HTTPS version of an HTTP address didn't work; ask before going unencrypted. */
  httpsOnlyFailed(url: string): void {
    if (this.interstitial?.kind === 'https-only' && this.interstitial.url === url) return
    this.interstitial = { kind: 'https-only', url }
    this.url = url
    this.loading = false
    this.webContents?.stop()
    this.win.layout()
    this.changed()
  }

  /** Called from the request pipeline when a top-level load is on a malware or phishing list. */
  showDangerous(url: string, threat: Threat): void {
    this.interstitial = { kind: 'dangerous', url, threat }
    this.url = url
    this.title = ''
    this.favicon = null
    this.loading = false
    this.win.layout()
    this.changed()
  }

  continueToDangerous(): void {
    if (this.interstitial?.kind !== 'dangerous') return
    const url = this.interstitial.url
    passDangerous(url)
    this.load(url)
  }

  continueOverHttp(): void {
    if (this.interstitial?.kind !== 'https-only') return
    const url = this.interstitial.url
    allowHttp(url)
    this.load(url)
  }

  proceedInsecure(): void {
    if (this.interstitial?.kind !== 'error' || !this.interstitial.certificate) return
    const url = this.interstitial.url
    allowCertificate(hostOf(url))
    this.load(url)
  }

  // ── Page tools ──────────────────────────────────────────────────────────────

  setMuted(muted: boolean): void {
    this.muted = muted
    this.webContents?.setAudioMuted(muted)
    this.changed()
  }

  /** Applies the remembered or default zoom again, after either changed in Settings. */
  refreshZoom(): void {
    const wc = this.webContents
    if (wc && !this.page && !this.interstitial) wc.setZoomFactor(zoomFor(wc.getURL(), this.win.incognito))
  }

  zoomFactor(): number {
    return this.webContents?.getZoomFactor() ?? 1
  }

  zoom(direction: 'in' | 'out' | 'reset'): void {
    const wc = this.reader?.webContents ?? this.webContents
    if (!wc) return
    const current = wc.getZoomFactor()
    let next = 1
    if (direction === 'in') next = ZOOM_STEPS.find((step) => step > current + 0.001) ?? current
    if (direction === 'out') next = [...ZOOM_STEPS].reverse().find((step) => step < current - 0.001) ?? current
    wc.setZoomFactor(next)
    if (wc === this.webContents) rememberZoom(wc.getURL(), next, this.win.incognito)
    this.changed()
  }

  findInPage(text: string, forward: boolean, findNext: boolean): void {
    const wc = this.webContents
    if (!wc || this.page) return
    if (!text) {
      wc.stopFindInPage('clearSelection')
      this.find = { text: '', matches: 0, active: 0 }
      this.changed()
      return
    }
    const fresh = this.find?.text !== text
    this.find = { text, matches: this.find?.matches ?? 0, active: this.find?.active ?? 0 }
    wc.findInPage(text, { forward, findNext: fresh ? false : findNext })
  }

  stopFind(): void {
    this.webContents?.stopFindInPage('keepSelection')
    this.find = null
    this.changed()
  }

  recordBlocked(url: string): void {
    this.blocked++
    const host = hostOf(url)
    this.blockedHosts.set(host, (this.blockedHosts.get(host) ?? 0) + 1)
    this.changed()
  }

  private async checkReader(wc: WebContents): Promise<void> {
    if (!/^https?:/.test(wc.getURL())) return
    const url = wc.getURL()
    const available = await isReaderable(wc)
    if (!wc.isDestroyed() && wc.getURL() === url && available !== this.readerAvailable) {
      this.readerAvailable = available
      this.changed()
    }
  }

  async toggleReader(): Promise<void> {
    if (this.reader) {
      this.closeReader()
      this.win.layout()
      this.changed()
      return
    }
    const wc = this.webContents
    if (!wc || this.page || this.interstitial) return
    const token = ++this.readerToken
    const article = await extractArticle(wc)
    if (token !== this.readerToken || wc.isDestroyed()) return
    if (!article) {
      this.readerAvailable = false
      this.changed()
      return
    }
    const html = renderArticle(article, wc.getURL(), settings().language, this.win.isDark())
    this.readerArticle = article
    this.reader = new ReaderView(
      html,
      (url, newTab) => {
        if (newTab) this.win.openTab({ url, background: false, opener: this })
        else this.load(url)
      },
      wc.getZoomFactor(),
    )
    this.reader.webContents.on('context-menu', (_event, params) => showPageMenu(this, this.reader!.webContents, params))
    this.reader.webContents.on('before-input-event', (event, input) => {
      if (handleShortcut(this.win, input)) event.preventDefault()
    })
    this.win.layout()
    this.changed()
  }

  closeReader(): void {
    this.readerToken++
    this.readerArticle = null
    if (!this.reader) return
    this.reader.destroy()
    this.reader = null
  }

  // ── State ───────────────────────────────────────────────────────────────────

  private changed(): void {
    this.win.markDirty()
  }

  captureHistory(): SavedHistory | undefined {
    const wc = this.webContents
    if (!wc) return this.savedHistory ?? undefined
    const all = wc.navigationHistory.getAllEntries()
    if (all.length === 0) return undefined
    let index = wc.navigationHistory.getActiveIndex()
    let entries = all
    if (entries.length > 50) {
      const start = Math.max(0, Math.min(index - 25, entries.length - 50))
      entries = entries.slice(start, start + 50)
      index -= start
    }
    return {
      index: Math.max(0, Math.min(index, entries.length - 1)),
      entries: entries
        .filter((entry) => /^(https?|file):/.test(entry.url))
        .map((entry) => ({
          url: entry.url,
          title: entry.title,
          ...(entry.pageState && entry.pageState.length < 20_000 ? { pageState: entry.pageState } : {}),
        })),
    }
  }

  serialize(): SavedTab {
    const history = this.page ? undefined : this.captureHistory()
    return {
      url: this.url,
      title: this.title,
      pinned: this.pinned,
      muted: this.muted,
      ...(history && history.entries.length > 0 ? { history } : {}),
    }
  }

  displayURL(): string {
    if (this.page) return this.url.startsWith('peepwick:') ? this.url : internalURL(this.page)
    if (this.interstitial && 'url' in this.interstitial) return this.interstitial.url
    return this.url
  }

  displayTitle(): string {
    if (this.page) return t(PAGE_TITLES[this.page])
    if (this.interstitial && 'url' in this.interstitial) return hostOf(this.interstitial.url).replace(/^www\./, '')
    return this.title || hostOf(this.url).replace(/^www\./, '') || this.url
  }

  toState(): TabState {
    const url = this.displayURL()
    const security = securityFor(url)
    return {
      id: this.id,
      contentsId: this.webContents?.id ?? null,
      url,
      title: this.displayTitle(),
      favicon: this.page ? null : this.favicon,
      loading: this.loading,
      canGoBack: this.canGoBack(),
      canGoForward: this.canGoForward(),
      audible: this.audible,
      muted: this.muted,
      pinned: this.pinned,
      sleeping: this.sleeping,
      blocked: this.blocked,
      zoom: this.reader?.webContents.getZoomFactor() ?? this.zoomFactor(),
      security,
      page: this.page,
      interstitial: this.interstitial,
      readerAvailable: this.readerAvailable,
      reader: !!this.reader,
      find: this.find,
      media: this.media,
      pageLang: this.pageLang,
      translation: this.translation,
    }
  }

  // ── Translation ─────────────────────────────────────────────────────────────

  private async detectLanguage(wc: WebContents): Promise<void> {
    if (!/^https?:/.test(wc.getURL())) return
    const found = await pageLanguage(wc)
    // Only languages there are models for, so the address bar never offers what can't be done.
    const lang = found && canTranslate(found, defaultTarget(settings().language)) ? found : null
    if (!wc.isDestroyed() && lang !== this.pageLang) {
      this.pageLang = lang
      this.changed()
    }
  }

  private updateTranslation(run: number, patch: Partial<TranslationState>): void {
    if (run !== this.translationRun || !this.translation) return
    this.translation = { ...this.translation, ...patch }
    this.changed()
  }

  /** Opens the translate bar, or closes it when it is open. */
  async offerTranslation(): Promise<void> {
    if (this.translation) {
      this.closeTranslation()
      return
    }
    const wc = this.webContents
    if (!wc || this.page || this.interstitial) return
    const from = this.pageLang ?? (await pageLanguage(wc)) ?? 'en'
    this.chooseTranslation(from, defaultTarget(settings().language))
  }

  /** The bar's languages changed: look up how much would have to be downloaded. */
  chooseTranslation(from: string, to: string): void {
    const run = ++this.translationRun
    this.translation = { status: 'offer', from, to, progress: 0, download: -1 }
    this.changed()
    downloadSize(from, to).then(
      (download) => this.updateTranslation(run, { download }),
      (error: unknown) => this.updateTranslation(run, { download: 0, error: error instanceof Error && error.message === 'unsupported' ? 'unsupported' : 'offline' }),
    )
  }

  async startTranslation(from: string, to: string): Promise<void> {
    const wc = this.webContents
    if (!wc || this.page || this.interstitial || from === to) return
    const run = ++this.translationRun
    this.translation = { status: 'downloading', from, to, progress: 0, download: -1 }
    this.changed()
    try {
      await ensureModels(from, to, (progress) => this.updateTranslation(run, { progress }))
      this.updateTranslation(run, { status: 'translating', progress: 0, download: 0 })
      const finished = await translatePage(
        wc,
        from,
        to,
        (progress) => this.updateTranslation(run, { progress }),
        () => run !== this.translationRun || wc.isDestroyed(),
      )
      if (finished) this.updateTranslation(run, { status: 'done', progress: 1 })
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      console.warn('[translation]', message)
      const kind = message === 'unsupported' ? 'unsupported' : /HTTP|net::|checksum|fetch/i.test(message) ? 'offline' : 'failed'
      this.updateTranslation(run, { status: 'error', error: kind })
    }
  }

  async revertTranslation(): Promise<void> {
    const wc = this.webContents
    const current = this.translation
    this.translationRun++
    if (wc) await restorePage(wc)
    if (current) this.chooseTranslation(current.from, current.to)
  }

  /** Hides the bar; a translated page stays translated. */
  closeTranslation(): void {
    this.translationRun++
    this.translation = null
    this.changed()
  }

  // ── Media ───────────────────────────────────────────────────────────────────

  /** Runs a snippet in every frame (videos often live in embedded players). */
  private async inFrames<T>(code: string): Promise<T[]> {
    const wc = this.webContents
    if (!wc) return []
    const frames = wc.mainFrame.framesInSubtree.filter((f) => !f.isDestroyed())
    return Promise.all(frames.map((f) => f.executeJavaScript(code, true).catch(() => null))) as Promise<T[]>
  }

  /** Pauses what plays, or plays again what we paused. */
  async toggleMedia(): Promise<void> {
    const playing = await this.inFrames<boolean>(`(() => {
      const all = [...document.querySelectorAll('video, audio')]
      const now = all.filter((m) => !m.paused)
      now.forEach((m) => { m.dataset.pwPaused = '1'; m.pause() })
      return now.length > 0
    })()`)
    if (playing.some(Boolean)) return
    await this.inFrames(`(() => {
      const all = [...document.querySelectorAll('video, audio')]
      const ours = all.filter((m) => m.dataset.pwPaused)
      for (const m of ours.length ? ours : all.slice(0, 1)) { delete m.dataset.pwPaused; m.play().catch(() => {}) }
    })()`)
  }

  /** Puts the playing (or largest) video in a small window that stays on top; again to bring it back. */
  async pictureInPicture(): Promise<void> {
    await this.inFrames(`(async () => {
      if (document.pictureInPictureElement) return document.exitPictureInPicture()
      const videos = [...document.querySelectorAll('video')].filter((v) => v.readyState > 0)
      const area = (v) => v.clientWidth * v.clientHeight
      const video = videos.find((v) => !v.paused) ?? videos.sort((a, b) => area(b) - area(a))[0]
      if (!video) return
      // Some sites switch picture-in-picture off; the person asked for it.
      video.disablePictureInPicture = false
      return video.requestPictureInPicture()
    })()`)
  }
}
