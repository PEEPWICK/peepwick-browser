// State and message shapes shared by the main process, the preload and the UI.

export type Lang = 'tr' | 'en'
export type ThemePref = 'system' | 'dark' | 'light'
export type SearchEngineId = 'duckduckgo' | 'brave' | 'startpage' | 'google' | 'ecosia' | 'bing'
/** standard: ads and trackers · strict: also cookie notices and other annoyances */
export type ShieldLevel = 'off' | 'standard' | 'strict'
export type BookmarksBarMode = 'always' | 'newtab' | 'never'

export const WORKSPACE_ICONS = [
  'circle',
  'briefcase',
  'house',
  'book',
  'code',
  'palette',
  'music',
  'flask',
  'globe',
  'heart',
  'star',
  'flame',
] as const
export type WorkspaceIcon = (typeof WORKSPACE_ICONS)[number]

export interface Settings {
  language: Lang
  theme: ThemePref
  searchEngine: SearchEngineId
  searchSuggestions: boolean
  shieldLevel: ShieldLevel
  blockThirdPartyCookies: boolean
  sendGlobalPrivacyControl: boolean
  /** Registrable domains (eTLD+1) where the shield is switched off. */
  shieldAllowlist: string[]
  /** Domains that wait behind the focus gate while a focus session runs. */
  focusSites: string[]
  focusSilenceNotifications: boolean
  focusMinutes: number
  /** Background tabs sleep after this many minutes; 0 keeps them awake. */
  sleepAfterMinutes: number
  restoreSession: boolean
  askWhereToSave: boolean
  sidebarCollapsed: boolean
  bookmarksBar: BookmarksBarMode
  onboarded: boolean
}

export type TabSecurity = 'secure' | 'insecure' | 'internal' | 'local'

/** Something the UI draws in place of the page: an error, a crash or the focus gate. */
export type Interstitial =
  | { kind: 'error'; code: number; description: string; url: string; certificate: boolean }
  | { kind: 'crashed' }
  | { kind: 'focus'; url: string; host: string }

export type InternalPage = 'newtab' | 'settings' | 'history' | 'extensions' | 'bookmarks'

export const SETTINGS_SECTIONS = [
  'general',
  'appearance',
  'search',
  'tabs',
  'bookmarks',
  'privacy',
  'permissions',
  'focus',
  'downloads',
  'about',
] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

export interface TabState {
  id: number
  /** The page's webContents id, used by extension buttons; null while asleep. */
  contentsId: number | null
  url: string
  title: string
  favicon: string | null
  loading: boolean
  canGoBack: boolean
  canGoForward: boolean
  audible: boolean
  muted: boolean
  pinned: boolean
  sleeping: boolean
  /** Requests the shield blocked on the current page. */
  blocked: number
  zoom: number
  security: TabSecurity
  /** Set when the UI should render a built-in page instead of a web page. */
  page: InternalPage | null
  interstitial: Interstitial | null
  readerAvailable: boolean
  reader: boolean
  find: FindState | null
}

export interface FindState {
  text: string
  matches: number
  active: number
}

export interface SplitState {
  left: number
  right: number
  /** Width share of the left pane, 0.2–0.8. */
  ratio: number
}

export interface WorkspaceState {
  id: string
  name: string
  icon: WorkspaceIcon
  tabIds: number[]
  activeTabId: number | null
  split: SplitState | null
}

export interface PermissionPrompt {
  id: number
  tabId: number
  origin: string
  kinds: PermissionKind[]
}

export type PermissionKind =
  | 'camera'
  | 'microphone'
  | 'geolocation'
  | 'notifications'
  | 'midi'
  | 'clipboard'
  | 'screen'
  | 'external'
  | 'other'

export interface FocusState {
  active: boolean
  startedAt: number
  endsAt: number
  minutes: number
}

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'latest'
  | 'error'
  | 'unsupported'

export interface UpdateState {
  status: UpdateStatus
  version: string | null
  progress: number
  error: string | null
}

export type DownloadStatus = 'progressing' | 'paused' | 'completed' | 'cancelled' | 'interrupted'

export interface DownloadState {
  id: string
  filename: string
  path: string
  url: string
  received: number
  total: number
  status: DownloadStatus
  startedAt: number
}

export interface WindowState {
  windowId: number
  incognito: boolean
  maximized: boolean
  fullscreen: boolean
  workspaces: WorkspaceState[]
  activeWorkspaceId: string
  tabs: Record<number, TabState>
  permission: PermissionPrompt | null
  /** The active tab's site has the shield switched off. */
  shieldPaused: boolean
  extensionsPartition: string | null
}

export interface GlobalState {
  focus: FocusState
  update: UpdateState
  downloads: DownloadState[]
  version: string
}

export interface PaneRect {
  tabId: number
  x: number
  y: number
  width: number
  height: number
}

export type SuggestionKind = 'url' | 'search' | 'history' | 'tab' | 'command' | 'bookmark'

export interface Suggestion {
  kind: SuggestionKind
  title: string
  /** URL to open, or the search text for 'search'. */
  value: string
  favicon?: string | null
  tabId?: number
  command?: CommandId
}

export type CommandId =
  | 'new-tab'
  | 'new-window'
  | 'new-incognito'
  | 'focus-start'
  | 'focus-stop'
  | 'reader'
  | 'settings'
  | 'history'
  | 'extensions'
  | 'downloads'
  | 'toggle-sidebar'
  | 'split'
  | 'unsplit'
  | 'pin'
  | 'clear-data'
  | 'check-updates'
  | 'bookmark-page'
  | 'bookmark-tabs'
  | 'bookmarks'
  | 'toggle-bookmarks-bar'
  | 'import-bookmarks'

export interface HistoryEntry {
  id: number
  url: string
  title: string
  visitedAt: number
  favicon: string | null
}

export interface TopSite {
  url: string
  title: string
  favicon: string | null
}

export interface DayCount {
  day: string
  count: number
}

export interface Stats {
  blockedToday: number
  blockedWeek: number
  blockedTotal: number
  focusMinutesToday: number
  focusMinutesWeek: number
}

export interface ExtensionInfo {
  id: string
  name: string
  version: string
  description: string
  icon: string | null
  optionsUrl: string | null
}

export interface SitePermission {
  origin: string
  kind: PermissionKind
  allow: boolean
}

export interface AppInfo {
  /** The version people see, with the stage suffix (0.1.3a). */
  version: string
  stage: 'alpha' | 'beta' | 'stable'
  electron: string
  chrome: string
  node: string
  platform: string
  packaged: boolean
  isDefaultBrowser: boolean
}

/** Commands the main process asks the UI to carry out. */
export type UICommand =
  | { type: 'focus-omnibox'; text?: string }
  | { type: 'palette' }
  | { type: 'find' }
  | { type: 'downloads' }
  | { type: 'shield' }
  | { type: 'focus-panel' }
  | { type: 'escape' }
  | { type: 'focus-page-search' }
  | { type: 'bookmark-star'; created?: boolean }
  | { type: 'bookmark-edit'; id?: string; parentId?: string; folder?: boolean }
  | { type: 'bookmarks-removed'; count: number }
  | { type: 'import-bookmarks' }

export interface ImportSource {
  id: string
  /** Browser name as people know it: Google Chrome, Firefox… */
  browser: string
  profile: string
  kind: 'chromium' | 'firefox'
  links: number
}

export interface ImportResult {
  links: number
  folders: number
  duplicates: number
  skipped: number
  favicons: number
}
