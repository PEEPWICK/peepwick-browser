// State and message shapes shared by the main process, the preload and the UI.

export type Lang = 'tr' | 'en'
export type ThemePref = 'system' | 'dark' | 'light'
export type SearchEngineId = 'duckduckgo' | 'brave' | 'startpage' | 'google' | 'ecosia' | 'bing'
/** standard: ads and trackers · strict: also cookie notices and other annoyances */
export type ShieldLevel = 'off' | 'standard' | 'strict'
export type BookmarksBarMode = 'always' | 'newtab' | 'never'
/** DNS over HTTPS: 'automatic' upgrades when the system's resolver supports it. */
export type SecureDns = 'off' | 'automatic' | 'cloudflare' | 'quad9' | 'mullvad' | 'google'
/** What can be cleared, now or when the browser closes. */
export type ClearKind = 'history' | 'downloads' | 'cookies' | 'cache' | 'permissions'
export const CLEAR_KINDS: ClearKind[] = ['history', 'downloads', 'cookies', 'cache', 'permissions']
/** How far back clearing goes, in hours; 0 is all time. */
export type ClearRange = 1 | 24 | 168 | 672 | 0

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
  /** Keep the error log in a file on this computer between sessions. Nothing is sent. */
  errorLog: boolean
  /** Cleared every time the browser closes. */
  clearOnExit: ClearKind[]
  secureDns: SecureDns
  /** Where downloads go; empty for the system's Downloads folder. */
  downloadDir: string
  /** Takes effect after a restart. */
  hardwareAcceleration: boolean
}

/**
 * secure: HTTPS and the certificate checked out · unverified: HTTPS, but no check
 * was seen this session (a page from the cache) · insecure: plain HTTP ·
 * dangerous: HTTPS whose certificate failed and the warning was skipped.
 */
export type TabSecurity = 'secure' | 'unverified' | 'insecure' | 'dangerous' | 'internal' | 'local'

/** Something the UI draws in place of the page: an error, a crash or the focus gate. */
export type Interstitial =
  | { kind: 'error'; code: number; description: string; url: string; certificate: boolean }
  | { kind: 'crashed' }
  | { kind: 'focus'; url: string; host: string }

export type InternalPage = 'newtab' | 'settings' | 'history' | 'extensions' | 'bookmarks' | 'downloads'

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
  'backup',
  'report',
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
  endedAt?: number
  /** Bytes per second, smoothed; 0 when not moving. */
  speed?: number
  /** The finished file is no longer where it was saved. */
  missing?: boolean
  /** From a private window: listed while the browser runs, never saved. */
  incognito?: boolean
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
  | 'report'
  | 'backup'

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
  /** Whether this run uses the graphics card (the setting applies after a restart). */
  hardwareAcceleration: boolean
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
  | { type: 'menu'; menu: MenuModel }
  | { type: 'confirm'; dialog: ConfirmModel }
  | { type: 'clear-data' }
  | { type: 'history-search'; text: string }
  | { type: 'history-changed' }
  | { type: 'certificate'; host: string }
  | { type: 'bookmark-rename'; id: string }
  | { type: 'backup-restore'; token: string; name: string }

/**
 * A menu the UI draws. Native menus and message boxes take their colours from the
 * system (GTK) theme, so they stayed light in a dark browser.
 */
export interface MenuModel {
  id: number
  items: MenuNode[]
  /** Window coordinates of the menu's top left corner (top right with align 'end'). */
  x: number
  y: number
  align?: 'end'
  /** Opened from an item of this menu (right-click in a bookmark folder): it stays open below. */
  over?: number
}

export type MenuNode =
  | { kind: 'separator' }
  | {
      kind: 'item' | 'check' | 'submenu'
      id: number
      label: string
      accelerator?: string
      enabled: boolean
      checked?: boolean
      /** Page address whose site icon is shown. */
      favicon?: string
      /** Image data URL (extension icons). */
      icon?: string
      folder?: boolean
      submenu?: MenuNode[]
      /** The item has its own right-click menu. */
      context?: boolean
    }

export interface ConfirmModel {
  id: number
  title: string
  body?: string
  /** Shown as a list under the body (extension permissions). */
  lines?: string[]
  /** Image data URL. */
  icon?: string
  confirm: string
  cancel: string
  /** The confirming action loses something (removing, leaving). */
  danger?: boolean
}

export interface MenuClickEvent {
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
}

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

// ── Site security ────────────────────────────────────────────────────────────

export interface CertificateInfo {
  subject: string
  subjectOrganization: string
  issuer: string
  issuerOrganization: string
  validFrom: number
  validTo: number
  /** Names the certificate covers (subjectAltName). */
  names: string[]
  serialNumber: string
  sha256: string
  sha1: string
  keyType: string
  signatureAlgorithm: string
  /** The issuers above it, nearest first. */
  chain: { subject: string; issuer: string; validTo: number }[]
}

export interface SiteInfo {
  url: string
  host: string
  origin: string
  security: TabSecurity
  /** Chromium's verification result for the host, as a net error name (net::OK when fine). */
  verification: string | null
  errorCode: number
  /** Chains to a root the system trusts out of the box, rather than one added locally. */
  knownRoot: boolean
  /** The certificate warning was skipped for this host this session. */
  exception: boolean
  certificate: CertificateInfo | null
  permissions: SitePermission[]
  cookies: number
}

// ── Clearing browsing data ───────────────────────────────────────────────────

export interface ClearRequest {
  range: ClearRange
  kinds: ClearKind[]
}

export interface ClearCounts {
  history: number
  downloads: number
  cookies: number
  permissions: number
}

// ── Error report ─────────────────────────────────────────────────────────────

export interface ErrorRecord {
  time: number
  /** main, interface, page, gpu, network, update… */
  source: string
  message: string
  count: number
}

export interface Report {
  /** Plain text to paste into an issue. */
  text: string
  version: string
  system: string
  errors: ErrorRecord[]
  /** Page load failures by Chromium error name, no addresses. */
  loadErrors: { name: string; count: number }[]
  logFile: boolean
}

// ── Backup ───────────────────────────────────────────────────────────────────

export type BackupCategory = 'settings' | 'bookmarks' | 'workspaces' | 'tabs' | 'focus' | 'permissions' | 'extensions' | 'history'
export const BACKUP_CATEGORIES: BackupCategory[] = ['settings', 'bookmarks', 'workspaces', 'tabs', 'focus', 'permissions', 'extensions', 'history']
/** Ticked when exporting, unless the person changes it. */
export const BACKUP_DEFAULTS: BackupCategory[] = ['settings', 'bookmarks', 'workspaces', 'focus', 'permissions', 'extensions']

/** How much of each category there is (on this computer, or in a backup file). */
export type BackupCounts = Partial<Record<BackupCategory, number>>

export interface BackupSummary {
  token: string
  createdAt: number
  version: string
  counts: BackupCounts
  extensions: { id: string; name: string }[]
}

export interface RestoreResult {
  settings: boolean
  bookmarks: number
  workspaces: number
  tabs: number
  focusDays: number
  permissions: number
  extensions: number
  history: number
}
