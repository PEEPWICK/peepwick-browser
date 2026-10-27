// The reading list (peepwick://reading): pages saved for later, unread first. Items
// with a saved article open their offline copy; the others open the page itself.
import { useEffect, useMemo, useState } from 'react'
import { BookOpenCheck, Check, ExternalLink, RotateCcw, Trash2, WifiOff } from 'lucide-react'
import type { ReadingItem } from '@shared/types'
import { api, call } from '../api'
import { useUI } from '../context'
import { useSnapshot, useT } from '../store'
import { Favicon, Icon, IconButton, faviconFor } from '../ui'

type Filter = 'unread' | 'read' | 'all'

export function ReadingPage() {
  const { settings } = useSnapshot()
  const { showToast } = useUI()
  const t = useT()
  const [items, setItems] = useState<ReadingItem[] | null>(null)
  const [filter, setFilter] = useState<Filter>('unread')

  useEffect(() => {
    const load = () => void call.readingList().then(setItems)
    load()
    window.addEventListener('pw-reading-changed', load)
    return () => window.removeEventListener('pw-reading-changed', load)
  }, [])

  const unread = items?.filter((i) => i.readAt === null).length ?? 0
  const read = (items?.length ?? 0) - unread
  const shown = useMemo(
    () => (items ?? []).filter((i) => filter === 'all' || (filter === 'unread' ? i.readAt === null : i.readAt !== null)),
    [items, filter],
  )
  const date = new Intl.DateTimeFormat(settings.language === 'tr' ? 'tr-TR' : 'en-GB', { day: 'numeric', month: 'short' })

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('reading.title')}</h1>
        </header>
        <p className="setting-hint reading-intro">{t('reading.intro')}</p>

        {items && items.length > 0 && (
          <div className="segmented reading-filter" role="radiogroup">
            {(['unread', 'read', 'all'] as Filter[]).map((value) => (
              <button key={value} type="button" role="radio" aria-checked={filter === value} className={filter === value ? 'is-active' : ''} onClick={() => setFilter(value)}>
                {t(`reading.${value}`)} {value === 'unread' ? unread : value === 'read' ? read : items.length}
              </button>
            ))}
          </div>
        )}

        {items && items.length === 0 && <p className="muted list-empty">{t('reading.empty')}</p>}
        {items && items.length > 0 && shown.length === 0 && (
          <p className="muted list-empty">{filter === 'unread' ? t('reading.allRead') : t('reading.noneRead')}</p>
        )}

        {shown.length > 0 && (
          <ul className="reading-list">
            {shown.map((item) => (
              <li key={item.id} className={item.readAt !== null ? 'is-read' : ''}>
                <Favicon src={faviconFor(item.url)} url={item.url} size={18} />
                <button
                  type="button"
                  className="reading-main"
                  title={item.url}
                  onClick={(event) => api.readingOpen(item.id, event.ctrlKey || event.metaKey)}
                  onAuxClick={(event) => event.button === 1 && api.readingOpen(item.id, true)}
                >
                  <span className="reading-title">{item.title}</span>
                  <span className="reading-meta muted">
                    {item.site}
                    {item.words > 0 && <> · {t('reader.minutes', { n: Math.max(1, Math.round(item.words / 200)) })}</>}
                    {' · '}
                    {date.format(item.addedAt)}
                    {item.saved && (
                      <span className="reading-offline" title={t('reading.offline')}>
                        <Icon icon={WifiOff} size={12} /> {t('reading.offlineShort')}
                      </span>
                    )}
                  </span>
                </button>
                <span className="login-actions">
                  {item.saved && <IconButton icon={ExternalLink} label={t('reading.original')} onClick={() => api.openURL(item.url, true)} />}
                  <IconButton
                    icon={item.readAt === null ? Check : RotateCcw}
                    label={item.readAt === null ? t('reading.markRead') : t('reading.markUnread')}
                    onClick={() => api.readingMark(item.id, item.readAt === null)}
                  />
                  <IconButton
                    icon={Trash2}
                    label={t('reading.remove')}
                    onClick={() => {
                      api.readingRemove(item.id)
                      showToast({ text: t('reading.removed') })
                    }}
                  />
                </span>
              </li>
            ))}
          </ul>
        )}
        {items && items.length > 0 && filter === 'unread' && unread === 0 && <Icon icon={BookOpenCheck} size={28} className="muted reading-done" />}
      </div>
    </div>
  )
}
