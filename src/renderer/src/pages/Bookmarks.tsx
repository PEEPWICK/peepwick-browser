// The bookmark manager: a folder tree on the left, the folder's contents (or search
// results) on the right. Multi-select with Ctrl and Shift, drag to reorder or move,
// keyboard: arrows, Enter, Delete, F2, Ctrl+A.
import { useEffect, useMemo, useRef, useState, type DragEvent, type KeyboardEvent, type MouseEvent } from 'react'
import { ChevronDown, ChevronRight, Download, Ellipsis, Folder, FolderPlus, Pencil, Plus, Search, Upload } from 'lucide-react'
import { locate, walk, type BookmarkFolder, type BookmarkNode } from '@shared/bookmarks'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
import { BOOKMARK_DRAG, dropInto } from '../components/BookmarksBar'
import { useFolderName } from '../components/BookmarkDialogs'
import { useUI } from '../context'
import { useBookmarks, useSnapshot, useT } from '../store'
import { Button, Favicon, Icon, IconButton, faviconFor } from '../ui'

interface Row {
  node: BookmarkNode
  /** Folder names leading to the node, shown in search results. */
  where?: string
}

function TreeFolder({
  folder,
  depth,
  current,
  expanded,
  onToggle,
  onSelect,
}: {
  folder: BookmarkFolder
  depth: number
  current: string
  expanded: Set<string>
  onToggle: (id: string) => void
  onSelect: (id: string) => void
}) {
  const name = useFolderName()
  const { state } = useSnapshot()
  const [drop, setDrop] = useState(false)
  const subfolders = folder.children.filter((node): node is BookmarkFolder => node.type === 'folder')
  const open = expanded.has(folder.id)
  return (
    <li>
      <div
        className={`tree-row ${current === folder.id ? 'is-active' : ''} ${drop ? 'is-drop' : ''}`}
        style={{ paddingLeft: 8 + depth * 16 }}
        onClick={() => onSelect(folder.id)}
        onContextMenu={(event) => {
          event.preventDefault()
          api.bookmarkMenu(folder.id, event.clientX, event.clientY)
        }}
        onDragOver={(event) => {
          event.preventDefault()
          setDrop(true)
        }}
        onDragLeave={() => setDrop(false)}
        onDrop={(event) => {
          event.preventDefault()
          setDrop(false)
          dropInto(event, folder.id, folder.children.length, state.tabs)
        }}
      >
        <button
          type="button"
          className="tree-twist"
          aria-label={open ? 'collapse' : 'expand'}
          style={{ visibility: subfolders.length ? 'visible' : 'hidden' }}
          onClick={(event) => {
            event.stopPropagation()
            onToggle(folder.id)
          }}
        >
          <Icon icon={open ? ChevronDown : ChevronRight} size={14} />
        </button>
        <Icon icon={Folder} size={15} />
        <span className="tree-name">{name(folder.id, folder.title)}</span>
      </div>
      {open && subfolders.length > 0 && (
        <ul>
          {subfolders.map((child) => (
            <TreeFolder key={child.id} folder={child} depth={depth + 1} current={current} expanded={expanded} onToggle={onToggle} onSelect={onSelect} />
          ))}
        </ul>
      )}
    </li>
  )
}

