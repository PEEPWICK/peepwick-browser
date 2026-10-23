// Menus and confirmations drawn by the browser UI. Native menus and message boxes
// take their colours from the system (GTK) theme, so in a dark browser on a light
// desktop they stayed white. The main process keeps the actions; the UI only gets
// labels and ids back.
import { dialog, type MenuItem, type NativeImage, type WebContents } from 'electron'
import type { ConfirmModel, MenuClickEvent, MenuNode } from '@shared/types'
import type { BrowserWin } from './window'

export interface MenuSpec {
  label?: string
  type?: 'normal' | 'separator' | 'checkbox'
  accelerator?: string
  enabled?: boolean
  checked?: boolean
  /** Page address whose site icon is shown. */
  favicon?: string
  /** Image data URL. */
  icon?: string
  folder?: boolean
  submenu?: MenuSpec[]
  click?: (event: MenuClickEvent) => void
}

export const separator: MenuSpec = { type: 'separator' }

interface OpenMenu {
  id: number
  actions: Map<number, (event: MenuClickEvent) => void>
}

const openMenus = new WeakMap<BrowserWin, OpenMenu>()
let nextId = 1

function build(specs: MenuSpec[], actions: OpenMenu['actions']): MenuNode[] {
  const out: MenuNode[] = []
  for (const spec of specs) {
    if (spec.type === 'separator') {
      if (out.length > 0 && out.at(-1)?.kind !== 'separator') out.push({ kind: 'separator' })
      continue
    }
    const id = nextId++
    if (spec.click) actions.set(id, spec.click)
    const submenu = spec.submenu ? build(spec.submenu, actions) : undefined
    out.push({
      kind: submenu ? 'submenu' : spec.type === 'checkbox' ? 'check' : 'item',
      id,
      label: (spec.label ?? '').slice(0, 200),
      accelerator: spec.accelerator,
      enabled: spec.enabled !== false && (!submenu || submenu.length > 0),
      checked: spec.checked,
      favicon: spec.favicon,
      icon: spec.icon,
      folder: spec.folder,
      submenu,
    })
  }
  while (out.at(-1)?.kind === 'separator') out.pop()
  return out
}

/** Opens a menu at window coordinates. A new menu replaces an open one. */
export function popupMenu(win: BrowserWin, specs: MenuSpec[], x: number, y: number, align?: 'end'): void {
  if (win.base.isDestroyed()) return
  const actions: OpenMenu['actions'] = new Map()
  const items = build(specs, actions)
  if (items.length === 0) return
  const id = nextId++
  openMenus.set(win, { id, actions })
  win.command({ type: 'menu', menu: { id, items, x: Math.round(x), y: Math.round(y), align } })
}

export function menuSelect(win: BrowserWin, menuId: number, itemId: number, event: MenuClickEvent): void {
  const menu = openMenus.get(win)
  if (!menu || menu.id !== menuId) return
  openMenus.delete(win)
  const action = menu.actions.get(itemId)
  if (!action) return
  // Lower the UI first, so focus that the action moves (a new window, the page)
  // stays where the action put it.
  win.setOverlay(false)
  action(event)
}

export function menuClosed(win: BrowserWin, menuId: number): void {
  if (openMenus.get(win)?.id === menuId) openMenus.delete(win)
}

export function imageURL(image: NativeImage | string | undefined | null): string | undefined {
  if (!image || typeof image === 'string' || image.isEmpty()) return undefined
  return image.toDataURL()
}

/** Electron menu items (extensions build these) as specs. */
export function fromElectron(win: BrowserWin, items: MenuItem[], target?: WebContents): MenuSpec[] {
  return items
    .filter((item) => item.visible)
    .map((item): MenuSpec => {
      if (item.type === 'separator') return separator
      const submenu = item.submenu ? fromElectron(win, item.submenu.items, target) : undefined
      return {
        label: item.label,
        type: item.type === 'checkbox' || item.type === 'radio' ? 'checkbox' : 'normal',
        enabled: item.enabled,
        checked: item.checked,
        icon: imageURL(item.icon),
        submenu,
        click: submenu ? undefined : () => item.click(undefined, win.base, target),
      }
    })
}

// ── Confirmations ─────────────────────────────────────────────────────────────

export type ConfirmOptions = Omit<ConfirmModel, 'id'>

const pending = new Map<number, (ok: boolean) => void>()

/**
 * Asks in the window's own UI. Without a window that can show it (none open, or
 * the UI still loading) it falls back to a system message box.
 */
export function confirm(win: BrowserWin | null | undefined, options: ConfirmOptions): Promise<boolean> {
  if (!win || win.base.isDestroyed() || !win.isUIReady) return systemConfirm(win, options)
  const id = nextId++
  return new Promise((resolve) => {
    const closed = () => settle(false)
    const settle = (ok: boolean) => {
      pending.delete(id)
      if (!win.base.isDestroyed()) win.base.off('closed', closed)
      resolve(ok)
    }
    pending.set(id, settle)
    win.base.once('closed', closed)
    win.command({ type: 'confirm', dialog: { id, ...options } })
  })
}

export function confirmReply(id: number, ok: boolean): void {
  pending.get(id)?.(ok)
}

async function systemConfirm(win: BrowserWin | null | undefined, options: ConfirmOptions): Promise<boolean> {
  const detail = [options.body, ...(options.lines ?? []).map((line) => `• ${line}`)].filter(Boolean).join('\n')
  const box = {
    type: 'question' as const,
    buttons: [options.confirm, options.cancel],
    defaultId: options.danger ? 1 : 0,
    cancelId: 1,
    message: options.title,
    detail,
  }
  const parent = win && !win.base.isDestroyed() ? win.base : null
  const { response } = parent ? await dialog.showMessageBox(parent, box) : await dialog.showMessageBox(box)
  return response === 0
}
