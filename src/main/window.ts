// A browser window: one UI view (sidebar, toolbar, built-in pages) and the views of
// the visible tabs. The UI measures where panes go and reports their rectangles;
// the window places tab views there. While a menu or the command bar is open the
// UI view is raised above the pages, with a transparent background.
import {
  BaseWindow,
  WebContentsView,
  nativeTheme,
  screen,
  type Rectangle,
  type Session,
} from 'electron'
import { randomUUID } from 'node:crypto'
import { IPC_EVENT, type EventMap, type EventName } from '@shared/ipc'
import { NEW_TAB_URL } from '@shared/url'
import { WORKSPACE_ICONS, type PaneRect, type SplitState, type UICommand, type WindowState, type WorkspaceIcon } from '@shared/types'
import { getExtensions } from './extensions'
import { bus } from './events'
import { devServerURL, preloadPath, resourcePath } from './paths'
import { pendingPromptFor } from './permissions'
import { BROWSING_PARTITION, uiSession } from './sessions'
import { settings, t } from './settings'
import { shieldActiveFor } from './shield'
import { handleShortcut } from './shortcuts'
import { Tab, type SavedTab, type TabOptions } from './tab'

export interface SavedWorkspace {
  id: string
  name: string
  icon: WorkspaceIcon
  activeIndex: number
  split?: { left: number; right: number; ratio: number }
  tabs: SavedTab[]
}

export interface SavedWindow {
  bounds?: Rectangle
  maximized?: boolean
  activeWorkspaceId: string
  workspaces: SavedWorkspace[]
}

interface Workspace {
  id: string
  name: string
  icon: WorkspaceIcon
  tabIds: number[]
  activeTabId: number | null
  split: SplitState | null
  /** Most recently used tabs first, to pick the next tab when one closes. */
  recent: number[]
}

interface ClosedTab {
  workspaceId: string
  index: number
  saved: SavedTab
}

export interface OpenTabOptions extends TabOptions {
  background?: boolean
  workspaceId?: string
  opener?: Tab
  index?: number
}

const GECE = '#0B0B0C'
const SIS = '#E7E6E2'

export class BrowserWin {
  static readonly all = new Map<number, BrowserWin>()
  static lastFocused: BrowserWin | null = null

  readonly base: BaseWindow
  readonly ui: WebContentsView
  readonly id: number
  readonly tabs = new Map<number, Tab>()
  workspaces: Workspace[] = []
  activeWorkspaceId = ''
  private panes: PaneRect[] = []
  private overlay = false
  private fullscreenTab: Tab | null = null
  private closedTabs: ClosedTab[] = []
  private dirty = false
  private uiReady = false
  private sleepTimer: NodeJS.Timeout

  constructor(
    readonly session: Session,
    readonly incognito: boolean,
    saved?: SavedWindow,
    urls: string[] = [],
  ) {
    const bounds = saved?.bounds && visibleOnScreen(saved.bounds) ? saved.bounds : defaultBounds()
    this.base = new BaseWindow({
      ...bounds,
      minWidth: 560,
      minHeight: 400,
      frame: false,
      show: false,
      backgroundColor: this.isDark() ? GECE : SIS,
      title: t('app.name'),
      icon: resourcePath('icon.png'),
    })
    this.id = this.base.id
    BrowserWin.all.set(this.id, this)
    BrowserWin.lastFocused = this

    this.ui = new WebContentsView({
      webPreferences: {
        session: uiSession(),
        preload: preloadPath(),
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        spellcheck: false,
        navigateOnDragDrop: false,
      },
    })
    this.ui.setBackgroundColor('#00000000')
    this.base.contentView.addChildView(this.ui)
    this.wireUI()
    this.wireWindow()

    this.restore(saved, urls)
    if (saved?.maximized) this.base.maximize()
    this.layout()
    const dev = devServerURL()
    void this.ui.webContents.loadURL(dev ? `${dev}/index.html` : 'peepwick://ui/index.html')
    this.sleepTimer = setInterval(() => this.sleepIdleTabs(), 60_000)
  }

  // ── Setup ───────────────────────────────────────────────────────────────────

