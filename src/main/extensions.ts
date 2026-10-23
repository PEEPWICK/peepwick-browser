// Chrome extension support: electron-chrome-extensions provides the chrome.* APIs
// (tabs, windows, action popups…) and electron-chrome-web-store installs and updates
// extensions from the Chrome Web Store. Both are wired to our tabs and windows.
import type { BaseWindow, MenuItem, Session, WebContents } from 'electron'
import { ElectronChromeExtensions } from 'electron-chrome-extensions'
import { installChromeWebStore, uninstallExtension } from 'electron-chrome-web-store'
import type { ExtensionInfo } from '@shared/types'
import { openPage } from './actions'
import { confirm, fromElectron, imageURL, popupMenu, separator } from './popup'
import { tabForContents } from './registry'
import { BROWSING_PARTITION } from './sessions'
import { settings, t } from './settings'
import { BrowserWin } from './window'
import { createWindow } from './windows'

let extensions: ElectronChromeExtensions | null = null
let browsing: Session | null = null

export function getExtensions(): ElectronChromeExtensions | null {
  return extensions
}

function windowFor(base: BaseWindow | undefined): BrowserWin | undefined {
  return base ? BrowserWin.all.get(base.id) : undefined
}

function normalWindow(): BrowserWin {
  const focused = BrowserWin.lastFocused
  if (focused && !focused.incognito) return focused
  return [...BrowserWin.all.values()].find((w) => !w.incognito) ?? createWindow({ incognito: false })
}

function describePermissions(manifest: chrome.runtime.Manifest): string[] {
  const permissions = [...((manifest.permissions as string[] | undefined) ?? [])]
  const hosts = [
    ...(((manifest as { host_permissions?: string[] }).host_permissions as string[] | undefined) ?? []),
    ...(manifest.content_scripts?.flatMap((script) => script.matches ?? []) ?? []),
  ]
  const lines: string[] = []
  const tr = settings().language === 'tr'
  if (hosts.some((h) => h === '<all_urls>' || h.startsWith('*://*/') || h.startsWith('http://*/') || h.startsWith('https://*/'))) {
    lines.push(tr ? 'Bütün sitelerdeki verilerinizi okuyup değiştirebilir' : 'Read and change your data on all websites')
  } else if (hosts.length > 0) {
    const shown = [...new Set(hosts)].slice(0, 4).join(', ')
    lines.push(tr ? `Şu sitelerdeki verilerinizi okuyup değiştirebilir: ${shown}` : `Read and change your data on: ${shown}`)
  }
  const named: Record<string, [string, string]> = {
    tabs: ['Sekmelerinizi ve gezinme etkinliğinizi görebilir', 'See your tabs and browsing activity'],
    history: ['Geçmişinizi okuyup değiştirebilir', 'Read and change your browsing history'],
    bookmarks: ['Yer imlerinizi okuyup değiştirebilir', 'Read and change your bookmarks'],
    cookies: ['Çerezlerinizi okuyup değiştirebilir', 'Read and change your cookies'],
    downloads: ['İndirmelerinizi yönetebilir', 'Manage your downloads'],
    clipboardRead: ['Panonuzu okuyabilir', 'Read your clipboard'],
    clipboardWrite: ['Panonuza yazabilir', 'Write to your clipboard'],
    geolocation: ['Konumunuzu öğrenebilir', 'Know your location'],
    notifications: ['Bildirim gösterebilir', 'Show notifications'],
    webRequest: ['Ağ isteklerini görebilir', 'See network requests'],
    management: ['Diğer eklentileri yönetebilir', 'Manage your other extensions'],
    nativeMessaging: ['Bilgisayarınızdaki uygulamalarla konuşabilir', 'Talk to apps on your computer'],
  }
  for (const permission of permissions) {
    const text = named[permission]
    if (text) lines.push(tr ? text[0] : text[1])
  }
  return lines
}

