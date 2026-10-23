import { EventEmitter } from 'node:events'
import { describe, expect, it, vi } from 'vitest'

// popup.ts only needs Electron for the system fallback, which these tests avoid.
vi.mock('electron', () => ({ dialog: {} }))

const { confirm, confirmReply, menuClosed, menuSelect, popupMenu, separator } = await import('../src/main/popup')
import type { MenuModel, MenuNode, UICommand } from '@shared/types'
import type { BrowserWin } from '../src/main/window'

function fakeWindow() {
  const base = Object.assign(new EventEmitter(), { isDestroyed: () => false })
  const commands: UICommand[] = []
  const calls: string[] = []
  const win = {
    base,
    isUIReady: true,
    command: (command: UICommand) => commands.push(command),
    setOverlay: (open: boolean) => calls.push(`overlay:${open}`),
  }
  return { win: win as unknown as BrowserWin, base, commands, calls }
}

const lastMenu = (commands: UICommand[]): MenuModel => {
  const command = commands.at(-1)
  if (command?.type !== 'menu') throw new Error('no menu sent')
  return command.menu
}

const entry = (node: MenuNode) => {
  if (node.kind === 'separator') throw new Error('separator')
  return node
}

describe('menus drawn by the UI', () => {
  it('drops leading, trailing and doubled separators', () => {
    const { win, commands } = fakeWindow()
    popupMenu(win, [separator, { label: 'A' }, separator, separator, { label: 'B' }, separator], 10, 20)
    const menu = lastMenu(commands)
    expect(menu.items.map((item) => item.kind)).toEqual(['item', 'separator', 'item'])
    expect([menu.x, menu.y]).toEqual([10, 20])
  })

  it('sends submenus and checkboxes, and disables empty submenus', () => {
    const { win, commands } = fakeWindow()
    popupMenu(win, [{ label: 'More', submenu: [{ label: 'One', type: 'checkbox', checked: true }] }, { label: 'Empty', submenu: [] }], 0, 0)
    const [more, empty] = lastMenu(commands).items.map(entry)
    expect(more.kind).toBe('submenu')
    expect(more.submenu?.map(entry)[0]).toMatchObject({ kind: 'check', checked: true })
    expect(empty.enabled).toBe(false)
  })

  it('runs the chosen item once, after lowering the UI', () => {
    const { win, commands, calls } = fakeWindow()
    const click = vi.fn(() => calls.push('click'))
    popupMenu(win, [{ label: 'Go', click }], 0, 0)
    const menu = lastMenu(commands)
    const event = { ctrlKey: true, shiftKey: false, altKey: false, metaKey: false }
    menuSelect(win, menu.id, entry(menu.items[0]).id, event)
    menuSelect(win, menu.id, entry(menu.items[0]).id, event)
    expect(click).toHaveBeenCalledTimes(1)
    expect(click).toHaveBeenCalledWith(event)
    expect(calls).toEqual(['overlay:false', 'click'])
  })

  it('ignores answers for a menu that was replaced or closed', () => {
    const { win, commands } = fakeWindow()
    const first = vi.fn()
    popupMenu(win, [{ label: 'First', click: first }], 0, 0)
    const old = lastMenu(commands)
    popupMenu(win, [{ label: 'Second' }], 0, 0)
    const event = { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false }
    menuSelect(win, old.id, entry(old.items[0]).id, event)
    const current = lastMenu(commands)
    menuClosed(win, current.id)
    menuSelect(win, current.id, entry(current.items[0]).id, event)
    expect(first).not.toHaveBeenCalled()
  })
})

describe('confirmations drawn by the UI', () => {
  it('resolves with the answer from the UI', async () => {
    const { win, commands } = fakeWindow()
    const answer = confirm(win, { title: 'Add?', confirm: 'Add', cancel: 'Cancel' })
    const command = commands.at(-1)
    if (command?.type !== 'confirm') throw new Error('no dialog sent')
    confirmReply(command.dialog.id, true)
    await expect(answer).resolves.toBe(true)
  })

  it('is a no when the window closes first', async () => {
    const { win, base } = fakeWindow()
    const answer = confirm(win, { title: 'Quit?', confirm: 'Quit', cancel: 'Cancel' })
    base.emit('closed')
    await expect(answer).resolves.toBe(false)
  })
})