  private wireUI(): void {
    const wc = this.ui.webContents
    wc.on('will-navigate', (event) => event.preventDefault())
    wc.setWindowOpenHandler(() => ({ action: 'deny' }))
    wc.on('before-input-event', (event, input) => {
      if (handleShortcut(this, input)) event.preventDefault()
    })
    wc.once('did-finish-load', () => {
      if (!this.base.isDestroyed()) this.base.show()
    })
    wc.on('render-process-gone', () => {
      // Recover from a crashed UI rather than leaving a blank window.
      if (!this.base.isDestroyed()) wc.reload()
    })
  }

  private wireWindow(): void {
    this.base.on('resize', () => this.layout())
    this.base.on('maximize', () => this.markDirty())
    this.base.on('unmaximize', () => this.markDirty())
    this.base.on('enter-full-screen', () => this.markDirty())
    this.base.on('leave-full-screen', () => this.markDirty())
    this.base.on('focus', () => {
      BrowserWin.lastFocused = this
    })
    this.base.on('closed', () => this.dispose())
    const onTheme = () => this.base.setBackgroundColor(this.isDark() ? GECE : SIS)
    nativeTheme.on('updated', onTheme)
    bus.on('settings', onTheme)
    this.base.once('closed', () => {
      nativeTheme.off('updated', onTheme)
      bus.off('settings', onTheme)
    })
  }

  private restore(saved: SavedWindow | undefined, urls: string[]): void {
    if (saved && saved.workspaces.length > 0) {
      for (const ws of saved.workspaces) {
        const workspace = this.addWorkspace(ws.name, ws.icon, ws.id)
        const tabs = ws.tabs.map((savedTab) => this.addTab(workspace, savedTab))
        const active = tabs[Math.min(Math.max(ws.activeIndex, 0), tabs.length - 1)]
        if (active) this.setActiveInWorkspace(workspace, active)
        if (ws.split) {
          const left = tabs[ws.split.left]
          const right = tabs[ws.split.right]
          if (left && right && left !== right) workspace.split = { left: left.id, right: right.id, ratio: ws.split.ratio }
        }
      }
      this.activeWorkspaceId = this.workspaces.some((w) => w.id === saved.activeWorkspaceId)
        ? saved.activeWorkspaceId
        : this.workspaces[0].id
    } else {
      const workspace = this.addWorkspace(t('ws.default'), 'circle')
      this.activeWorkspaceId = workspace.id
    }
    for (const url of urls) this.openTab({ url })
    const ws = this.activeWorkspace()
    if (ws.tabIds.length === 0) this.openTab({ url: NEW_TAB_URL })
  }

  private dispose(): void {
    clearInterval(this.sleepTimer)
    for (const tab of this.tabs.values()) tab.destroy()
    this.tabs.clear()
    if (!this.ui.webContents.isDestroyed()) this.ui.webContents.close()
    BrowserWin.all.delete(this.id)
    if (BrowserWin.lastFocused === this) BrowserWin.lastFocused = [...BrowserWin.all.values()].at(-1) ?? null
  }

  isDark(): boolean {
    const theme = settings().theme
    return this.incognito || theme === 'dark' || (theme === 'system' && nativeTheme.shouldUseDarkColors)
  }

  // ── Workspaces ──────────────────────────────────────────────────────────────

  private addWorkspace(name: string, icon: WorkspaceIcon, id: string = randomUUID()): Workspace {
    const workspace: Workspace = {
      id,
      name: name.slice(0, 40) || t('ws.default'),
      icon: WORKSPACE_ICONS.includes(icon) ? icon : 'circle',
      tabIds: [],
      activeTabId: null,
      split: null,
      recent: [],
    }
    this.workspaces.push(workspace)
    return workspace
  }

  activeWorkspace(): Workspace {
    return this.workspaces.find((w) => w.id === this.activeWorkspaceId) ?? this.workspaces[0]
  }

  workspaceOf(tab: Tab): Workspace | undefined {
    return this.workspaces.find((w) => w.tabIds.includes(tab.id))
  }

  createWorkspace(name: string, icon: WorkspaceIcon): void {
    if (this.workspaces.length >= 12) return
    const workspace = this.addWorkspace(name, icon)
    this.activeWorkspaceId = workspace.id
    this.openTab({ url: NEW_TAB_URL, workspaceId: workspace.id })
    this.layout()
    this.markDirty()
  }

  updateWorkspace(id: string, patch: { name?: string; icon?: WorkspaceIcon }): void {
    const workspace = this.workspaces.find((w) => w.id === id)
    if (!workspace) return
    if (typeof patch.name === 'string' && patch.name.trim()) workspace.name = patch.name.trim().slice(0, 40)
    if (patch.icon && WORKSPACE_ICONS.includes(patch.icon)) workspace.icon = patch.icon
    this.markDirty()
  }

