import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Copy,
  Ellipsis,
  Globe,
  History,
  Minus,
  PanelLeftOpen,
  RotateCw,
  Search,
  ShieldCheck,
  ShieldOff,
  Square,
  SquareArrowOutUpRight,
  Star,
  Terminal,
  X,
} from 'lucide-react'
import { formatNumber } from '@shared/i18n'
import { SEARCH_ENGINES, searchURL } from '@shared/search'
import type { Suggestion, TabState } from '@shared/types'
import { displayParts } from '@shared/url'
import { api, call } from '../api'
import { useUI } from '../context'
import { activeTab, useBookmarks, useSnapshot, useT } from '../store'
import { findLink } from './BookmarkDialogs'
import { SiteButton } from './SiteInfo'
import { Favicon, Icon, IconButton, useOverlay } from '../ui'

function ShieldBadge({ tab }: { tab: TabState }) {
  const { state, settings } = useSnapshot()
  const t = useT()
  const { togglePanel } = useUI()
  if (tab.page || !/^https?:/.test(tab.url)) return null
  const off = settings.shieldLevel === 'off' || state.shieldPaused
  return (
    <button
      type="button"
      data-anchor="shield"
      className={`omni-badge ${off ? 'is-off' : ''} ${!off && tab.blocked > 0 ? 'is-blocking' : ''}`}
      title={off ? t('shield.paused') : t('shield.on')}
      aria-label={t('shield.title')}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => togglePanel({ kind: 'shield', anchor: event.currentTarget.getBoundingClientRect() })}
    >
      <Icon icon={off ? ShieldOff : ShieldCheck} size={15} />
      {!off && tab.blocked > 0 && <span className="mono">{formatNumber(settings.language, tab.blocked)}</span>}
    </button>
  )
}

const SUGGESTION_ICONS = { url: Globe, search: Search, history: History, tab: SquareArrowOutUpRight, command: Terminal, bookmark: Star }

/** The star: bookmarks the page on first click, then opens the editing panel. */
function StarButton({ tab }: { tab: TabState }) {
  const tree = useBookmarks()
  const t = useT()
  const { openPanel } = useUI()
  const bookmarked = !!findLink(tree, tab.url)
  return (
    <button
      type="button"
      data-anchor="star"
      className={`icon-button omni-action ${bookmarked ? 'is-active' : ''}`}
      aria-label={bookmarked ? t('omnibox.bookmarked') : t('omnibox.bookmark')}
      title={`${bookmarked ? t('omnibox.bookmarked') : t('omnibox.bookmark')} · Ctrl+D`}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => {
        if (bookmarked) openPanel({ kind: 'star', anchor: event.currentTarget.getBoundingClientRect(), created: false })
        else void call.bookmarkPage()
      }}
    >
      <Icon icon={Star} size={16} fill={bookmarked ? 'currentColor' : 'none'} />
    </button>
  )
}