export function BookmarksPage() {
  const tree = useBookmarks()
  const { state } = useSnapshot()
  const { openPanel, showToast } = useUI()
  const t = useT()
  const name = useFolderName()
  const [current, setCurrent] = useState('bar')
  const [expanded, setExpanded] = useState(() => new Set(['bar', 'other']))
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<string[]>([])
  const [anchor, setAnchor] = useState<string | null>(null)
  const [dropAt, setDropAt] = useState<{ index: number; into?: string } | null>(null)
  const list = useRef<HTMLDivElement>(null)

  const located = locate(tree, current)
  const folder = located && located.node.type === 'folder' ? located.node : tree.bar
  useEffect(() => {
    if (!located) setCurrent('bar')
  }, [located])

  const rows: Row[] = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase('tr')
    if (!needle) return folder.children.map((node) => ({ node }))
    const out: Row[] = []
    walk(tree, (node, path) => {
      const haystack = `${node.title} ${node.type === 'url' ? node.url : ''}`.toLocaleLowerCase('tr')
      if (haystack.includes(needle)) out.push({ node, where: path.map((f) => name(f.id, f.title)).join(' › ') })
    })
    return out
  }, [query, folder, tree, name])

  useEffect(() => setSelected([]), [current, query])

  const ids = rows.map((row) => row.node.id)
  const select = (event: MouseEvent, id: string) => {
    if (event.shiftKey && anchor && ids.includes(anchor)) {
      const [a, b] = [ids.indexOf(anchor), ids.indexOf(id)].sort((x, y) => x - y)
      setSelected(ids.slice(a, b + 1))
    } else if (event.ctrlKey || event.metaKey) {
      setSelected((list) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]))
      setAnchor(id)
    } else {
      setSelected([id])
      setAnchor(id)
    }
  }

  const open = (node: BookmarkNode, how: 'current' | 'background' = 'current') => {
    if (node.type === 'folder') {
      setQuery('')
      setCurrent(node.id)
      setExpanded((set) => new Set([...set, ...(locate(tree, node.id)?.path.map((f) => f.id) ?? []), node.id]))
    } else api.bookmarkOpen([node.id], how)
  }

  const removeSelected = () => {
    if (selected.length > 0) api.bookmarkRemove(selected)
  }

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.target as HTMLElement).tagName === 'INPUT') return
    const index = selected.length ? ids.indexOf(selected[selected.length - 1]) : -1
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const next = Math.max(0, Math.min(ids.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)))
      if (ids[next]) {
        setSelected([ids[next]])
        setAnchor(ids[next])
        list.current?.querySelectorAll('.bm-row')[next]?.scrollIntoView({ block: 'nearest' })
      }
    } else if (event.key === 'Enter' && index >= 0) {
      open(rows[index].node, event.ctrlKey ? 'background' : 'current')
    } else if (event.key === 'Delete') {
      removeSelected()
    } else if (event.key === 'F2' && selected.length === 1) {
      openPanel({ kind: 'bookmark-edit', id: selected[0] })
    } else if (event.key === 'a' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      setSelected(ids)
    } else if (event.key === 'Backspace' && located?.parent && !query) {
      setCurrent(located.parent.id)
    }
  }

  const onRowDrop = (event: DragEvent, index: number, into?: string) => {
    event.preventDefault()
    setDropAt(null)
    const dragged = event.dataTransfer.getData(BOOKMARK_DRAG)
    const many = dragged && selected.includes(dragged) ? selected : null
    if (many) api.bookmarkMove(many, into ?? folder.id, into ? Number.MAX_SAFE_INTEGER : index)
    else dropInto(event, into ?? folder.id, into ? Number.MAX_SAFE_INTEGER : index, state.tabs)
  }

  const breadcrumb = located ? [...located.path, ...(located.node.type === 'folder' ? [located.node] : [])] : []
  const total = useMemo(() => {
    let n = 0
    walk(tree, (node) => node.type === 'url' && n++)
    return n
  }, [tree])

  return (
    <div className="page bookmarks-page" onKeyDown={onKeyDown}>
      <aside className="bm-tree">
        <h1 className="settings-heading">{t('bookmarks.title')}</h1>
        <ul>
          {[tree.bar, tree.other].map((root) => (
            <TreeFolder
              key={root.id}
              folder={root}
              depth={0}
              current={query ? '' : current}
              expanded={expanded}
              onToggle={(id) => setExpanded((set) => {
                const next = new Set(set)
                if (next.has(id)) next.delete(id)
                else next.add(id)
                return next
              })}
              onSelect={(id) => {
                setQuery('')
                setCurrent(id)
              }}
            />
          ))}
        </ul>
        <div className="bm-tree-foot">
          <span className="label">{t('import.links', { n: total })}</span>
          <button type="button" className="link-button" onClick={() => openPanel({ kind: 'import' })}>
            <Icon icon={Download} size={13} /> {t('import.menu')}
          </button>
          <button
            type="button"
            className="link-button"
            onClick={() => void call.exportBookmarks().then((ok) => ok && showToast({ text: t('bookmarks.exported') }))}
          >
            <Icon icon={Upload} size={13} /> {t('bookmarks.exportMenu')}
          </button>
        </div>
      </aside>

      <section className="bm-main">
        <header className="bm-head">
          <label className="search-field bm-search">
            <Icon icon={Search} size={16} />
            <input value={query} placeholder={t('bookmarks.search')} spellCheck={false} onChange={(event) => setQuery(event.target.value)} />
          </label>
          <Button onClick={() => openPanel({ kind: 'bookmark-edit', parentId: folder.id, folder: true })}>
            <Icon icon={FolderPlus} size={15} /> {t('bookmarks.addFolder')}
          </Button>
          <Button onClick={() => openPanel({ kind: 'bookmark-edit', parentId: folder.id })}>
            <Icon icon={Plus} size={15} /> {t('bookmarks.addPage')}
          </Button>
          <IconButton
            icon={Ellipsis}
            label={t('nav.menu')}
            onClick={(event) => {
              const rect = event.currentTarget.getBoundingClientRect()
              api.bookmarkMenu(folder.id, rect.left, rect.bottom + 4)
            }}
          />
        </header>

        <div className="bm-crumbs">
          {query ? (
            <span className="label">{t('bookmarks.results', { n: rows.length })}</span>
          ) : (
            breadcrumb.map((f, i) => (
              <span key={f.id} className="bm-crumb">
                {i > 0 && <span className="muted"> › </span>}
                <button type="button" className={i === breadcrumb.length - 1 ? 'is-current' : ''} onClick={() => setCurrent(f.id)}>
                  {name(f.id, f.title)}
                </button>
              </span>
            ))
          )}
          {selected.length > 1 && (
            <span className="bm-selection">
              <span className="label">{t('bookmarks.selected', { n: selected.length })}</span>
              <button type="button" className="link-button" onClick={() => api.bookmarkOpen(selected, 'tab')}>
                {t('bookmarks.openSelected')}
              </button>
              <button type="button" className="link-button" onClick={removeSelected}>
                {t('bookmarks.deleteSelected')}
              </button>
            </span>
          )}
        </div>

        <div
          className="bm-list"
          ref={list}
          tabIndex={0}
          onDragOver={(event) => {
            if (event.target === event.currentTarget) {
              event.preventDefault()
              setDropAt({ index: rows.length })
            }
          }}
          onDrop={(event) => event.target === event.currentTarget && onRowDrop(event, folder.children.length)}
          onClick={(event) => event.target === event.currentTarget && setSelected([])}
        >
          {rows.length === 0 && (
            <p className="muted list-empty">{query ? t('bookmarks.noResults') : total === 0 ? t('bookmarks.emptyAll') : t('bookmarks.emptyFolder')}</p>
          )}
          {rows.map(({ node, where }, index) => {
            const isSelected = selected.includes(node.id)
            const zoneOf = (event: DragEvent): { index: number; into?: string } => {
              const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
              const y = (event.clientY - rect.top) / rect.height
              if (node.type === 'folder' && y > 0.25 && y < 0.75) return { index, into: node.id }
              return { index: y < 0.5 ? index : index + 1 }
            }
            const dropClass =
              dropAt && !query ? (dropAt.into === node.id ? 'drop-into' : dropAt.index === index && !dropAt.into ? 'drop-before' : dropAt.index === index + 1 && !dropAt.into && index === rows.length - 1 ? 'drop-after' : '') : ''
            return (
              <div
                key={node.id}
                className={`bm-row ${isSelected ? 'is-selected' : ''} ${dropClass}`}
                draggable
                onDragStart={(event) => {
                  if (!isSelected) setSelected([node.id])
                  event.dataTransfer.setData(BOOKMARK_DRAG, node.id)
                  if (node.type === 'url') event.dataTransfer.setData('text/uri-list', node.url)
                }}
                onDragOver={(event) => {
                  if (query) return
                  event.preventDefault()
                  setDropAt(zoneOf(event))
                }}
                onDragLeave={() => setDropAt(null)}
                onDrop={(event) => {
                  if (query) return
                  const zone = zoneOf(event)
                  onRowDrop(event, zone.index, zone.into)
                }}
                onClick={(event) => select(event, node.id)}
                onDoubleClick={() => open(node)}
                onAuxClick={(event) => event.button === 1 && node.type === 'url' && api.bookmarkOpen([node.id], 'background')}
                onContextMenu={(event) => {
                  event.preventDefault()
                  if (!isSelected) setSelected([node.id])
                  api.bookmarkMenu(node.id, event.clientX, event.clientY)
                }}
              >
                <span className="bm-icon">
                  {node.type === 'folder' ? <Icon icon={Folder} size={16} /> : <Favicon src={faviconFor(node.url)} url={node.url} size={16} />}
                </span>
                <span className="bm-title">{node.title}</span>
                <span className="bm-meta mono muted">
                  {where ?? (node.type === 'url' ? displayHost(node.url) : t('bookmarks.items', { n: node.children.length }))}
                </span>
                <span className="bm-actions">
                  <IconButton icon={Pencil} label={t('bookmarks.edit')} onClick={() => openPanel({ kind: 'bookmark-edit', id: node.id })} />
                  <IconButton
                    icon={Ellipsis}
                    label={t('nav.menu')}
                    onClick={(event) => {
                      const rect = event.currentTarget.getBoundingClientRect()
                      api.bookmarkMenu(node.id, rect.left, rect.bottom + 4)
                    }}
                  />
                </span>
              </div>
            )
          })}
        </div>
      </section>
    </div>
  )
}
