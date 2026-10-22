// The contract between the UI renderer and the main process. The preload exposes
// three generic entry points (invoke, send, on); the main process routes them by
// method name and only accepts calls from the browser UI it created.
import type {
  AppInfo,
  CommandId,
  ExtensionInfo,
  GlobalState,
  HistoryEntry,
  InternalPage,
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
}

export interface SendMethods {
  windowControl(action: 'minimize' | 'maximize' | 'close'): void
  setPanes(rects: PaneRect[]): void
  setOverlay(open: boolean): void
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
}

export interface EventMap {
  state: WindowState
  global: GlobalState
  settings: Settings
  command: UICommand
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
