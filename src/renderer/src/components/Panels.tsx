import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { FileDown, FolderOpen, Pause, Play, Trash2, X } from 'lucide-react'
import { formatBytes, formatNumber } from '@shared/i18n'
import { WORKSPACE_ICONS, type DownloadState, type Stats, type WorkspaceIcon } from '@shared/types'
import { displayHost } from '@shared/url'
import { api, call } from '../api'
import { LookMark } from '../brand'
import { useUI } from '../context'
import { activeTab, useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, Segmented, Toggle, WORKSPACE_GLYPHS, remaining, useNow, useOverlay } from '../ui'

function Popover({ anchor, width = 320, label, children }: { anchor: DOMRect; width?: number; label: string; children: ReactNode }) {
  useOverlay(true)
  const { closePanel } = useUI()
  const ref = useRef<HTMLDivElement>(null)
  const [style, setStyle] = useState<React.CSSProperties>({ visibility: 'hidden', width })

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const height = el.offsetHeight
    const margin = 8
    const below = anchor.bottom + margin + height <= window.innerHeight - margin
    const top = below ? anchor.bottom + margin : Math.max(margin, anchor.top - margin - height)
    const left = Math.min(Math.max(margin, anchor.left), window.innerWidth - width - margin)
    setStyle({ top, left, width, visibility: 'visible' })
  }, [anchor, width])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => event.key === 'Escape' && closePanel()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [closePanel])

  return (
    <div className="scrim is-clear" onMouseDown={closePanel}>
      <div className="popover" ref={ref} style={style} role="dialog" aria-label={label} onMouseDown={(event) => event.stopPropagation()}>
        {children}
      </div>
    </div>
  )
}

function useStats(): Stats | null {
  const [stats, setStats] = useState<Stats | null>(null)
  useEffect(() => {
    void call.stats().then(setStats)
  }, [])
  return stats
}

export function ShieldPanel({ anchor }: { anchor: DOMRect }) {
  const { state, settings } = useSnapshot()
  const t = useT()
  const tab = activeTab(state)
  const stats = useStats()
  const [report, setReport] = useState<{ host: string; count: number }[]>([])
  useEffect(() => {
    void call.shieldReport().then(setReport)
  }, [tab?.blocked])

  const web = !!tab && !tab.page && /^https?:/.test(tab.url)
  const off = settings.shieldLevel === 'off'
  return (
    <Popover anchor={anchor} label={t('shield.title')}>
      <div className="panel-head">
        <span className="label">{t('shield.title')}</span>
        {web && <span className="mono muted panel-host">{displayHost(tab.url)}</span>}
      </div>
      {!web ? (
        <p className="panel-text">{t('shield.internal')}</p>
      ) : off ? (
        <>
          <p className="panel-text">{t('shield.offGlobal')}</p>
          <Button onClick={() => api.openPage('settings', 'privacy')}>{t('settings.title')}</Button>
        </>
      ) : (
        <>
          <div className="shield-count">
            <span className="big-number">{formatNumber(settings.language, state.shieldPaused ? 0 : tab.blocked)}</span>
            <span className="muted">{t('shield.blockedHere')}</span>
          </div>
          {!state.shieldPaused && report.length > 0 && (
            <ul className="shield-hosts">
              {report.map((row) => (
                <li key={row.host}>
                  <span className="mono">{row.host}</span>
                  <span className="mono muted">{formatNumber(settings.language, row.count)}</span>
                </li>
              ))}
            </ul>
          )}
          {!state.shieldPaused && report.length === 0 && <p className="panel-text muted">{t('shield.none')}</p>}
          <div className="panel-row">
            <span>{state.shieldPaused ? t('shield.paused') : t('shield.on')}</span>
            <Toggle checked={!state.shieldPaused} label={t('shield.title')} onChange={() => api.shieldToggleSite()} />
          </div>
          <p className="panel-hint muted">{t('shield.reloadHint')}</p>
        </>
      )}
      {stats && (
        <p className="panel-foot">
          <LookMark className="accent" />
          <span>{t('shield.week', { n: stats.blockedWeek })}</span>
        </p>
      )}
    </Popover>
  )
}

