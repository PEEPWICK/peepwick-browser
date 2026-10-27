// The downloads page (peepwick://downloads, Ctrl+J): every download with its
// progress, speed and time left, grouped by day, searchable.
import { useEffect, useMemo, useRef, useState } from 'react'
import {
  Ellipsis,
  File,
  FileArchive,
  FileAudio,
  FileImage,
  FileText,
  FileVideo,
  FolderOpen,
  Pause,
  Play,
  RotateCw,
  Search,
  X,
  type LucideIcon,
} from 'lucide-react'
import type { DownloadState, Lang } from '@shared/types'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
import { useUI } from '../context'
import { downloadStatus } from '../downloads'
import { useSnapshot, useT } from '../store'
import { Button, Icon, IconButton } from '../ui'

const KINDS: [RegExp, LucideIcon][] = [
  [/\.(zip|rar|7z|tar|gz|bz2|xz|zst|deb|rpm|appimage|iso)$/i, FileArchive],
  [/\.(png|jpe?g|gif|webp|avif|svg|bmp|heic|ico)$/i, FileImage],
  [/\.(mp4|mkv|webm|mov|avi|m4v)$/i, FileVideo],
  [/\.(mp3|flac|ogg|opus|wav|m4a|aac)$/i, FileAudio],
  [/\.(pdf|txt|md|docx?|odt|rtf|csv|xlsx?|ods|pptx?|odp|epub)$/i, FileText],
]

const iconFor = (name: string): LucideIcon => KINDS.find(([pattern]) => pattern.test(name))?.[1] ?? File

function dayLabel(time: number, lang: Lang, t: ReturnType<typeof useT>): string {
  const date = new Date(time)
  const today = new Date()
  const yesterday = new Date()
  yesterday.setDate(today.getDate() - 1)
  const formatted = new Intl.DateTimeFormat(lang === 'tr' ? 'tr-TR' : 'en-US', { weekday: 'long', day: 'numeric', month: 'long' }).format(date)
  if (date.toDateString() === today.toDateString()) return `${t('history.today')} · ${formatted}`
  if (date.toDateString() === yesterday.toDateString()) return `${t('history.yesterday')} · ${formatted}`
  return formatted
}

function DownloadItemRow({ item }: { item: DownloadState }) {
  const { settings } = useSnapshot()
  const t = useT()
  const lang = settings.language
  const active = item.status === 'progressing' || item.status === 'paused'
  const openable = item.status === 'completed' && !item.missing
  const failed = item.status === 'interrupted' || item.status === 'cancelled'
  const progress = item.total > 0 ? Math.min(1, item.received / item.total) : 0
  const menu = (x: number, y: number) => api.downloadMenu(item.id, x, y)

  return (
    <li
      className={`dl-row ${failed || item.missing ? 'is-dim' : ''}`}
      onContextMenu={(event) => {
        event.preventDefault()
        menu(event.clientX, event.clientY)
      }}
    >
      <span className="dl-icon">
        <Icon icon={iconFor(item.filename)} size={22} />
      </span>
      <div className="dl-text">
        {openable ? (
          <button type="button" className="dl-name" onClick={() => api.download(item.id, 'open')}>
            {item.filename}
          </button>
        ) : (
          <span className={`dl-name ${item.status === 'cancelled' || item.missing ? 'is-struck' : ''}`}>{item.filename}</span>
        )}
        <span className="dl-meta mono">
          {downloadStatus(item, lang, t)}
          {item.url && !/^(data|blob):/.test(item.url) && <span className="dl-host"> · {displayHost(item.url)}</span>}
          {item.incognito && <span className="label dl-private">{t('downloads.private')}</span>}
        </span>
        {active && (
          <span className={`download-bar ${item.status === 'paused' ? 'is-paused' : ''} ${item.total > 0 ? '' : 'is-unknown'}`}>
            <span style={{ width: `${Math.round(progress * 100)}%` }} />
          </span>
        )}
      </div>
      <span className="dl-actions">
        {item.status === 'progressing' && <IconButton icon={Pause} label={t('downloads.pause')} onClick={() => api.download(item.id, 'pause')} />}
        {item.status === 'paused' && <IconButton icon={Play} label={t('downloads.resume')} onClick={() => api.download(item.id, 'resume')} />}
        {active && <IconButton icon={X} label={t('downloads.cancel')} onClick={() => api.download(item.id, 'cancel')} />}
        {failed && (
          <Button onClick={() => api.download(item.id, 'retry')}>
            <Icon icon={RotateCw} size={14} /> {t('downloads.retry')}
          </Button>
        )}
        {openable && <IconButton icon={FolderOpen} label={t('downloads.show')} onClick={() => api.download(item.id, 'show')} />}
        <IconButton
          icon={Ellipsis}
          label={t('downloads.more')}
          onClick={(event) => {
            const rect = event.currentTarget.getBoundingClientRect()
            menu(rect.left, rect.bottom + 4)
          }}
        />
        {!active && <IconButton icon={X} label={t('downloads.remove')} className="dl-remove" onClick={() => api.download(item.id, 'remove')} />}
      </span>
    </li>
  )
}

export function DownloadsPage() {
  const { global, settings } = useSnapshot()
  const { ask } = useUI()
  const t = useT()
  const [text, setText] = useState('')
  const [items, setItems] = useState<DownloadState[] | null>(null)
  const request = useRef(0)

  // The window's state carries recent downloads; any change there refreshes the full list.
  useEffect(() => {
    const id = ++request.current
    const timer = setTimeout(() => void call.downloadsList(text).then((list) => id === request.current && setItems(list)), 60)
    return () => clearTimeout(timer)
  }, [text, global.downloads])

  const groups = useMemo(() => {
    const out: { label: string; items: DownloadState[] }[] = []
    for (const item of items ?? []) {
      const label = dayLabel(item.startedAt, settings.language, t)
      const last = out.at(-1)
      if (last?.label === label) last.items.push(item)
      else out.push({ label, items: [item] })
    }
    return out
  }, [items, settings.language, t])

  const clearable = (items ?? []).some((d) => d.status !== 'progressing' && d.status !== 'paused')

  return (
    <div className="page list-page">
      <div className="list-page-inner">
        <header className="list-head">
          <h1 className="display small">{t('downloads.title')}</h1>
          <div className="list-head-actions">
            <Button variant="quiet" onClick={() => api.downloadsOpenFolder()}>
              <Icon icon={FolderOpen} size={15} /> {t('downloads.openFolder')}
            </Button>
            <Button
              variant="quiet"
              disabled={!clearable}
              onClick={() =>
                void ask({ title: t('downloads.clearConfirm'), body: t('downloads.clearBody'), confirm: t('downloads.clearAll'), cancel: t('common.cancel') }).then(
                  (ok) => ok && api.downloadsClear(),
                )
              }
            >
              {t('downloads.clearAll')}
            </Button>
          </div>
        </header>
        <label className="search-field">
          <Icon icon={Search} size={16} />
          <input value={text} placeholder={t('downloads.search')} spellCheck={false} onChange={(event) => setText(event.target.value)} autoFocus />
        </label>

        {items && items.length === 0 && <p className="muted list-empty">{text ? t('downloads.noResults') : t('downloads.empty')}</p>}

        {groups.map((group) => (
          <section key={group.label} className="history-day">
            <h2 className="label">{group.label}</h2>
            <ul className="dl-list">
              {group.items.map((item) => (
                <DownloadItemRow key={item.id} item={item} />
              ))}
            </ul>
          </section>
        ))}
      </div>
    </div>
  )
}