export async function initExtensions(session: Session, uiSession: Session): Promise<void> {
  browsing = session
  extensions = new ElectronChromeExtensions({
    license: 'GPL-3.0',
    session,
    async createTab(details) {
      const win = (details.windowId !== undefined ? BrowserWin.all.get(details.windowId) : undefined) ?? normalWindow()
      const tab = win.openTab({ url: details.url, background: details.active === false })
      const view = tab.ensureView()
      return [view.webContents, win.base]
    },
    selectTab(wc: WebContents) {
      const tab = tabForContents(wc.id)
      tab?.win.activateTab(tab)
    },
    removeTab(wc: WebContents) {
      const tab = tabForContents(wc.id)
      tab?.win.closeTab(tab)
    },
    async createWindow(details) {
      const urls = typeof details.url === 'string' ? [details.url] : (details.url ?? [])
      return createWindow({ incognito: false, urls }).base
    },
    removeWindow(base) {
      windowFor(base)?.base.close()
    },
  })
  ElectronChromeExtensions.handleCRXProtocol(uiSession)
  // The extension button's right-click menu: ours instead of the library's native one.
  const internals = extensions as unknown as {
    api: { browserAction: { activateContextMenu: (details: ActionMenuDetails) => void } }
    ctx: { store: { buildMenuItems: (extensionId: string, menuType: string) => MenuItem[] } }
  }
  internals.api.browserAction.activateContextMenu = (details) =>
    showActionMenu(details, internals.ctx.store.buildMenuItems(details.extensionId, 'browser_action'))

  await installChromeWebStore({
    session,
    autoUpdate: true,
    async beforeInstall(details) {
      const lines = describePermissions(details.manifest)
      const ok = await confirm(BrowserWin.lastFocused, {
        title: t('ext.install.title', { name: details.localizedName }),
        body: lines.length > 0 ? t('ext.install.body') : t('ext.install.none'),
        lines,
        icon: imageURL(details.icon),
        confirm: t('ext.install.add'),
        cancel: t('ext.install.cancel'),
      })
      return { action: ok ? 'allow' : 'deny' }
    },
  })
}

interface ActionMenuDetails {
  extensionId: string
  tabId: number
  anchorRect: { x: number; y: number; width: number; height: number }
}

function showActionMenu(details: ActionMenuDetails, extensionItems: MenuItem[]): void {
  const extension = browsing?.extensions.getExtension(details.extensionId)
  const win = tabForContents(details.tabId)?.win ?? BrowserWin.lastFocused
  if (!extension || !win) return
  const manifest = extension.manifest as chrome.runtime.Manifest & { options_ui?: { page?: string }; options_page?: string }
  const options = manifest.options_ui?.page ?? manifest.options_page
  const home = manifest.homepage_url || `https://chromewebstore.google.com/detail/${extension.id}`
  const { x, y, height } = details.anchorRect
  popupMenu(
    win,
    [
      { label: extension.name, click: () => win.openTab({ url: home }) },
      separator,
      ...fromElectron(win, extensionItems),
      separator,
      {
        label: t('ext.options'),
        enabled: !!options,
        click: () => win.openTab({ url: `chrome-extension://${extension.id}/${options?.replace(/^\//, '')}` }),
      },
      { label: t('ext.remove'), click: () => void removeExtension(extension.id) },
      separator,
      { label: t('menu.extensions'), click: () => openPage(win, 'extensions') },
    ],
    x,
    y + height,
  )
}

function iconURL(extension: Electron.Extension): string | null {
  const icons = (extension.manifest as { icons?: Record<string, string> }).icons
  if (!icons) return null
  return `crx://extension-icon/${extension.id}/48/2?partition=${encodeURIComponent(BROWSING_PARTITION)}`
}

export function listExtensions(): ExtensionInfo[] {
  if (!browsing) return []
  return browsing.extensions.getAllExtensions().map((extension) => {
    const manifest = extension.manifest as chrome.runtime.Manifest & { options_ui?: { page?: string }; options_page?: string }
    const options = manifest.options_ui?.page ?? manifest.options_page ?? null
    return {
      id: extension.id,
      name: extension.name,
      version: extension.version,
      description: (manifest.description as string | undefined) ?? '',
      icon: iconURL(extension),
      optionsUrl: options ? `chrome-extension://${extension.id}/${options.replace(/^\//, '')}` : null,
    }
  })
}

export async function removeExtension(id: string): Promise<void> {
  if (!browsing) return
  const extension = browsing.extensions.getExtension(id)
  if (!extension) return
  const ok = await confirm(BrowserWin.lastFocused, {
    title: t('ext.removeConfirm', { name: extension.name }),
    confirm: t('ext.remove'),
    cancel: t('common.cancel'),
    danger: true,
  })
  if (!ok) return
  await uninstallExtension(id, { session: browsing })
}