export function FocusPanel({ anchor }: { anchor: DOMRect }) {
  const { global, settings } = useSnapshot()
  const t = useT()
  const { closePanel } = useUI()
  const now = useNow(1000, global.focus.active)
  const stats = useStats()
  const durations = [...new Set([25, 50, 90, settings.focusMinutes])].sort((a, b) => a - b)
  const [minutes, setMinutes] = useState(settings.focusMinutes)

  return (
    <Popover anchor={anchor} label={t('focus.title')} width={300}>
      <div className="panel-head">
        <span className="label">{t('focus.title')}</span>
        {global.focus.active && <span className="dot" aria-hidden />}
      </div>
      {global.focus.active ? (
        <>
          <div className="focus-clock mono">{remaining(global.focus.endsAt - now)}</div>
          <p className="panel-text muted">{t('focus.sites', { n: settings.focusSites.length })}</p>
          <div className="panel-actions">
            <Button variant="quiet" onClick={() => api.openPage('settings', 'focus')}>
              {t('focus.editSites')}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                api.focusStop()
                closePanel()
              }}
            >
              {t('focus.stop')}
            </Button>
          </div>
        </>
      ) : (
        <>
          <p className="panel-text">{t('focus.lead')}</p>
          <Segmented
            label={t('settings.focusMinutes')}
            value={minutes}
            options={durations.map((n) => ({ value: n, label: <span className="mono">{t('focus.minutes', { n })}</span> }))}
            onChange={setMinutes}
          />
          <div className="panel-actions">
            <Button variant="quiet" onClick={() => api.openPage('settings', 'focus')}>
              {t('focus.editSites')}
            </Button>
            <Button
              variant="accent"
              onClick={() => {
                api.focusStart(minutes)
                closePanel()
              }}
            >
              {t('focus.start')}
            </Button>
          </div>
        </>
      )}
      {stats && stats.focusMinutesToday > 0 && (
        <p className="panel-foot">
          <LookMark className="accent" />
          <span>{t('focus.today', { n: stats.focusMinutesToday })}</span>
        </p>
      )}
    </Popover>
  )
}

function DownloadRow({ item }: { item: DownloadState }) {
  const { settings } = useSnapshot()
  const t = useT()
  const lang = settings.language
  const progress = item.total > 0 ? item.received / item.total : 0
  let status = ''
  if (item.status === 'progressing') {
    status = item.total > 0 ? `${formatBytes(lang, item.received)} / ${formatBytes(lang, item.total)}` : formatBytes(lang, item.received)
  } else if (item.status === 'paused') status = t('downloads.paused')
  else if (item.status === 'cancelled') status = t('downloads.cancelled')
  else if (item.status === 'interrupted') status = t('downloads.failed')
  else status = formatBytes(lang, item.total || item.received)

  const done = item.status === 'completed'
  return (
    <li className={`download ${done ? 'is-done' : ''}`}>
      <button type="button" className="download-main" disabled={!done} onClick={() => api.download(item.id, 'open')} title={item.filename}>
        <Icon icon={FileDown} size={18} />
        <span className="download-text">
          <span className="download-name">{item.filename}</span>
          <span className="download-status mono">{status}</span>
          {(item.status === 'progressing' || item.status === 'paused') && (
            <span className="download-bar">
              <span style={{ width: `${Math.round(progress * 100)}%` }} />
            </span>
          )}
        </span>
      </button>
      <span className="download-actions">
        {item.status === 'progressing' && <IconButton icon={Pause} label={t('downloads.pause')} onClick={() => api.download(item.id, 'pause')} />}
        {item.status === 'paused' && <IconButton icon={Play} label={t('downloads.resume')} onClick={() => api.download(item.id, 'resume')} />}
        {(item.status === 'progressing' || item.status === 'paused') && (
          <IconButton icon={X} label={t('downloads.cancel')} onClick={() => api.download(item.id, 'cancel')} />
        )}
        {done && <IconButton icon={FolderOpen} label={t('downloads.show')} onClick={() => api.download(item.id, 'show')} />}
        {!(item.status === 'progressing' || item.status === 'paused') && (
          <IconButton icon={Trash2} label={t('downloads.remove')} onClick={() => api.download(item.id, 'remove')} />
        )}
      </span>
    </li>
  )
}

