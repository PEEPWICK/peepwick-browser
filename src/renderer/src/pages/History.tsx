import { useEffect, useMemo, useRef, useState } from 'react'
import { Search, X } from 'lucide-react'
import type { HistoryEntry } from '@shared/types'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
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
  const [text, setText] = useState('')
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null)
  const [more, setMore] = useState(false)
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

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('history.title')}</h1>
          <Button
            variant="quiet"
            disabled={!entries || entries.length === 0}
            onClick={() => {
              if (window.confirm(t('history.clearConfirm'))) {
                api.historyClear()
                setTimeout(() => load(text), 100)
              }
            }}
          >
            {t('history.clear')}
          </Button>
        </header>
        <label className="search-field">
          <Icon icon={Search} size={16} />
          <input value={text} placeholder={t('history.search')} spellCheck={false} onChange={(event) => setText(event.target.value)} autoFocus />
        </label>

        {entries && entries.length === 0 && <p className="muted list-empty">{text ? t('history.noResults') : t('history.empty')}</p>}

        {groups.map((group) => (
          <section key={group.label} className="history-day">
            <h2 className="label">{group.label}</h2>
            <ul>
              {group.items.map((entry) => (
                <li key={entry.id} className="history-row">
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
                    icon={X}
                    label={t('history.remove')}
                    className="history-remove"
                    onClick={() => {
                      api.historyRemove(entry.id)
                      setEntries((current) => current?.filter((e) => e.id !== entry.id) ?? null)
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
