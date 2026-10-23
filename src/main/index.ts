import { gtkThemeSet } from './early'
import { app, Menu, nativeTheme } from 'electron'
import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { hostOf } from '@shared/url'
import { flushBookmarks, loadBookmarks } from './bookmarks'
import { activeDownloadCount, cancelAllDownloads, flushDownloads, loadDownloads } from './downloads'
import { bus } from './events'
import { initExtensions } from './extensions'
import { loadFocus } from './focus'
import { startHarness } from './harness'
import { closeHistory, openHistory } from './history'
import { installIPC } from './ipc'
import { lifecycle } from './lifecycle'
import { flushPermissions, loadPermissions } from './permissions'
import { confirm } from './popup'
import { tabForContents } from './registry'
import { browsingSession, configureUISession, registerSchemes, setUserAgent, uiSession } from './sessions'
import { flushSettings, loadSettings, settings, t } from './settings'
import { ensureShield, installCosmeticHandlers } from './shield'
import { flushStats, loadStats } from './stats'
import { certificateAllowed } from './tab'
import { BrowserWin } from './window'
import { startUpdater } from './updater'
import { anyWindow, createWindow, markQuitting, restoreSession, wireBroadcasts } from './windows'

process.env['CHROME_DESKTOP'] = 'peepwick-browser.desktop'

// Electron reports every failed frame load as a process warning. Requests the shield
// or the focus gate cancel on purpose would flood stderr with tracker URLs, so only
// those are dropped; every other warning is still printed.
process.removeAllListeners('warning')
process.on('warning', (warning) => {
  if (warning.name === 'electron' && warning.message.includes('ERR_BLOCKED_BY_CLIENT')) return
  console.warn(`${warning.name}: ${warning.message}`)
})
registerSchemes()
setUserAgent()

function urlsFrom(argv: string[]): string[] {
  const urls: string[] = []
  for (const arg of argv.slice(1)) {
    if (arg.startsWith('-')) continue
    if (/^(https?|file):\/\//i.test(arg)) urls.push(arg)
    else if (/\.(html?|xhtml|svg|pdf)$/i.test(arg) && existsSync(arg)) urls.push(pathToFileURL(resolve(arg)).href)
  }
  return urls
}

async function boot(): Promise<void> {
  Menu.setApplicationMenu(null)
  if (gtkThemeSet) delete process.env['GTK_THEME']
  loadSettings()
  // Pages (prefers-color-scheme), scrollbars and form controls follow the browser's
  // theme, as in Chrome, not only the desktop's.
  const applyTheme = () => {
    if (nativeTheme.themeSource !== settings().theme) nativeTheme.themeSource = settings().theme
  }
  applyTheme()
  bus.on('settings', applyTheme)
  loadStats()
  loadFocus()
  loadPermissions()
  loadDownloads()
  loadBookmarks()
  openHistory()
  configureUISession()
  installIPC()
  wireBroadcasts()
  installCosmeticHandlers((event) => tabForContents(event.sender.id)?.pageURL() ?? null)

  const browsing = browsingSession()
  // Start blocking before the first page loads, but never hold the window back for long.
  await Promise.race([ensureShield(), new Promise((r) => setTimeout(r, 2500))])
  let level = settings().shieldLevel
  bus.on('settings', () => {
    if (settings().shieldLevel !== level) {
      level = settings().shieldLevel
      void ensureShield()
    }
  })

  try {
    await initExtensions(browsing, uiSession())
  } catch (error) {
    console.error('[extensions] could not start', error)
  }

  restoreSession(urlsFrom(process.argv))
  startUpdater()
  startHarness()
}

if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app.on('second-instance', (_event, argv) => {
    const urls = urlsFrom(argv)
    const win = anyWindow()
    if (!win || win.base.isDestroyed() || win.incognito) {
      createWindow({ incognito: false, urls })
      return
    }
    for (const url of urls) win.openTab({ url })
    if (win.base.isMinimized()) win.base.restore()
    win.base.show()
    win.base.focus()
  })

  app.on('certificate-error', (event, _wc, url, _error, _certificate, callback) => {
    if (certificateAllowed(hostOf(url))) {
      event.preventDefault()
      callback(true)
    } else {
      callback(false)
    }
  })

  app.on('web-contents-created', (_event, contents) => {
    contents.on('will-attach-webview', (event) => event.preventDefault())
  })

  app.on('window-all-closed', () => app.quit())

  // Logout and shutdown send SIGTERM: quit cleanly without blocking on a dialog.
  for (const signal of ['SIGTERM', 'SIGINT', 'SIGHUP'] as const) {
    process.on(signal, () => {
      lifecycle.forceQuit = true
      app.quit()
    })
  }

  let quitConfirmed = false
  let askingToQuit = false
  app.on('before-quit', (event) => {
    const active = activeDownloadCount()
    if (active > 0 && !lifecycle.forceQuit && !quitConfirmed) {
      event.preventDefault()
      if (askingToQuit) return
      askingToQuit = true
      void confirm(BrowserWin.lastFocused, {
        title: t('dialog.quitDownloads', { n: active }),
        confirm: t('dialog.quit'),
        cancel: t('common.cancel'),
        danger: true,
      }).then((ok) => {
        askingToQuit = false
        if (!ok) return
        quitConfirmed = true
        app.quit()
      })
      return
    }
    if (active > 0) cancelAllDownloads()
    markQuitting()
  })

  app.on('will-quit', () => {
    flushSettings()
    flushStats()
    flushPermissions()
    flushDownloads()
    flushBookmarks()
    closeHistory()
  })

  void app.whenReady().then(boot)
}
