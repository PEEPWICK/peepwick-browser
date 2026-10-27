import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { BookMarked, Copy, Download, ExternalLink, Upload, X } from 'lucide-react'
import { walk } from '@shared/bookmarks'
import type { MessageKey } from '@shared/i18n'
import { SEARCH_ENGINES } from '@shared/search'
import { normalizeDomain, SECURE_DNS_SERVERS } from '@shared/settings'
import {
  SETTINGS_SECTIONS,
  type AppInfo,
  type BookmarksBarMode,
  type ClearKind,
  type PermissionKind,
  type Report,
  type SecureDns,
  type SearchEngineId,
  type SettingsSection,
  type ShieldLevel,
  type SitePermission,
  type ThemePref,
} from '@shared/types'
import { api, call } from '../api'
import { DoubleEye, LookMark } from '../brand'
import { useUI } from '../context'
import { useBookmarks, useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, Segmented, Toggle } from '../ui'


function Row({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-text">
        <div className="setting-title">{title}</div>
        {hint && <div className="setting-hint">{hint}</div>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  )
}

function DomainList({ items, onChange, empty, placeholder }: { items: string[]; onChange: (items: string[]) => void; empty: string; placeholder: string }) {
  const t = useT()
  const [value, setValue] = useState('')
  const add = () => {
    const domain = normalizeDomain(value)
    if (!domain) return
    if (!items.includes(domain)) onChange([...items, domain])
    setValue('')
  }
  return (
    <div className="domain-list">
      <div className="domain-add">
        <input
          className="text-input"
          value={value}
          placeholder={placeholder}
          spellCheck={false}
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && add()}
        />
        <Button onClick={add} disabled={!normalizeDomain(value)}>
          {t('settings.add')}
        </Button>
      </div>
      {items.length === 0 ? (
        <p className="setting-hint">{empty}</p>
      ) : (
        <ul>
          {items.map((domain) => (
            <li key={domain}>
              <span className="mono">{domain}</span>
              <IconButton icon={X} label={t('settings.remove')} onClick={() => onChange(items.filter((d) => d !== domain))} />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/** Settings › Privacy: what to clear every time the browser closes. */
function ClearOnExit() {
  const { settings: s } = useSnapshot()
  const t = useT()
  const kinds: ClearKind[] = ['history', 'downloads', 'cookies', 'cache']
  return (
    <div className="clear-kinds">
      {kinds.map((kind) => (
        <label key={kind} className="check">
          <input
            type="checkbox"
            checked={s.clearOnExit.includes(kind)}
            onChange={(event) =>
              api.setSettings({ clearOnExit: event.target.checked ? [...s.clearOnExit, kind] : s.clearOnExit.filter((k) => k !== kind) })
            }
          />
          <span>{t(`clear.${kind}` as MessageKey)}</span>
        </label>
      ))}
    </div>
  )
}

function SecureDnsRow() {
  const { settings: s } = useSnapshot()
  const t = useT()
  const providers = Object.entries(SECURE_DNS_SERVERS) as [Exclude<SecureDns, 'off' | 'automatic'>, { name: string }][]
  const strict = s.secureDns !== 'off' && s.secureDns !== 'automatic'
  return (
    <Row title={t('settings.secureDns')} hint={strict ? `${t('settings.secureDnsHint')} ${t('settings.secureDnsStrict')}` : t('settings.secureDnsHint')}>
      <select className="select" value={s.secureDns} onChange={(event) => api.setSettings({ secureDns: event.target.value as SecureDns })}>
        <option value="automatic">{t('settings.secureDns.automatic')}</option>
        {providers.map(([id, provider]) => (
          <option key={id} value={id}>
            {provider.name}
          </option>
        ))}
        <option value="off">{t('settings.secureDns.off')}</option>
      </select>
    </Row>
  )
}

function DownloadFolderRow() {
  const { settings: s } = useSnapshot()
  const t = useT()
  return (
    <Row title={t('settings.downloadFolder')} hint={s.downloadDir || t('settings.downloadFolderDefault')}>
      <div className="setting-buttons">
        {s.downloadDir && (
          <Button variant="quiet" onClick={() => api.setSettings({ downloadDir: '' })}>
            {t('settings.useDefault')}
          </Button>
        )}
        <Button onClick={() => void call.chooseDownloadFolder()}>{t('settings.change')}</Button>
      </div>
    </Row>
  )
}

function SystemRows() {
  const { settings: s } = useSnapshot()
  const { ask, showToast } = useUI()
  const t = useT()
  const [info, setInfo] = useState<AppInfo | null>(null)
  useEffect(() => {
    void call.appInfo().then(setInfo)
  }, [])
  const pending = !!info && info.hardwareAcceleration !== s.hardwareAcceleration
  return (
    <>
      <Row title={t('settings.hardware')} hint={t('settings.hardwareHint')}>
        <div className="setting-buttons">
          {pending && <Button onClick={() => api.restart()}>{t('settings.restart')}</Button>}
          <Toggle checked={s.hardwareAcceleration} label={t('settings.hardware')} onChange={(hardwareAcceleration) => api.setSettings({ hardwareAcceleration })} />
        </div>
      </Row>
      <Row title={t('settings.reset')} hint={t('settings.resetHint')}>
        <Button
          variant="quiet"
          onClick={() =>
            void ask({ title: t('settings.resetConfirm'), body: t('settings.resetHint'), confirm: t('settings.resetRun'), cancel: t('common.cancel'), danger: true }).then(
              (ok) => {
                if (!ok) return
                void call.resetSettings().then(() => showToast({ text: t('settings.resetDone') }))
              },
            )
          }
        >
          {t('settings.reset')}
        </Button>
      </Row>
    </>
  )
}

function BackupSection() {
  const { openPanel, showToast } = useUI()
  const t = useT()
  return (
    <>
      <p className="setting-hint section-lead">{t('backup.lead')}</p>
      <Row title={t('backup.export')} hint={t('backup.exportHint')}>
        <Button onClick={() => openPanel({ kind: 'backup-export' })}>
          <Icon icon={Upload} size={15} /> {t('backup.create')}
        </Button>
      </Row>
      <Row title={t('backup.import')} hint={t('backup.importHint')}>
        <Button
          onClick={() =>
            void call.backupOpen().then((file) => {
              if (!file) return
              if ('error' in file) showToast({ text: t('backup.notBackup') })
              else openPanel({ kind: 'backup-restore', token: file.token, name: file.name })
            })
          }
        >
          <Icon icon={Download} size={15} /> {t('backup.choose')}
        </Button>
      </Row>
    </>
  )
}

function ReportSection() {
  const { settings: s } = useSnapshot()
  const { showToast } = useUI()
  const t = useT()
  const [report, setReport] = useState<Report | null>(null)
  const load = () => void call.report().then(setReport)
  useEffect(load, [s.errorLog])
  if (!report) return null
  const loads = report.loadErrors.reduce((n, l) => n + l.count, 0)
  return (
    <>
      <p className="setting-hint section-lead">{t('report.lead')}</p>
      <dl className="report-facts">
        <dt className="label">{t('report.version')}</dt>
        <dd className="mono">{report.version}</dd>
        <dt className="label">{t('report.system')}</dt>
        <dd className="mono">{report.system}</dd>
        <dt className="label">{t('report.errors')}</dt>
        <dd>{report.errors.length === 0 ? <span className="muted">{t('report.errorsNone')}</span> : t('report.errorsCount', { n: report.errors.length })}</dd>
        {loads > 0 && (
          <>
            <dt className="label">{t('report.loadErrors')}</dt>
            <dd className="mono">
              {report.loadErrors
                .slice(0, 4)
                .map((l) => `${l.name} ×${l.count}`)
                .join(' · ')}
            </dd>
          </>
        )}
      </dl>
      <div className="setting-buttons report-actions">
        <Button variant="primary" onClick={() => api.reportIssue()}>
          <Icon icon={ExternalLink} size={15} /> {t('report.issue')}
        </Button>
        <Button
          onClick={() => {
            api.reportCopy()
            showToast({ text: t('report.copied') })
          }}
        >
          <Icon icon={Copy} size={15} /> {t('report.copy')}
        </Button>
      </div>
      <p className="setting-hint">{t('report.issueHint')}</p>
      <details className="report-details" onToggle={(event) => (event.currentTarget as HTMLDetailsElement).open && load()}>
        <summary className="label">{t('report.show')}</summary>
        <pre className="report-text mono">{report.text}</pre>
      </details>
      <Row title={t('report.log')} hint={t('report.logHint')}>
        <Toggle checked={s.errorLog} label={t('report.log')} onChange={(errorLog) => api.setSettings({ errorLog })} />
      </Row>
      <Row title={t('report.clear')}>
        <Button
          variant="quiet"
          disabled={report.errors.length === 0 && loads === 0}
          onClick={() => {
            api.errorLogClear()
            showToast({ text: t('report.cleared') })
            setTimeout(load, 100)
          }}
        >
          {t('report.clear')}
        </Button>
      </Row>
    </>
  )
}

function Permissions() {
  const t = useT()
  const [list, setList] = useState<SitePermission[] | null>(null)
  const load = () => void call.permissionsList().then(setList)
  useEffect(load, [])
  if (!list) return null
  if (list.length === 0) return <p className="setting-hint">{t('settings.permissionsEmpty')}</p>
  return (
    <ul className="permission-list">
      {list.map((p) => (
        <li key={`${p.origin}-${p.kind}`}>
          <span className="mono">{p.origin.replace(/^https?:\/\//, '')}</span>
          <span>{t(`perm.${p.kind}` as MessageKey)}</span>
          <span className={`label ${p.allow ? 'is-allowed' : ''}`}>{p.allow ? t('settings.allowed') : t('settings.denied')}</span>
          <IconButton
            icon={X}
            label={t('settings.remove')}
            onClick={() => {
              api.permissionRemove(p.origin, p.kind as PermissionKind)
              setTimeout(load, 100)
            }}
          />
        </li>
      ))}
    </ul>
  )
}

function About() {
  const { global } = useSnapshot()
  const t = useT()
  const [info, setInfo] = useState<AppInfo | null>(null)
  useEffect(() => {
    void call.appInfo().then(setInfo)
  }, [])
  const update = global.update
  let status = ''
  if (update.status === 'checking') status = t('update.checking')
  else if (update.status === 'available') status = t('update.available', { version: update.version ?? '' })
  else if (update.status === 'downloading') status = t('update.downloading', { n: update.progress })
  else if (update.status === 'latest') status = t('update.latest')
  else if (update.status === 'error') status = t('update.error')
  else if (update.status === 'unsupported') status = t('update.unsupported')
  else if (update.status === 'ready') status = t('update.readyBody', { version: update.version ?? '' })

  return (
    <div className="about">
      <div className="about-brand">
        <DoubleEye height={36} />
        <div>
          <div className="about-name">{t('app.name')}</div>
          <div className="about-version">
            <span className="label">{t('settings.version')}</span>
            <span className="mono">{info?.version}</span>
            {info && info.stage !== 'stable' && <span className="label stage-label">{t(`stage.${info.stage}`)}</span>}
          </div>
        </div>
      </div>
      {info && info.stage !== 'stable' && (
        <div className="stage-note">
          <p className="setting-hint">{t('stage.note')}</p>
          <button
            type="button"
            className="link-button"
            onClick={() => api.openPage('settings', 'report')}
          >
            {t('stage.report')}
          </button>
        </div>
      )}
      <Row title={t('update.check')} hint={status || undefined}>
        {update.status === 'ready' ? (
          <Button variant="accent" onClick={() => api.updateInstall()}>
            {t('update.restart')}
          </Button>
        ) : (
          <Button disabled={update.status === 'checking' || update.status === 'downloading'} onClick={() => api.updateCheck()}>
            {t('update.check')}
          </Button>
        )}
      </Row>
      {info && (
        <dl className="about-facts mono">
          <dt>{t('settings.engineInfo')}</dt>
          <dd>
            Chromium {info.chrome} · Electron {info.electron}
          </dd>
          <dt>Node</dt>
          <dd>{info.node}</dd>
          <dt>{t('settings.license')}</dt>
          <dd>GPL-3.0</dd>
        </dl>
      )}
      <p className="setting-hint">{t('settings.licenseBody')}</p>
      <p>
        <button type="button" className="link-button" onClick={() => api.openURL('https://github.com/PEEPWICK/peepwick-browser', true)}>
          {t('settings.source')} <Icon icon={ExternalLink} size={13} />
        </button>
      </p>
      <p className="about-sign">
        <LookMark className="accent" /> {t('app.tagline')} {t('settings.made')}
      </p>
    </div>
  )
}

function DefaultBrowser() {
  const t = useT()
  const [info, setInfo] = useState<AppInfo | null>(null)
  useEffect(() => {
    void call.appInfo().then(setInfo)
  }, [])
  if (!info) return null
  return (
    <Row title={t('settings.defaultBrowser')} hint={info.isDefaultBrowser ? t('settings.defaultBrowser.is') : t('settings.defaultBrowser.not')}>
      {!info.isDefaultBrowser && (
        <Button onClick={() => void call.setDefaultBrowser().then((ok) => setInfo({ ...info, isDefaultBrowser: ok }))}>
          {t('settings.defaultBrowser.make')}
        </Button>
      )}
    </Row>
  )
}

function BookmarksSection() {
  const tree = useBookmarks()
  const { openPanel, showToast } = useUI()
  const t = useT()
  const counts = useMemo(() => {
    let links = 0
    walk(tree, (node) => node.type === 'url' && links++)
    return links
  }, [tree])
  return (
    <>
      <Row title={t('settings.bookmarksCount')} hint={t('import.links', { n: counts })}>
        <Button onClick={() => api.openPage('bookmarks')}>
          <Icon icon={BookMarked} size={15} /> {t('settings.manager')}
        </Button>
      </Row>
      <Row title={t('import.title')} hint={t('settings.importHint')}>
        <Button onClick={() => openPanel({ kind: 'import' })}>
          <Icon icon={Download} size={15} /> {t('import.run')}
        </Button>
      </Row>
      <Row title={t('bookmarks.export')} hint={t('settings.exportHint')}>
        <Button onClick={() => void call.exportBookmarks().then((ok) => ok && showToast({ text: t('bookmarks.exported') }))}>
          <Icon icon={Upload} size={15} /> {t('bookmarks.exportMenu')}
        </Button>
      </Row>
    </>
  )
}

export function Settings({ section }: { section: string }) {
  const { settings: s, state } = useSnapshot()
  const { openPanel } = useUI()
  const t = useT()
  const current: SettingsSection = (SETTINGS_SECTIONS as readonly string[]).includes(section) ? (section as SettingsSection) : 'general'
  const scroller = useRef<HTMLDivElement>(null)

  // Each section is its own page: start at the top when switching.
  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 })
  }, [current])

  const sleepOptions = [0, 30, 60, 240, 720]
  const sleepLabel = (n: number) =>
    n === 0 ? t('settings.sleep.never') : n < 60 ? t('settings.sleep.minutes', { n }) : t('settings.sleep.hours', { n: n / 60 })

  const sections: Record<SettingsSection, ReactNode> = {
    general: (
      <>
        <Row title={t('settings.language')}>
          <Segmented
            label={t('settings.language')}
            value={s.language}
            options={[
              { value: 'tr', label: 'Türkçe' },
              { value: 'en', label: 'English' },
            ]}
            onChange={(language) => api.setSettings({ language })}
          />
        </Row>
        <Row title={t('settings.restore')} hint={t('settings.restoreHint')}>
          <Toggle checked={s.restoreSession} label={t('settings.restore')} onChange={(restoreSession) => api.setSettings({ restoreSession })} />
        </Row>
        {!state.incognito && <DefaultBrowser />}
        <h3 className="settings-subhead label">{t('settings.system')}</h3>
        <SystemRows />
      </>
    ),
    appearance: (
      <>
        <Row title={t('settings.theme')}>
          <Segmented<ThemePref>
            label={t('settings.theme')}
            value={s.theme}
            options={[
              { value: 'dark', label: t('settings.theme.dark') },
              { value: 'light', label: t('settings.theme.light') },
              { value: 'system', label: t('settings.theme.system') },
            ]}
            onChange={(theme) => api.setSettings({ theme })}
          />
        </Row>
        <Row title={t('settings.bookmarksBar')}>
          <Segmented<BookmarksBarMode>
            label={t('settings.bookmarksBar')}
            value={s.bookmarksBar}
            options={[
              { value: 'always', label: t('settings.bookmarksBar.always') },
              { value: 'newtab', label: t('settings.bookmarksBar.newtab') },
              { value: 'never', label: t('settings.bookmarksBar.never') },
            ]}
            onChange={(bookmarksBar) => api.setSettings({ bookmarksBar })}
          />
        </Row>
        <Row title={t('settings.sidebarCollapsed')} hint={t('settings.sidebarHint')}>
          <Toggle
            checked={s.sidebarCollapsed}
            label={t('settings.sidebarCollapsed')}
            onChange={(sidebarCollapsed) => api.setSettings({ sidebarCollapsed })}
          />
        </Row>
      </>
    ),
    search: (
      <>
        <Row title={t('settings.engine')}>
          <select className="select" value={s.searchEngine} onChange={(event) => api.setSettings({ searchEngine: event.target.value as SearchEngineId })}>
            {(Object.keys(SEARCH_ENGINES) as SearchEngineId[]).map((id) => (
              <option key={id} value={id}>
                {SEARCH_ENGINES[id].name}
              </option>
            ))}
          </select>
        </Row>
        <Row title={t('settings.suggestions')} hint={t('settings.suggestionsHint')}>
          <Toggle checked={s.searchSuggestions} label={t('settings.suggestions')} onChange={(searchSuggestions) => api.setSettings({ searchSuggestions })} />
        </Row>
      </>
    ),
    tabs: (
      <Row title={t('settings.sleep')} hint={t('settings.sleepHint')}>
        <select className="select" value={s.sleepAfterMinutes} onChange={(event) => api.setSettings({ sleepAfterMinutes: Number(event.target.value) })}>
          {[...new Set([...sleepOptions, s.sleepAfterMinutes])].sort((a, b) => a - b).map((n) => (
            <option key={n} value={n}>
              {sleepLabel(n)}
            </option>
          ))}
        </select>
      </Row>
    ),
    bookmarks: <BookmarksSection />,
    privacy: (
      <>
        <Row
          title={t('settings.shieldLevel')}
          hint={t(s.shieldLevel === 'strict' ? 'settings.shield.strictHint' : s.shieldLevel === 'off' ? 'settings.shield.offHint' : 'settings.shield.standardHint')}
        >
          <Segmented<ShieldLevel>
            label={t('settings.shieldLevel')}
            value={s.shieldLevel}
            options={[
              { value: 'off', label: t('settings.shield.off') },
              { value: 'standard', label: t('settings.shield.standard') },
              { value: 'strict', label: t('settings.shield.strict') },
            ]}
            onChange={(shieldLevel) => api.setSettings({ shieldLevel })}
          />
        </Row>
        <Row title={t('settings.thirdPartyCookies')} hint={t('settings.thirdPartyCookiesHint')}>
          <Toggle
            checked={s.blockThirdPartyCookies}
            label={t('settings.thirdPartyCookies')}
            onChange={(blockThirdPartyCookies) => api.setSettings({ blockThirdPartyCookies })}
          />
        </Row>
        <Row title={t('settings.gpc')} hint={t('settings.gpcHint')}>
          <Toggle
            checked={s.sendGlobalPrivacyControl}
            label={t('settings.gpc')}
            onChange={(sendGlobalPrivacyControl) => api.setSettings({ sendGlobalPrivacyControl })}
          />
        </Row>
        <div className="setting-block">
          <div className="setting-title">{t('settings.allowlist')}</div>
          <DomainList
            items={s.shieldAllowlist}
            placeholder="example.com"
            empty={t('settings.allowlistEmpty')}
            onChange={(shieldAllowlist) => api.setSettings({ shieldAllowlist })}
          />
        </div>
        <SecureDnsRow />
        <Row title={t('settings.clearData')} hint={t('settings.clearDataHint')}>
          <Button onClick={() => openPanel({ kind: 'clear-data' })}>{t('settings.clearDataOpen')}</Button>
        </Row>
        <div className="setting-block">
          <div className="setting-title">{t('clear.onExit')}</div>
          <div className="setting-hint">{t('clear.onExitHint')}</div>
          <ClearOnExit />
        </div>
      </>
    ),
    permissions: <Permissions />,
    focus: (
      <>
        <Row title={t('settings.focusMinutes')}>
          <Segmented
            label={t('settings.focusMinutes')}
            value={s.focusMinutes}
            options={[25, 50, 90].map((n) => ({ value: n, label: <span className="mono">{t('focus.minutes', { n })}</span> }))}
            onChange={(focusMinutes) => api.setSettings({ focusMinutes })}
          />
        </Row>
        <Row title={t('settings.focusSilence')}>
          <Toggle
            checked={s.focusSilenceNotifications}
            label={t('settings.focusSilence')}
            onChange={(focusSilenceNotifications) => api.setSettings({ focusSilenceNotifications })}
          />
        </Row>
        <div className="setting-block">
          <div className="setting-title">{t('settings.focusSites')}</div>
          <div className="setting-hint">{t('settings.focusSitesHint')}</div>
          <DomainList items={s.focusSites} placeholder="youtube.com" empty="—" onChange={(focusSites) => api.setSettings({ focusSites })} />
        </div>
      </>
    ),
    downloads: (
      <>
        <DownloadFolderRow />
        <Row title={t('settings.askSave')}>
          <Toggle checked={s.askWhereToSave} label={t('settings.askSave')} onChange={(askWhereToSave) => api.setSettings({ askWhereToSave })} />
        </Row>
      </>
    ),
    backup: <BackupSection />,
    report: <ReportSection />,
    about: <About />,
  }

  return (
    <div className="page settings">
      <nav className="settings-nav">
        <h1 className="settings-heading">{t('settings.title')}</h1>
        {SETTINGS_SECTIONS.filter((id) => !(state.incognito && (id === 'bookmarks' || id === 'backup'))).map((id) => (
          <button
            key={id}
            type="button"
            className={current === id ? 'is-active' : ''}
            aria-current={current === id ? 'page' : undefined}
            onClick={() => api.openPage('settings', id)}
          >
            {t(`settings.${id}` as MessageKey)}
          </button>
        ))}
      </nav>
      <div className="settings-scroll" ref={scroller}>
        <div className="settings-body">
          <section key={current} className="settings-section">
            <h2>{t(`settings.${current}` as MessageKey)}</h2>
            {sections[current]}
          </section>
        </div>
      </div>
    </div>
  )
}