  deleteWorkspace(id: string): void {
    if (this.workspaces.length <= 1) return
    const workspace = this.workspaces.find((w) => w.id === id)
    if (!workspace) return
    for (const tabId of [...workspace.tabIds]) {
      const tab = this.tabs.get(tabId)
      if (tab) this.closeTab(tab, { remember: false, keepWorkspaceAlive: true })
    }
    this.workspaces = this.workspaces.filter((w) => w !== workspace)
    if (this.activeWorkspaceId === id) this.activateWorkspace(this.workspaces[0].id)
    this.markDirty()
  }

  activateWorkspace(id: string): void {
    const workspace = this.workspaces.find((w) => w.id === id)
    if (!workspace) return
    this.activeWorkspaceId = id
    if (workspace.tabIds.length === 0) this.openTab({ url: NEW_TAB_URL, workspaceId: id })
    const active = this.activeTab()
    if (active) this.activateTab(active)
    this.layout()
    this.markDirty()
  }

  // ── Tabs ────────────────────────────────────────────────────────────────────

  activeTab(): Tab | null {
    const id = this.activeWorkspace().activeTabId
    return id === null ? null : (this.tabs.get(id) ?? null)
  }

  private addTab(workspace: Workspace, options: TabOptions, index?: number): Tab {
    const tab = new Tab(this, options)
    this.tabs.set(tab.id, tab)
    const pinnedCount = workspace.tabIds.filter((id) => this.tabs.get(id)?.pinned).length
    let position = index ?? workspace.tabIds.length
    position = tab.pinned ? Math.min(position, pinnedCount) : Math.max(position, pinnedCount)
    workspace.tabIds.splice(position, 0, tab.id)
    return tab
  }

  openTab(options: OpenTabOptions = {}): Tab {
    const workspace =
      this.workspaces.find((w) => w.id === options.workspaceId) ??
      (options.opener ? this.workspaceOf(options.opener) : undefined) ??
      this.activeWorkspace()
    let index = options.index
    if (index === undefined && options.opener) {
      // Open next to the tab that opened it, after its other children.
      const tabIds = workspace.tabIds
      let position = tabIds.indexOf(options.opener.id) + 1
      while (position < tabIds.length && this.tabs.get(tabIds[position])?.openerId === options.opener.id) position++
      index = position
    }
    const tab = this.addTab(workspace, { url: options.url, ...options }, index)
    if (options.opener) tab.openerId = options.opener.id
    if (!options.background) {
      this.activeWorkspaceId = workspace.id
      this.activateTab(tab)
    }
    this.markDirty()
    return tab
  }

  private setActiveInWorkspace(workspace: Workspace, tab: Tab): void {
    workspace.activeTabId = tab.id
    workspace.recent = [tab.id, ...workspace.recent.filter((id) => id !== tab.id)].slice(0, 30)
  }

  activateTab(tab: Tab): void {
    const workspace = this.workspaceOf(tab)
    if (!workspace) return
    const previous = this.activeTab()
    this.activeWorkspaceId = workspace.id
    this.setActiveInWorkspace(workspace, tab)
    tab.lastActive = Date.now()
    if (previous && previous !== tab) previous.lastActive = Date.now()
    // Reuse the current pane until the UI reports new geometry, to avoid a blank frame.
    const inSplit = workspace.split && (workspace.split.left === tab.id || workspace.split.right === tab.id)
    if (this.panes.length === 1 && !inSplit) this.panes = [{ ...this.panes[0], tabId: tab.id }]
    const wc = tab.webContents
    if (wc && !this.incognito) getExtensions()?.selectTab(wc)
    this.layout()
    if (!this.overlay && !tab.page && !tab.interstitial) tab.ensureView().webContents.focus()
    this.updateTitle()
    this.markDirty()
  }

