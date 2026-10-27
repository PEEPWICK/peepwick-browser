// Menus the main process asks for: the page, tab, app, bookmark and extension menus.
// They are drawn here so they follow the browser's theme; the main process keeps
// what each item does and gets back only the item's id.
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { Check, ChevronRight, Folder } from 'lucide-react'
import type { MenuModel, MenuNode } from '@shared/types'
import { api } from '../api'
import { useUI } from '../context'
import { Favicon, Icon, faviconFor, useOverlay } from '../ui'

type Entry = Extract<MenuNode, { id: number }>

const MARGIN = 8
const SUBMENU_DELAY = 180

const isEntry = (node: MenuNode | undefined): node is Entry => !!node && node.kind !== 'separator'
const usable = (node: MenuNode | undefined): node is Entry => isEntry(node) && node.enabled

function keyLabel(accelerator: string): string {
  return accelerator
    .replace(/CommandOrControl|CmdOrCtrl|Control/g, 'Ctrl')
    .replace(/\bLeft\b/, '←')
    .replace(/\bRight\b/, '→')
}

/** The next usable item from `from`, stepping by `step` and wrapping around. */
function step(items: MenuNode[], from: number | null, by: 1 | -1): number | null {
  const n = items.length
  for (let i = 1; i <= n; i++) {
    const index = ((from ?? (by === 1 ? -1 : n)) + by * i + n * 2) % n
    if (usable(items[index])) return index
  }
  return null
}

interface Anchor {
  x: number
  y: number
  align?: 'end'
  /** Submenus: the parent row, to open beside it (or flip to the other side). */
  row?: DOMRect
}

function Level({
  items,
  anchor,
  active,
  openIndex,
  onHover,
  onActivate,
  onContext,
  levelRef,
}: {
  items: MenuNode[]
  anchor: Anchor
  active: number | null
  openIndex: number | null
  onHover: (index: number | null) => void
  onActivate: (index: number, event: MouseEvent) => void
  onContext: (index: number, event: MouseEvent) => void
  levelRef: (el: HTMLDivElement | null) => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const withIcons = items.some((item) => isEntry(item) && (item.kind === 'check' || item.favicon || item.icon || item.folder))

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const { offsetWidth: width, offsetHeight: height } = el
    const maxX = window.innerWidth - MARGIN
    const maxY = window.innerHeight - MARGIN
    let left: number
    let top: number
    if (anchor.row) {
      // Beside the parent row, its first item level with the row.
      left = anchor.row.right - 2
      if (left + width > maxX) left = Math.max(MARGIN, anchor.row.left - width + 2)
      top = anchor.row.top - 5
    } else {
      // Like native context menus: open up or to the left when there is no room.
      if (anchor.align === 'end') left = anchor.x - width
      else left = anchor.x + width > maxX ? anchor.x - width : anchor.x
      left = Math.max(MARGIN, Math.min(left, maxX - width))
      top = anchor.y + height > maxY && anchor.y - height >= MARGIN ? anchor.y - height : anchor.y
    }
    top = Math.max(MARGIN, Math.min(top, maxY - height))
    setPos({ left, top })
  }, [anchor.x, anchor.y, anchor.align, anchor.row, items])

  useEffect(() => {
    if (active === null) return
    ref.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' })
  }, [active])

  return (
    <div
      ref={(el) => {
        ref.current = el
        levelRef(el)
      }}
      className="menu"
      role="menu"
      style={pos ? { left: pos.left, top: pos.top } : { left: anchor.x, top: anchor.y, visibility: 'hidden' }}
      onMouseLeave={() => onHover(null)}
    >
      {items.map((item, index) =>
        item.kind === 'separator' ? (
          <div key={`sep-${index}`} className="menu-separator" role="separator" />
        ) : (
          <div
            key={item.id}
            data-index={index}
            role={item.kind === 'check' ? 'menuitemcheckbox' : 'menuitem'}
            aria-checked={item.kind === 'check' ? item.checked === true : undefined}
            aria-disabled={!item.enabled || undefined}
            aria-haspopup={item.kind === 'submenu' || undefined}
            aria-expanded={item.kind === 'submenu' ? openIndex === index : undefined}
            className={`menu-item ${index === active ? 'is-active' : ''} ${item.enabled ? '' : 'is-disabled'}`}
            onMouseEnter={() => onHover(index)}
            onClick={(event) => item.enabled && onActivate(index, event)}
            onAuxClick={(event) => item.enabled && event.button === 1 && item.favicon && onActivate(index, event)}
            onContextMenu={(event) => {
              event.preventDefault()
              event.stopPropagation()
              if (item.context) onContext(index, event)
            }}
          >
            {withIcons && (
              <span className="menu-icon">
                {item.kind === 'check' ? (
                  item.checked && <Icon icon={Check} size={15} />
                ) : item.favicon ? (
                  <Favicon src={faviconFor(item.favicon)} url={item.favicon} size={16} />
                ) : item.icon ? (
                  <img src={item.icon} width={16} height={16} alt="" draggable={false} />
                ) : item.folder ? (
                  <Icon icon={Folder} size={15} />
                ) : null}
              </span>
            )}
            <span className="menu-label">{item.label}</span>
            {item.accelerator && <span className="menu-key mono">{keyLabel(item.accelerator)}</span>}
            {item.kind === 'submenu' && <Icon icon={ChevronRight} size={14} className="menu-arrow" />}
          </div>
        ),
      )}
    </div>
  )
}

