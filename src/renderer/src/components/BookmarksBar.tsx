// The bookmarks bar under the toolbar. Folders open as native menus (nested folders
// cascade); items that don't fit move into the » menu.
import { useLayoutEffect, useRef, useState, type DragEvent, type MouseEvent } from 'react'
import { ChevronsRight, Folder, FolderOpen } from 'lucide-react'
import type { BookmarkNode } from '@shared/bookmarks'
import { api, call } from '../api'
import { useUI } from '../context'
import { useBookmarks, useSnapshot, useT } from '../store'
import { Favicon, Icon, faviconFor } from '../ui'

export const BOOKMARK_DRAG = 'application/x-peepwick-bookmark'
const TAB_DRAG = 'application/x-peepwick-tab'

function openHow(event: MouseEvent): 'current' | 'background' | 'window' {
  if (event.button === 1 || event.ctrlKey || event.metaKey) return 'background'
  if (event.shiftKey) return 'window'
  return 'current'
}

/** Creates or moves a bookmark from whatever was dropped. */
export function dropInto(event: DragEvent, parentId: string, index: number, tabs: Record<number, { url: string; title: string }>): boolean {
  const bookmarkId = event.dataTransfer.getData(BOOKMARK_DRAG)
  if (bookmarkId) {
    api.bookmarkMove([bookmarkId], parentId, index)
    return true
  }
  const tabId = Number(event.dataTransfer.getData(TAB_DRAG))
  const tab = Number.isFinite(tabId) ? tabs[tabId] : undefined
  if (tab && /^(https?|file):/.test(tab.url)) {
    void call.bookmarkCreate({ parentId, index, title: tab.title, url: tab.url })
    return true
  }
  const url = event.dataTransfer.getData('text/uri-list').split('\n')[0]?.trim() || event.dataTransfer.getData('text/plain').trim()
  if (/^(https?|file):/.test(url)) {
    void call.bookmarkCreate({ parentId, index, title: url, url })
    return true
  }
  return false
}

function BarItem({
  node,
  index,
  onDropAt,
}: {
  node: BookmarkNode
  index: number
  onDropAt: (event: DragEvent, index: number, into?: string) => void
}) {
  const [drop, setDrop] = useState<'before' | 'after' | 'into' | null>(null)
  const isFolder = node.type === 'folder'

  const zone = (event: DragEvent): 'before' | 'after' | 'into' => {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
    const x = (event.clientX - rect.left) / rect.width
    if (isFolder && x > 0.25 && x < 0.75) return 'into'
    return x < 0.5 ? 'before' : 'after'
  }

  return (
    <button
      type="button"
      className={`bar-item ${drop ? `drop-${drop}` : ''}`}
      title={node.type === 'url' ? `${node.title}\n${node.url}` : node.title}
      draggable
      onDragStart={(event) => {
        event.dataTransfer.setData(BOOKMARK_DRAG, node.id)
        if (node.type === 'url') event.dataTransfer.setData('text/uri-list', node.url)
        event.dataTransfer.effectAllowed = 'move'
      }}
      onDragOver={(event) => {
        event.preventDefault()
        setDrop(zone(event))
      }}
      onDragLeave={() => setDrop(null)}
      onDrop={(event) => {
        event.preventDefault()
        const where = zone(event)
        setDrop(null)
        if (where === 'into') onDropAt(event, 0, node.id)
        else onDropAt(event, where === 'before' ? index : index + 1)
      }}
      onMouseDown={(event) => event.button === 1 && event.preventDefault()}
      onClick={(event) => {
        if (node.type === 'folder') {
          const rect = event.currentTarget.getBoundingClientRect()
          api.bookmarkFolderMenu(node.id, rect.left, rect.bottom + 2)
        } else api.bookmarkOpen([node.id], openHow(event))
      }}
      onAuxClick={(event) => {
        if (event.button === 1) api.bookmarkOpen([node.id], 'background')
      }}
      onContextMenu={(event) => {
        event.preventDefault()
        api.bookmarkMenu(node.id, event.clientX, event.clientY)
      }}
    >
      {isFolder ? <Icon icon={Folder} size={14} /> : <Favicon src={faviconFor(node.url)} url={node.url} size={14} />}
      <span className="bar-label">{node.title}</span>
    </button>
  )
}

