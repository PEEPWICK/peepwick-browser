import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Globe, History, Search, SquareArrowOutUpRight, Star, Terminal } from 'lucide-react'
import { SEARCH_ENGINES, searchURL } from '@shared/search'
import type { Stats, Suggestion, TopSite } from '@shared/types'
import { displayParts } from '@shared/url'
import { api, call } from '../api'
import { Crowd, DoubleEye, LookMark } from '../brand'
import { useSnapshot, useT } from '../store'
import { Favicon, Icon } from '../ui'

const ICONS = { url: Globe, search: Search, history: History, tab: SquareArrowOutUpRight, command: Terminal, bookmark: Star }

/** The search box on the new tab page. It works on its own, with suggestions below it. */
function PageSearch({ tabId }: { tabId: number }) {
  const { settings } = useSnapshot()
  const t = useT()
  const input = useRef<HTMLInputElement>(null)
  const [text, setText] = useState('')
  const [items, setItems] = useState<Suggestion[]>([])
  const [selected, setSelected] = useState(0)
  const [focused, setFocused] = useState(false)
  const request = useRef(0)

  // Focus once when the page opens, and again when asked (Ctrl+T on a new tab).
  useEffect(() => {
    const focus = () => input.current?.focus()
    focus()
    window.addEventListener('pw-page-search', focus)
    // A key pressed elsewhere on the page lands in the box: focusing it during
    // keydown lets the browser type the character there.
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.ctrlKey || event.metaKey || event.altKey || event.key.length !== 1) return
      const target = event.target as HTMLElement
      if (target.closest('input, textarea, select, [contenteditable]') || !target.closest('.newtab')) return
      input.current?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('pw-page-search', focus)
      window.removeEventListener('keydown', onKey)
    }
  }, [])

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

  const go = (item: Suggestion | null, newTab: boolean) => {
    let target = text.trim()
    if (item) {
      if (item.kind === 'tab' && item.tabId !== undefined) {
        api.activateTab(item.tabId)
        return
      }
      target = item.kind === 'search' ? searchURL(settings.searchEngine, item.value) : item.value
    }
    if (!target) return
    request.current++
    setItems([])
    if (newTab) api.newTab({ url: target })
    else api.navigate(tabId, target)
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
      else setText('')
    }
  }

  const open = focused && items.length > 0
  return (
    <div className={`page-search ${open ? 'is-open' : ''}`}>
      <label className="page-search-box">
        <Icon icon={Search} size={18} />
        <input
          ref={input}
          value={text}
          placeholder={t('newtab.search')}
          spellCheck={false}
          autoComplete="off"
          aria-label={t('newtab.search')}
          aria-expanded={open}
          onFocus={() => setFocused(true)}
          onBlur={() => setTimeout(() => setFocused(false), 120)}
          onChange={(event) => {
            setText(event.target.value)
            void query(event.target.value)
          }}
          onKeyDown={onKeyDown}
        />
      </label>
      {open && (
        <ul className="page-search-list" role="listbox" onMouseDown={(event) => event.preventDefault()}>
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
                {item.favicon ? <Favicon src={item.favicon} url={item.value} size={16} /> : <Icon icon={ICONS[item.kind]} size={15} />}
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

function today(lang: string): string {
  return new Intl.DateTimeFormat(lang === 'tr' ? 'tr-TR' : 'en-US', { weekday: 'long', day: 'numeric', month: 'long' }).format(new Date())
}

export function NewTab({ tabId }: { tabId: number }) {
  const { state, settings } = useSnapshot()
  const t = useT()
  const [sites, setSites] = useState<TopSite[] | null>(null)
  const [stats, setStats] = useState<Stats | null>(null)

  useEffect(() => {
    void call.topSites().then(setSites)
    void call.stats().then(setStats)
  }, [])

  if (state.incognito) {
    return (
      <div className="page newtab is-incognito">
        <div className="newtab-center">
          <DoubleEye height={48} mode="closed" />
          <h1 className="display">{t('incognito.title')}</h1>
          <p className="lead narrow">{t('incognito.body')}</p>
          <PageSearch tabId={tabId} />
        </div>
      </div>
    )
  }

  return (
    <div className="page newtab">
      <Crowd />
      <div className="newtab-date label">{today(settings.language)}</div>
      <div className="newtab-center">
        <h1 className="display">{t('app.tagline')}</h1>
        <PageSearch tabId={tabId} />
        {sites && sites.length > 0 && (
          <nav className="top-sites" aria-label="Top sites">
            {sites.map((site) => (
              <a
                key={site.url}
                className="top-site"
                href={site.url}
                onClick={(event) => {
                  event.preventDefault()
                  api.openURL(site.url, event.ctrlKey || event.metaKey)
                }}
                onAuxClick={(event) => {
                  if (event.button === 1) {
                    event.preventDefault()
                    api.openURL(site.url, true)
                  }
                }}
              >
                <Favicon src={site.favicon} url={site.url} size={24} />
                <span className="mono">{site.title}</span>
              </a>
            ))}
          </nav>
        )}
        {sites && sites.length === 0 && <p className="muted newtab-empty">{t('newtab.empty')}</p>}
      </div>
      {stats && (stats.blockedWeek > 0 || stats.focusMinutesToday > 0) && (
        <footer className="newtab-foot mono">
          <LookMark className="accent" />
          {stats.blockedWeek > 0 && <span>{t('shield.week', { n: stats.blockedWeek })}</span>}
          {stats.blockedWeek > 0 && stats.focusMinutesToday > 0 && <span aria-hidden>·</span>}
          {stats.focusMinutesToday > 0 && <span>{t('focus.today', { n: stats.focusMinutesToday })}</span>}
        </footer>
      )}
    </div>
  )
}
