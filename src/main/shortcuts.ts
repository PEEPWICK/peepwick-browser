// Keyboard shortcuts. Every web view and the UI forward key presses here through
// before-input-event, so shortcuts work wherever keyboard focus is.
import { app, type Input } from 'electron'
import { createWindow } from './windows'
import { newTab, openDevTools, openPage, runCommand, savePage, toggleFullscreen, toggleSplit } from './actions'
import type { BrowserWin } from './window'

function letter(input: Input): string {
  const key = input.key.length === 1 ? input.key.toLocaleLowerCase('en') : ''
  if (/^[a-z]$/.test(key)) return key
  const code = /^Key([A-Z])$/.exec(input.code)
  return code ? code[1].toLowerCase() : key
}

function digit(input: Input): number | null {
  const match = /^(?:Digit|Numpad)([0-9])$/.exec(input.code)
  return match ? Number(match[1]) : null
}

export function handleShortcut(win: BrowserWin, input: Input): boolean {
  if (input.type !== 'keyDown') return false
  const ctrl = input.control || input.meta
  const { shift, alt } = input
  const key = input.key
  const tab = win.activeTab()
  const wc = tab?.webContents
  const l = letter(input)
  const n = digit(input)

  // ── Without Ctrl ──
  if (!ctrl) {
    if (alt && !shift && key === 'ArrowLeft') return tab?.back(), true
    if (alt && !shift && key === 'ArrowRight') return tab?.forward(), true
    if (key === 'BrowserBack') return tab?.back(), true
    if (key === 'BrowserForward') return tab?.forward(), true
    if (alt && !shift && n !== null && n > 0) {
      const workspace = win.workspaces[n - 1]
      if (workspace) win.activateWorkspace(workspace.id)
      return true
    }
    if (alt && !shift && l === 'd') return win.command({ type: 'focus-omnibox' }), true
    if (alt) return false
    switch (key) {
      case 'F5':
        tab?.reload(shift)
        return true
      case 'F6':
        win.command({ type: 'focus-omnibox' })
        return true
      case 'F3':
        win.command({ type: 'find' })
        if (tab?.find?.text) tab.findInPage(tab.find.text, !shift, true)
        return true
      case 'F11':
        toggleFullscreen(win)
        return true
      case 'F12':
        openDevTools(win)
        return true
      case 'Escape':
        if (tab?.find) {
          tab.stopFind()
          win.send('command', { type: 'escape' })
          return true
        }
        if (tab?.loading && wc) {
          wc.stop()
          return false
        }
        return false
    }
    return false
  }

  // ── With Ctrl ──
  if (alt) {
    if (l === 'r') return void tab?.toggleReader(), true
    return false
  }

  if (n !== null && !shift) {
    if (n === 0) return tab?.zoom('reset'), true
    const tabs = win.tabsInOrder()
    const target = n === 9 ? tabs.at(-1) : tabs[n - 1]
    if (target) win.activateTab(target)
    return true
  }

  if (key === '=' || key === '+' || input.code === 'NumpadAdd') return tab?.zoom('in'), true
  if (key === '-' || key === '_' || input.code === 'NumpadSubtract') return tab?.zoom('out'), true

  if (key === 'Tab' || key === 'PageDown' || key === 'PageUp') {
    const tabs = win.tabsInOrder()
    if (tabs.length < 2 || !tab) return true
    const back = key === 'PageUp' || (key === 'Tab' && shift)
    const index = tabs.indexOf(tab)
    const next = tabs[(index + (back ? -1 : 1) + tabs.length) % tabs.length]
    win.activateTab(next)
    return true
  }

  if (key === 'F4') {
    if (tab) win.closeTab(tab)
    return true
  }
  if (key === 'Delete' && shift) return runCommand(win, 'clear-data'), true
  if (key === ',') return openPage(win, 'settings'), true

  if (shift) {
    switch (l) {
      case 't':
        win.reopenClosedTab()
        return true
      case 'n':
        createWindow({ incognito: true })
        return true
      case 'w':
        win.base.close()
        return true
      case 'r':
        tab?.reload(true)
        return true
      case 'g':
        if (tab?.find?.text) tab.findInPage(tab.find.text, false, true)
        return true
      case 'i':
      case 'j':
        openDevTools(win)
        return true
      case 's':
        toggleSplit(win)
        return true
      case 'b':
        runCommand(win, 'toggle-bookmarks-bar')
        return true
      case 'd':
        runCommand(win, 'bookmark-tabs')
        return true
      case 'o':
        runCommand(win, 'bookmarks')
        return true
    }
    return false
  }

  switch (l) {
    case 't':
      newTab(win)
      return true
    case 'n':
      createWindow({ incognito: false })
      return true
    case 'w':
      if (tab) win.closeTab(tab)
      return true
    case 'l':
      win.command({ type: 'focus-omnibox' })
      return true
    case 'k':
      win.command({ type: 'palette' })
      return true
    case 'r':
      tab?.reload()
      return true
    case 'f':
      win.command({ type: 'find' })
      return true
    case 'g':
      if (tab?.find?.text) tab.findInPage(tab.find.text, true, true)
      return true
    case 'd':
      runCommand(win, 'bookmark-page')
      return true
    case 'b':
      runCommand(win, 'toggle-sidebar')
      return true
    case 'h':
      openPage(win, 'history')
      return true
    case 'j':
      openPage(win, 'downloads')
      return true
    case 'p':
      if (wc && !tab?.page) wc.print()
      return true
    case 's':
      void savePage(win)
      return true
    case 'q':
      app.quit()
      return true
  }
  return false
}