export function BookmarksBar() {
  const tree = useBookmarks()
  const { state } = useSnapshot()
  const { openPanel } = useUI()
  const t = useT()
  const items = tree.bar.children
  const row = useRef<HTMLDivElement>(null)
  const widths = useRef<number[]>([])
  const [visible, setVisible] = useState(items.length)
  const [measuring, setMeasuring] = useState(true)
  const [barDrop, setBarDrop] = useState(false)

  // Measure every item once per change, then fit them to the available width.
  useLayoutEffect(() => {
    setMeasuring(true)
    setVisible(items.length)
  }, [items])

  useLayoutEffect(() => {
    const el = row.current
    if (!el) return
    if (measuring) {
      widths.current = [...el.querySelectorAll<HTMLElement>('.bar-item')].map((item) => item.offsetWidth + 2)
      setMeasuring(false)
    }
    const fit = () => {
      const available = el.clientWidth - 40
      let used = 0
      let count = 0
      for (const width of widths.current) {
        if (used + width > available) break
        used += width
        count++
      }
      setVisible((current) => (current === count ? current : count))
    }
    fit()
    const observer = new ResizeObserver(fit)
    observer.observe(el)
    return () => observer.disconnect()
  }, [measuring])

  const tabs = state.tabs
  const onDropAt = (event: DragEvent, index: number, into?: string) => {
    dropInto(event, into ?? 'bar', index, tabs)
  }

  const shown = measuring ? items : items.slice(0, visible)
  return (
    <div
      className={`bookmarks-bar ${barDrop ? 'is-drop' : ''}`}
      ref={row}
      onDragOver={(event) => {
        if (event.target !== event.currentTarget) return
        event.preventDefault()
        setBarDrop(true)
      }}
      onDragLeave={() => setBarDrop(false)}
      onDrop={(event) => {
        setBarDrop(false)
        if (event.target !== event.currentTarget) return
        event.preventDefault()
        dropInto(event, 'bar', items.length, tabs)
      }}
    >
      {items.length === 0 && (
        <span className="bar-empty">
          {t('bookmarks.barEmpty')}
          <button type="button" className="link-button" onClick={() => openPanel({ kind: 'import' })}>
            {t('import.menu')}
          </button>
        </span>
      )}
      {shown.map((node, index) => (
        <BarItem key={node.id} node={node} index={index} onDropAt={onDropAt} />
      ))}
      <span className="spacer" />
      {!measuring && visible < items.length && (
        <button
          type="button"
          className="bar-item bar-more"
          title={t('bookmarks.more')}
          aria-label={t('bookmarks.more')}
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            api.bookmarkFolderMenu('bar', rect.left, rect.bottom + 2, visible)
          }}
        >
          <Icon icon={ChevronsRight} size={14} />
        </button>
      )}
      {tree.other.children.length > 0 && (
        <button
          type="button"
          className="bar-item"
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            api.bookmarkFolderMenu('other', rect.left, rect.bottom + 2)
          }}
          onContextMenu={(event) => {
            event.preventDefault()
            api.bookmarkMenu('other', event.clientX, event.clientY)
          }}
          onDragOver={(event) => event.preventDefault()}
          onDrop={(event) => {
            event.preventDefault()
            dropInto(event, 'other', tree.other.children.length, tabs)
          }}
        >
          <Icon icon={FolderOpen} size={14} />
          <span className="bar-label">{t('bookmarks.other')}</span>
        </button>
      )}
    </div>
  )
}