/**
 * A menu, or a right-click menu stacked on a folder menu: then `onDismiss` (Esc)
 * closes only it, and the folder menu below stays `inactive` until it does.
 */
export function MenuPopup({ menu, inactive = false, onDismiss }: { menu: MenuModel; inactive?: boolean; onDismiss?: () => void }) {
  useOverlay(true)
  const { closePanel } = useUI()
  const root = useRef<HTMLDivElement>(null)
  // The item whose submenu is open, on each level that has one.
  const [opened, setOpened] = useState<number[]>([])
  // The highlighted item on each level shown.
  const [active, setActive] = useState<(number | null)[]>([null])
  const levels = useRef<(HTMLDivElement | null)[]>([])
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const done = useRef(false)

  const shown: MenuNode[][] = [menu.items]
  for (const [level, index] of opened.entries()) {
    const parent = shown[level][index]
    if (!isEntry(parent) || !parent.submenu) break
    shown.push(parent.submenu)
  }

  const close = () => {
    if (!done.current) {
      done.current = true
      api.menuClosed(menu.id)
    }
    closePanel()
  }

  const dismiss = () => {
    if (!onDismiss) return close()
    done.current = true
    api.menuClosed(menu.id)
    onDismiss()
  }

  // Keys go to the top menu; the folder menu takes them back when its right-click
  // menu closes.
  useEffect(() => {
    if (!inactive) root.current?.focus()
  }, [inactive])

  useEffect(() => {
    const onBlur = () => close()
    window.addEventListener('blur', onBlur)
    window.addEventListener('resize', onBlur)
    return () => {
      clearTimeout(timer.current)
      window.removeEventListener('blur', onBlur)
      window.removeEventListener('resize', onBlur)
      if (!done.current) api.menuClosed(menu.id)
    }
  }, [menu.id])

  const openSubmenu = (level: number, index: number, highlightFirst: boolean) => {
    const item = shown[level]?.[index]
    if (!usable(item) || !item.submenu) return
    clearTimeout(timer.current)
    setOpened([...opened.slice(0, level), index])
    setActive([...opened.slice(0, level), index, highlightFirst ? step(item.submenu, null, 1) : null])
  }

  const closeTo = (level: number) => {
    setOpened(opened.slice(0, level))
    setActive([...active.slice(0, level), opened[level] ?? null])
  }

  // Submenus open and close after a short pause, so moving the pointer across
  // other items on the way into an open submenu doesn't switch it.
  const hover = (level: number, index: number | null) => {
    clearTimeout(timer.current)
    if (index === null) {
      setActive([...active.slice(0, level), opened[level] ?? null])
      return
    }
    setActive([...opened.slice(0, level), index])
    if (opened[level] === index) {
      setOpened(opened.slice(0, level + 1))
      return
    }
    const item = shown[level][index]
    if (usable(item) && item.submenu) {
      timer.current = setTimeout(() => setOpened((o) => [...o.slice(0, level), index]), SUBMENU_DELAY)
    } else if (opened.length > level) {
      timer.current = setTimeout(() => setOpened((o) => o.slice(0, level)), SUBMENU_DELAY)
    }
  }

  const select = (level: number, index: number, event: { ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean; button?: number }) => {
    const item = shown[level][index]
    if (!usable(item)) return
    if (item.submenu) {
      openSubmenu(level, index, false)
      return
    }
    done.current = true
    api.menuSelect(menu.id, item.id, {
      // A middle click opens in the background, like Ctrl+click.
      ctrlKey: event.ctrlKey || event.button === 1,
      shiftKey: event.shiftKey,
      altKey: event.altKey,
      metaKey: event.metaKey,
    })
    closePanel()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (inactive) return
    const level = shown.length - 1
    const items = shown[level]
    const current = active[level] ?? null
    const set = (index: number | null) => setActive([...active.slice(0, level), index])
    event.stopPropagation()
    switch (event.key) {
      case 'ArrowDown':
        set(step(items, current, 1))
        break
      case 'ArrowUp':
        set(step(items, current, -1))
        break
      case 'Home':
        set(step(items, null, 1))
        break
      case 'End':
        set(step(items, null, -1))
        break
      case 'ArrowRight':
        if (current !== null) openSubmenu(level, current, true)
        break
      case 'ArrowLeft':
        if (level > 0) closeTo(level - 1)
        break
      case 'Escape':
        if (level > 0) closeTo(level - 1)
        else dismiss()
        break
      case 'Enter':
      case ' ':
        if (current !== null) {
          const item = items[current]
          if (usable(item) && item.submenu) openSubmenu(level, current, true)
          else select(level, current, event)
        }
        break
      case 'Tab':
        break
      default: {
        // Jump to the next item starting with the typed letter.
        if (event.key.length !== 1 || event.ctrlKey || event.altKey || event.metaKey) return
        const letter = event.key.toLocaleLowerCase()
        const n = items.length
        for (let i = 1; i <= n; i++) {
          const index = ((current ?? -1) + i) % n
          const item = items[index]
          if (usable(item) && item.label.toLocaleLowerCase().startsWith(letter)) {
            set(index)
            break
          }
        }
        return
      }
    }
    event.preventDefault()
  }

  const rowOf = (level: number): DOMRect | undefined =>
    levels.current[level]?.querySelector(`[data-index="${opened[level]}"]`)?.getBoundingClientRect()

  return (
    <div
      ref={root}
      className="scrim is-clear menu-layer"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onMouseDown={(event) => event.target === event.currentTarget && close()}
      onContextMenu={(event) => {
        event.preventDefault()
        if (event.target === event.currentTarget) close()
      }}
    >
      {shown.map((items, level) => (
        <Level
          key={level === 0 ? 'root' : `${level}-${opened[level - 1]}`}
          items={items}
          anchor={level === 0 ? { x: menu.x, y: menu.y, align: menu.align } : { x: 0, y: 0, row: rowOf(level - 1) }}
          active={active[level] ?? null}
          openIndex={opened[level] ?? null}
          onHover={(index) => hover(level, index)}
          onActivate={(index, event) => select(level, index, event)}
          onContext={(index, event) => {
            const item = items[index]
            if (isEntry(item)) api.menuContext(menu.id, item.id, event.clientX, event.clientY)
          }}
          levelRef={(el) => {
            levels.current[level] = el
          }}
        />
      ))}
    </div>
  )
}