function Omnibox({ tab }: { tab: TabState | null }) {
  const { settings, state } = useSnapshot()
  const t = useT()
  const { registerOmnibox } = useUI()
  const input = useRef<HTMLInputElement>(null)
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState('')
  const [items, setItems] = useState<Suggestion[]>([])
  const [selected, setSelected] = useState(0)
  const request = useRef(0)
  const open = editing && items.length > 0
  useOverlay(open)

  const startEditing = (initial?: string) => {
    setEditing(true)
    const value = initial ?? (tab && !tab.page && !tab.interstitial ? tab.url : tab?.interstitial && 'url' in tab.interstitial ? tab.interstitial.url : '')
    setText(value)
    setItems([])
    setSelected(0)
    requestAnimationFrame(() => {
      input.current?.focus()
      if (initial === undefined) input.current?.select()
      if (initial) void query(initial)
    })
  }

  useEffect(() => registerOmnibox((initial) => startEditing(initial)))

  const stopEditing = () => {
    setEditing(false)
    setItems([])
    request.current++
  }

  useEffect(() => {
    const onBlur = () => stopEditing()
    window.addEventListener('blur', onBlur)
    return () => window.removeEventListener('blur', onBlur)
  }, [])

  // Leave edit mode when the page navigates under us.
  useEffect(() => stopEditing(), [tab?.id])

  const query = async (value: string) => {
    const id = ++request.current
    if (!value.trim()) {
      setItems([])
      return
    }
    const result = await call.suggest(value)
    if (id === request.current) {
      setItems(result)
      setSelected(0)
    }
  }

  const go = (suggestion: Suggestion | null, newTab: boolean) => {
    let target = text.trim()
    if (suggestion) {
      if (suggestion.kind === 'tab' && suggestion.tabId !== undefined) {
        api.activateTab(suggestion.tabId)
        stopEditing()
        return
      }
      target = suggestion.kind === 'search' ? searchURL(settings.searchEngine, suggestion.value) : suggestion.value
    }
    if (!target) return
    stopEditing()
    input.current?.blur()
    if (newTab || !tab) api.newTab({ url: target })
    else api.navigate(tab.id, target)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setSelected((s) => Math.min(items.length - 1, s + 1))
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      setSelected((s) => Math.max(0, s - 1))
    } else if (event.key === 'Enter') {
      event.preventDefault()
      go(items[selected] ?? null, event.altKey)
    } else if (event.key === 'Escape') {
      event.preventDefault()
      if (items.length > 0) setItems([])
      else {
        stopEditing()
        input.current?.blur()
        api.focusTab(tab?.id ?? -1)
      }
    }
  }

  const parts = tab ? displayParts(tab.url) : null
  const showDisplay = !editing && tab && !tab.page && parts
  const zoom = tab ? Math.round(tab.zoom * 100) : 100

  return (
    <div className={`omnibox ${editing ? 'is-editing' : ''} no-drag`}>
      {tab && <SiteButton tab={tab} />}
      <div className="omni-field" onMouseDown={() => !editing && startEditing()}>
        <input
          ref={input}
          className="omni-input"
          value={editing ? text : ''}
          placeholder={showDisplay ? '' : t('omnibox.placeholder')}
          spellCheck={false}
          autoComplete="off"
          aria-label={t('omnibox.placeholder')}
          aria-expanded={open}
          aria-controls="omni-list"
          onFocus={() => !editing && startEditing()}
          onBlur={() => setTimeout(stopEditing, 0)}
          onChange={(event) => {
            setText(event.target.value)
            void query(event.target.value)
          }}
          onKeyDown={onKeyDown}
        />
        {showDisplay && (
          <div className="omni-display" aria-hidden>
            {parts.prefix && <span className="omni-rest">{parts.prefix}</span>}
            <span className="omni-host">{parts.host}</span>
            <span className="omni-rest">{parts.rest}</span>
          </div>
        )}
      </div>
      {tab && !editing && zoom !== 100 && (
        <button type="button" className="omni-chip mono" title={t('menu.zoomReset')} onClick={() => api.zoom('reset')}>
          %{zoom}
        </button>
      )}
      {tab && !editing && (tab.readerAvailable || tab.reader) && (
        <IconButton
          icon={BookOpen}
          label={tab.reader ? t('reader.close') : t('reader.open')}
          active={tab.reader}
          className="omni-action"
          onClick={() => api.toggleReader()}
        />
      )}
      {tab && !editing && <ShieldBadge tab={tab} />}
      {tab && !editing && !tab.page && !state.incognito && /^(https?|file):/.test(tab.url) && <StarButton tab={tab} />}
      {open && (
        <ul className="omni-list" id="omni-list" role="listbox" onMouseDown={(event) => event.preventDefault()}>
          {items.map((item, index) => (
            <li
              key={`${item.kind}-${item.value}-${index}`}
              role="option"
              aria-selected={index === selected}
              className={`omni-item ${index === selected ? 'is-selected' : ''}`}
              onMouseEnter={() => setSelected(index)}
              onClick={(event) => go(item, event.ctrlKey || event.metaKey)}
            >
              <span className="omni-item-icon">
                {item.favicon || item.kind === 'history' || item.kind === 'bookmark' ? (
                  <Favicon src={item.favicon} url={item.value} size={16} />
                ) : (
                  <Icon icon={SUGGESTION_ICONS[item.kind]} size={15} />
                )}
              </span>
              <span className="omni-item-title">{item.title}</span>
              {item.kind === 'search' && (
                <span className="omni-item-meta label">{t('omnibox.searchWith', { engine: SEARCH_ENGINES[settings.searchEngine].name })}</span>
              )}
              {item.kind === 'tab' && <span className="omni-item-meta label">{t('omnibox.switchTo')}</span>}
              {(item.kind === 'history' || item.kind === 'url' || item.kind === 'bookmark') && item.title !== item.value && (
                <span className="omni-item-meta mono">{displayParts(item.value).host}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function WindowControls() {
  const { state } = useSnapshot()
  const t = useT()
  return (
    <div className="window-controls no-drag">
      <button type="button" aria-label={t('window.minimize')} title={t('window.minimize')} onClick={() => api.windowControl('minimize')}>
        <Icon icon={Minus} size={16} />
      </button>
      <button type="button" aria-label={t('window.maximize')} title={t('window.maximize')} onClick={() => api.windowControl('maximize')}>
        <Icon icon={state.maximized ? Copy : Square} size={state.maximized ? 14 : 13} />
      </button>
      <button type="button" className="is-close" aria-label={t('common.close')} onClick={() => api.windowControl('close')}>
        <Icon icon={X} size={16} />
      </button>
    </div>
  )
}

export function Toolbar() {
  const { state, settings } = useSnapshot()
  const t = useT()
  const tab = activeTab(state)
  return (
    <header className="toolbar drag" onDoubleClick={(event) => event.target === event.currentTarget && api.windowControl('maximize')}>
      {settings.sidebarCollapsed && (
        <IconButton
          icon={PanelLeftOpen}
          label={t('cmd.toggle-sidebar')}
          className="no-drag"
          onClick={() => api.setSettings({ sidebarCollapsed: false })}
        />
      )}
      <div className="nav-buttons no-drag">
        <IconButton icon={ArrowLeft} label={t('nav.back')} disabled={!tab?.canGoBack} onClick={() => tab && api.back(tab.id)} />
        <IconButton icon={ArrowRight} label={t('nav.forward')} disabled={!tab?.canGoForward} onClick={() => tab && api.forward(tab.id)} />
        {tab?.loading ? (
          <IconButton icon={X} label={t('nav.stop')} onClick={() => api.stop(tab.id)} />
        ) : (
          <IconButton icon={RotateCw} label={t('nav.reload')} disabled={!tab || !!tab.page} onClick={(e) => tab && api.reload(tab.id, e.shiftKey)} />
        )}
      </div>
      <Omnibox tab={tab} />
      <div className="toolbar-actions no-drag">
        {state.extensionsPartition && tab?.contentsId != null && (
          <browser-action-list partition={state.extensionsPartition} tab={String(tab.contentsId)} alignment="bottom left" />
        )}
        <IconButton
          icon={Ellipsis}
          label={t('nav.menu')}
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            api.appMenu(rect.right, rect.bottom + 4)
          }}
        />
      </div>
      <WindowControls />
    </header>
  )
}