export function DownloadsPanel({ anchor }: { anchor: DOMRect }) {
  const { global } = useSnapshot()
  const t = useT()
  const list = global.downloads.slice(0, 30)
  return (
    <Popover anchor={anchor} label={t('downloads.title')} width={360}>
      <div className="panel-head">
        <span className="label">{t('downloads.title')}</span>
        {list.some((d) => d.status !== 'progressing' && d.status !== 'paused') && (
          <button type="button" className="link-button" onClick={() => api.downloadsClear()}>
            {t('downloads.clear')}
          </button>
        )}
      </div>
      {list.length === 0 ? (
        <p className="panel-text muted">{t('downloads.empty')}</p>
      ) : (
        <ul className="downloads">
          {list.map((item) => (
            <DownloadRow key={item.id} item={item} />
          ))}
        </ul>
      )}
    </Popover>
  )
}

export function WorkspacePanel({ id, anchor }: { id: string | null; anchor: DOMRect }) {
  const { state } = useSnapshot()
  const t = useT()
  const { closePanel } = useUI()
  const existing = id ? state.workspaces.find((w) => w.id === id) : undefined
  const [name, setName] = useState(existing?.name ?? '')
  const [icon, setIcon] = useState<WorkspaceIcon>(existing?.icon ?? 'circle')
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])

  const save = () => {
    const clean = name.trim() || t('ws.new')
    if (existing) api.updateWorkspace(existing.id, { name: clean, icon })
    else api.createWorkspace(clean, icon)
    closePanel()
  }

  return (
    <Popover anchor={anchor} label={existing ? t('ws.rename') : t('ws.new')} width={280}>
      <div className="panel-head">
        <span className="label">{existing ? t('ws.rename') : t('ws.new')}</span>
      </div>
      <label className="field">
        <span className="label">{t('ws.name')}</span>
        <input
          ref={input}
          className="text-input"
          value={name}
          maxLength={40}
          placeholder={t('ws.new')}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && save()}
        />
      </label>
      <div className="field">
        <span className="label">{t('ws.icon')}</span>
        <div className="icon-grid" role="radiogroup" aria-label={t('ws.icon')}>
          {WORKSPACE_ICONS.map((glyph) => (
            <button
              key={glyph}
              type="button"
              role="radio"
              aria-checked={icon === glyph}
              className={icon === glyph ? 'is-active' : ''}
              onClick={() => setIcon(glyph)}
            >
              <Icon icon={WORKSPACE_GLYPHS[glyph]} size={16} />
            </button>
          ))}
        </div>
      </div>
      <div className="panel-actions">
        {existing && state.workspaces.length > 1 && (
          <Button
            variant="quiet"
            onClick={() => {
              const count = existing.tabIds.length
              if (count === 0 || window.confirm(t('ws.deleteConfirm', { name: existing.name, n: count }))) {
                api.deleteWorkspace(existing.id)
                closePanel()
              }
            }}
          >
            {t('ws.delete')}
          </Button>
        )}
        <span className="spacer" />
        <Button variant="quiet" onClick={closePanel}>
          {t('ws.cancel')}
        </Button>
        <Button variant="primary" onClick={save}>
          {t('ws.save')}
        </Button>
      </div>
    </Popover>
  )
}