  closeTab(tab: Tab, options: { remember?: boolean; keepWorkspaceAlive?: boolean } = {}): void {
    const workspace = this.workspaceOf(tab)
    if (!workspace || !this.tabs.has(tab.id)) return
    const index = workspace.tabIds.indexOf(tab.id)
    if (options.remember !== false && (!tab.page || tab.page !== 'newtab')) {
      this.closedTabs.push({ workspaceId: workspace.id, index, saved: tab.serialize() })
      if (this.closedTabs.length > 25) this.closedTabs.shift()
    }
    workspace.tabIds.splice(index, 1)
    workspace.recent = workspace.recent.filter((id) => id !== tab.id)
    if (workspace.split && (workspace.split.left === tab.id || workspace.split.right === tab.id)) workspace.split = null
    if (this.fullscreenTab === tab) this.fullscreenTab = null
    this.tabs.delete(tab.id)
    const wasActive = workspace.activeTabId === tab.id
    tab.destroy()

    if (wasActive) {
      workspace.activeTabId = null
      const opener = tab.openerId !== null ? this.tabs.get(tab.openerId) : undefined
      const next =
        (opener && workspace.tabIds.includes(opener.id) ? opener : undefined) ??
        workspace.recent.map((id) => this.tabs.get(id)).find((t) => t !== undefined) ??
        this.tabs.get(workspace.tabIds[Math.min(index, workspace.tabIds.length - 1)])
      if (next) this.setActiveInWorkspace(workspace, next)
    }
    if (workspace.tabIds.length === 0 && !options.keepWorkspaceAlive) {
      if (this.incognito && this.tabs.size === 0) {
        this.base.close()
        return
      }
      if (workspace.id === this.activeWorkspaceId) {
        this.openTab({ url: NEW_TAB_URL, workspaceId: workspace.id })
        return
      }
    }
    const active = this.activeTab()
    if (wasActive && active && workspace.id === this.activeWorkspaceId) this.activateTab(active)
    this.layout()
    this.markDirty()
  }

  reopenClosedTab(): void {
    const closed = this.closedTabs.pop()
    if (!closed) return
    const workspace = this.workspaces.find((w) => w.id === closed.workspaceId) ?? this.activeWorkspace()
    const tab = this.addTab(workspace, { ...closed.saved }, Math.min(closed.index, workspace.tabIds.length))
    this.activeWorkspaceId = workspace.id
    this.activateTab(tab)
  }

  duplicateTab(tab: Tab): void {
    const saved = tab.serialize()
    const workspace = this.workspaceOf(tab) ?? this.activeWorkspace()
    const copy = this.addTab(workspace, { ...saved, pinned: false }, workspace.tabIds.indexOf(tab.id) + 1)
    this.activateTab(copy)
  }

  moveTab(tab: Tab, workspaceId: string, index: number, pinned?: boolean): void {
    const from = this.workspaceOf(tab)
    const to = this.workspaces.find((w) => w.id === workspaceId)
    if (!from || !to) return
    from.tabIds.splice(from.tabIds.indexOf(tab.id), 1)
    from.recent = from.recent.filter((id) => id !== tab.id)
    if (from.split && (from.split.left === tab.id || from.split.right === tab.id)) from.split = null
    if (typeof pinned === 'boolean') tab.pinned = pinned
    const pinnedCount = to.tabIds.filter((id) => this.tabs.get(id)?.pinned).length
    let position = Math.max(0, Math.min(index, to.tabIds.length))
    position = tab.pinned ? Math.min(position, pinnedCount) : Math.max(position, pinnedCount)
    to.tabIds.splice(position, 0, tab.id)
    if (from.activeTabId === tab.id) {
      from.activeTabId = null
      const next = from.recent.map((id) => this.tabs.get(id)).find((t) => t !== undefined) ?? this.tabs.get(from.tabIds[0])
      if (next) this.setActiveInWorkspace(from, next)
      else if (from.id === this.activeWorkspaceId) {
        this.activeWorkspaceId = to.id
      }
    }
    if (from !== to && from.tabIds.length === 0 && from.id === this.activeWorkspaceId) {
      this.activeWorkspaceId = to.id
      this.activateTab(tab)
    }
    this.layout()
    this.markDirty()
  }

  togglePin(tab: Tab): void {
    const workspace = this.workspaceOf(tab)
    if (!workspace) return
    tab.pinned = !tab.pinned
    workspace.tabIds.splice(workspace.tabIds.indexOf(tab.id), 1)
    const pinnedCount = workspace.tabIds.filter((id) => this.tabs.get(id)?.pinned).length
    workspace.tabIds.splice(pinnedCount, 0, tab.id)
    this.markDirty()
  }

  tabsInOrder(workspace = this.activeWorkspace()): Tab[] {
    return workspace.tabIds.map((id) => this.tabs.get(id)).filter((t): t is Tab => t !== undefined)
  }

