import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { Ellipsis, Search, X } from 'lucide-react'
import type { HistoryEntry } from '@shared/types'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
import { useUI } from '../context'
import { useSettings, useT } from '../store'
import { Button, Favicon, Icon, IconButton } from '../ui'

const PAGE = 150

function dayLabel(time: number, lang: string, t: ReturnType<typeof useT>): string {
  const date = new Date(time)
  const today = new Date()
  const yesterday = new Date()
  yesterday.setDate(today.getDate() - 1)
  const same = (a: Date, b: Date) => a.toDateString() === b.toDateString()
  const formatted = new Intl.DateTimeFormat(lang === 'tr' ? 'tr-TR' : 'en-US', { weekday: 'long', day: 'numeric', month: 'long' }).format(date)
  if (same(date, today)) return `${t('history.today')} · ${formatted}`
  if (same(date, yesterday)) return `${t('history.yesterday')} · ${formatted}`
  return formatted
}

export function HistoryPage() {
  const t = useT()
  const { language } = useSettings()
  const { openPanel } = useUI()
  const [text, setText] = useState('')
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null)
  const [more, setMore] = useState(false)
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const lastClicked = useRef<number | null>(null)
  const request = useRef(0)

  const load = (query: string, before?: number) => {
    const id = ++request.current
    void call.historyQuery({ text: query, before, limit: PAGE }).then((rows) => {
      if (id !== request.current) return
      setEntries((current) => (before && current ? [...current, ...rows] : rows))
      setMore(rows.length === PAGE)
    })
  }

  useEffect(() => {
    const timer = setTimeout(() => load(text), 120)
    return () => clearTimeout(timer)
  }, [text])

  // "More from this site" from a row's menu, and deletions made elsewhere.
  useEffect(() => {
    const onSearch = (event: Event) => setText((event as CustomEvent<string>).detail)
    const onChanged = () => {
      setSelected(new Set())
      load(text)
    }
    window.addEventListener('pw-history-search', onSearch)
    window.addEventListener('pw-history-changed', onChanged)
    return () => {
      window.removeEventListener('pw-history-search', onSearch)
      window.removeEventListener('pw-history-changed', onChanged)
    }
  }, [text])

  const groups = useMemo(() => {
    const out: { label: string; items: HistoryEntry[] }[] = []
    for (const entry of entries ?? []) {
      const label = dayLabel(entry.visitedAt, language, t)
      const last = out.at(-1)
      if (last && last.label === label) last.items.push(entry)
      else out.push({ label, items: [entry] })
    }
    return out
  }, [entries, language, t])

  const time = new Intl.DateTimeFormat(language === 'tr' ? 'tr-TR' : 'en-US', { hour: '2-digit', minute: '2-digit' })

  /** Shift-click selects everything between the last click and this one. */
  const toggleSelected = (entry: HistoryEntry, event: MouseEvent) => {
    const list = entries ?? []
    const next = new Set(selected)
    const on = !next.has(entry.id)
    if (event.shiftKey && lastClicked.current !== null) {
      const a = list.findIndex((e) => e.id === lastClicked.current)
      const b = list.findIndex((e) => e.id === entry.id)
      if (a >= 0 && b >= 0) for (const e of list.slice(Math.min(a, b), Math.max(a, b) + 1)) on ? next.add(e.id) : next.delete(e.id)
    } else if (on) next.add(entry.id)
    else next.delete(entry.id)
    lastClicked.current = entry.id
    setSelected(next)
  }

  const removeSelected = () => {
    const ids = [...selected]
    api.historyRemove(ids)
    setEntries((current) => current?.filter((e) => !selected.has(e.id)) ?? null)
    setSelected(new Set())
  }

  const menu = (entry: HistoryEntry, x: number, y: number) => api.historyMenu(entry.id, entry.url, x, y)

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('history.title')}</h1>
          <Button variant="quiet" onClick={() => openPanel({ kind: 'clear-data' })}>
            {t('history.clearData')}
          </Button>
        </header>
        <label className="search-field">
          <Icon icon={Search} size={16} />
          <input value={text} placeholder={t('history.search')} spellCheck={false} onChange={(event) => setText(event.target.value)} autoFocus />
          {text && <IconButton icon={X} label={t('common.close')} onClick={() => setText('')} />}
        </label>

        {selected.size > 0 && (
          <div className="selection-bar">
            <span className="mono">{t('history.selected', { n: selected.size })}</span>
            <span className="spacer" />
            <Button variant="quiet" onClick={() => setSelected(new Set())}>
              {t('history.unselect')}
            </Button>
            <Button variant="primary" onClick={removeSelected}>
              {t('history.removeSelected')}
            </Button>
          </div>
        )}

        {entries && entries.length === 0 && <p className="muted list-empty">{text ? t('history.noResults') : t('history.empty')}</p>}

        {groups.map((group) => (
          <section key={group.label} className="history-day">
            <h2 className="label">{group.label}</h2>
            <ul>
              {group.items.map((entry) => (
                <li
                  key={entry.id}
                  className={`history-row ${selected.has(entry.id) ? 'is-selected' : ''} ${selected.size > 0 ? 'is-selecting' : ''}`}
                  onContextMenu={(event) => {
                    event.preventDefault()
                    menu(entry, event.clientX, event.clientY)
                  }}
                >
                  <input
                    type="checkbox"
                    className="history-check"
                    aria-label={t('history.select')}
                    checked={selected.has(entry.id)}
                    onChange={() => {}}
                    onClick={(event) => toggleSelected(entry, event)}
                  />
                  <span className="mono muted history-time">{time.format(entry.visitedAt)}</span>
                  <a
                    href={entry.url}
                    className="history-link"
                    onClick={(event) => {
                      event.preventDefault()
                      api.openURL(entry.url, event.ctrlKey || event.metaKey)
                    }}
                    onAuxClick={(event) => {
                      if (event.button === 1) {
                        event.preventDefault()
                        api.openURL(entry.url, true)
                      }
                    }}
                  >
                    <Favicon src={entry.favicon} url={entry.url} size={16} />
                    <span className="history-title">{entry.title || entry.url}</span>
                    <span className="mono muted history-host">{displayHost(entry.url)}</span>
                  </a>
                  <IconButton
                    icon={Ellipsis}
                    label={t('history.menu')}
                    className="history-remove"
                    onClick={(event) => {
                      const rect = event.currentTarget.getBoundingClientRect()
                      menu(entry, rect.left, rect.bottom + 4)
                    }}
                  />
                </li>
              ))}
            </ul>
          </section>
        ))}

        {more && entries && (
          <div className="list-more">
            <Button onClick={() => load(text, entries.at(-1)?.visitedAt)}>{t('history.more')}</Button>
          </div>
        )}
      </div>
    </div>
  )
}
