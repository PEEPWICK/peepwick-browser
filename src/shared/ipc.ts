// The contract between the UI renderer and the main process. The preload exposes
// three generic entry points (invoke, send, on); the main process routes them by
// method name and only accepts calls from the browser UI it created.
import type { BookmarkTree } from './bookmarks'
import type {
  AppInfo,
  CommandId,
  ExtensionInfo,
  GlobalState,
  HistoryEntry,
  ImportResult,
  ImportSource,
  InternalPage,
  MenuClickEvent,
  PaneRect,
  PermissionKind,
  Settings,
  SitePermission,
  Stats,
  Suggestion,
  TopSite,
  UICommand,
  WindowState,
  WorkspaceIcon,
} from './types'

export interface InitialData {
  state: WindowState
  global: GlobalState
  settings: Settings
  app: AppInfo
  bookmarks: BookmarkTree
}

export type BookmarkOpen = 'current' | 'tab' | 'background' | 'window' | 'incognito'

export interface BookmarkInput {
  parentId: string
  index?: number
  title: string
  /** Absent for folders. */
  url?: string
}

export interface ClearDataOptions {
  history: boolean
  cookies: boolean
  cache: boolean
}

export interface InvokeMethods {
  ready(): InitialData
  suggest(input: string): Suggestion[]
  palette(input: string): Suggestion[]
  historyQuery(query: { text: string; before?: number; limit?: number }): HistoryEntry[]
  topSites(): TopSite[]
  stats(): Stats
  extensionsList(): ExtensionInfo[]
  permissionsList(): SitePermission[]
  clearData(options: ClearDataOptions): boolean
  appInfo(): AppInfo
  setDefaultBrowser(): boolean
  shieldReport(): { host: string; count: number }[]
  bookmarkCreate(input: BookmarkInput): string | null
  bookmarkPage(): { id: string; created: boolean } | null
  importSources(): ImportSource[]
  importBookmarks(sourceId: string, skipDuplicates: boolean): ImportResult | null
  exportBookmarks(): boolean
}

export interface SendMethods {
  windowControl(action: 'minimize' | 'maximize' | 'close'): void
  setPanes(rects: PaneRect[]): void
  setOverlay(open: boolean): void
  /** Raises the UI over the pages without moving focus (tooltips). */
  setRaised(raised: boolean): void
  menuSelect(menuId: number, itemId: number, event: MenuClickEvent): void
  menuClosed(menuId: number): void
  confirmReply(id: number, ok: boolean): void
  appMenu(x: number, y: number): void
  newTab(options?: { url?: string; background?: boolean }): void
  closeTab(id: number): void
  activateTab(id: number): void
  navigate(id: number, input: string): void
  back(id: number): void
  forward(id: number): void
  reload(id: number, hard?: boolean): void
  stop(id: number): void
  togglePin(id: number): void
  toggleMute(id: number): void
  moveTab(id: number, workspaceId: string, index: number, pinned?: boolean): void
  tabMenu(id: number, x: number, y: number): void
  openPage(page: InternalPage, hash?: string): void
  openURL(url: string, newTab: boolean): void
  split(left: number, right: number): void
  unsplit(): void
  splitRatio(ratio: number): void
  focusTab(id: number): void
  createWorkspace(name: string, icon: WorkspaceIcon): void
  updateWorkspace(id: string, patch: { name?: string; icon?: WorkspaceIcon }): void
  deleteWorkspace(id: string): void
  activateWorkspace(id: string): void
  find(text: string, forward: boolean, findNext: boolean): void
  stopFind(): void
  zoom(direction: 'in' | 'out' | 'reset'): void
  toggleReader(): void
  shieldToggleSite(): void
  focusStart(minutes: number): void
  focusStop(): void
  interstitial(tabId: number, action: 'retry' | 'proceed' | 'back' | 'open-anyway'): void
  command(id: CommandId): void
  permission(id: number, allow: boolean, remember: boolean): void
  download(id: string, action: 'open' | 'show' | 'cancel' | 'pause' | 'resume' | 'remove'): void
  downloadsClear(): void
  updateCheck(): void
  updateInstall(): void
  setSettings(patch: Partial<Settings>): void
  historyRemove(id: number): void
  historyClear(): void
  extensionRemove(id: string): void
  extensionOptions(id: string): void
  permissionRemove(origin: string, kind: PermissionKind): void
  onboardingDone(): void
  bookmarkUpdate(id: string, patch: { title?: string; url?: string }): void
  bookmarkMove(ids: string[], parentId: string, index: number): void
  bookmarkRemove(ids: string[]): void
  bookmarkUndo(): void
  bookmarkSort(folderId: string): void
  bookmarkOpen(ids: string[], how: BookmarkOpen): void
  bookmarkMenu(id: string, x: number, y: number): void
  bookmarkFolderMenu(folderId: string, x: number, y: number, fromIndex?: number): void
  bookmarkTabs(): void
}

export interface EventMap {
  state: WindowState
  global: GlobalState
  settings: Settings
  command: UICommand
  bookmarks: BookmarkTree
}

export type InvokeName = keyof InvokeMethods
export type SendName = keyof SendMethods
export type EventName = keyof EventMap

export const IPC_INVOKE = 'pw:invoke'
export const IPC_SEND = 'pw:send'
export const IPC_EVENT = 'pw:event'

/** Shape of `window.peepwick`, exposed by the UI preload. */
export interface PreloadBridge {
  invoke(method: string, ...args: unknown[]): Promise<unknown>
  send(method: string, ...args: unknown[]): void
  on(event: string, listener: (payload: unknown) => void): () => void
  platform: string
}