  // ── Split view ──────────────────────────────────────────────────────────────

  split(left: Tab, right: Tab): void {
    const workspace = this.workspaceOf(left)
    if (!workspace || left === right || this.workspaceOf(right) !== workspace) return
    workspace.split = { left: left.id, right: right.id, ratio: 0.5 }
    this.activateTab(left)
  }

  unsplit(): void {
    const workspace = this.activeWorkspace()
    if (!workspace.split) return
    workspace.split = null
    this.layout()
    this.markDirty()
  }

  setSplitRatio(ratio: number): void {
    const workspace = this.activeWorkspace()
    if (!workspace.split || !Number.isFinite(ratio)) return
    workspace.split.ratio = Math.min(0.8, Math.max(0.2, ratio))
    this.markDirty()
  }

  /** Clicking into a split pane makes that tab the active one. */
  paneFocused(tab: Tab): void {
    const workspace = this.activeWorkspace()
    if (!workspace.split || workspace.activeTabId === tab.id) return
    if (workspace.split.left !== tab.id && workspace.split.right !== tab.id) return
    this.setActiveInWorkspace(workspace, tab)
    this.updateTitle()
    this.markDirty()
  }

  /** The tab to split with when the user asks for side-by-side without choosing. */
  previousTab(): Tab | null {
    const workspace = this.activeWorkspace()
    const id = workspace.recent.find((tabId) => tabId !== workspace.activeTabId)
    return id !== undefined ? (this.tabs.get(id) ?? null) : null
  }

  // ── Layout ──────────────────────────────────────────────────────────────────

  setPanes(rects: PaneRect[]): void {
    this.panes = rects
      .filter((r) => this.tabs.has(r.tabId))
      .map((r) => ({
        tabId: r.tabId,
        x: Math.round(r.x),
        y: Math.round(r.y),
        width: Math.max(0, Math.round(r.width)),
        height: Math.max(0, Math.round(r.height)),
      }))
    this.layout()
  }

  setOverlay(open: boolean): void {
    if (this.overlay === open) return
    this.overlay = open
    this.layout()
    if (open) this.ui.webContents.focus()
    else this.focusActive()
  }

  focusActive(): void {
    const tab = this.activeTab()
    const wc = tab?.reader?.webContents ?? tab?.webContents
    if (tab && wc && !tab.page && !tab.interstitial) wc.focus()
    else this.ui.webContents.focus()
  }

  focusUI(): void {
    this.ui.webContents.focus()
  }

  setHtmlFullscreen(tab: Tab, on: boolean): void {
    this.fullscreenTab = on ? tab : this.fullscreenTab === tab ? null : this.fullscreenTab
    this.base.setFullScreen(on)
    this.layout()
    this.markDirty()
  }

  layout(): void {
    if (this.base.isDestroyed()) return
    const { width, height } = this.base.getContentBounds()
    this.ui.setBounds({ x: 0, y: 0, width, height })

    const placed: { view: WebContentsView; rect: Rectangle }[] = []
    const fullscreen = this.fullscreenTab
    if (fullscreen?.view) {
      placed.push({ view: fullscreen.view, rect: { x: 0, y: 0, width, height } })
    } else {
      for (const pane of this.panes) {
        const tab = this.tabs.get(pane.tabId)
        if (!tab || tab.page || tab.interstitial || !this.isShown(tab)) continue
        const rect = { x: pane.x, y: pane.y, width: pane.width, height: pane.height }
        placed.push({ view: tab.ensureView(), rect })
        if (tab.reader) placed.push({ view: tab.reader.view, rect })
      }
    }

    const desired = this.overlay ? [...placed.map((p) => p.view), this.ui] : [this.ui, ...placed.map((p) => p.view)]
    const container = this.base.contentView
    const current = container.children
    for (const child of current) if (!desired.includes(child as WebContentsView)) container.removeChildView(child)
    const order = container.children
    const sameOrder = order.length === desired.length && order.every((child, i) => child === desired[i])
    if (!sameOrder) for (const view of desired) container.addChildView(view)
    for (const { view, rect } of placed) view.setBounds(rect)
  }

