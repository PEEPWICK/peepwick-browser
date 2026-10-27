// The content area. The UI decides where panes go and reports their rectangles; the
// main process places web views exactly there. Built-in pages and interstitials are
// drawn here directly.
import { lazy, Suspense, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { Plus } from 'lucide-react'
import type { PaneRect, TabState } from '@shared/types'
import { internalSubpage } from '@shared/url'
import { api } from '../api'
import { activeWorkspace, useSnapshot, useT } from '../store'
import { Button, Icon } from '../ui'
import { Interstitial } from '../pages/Interstitial'
import { NewTab } from '../pages/NewTab'

const Settings = lazy(() => import('../pages/Settings').then((m) => ({ default: m.Settings })))
const HistoryPage = lazy(() => import('../pages/History').then((m) => ({ default: m.HistoryPage })))
const Extensions = lazy(() => import('../pages/Extensions').then((m) => ({ default: m.Extensions })))
const BookmarksPage = lazy(() => import('../pages/Bookmarks').then((m) => ({ default: m.BookmarksPage })))
const DownloadsPage = lazy(() => import('../pages/Downloads').then((m) => ({ default: m.DownloadsPage })))

function PageFor({ tab }: { tab: TabState }) {
  if (tab.interstitial) return <Interstitial tab={tab} interstitial={tab.interstitial} />
  switch (tab.page) {
    case 'newtab':
      return <NewTab tabId={tab.id} />
    case 'settings':
      return <Settings section={internalSubpage(tab.url)} />
    case 'bookmarks':
      return <BookmarksPage />
    case 'history':
      return <HistoryPage />
    case 'extensions':
      return <Extensions />
    case 'downloads':
      return <DownloadsPage />
    default:
      return null
  }
}

function Pane({ tab, active, split, register }: { tab: TabState; active: boolean; split: boolean; register: (id: number, el: HTMLDivElement | null) => void }) {
  const drawn = !!tab.page || !!tab.interstitial
  return (
    <div
      ref={(el) => register(tab.id, el)}
      className={`pane ${drawn ? 'is-drawn' : ''} ${split && active ? 'is-focused' : ''}`}
      onMouseDown={() => split && api.focusTab(tab.id)}
    >
      {drawn && (
        <Suspense fallback={null}>
          <PageFor tab={tab} />
        </Suspense>
      )}
    </div>
  )
}

export function Content() {
  const { state } = useSnapshot()
  const t = useT()
  const workspace = activeWorkspace(state)
  const activeId = workspace.activeTabId
  const split = workspace.split
  const showSplit = !!split && (split.left === activeId || split.right === activeId) && !!state.tabs[split.left] && !!state.tabs[split.right]
  const ids = showSplit ? [split.left, split.right] : activeId !== null ? [activeId] : []
  const panes = useRef(new Map<number, HTMLDivElement>())
  const area = useRef<HTMLDivElement>(null)
  const last = useRef('')
  const [dragging, setDragging] = useState(false)

  const register = (id: number, el: HTMLDivElement | null) => {
    if (el) panes.current.set(id, el)
    else panes.current.delete(id)
  }

  const report = () => {
    const rects: PaneRect[] = []
    for (const id of ids) {
      const el = panes.current.get(id)
      if (!el) continue
      const r = el.getBoundingClientRect()
      rects.push({ tabId: id, x: r.left, y: r.top, width: r.width, height: r.height })
    }
    const key = JSON.stringify(rects)
    if (key !== last.current) {
      last.current = key
      api.setPanes(rects)
    }
  }

  useLayoutEffect(() => {
    report()
    const observer = new ResizeObserver(() => report())
    if (area.current) observer.observe(area.current)
    for (const el of panes.current.values()) observer.observe(el)
    window.addEventListener('resize', report)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', report)
    }
  })

  const startDrag = (event: ReactPointerEvent) => {
    if (!area.current || !split) return
    event.preventDefault()
    const rect = area.current.getBoundingClientRect()
    setDragging(true)
    api.setOverlay(true)
    const move = (e: PointerEvent) => api.splitRatio((e.clientX - rect.left) / rect.width)
    const up = () => {
      setDragging(false)
      api.setOverlay(false)
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
  }

  return (
    <div className={`content ${showSplit ? 'is-split' : ''} ${dragging ? 'is-resizing' : ''}`} ref={area}>
      {ids.length === 0 && (
        <div className="content-empty">
          <p className="muted">{t('tabs.empty')}</p>
          <Button variant="primary" onClick={() => api.newTab()}>
            <Icon icon={Plus} size={16} /> {t('tabs.new')}
          </Button>
        </div>
      )}
      {ids.map((id, index) => {
        const tab = state.tabs[id]
        if (!tab) return null
        const flex = showSplit ? (index === 0 ? split!.ratio : 1 - split!.ratio) : 1
        return (
          <div key={id} className="pane-slot" style={{ flexGrow: flex, flexBasis: 0 }}>
            <Pane tab={tab} active={id === activeId} split={showSplit} register={register} />
            {showSplit && index === 0 && <div className="split-handle" onPointerDown={startDrag} role="separator" aria-orientation="vertical" />}
          </div>
        )
      })}
    </div>
  )
}