  /** Whether the tab is in the active workspace and currently on screen. */
  isShown(tab: Tab): boolean {
    const workspace = this.activeWorkspace()
    if (!workspace.tabIds.includes(tab.id)) return false
    if (workspace.activeTabId === tab.id) return true
    const split = workspace.split
    const activeInSplit = split && (split.left === workspace.activeTabId || split.right === workspace.activeTabId)
    return !!activeInSplit && (split.left === tab.id || split.right === tab.id)
  }

  private sleepIdleTabs(): void {
    const minutes = settings().sleepAfterMinutes
    if (minutes <= 0) return
    const cutoff = Date.now() - minutes * 60_000
    for (const tab of this.tabs.values()) {
      if (tab.view && !this.isShown(tab) && tab.lastActive < cutoff) tab.sleep()
    }
  }

  sleepTab(tab: Tab): void {
    if (!this.isShown(tab)) tab.sleep()
  }

  // ── UI messaging ────────────────────────────────────────────────────────────

  markDirty(): void {
    if (this.dirty) return
    this.dirty = true
    setTimeout(() => {
      this.dirty = false
      if (this.base.isDestroyed()) return
      this.send('state', this.toState())
      bus.emit('window-dirty', this.id)
    }, 16)
  }

  send<K extends EventName>(event: K, payload: EventMap[K]): void {
    const wc = this.ui.webContents
    if (!wc.isDestroyed()) wc.send(IPC_EVENT, event, payload)
  }

  command(command: UICommand): void {
    this.focusUI()
    this.send('command', command)
  }

  markUIReady(): void {
    this.uiReady = true
  }

  get isUIReady(): boolean {
    return this.uiReady
  }

  updateTitle(): void {
    const tab = this.activeTab()
    const label = tab ? tab.displayTitle() : ''
    const suffix = this.incognito ? `${t('app.name')} · ${t('incognito.label')}` : t('app.name')
    if (!this.base.isDestroyed()) this.base.setTitle(label ? `${label} — ${suffix}` : suffix)
  }

  toState(): WindowState {
    const active = this.activeTab()
    const tabs: WindowState['tabs'] = {}
    for (const tab of this.tabs.values()) tabs[tab.id] = tab.toState()
    return {
      windowId: this.id,
      incognito: this.incognito,
      maximized: this.base.isMaximized(),
      fullscreen: !!this.fullscreenTab,
      workspaces: this.workspaces.map((w) => ({
        id: w.id,
        name: w.name,
        icon: w.icon,
        tabIds: [...w.tabIds],
        activeTabId: w.activeTabId,
        split: w.split ? { ...w.split } : null,
      })),
      activeWorkspaceId: this.activeWorkspaceId,
      tabs,
      permission: pendingPromptFor(this),
      shieldPaused: !!active && !active.page && /^https?:/.test(active.url) && !shieldActiveFor(active.url) && settings().shieldLevel !== 'off',
      extensionsPartition: this.incognito ? null : BROWSING_PARTITION,
    }
  }

  serialize(): SavedWindow {
    const normal = this.base.getNormalBounds()
    return {
      bounds: normal,
      maximized: this.base.isMaximized(),
      activeWorkspaceId: this.activeWorkspaceId,
      workspaces: this.workspaces.map((w) => {
        const tabs = this.tabsInOrder(w)
        const indexOf = (id: number) => tabs.findIndex((tab) => tab.id === id)
        return {
          id: w.id,
          name: w.name,
          icon: w.icon,
          activeIndex: w.activeTabId === null ? 0 : Math.max(0, indexOf(w.activeTabId)),
          ...(w.split ? { split: { left: indexOf(w.split.left), right: indexOf(w.split.right), ratio: w.split.ratio } } : {}),
          tabs: tabs.map((tab) => tab.serialize()),
        }
      }),
    }
  }
}

function defaultBounds(): Rectangle {
  const area = screen.getPrimaryDisplay().workArea
  const width = Math.min(1440, Math.round(area.width * 0.86))
  const height = Math.min(940, Math.round(area.height * 0.88))
  return {
    x: area.x + Math.round((area.width - width) / 2),
    y: area.y + Math.round((area.height - height) / 2),
    width,
    height,
  }
}

function visibleOnScreen(bounds: Rectangle): boolean {
  return screen.getAllDisplays().some((display) => {
    const area = display.workArea
    return (
      bounds.x < area.x + area.width - 100 &&
      bounds.x + bounds.width > area.x + 100 &&
      bounds.y < area.y + area.height - 50 &&
      bounds.y + bounds.height > area.y
    )
  })
}
