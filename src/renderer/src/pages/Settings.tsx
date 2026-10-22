import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ExternalLink, X } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import { SEARCH_ENGINES } from '@shared/search'
import { normalizeDomain } from '@shared/settings'
import type { AppInfo, PermissionKind, SearchEngineId, ShieldLevel, SitePermission, ThemePref } from '@shared/types'
import { api, call } from '../api'
import { DoubleEye, LookMark } from '../brand'
import { useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, Segmented, Toggle } from '../ui'

const SECTIONS = ['general', 'search', 'privacy', 'focus', 'tabs', 'downloads', 'permissions', 'about'] as const
type Section = (typeof SECTIONS)[number]

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

function ClearData() {
  const t = useT()
  const [choice, setChoice] = useState({ history: true, cookies: false, cache: true })
  const [done, setDone] = useState(false)
  const keys: [keyof typeof choice, MessageKey][] = [
    ['history', 'settings.clearData.history'],
    ['cookies', 'settings.clearData.cookies'],
    ['cache', 'settings.clearData.cache'],
  ]
  return (
    <div className="clear-data">
      {keys.map(([key, label]) => (
        <label key={key} className="check">
          <input type="checkbox" checked={choice[key]} onChange={(event) => setChoice({ ...choice, [key]: event.target.checked })} />
          <span>{t(label)}</span>
        </label>
      ))}
      <div className="clear-data-actions">
        <Button
          disabled={!choice.history && !choice.cookies && !choice.cache}
          onClick={() => {
            setDone(false)
            void call.clearData(choice).then(() => setDone(true))
          }}
        >
          {t('settings.clearData.run')}
        </Button>
        {done && <span className="mono muted">{t('settings.clearData.done')}</span>}
      </div>
    </div>
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
          <div className="label">
            {t('settings.version')} {info?.version}
          </div>
        </div>
      </div>
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

export function Settings({ hash }: { hash: string }) {
  const { settings: s, state } = useSnapshot()
  const t = useT()
  const scroller = useRef<HTMLDivElement>(null)
  const [current, setCurrent] = useState<Section>((SECTIONS as readonly string[]).includes(hash) ? (hash as Section) : 'general')

  const go = (section: Section) => {
    setCurrent(section)
    const el = scroller.current?.querySelector(`#settings-${section}`)
    el?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  useEffect(() => {
    if ((SECTIONS as readonly string[]).includes(hash)) {
      requestAnimationFrame(() => scroller.current?.querySelector(`#settings-${hash}`)?.scrollIntoView({ block: 'start' }))
    }
  }, [hash])

  const onScroll = () => {
    const root = scroller.current
    if (!root) return
    const top = root.getBoundingClientRect().top
    let active: Section = 'general'
    for (const section of SECTIONS) {
      const el = root.querySelector(`#settings-${section}`)
      if (el && el.getBoundingClientRect().top - top < 120) active = section
    }
    setCurrent(active)
  }

  const sleepOptions = [0, 30, 60, 240, 720]
  const sleepLabel = (n: number) =>
    n === 0 ? t('settings.sleep.never') : n < 60 ? t('settings.sleep.minutes', { n }) : t('settings.sleep.hours', { n: n / 60 })

  return (
    <div className="page settings">
      <nav className="settings-nav">
        <h1 className="settings-heading">{t('settings.title')}</h1>
        {SECTIONS.map((section) => (
          <button key={section} type="button" className={current === section ? 'is-active' : ''} onClick={() => go(section)}>
            {t(`settings.${section}` as MessageKey)}
          </button>
        ))}
      </nav>
      <div className="settings-scroll" ref={scroller} onScroll={onScroll}>
        <div className="settings-body">
          <section id="settings-general">
            <h2>{t('settings.general')}</h2>
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
            <Row title={t('settings.restore')} hint={t('settings.restoreHint')}>
              <Toggle checked={s.restoreSession} label={t('settings.restore')} onChange={(restoreSession) => api.setSettings({ restoreSession })} />
            </Row>
            {!state.incognito && <DefaultBrowser />}
          </section>

          <section id="settings-search">
            <h2>{t('settings.search')}</h2>
            <Row title={t('settings.engine')}>
              <select
                className="select"
                value={s.searchEngine}
                onChange={(event) => api.setSettings({ searchEngine: event.target.value as SearchEngineId })}
              >
                {(Object.keys(SEARCH_ENGINES) as SearchEngineId[]).map((id) => (
                  <option key={id} value={id}>
                    {SEARCH_ENGINES[id].name}
                  </option>
                ))}
              </select>
            </Row>
            <Row title={t('settings.suggestions')} hint={t('settings.suggestionsHint')}>
              <Toggle
                checked={s.searchSuggestions}
                label={t('settings.suggestions')}
                onChange={(searchSuggestions) => api.setSettings({ searchSuggestions })}
              />
            </Row>
          </section>

          <section id="settings-privacy">
            <h2>{t('settings.privacy')}</h2>
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
            <div className="setting-block">
              <div className="setting-title">{t('settings.clearData')}</div>
              <div className="setting-hint">{t('settings.clearDataHint')}</div>
              <ClearData />
            </div>
          </section>

          <section id="settings-focus">
            <h2>{t('settings.focus')}</h2>
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
              <DomainList
                items={s.focusSites}
                placeholder="youtube.com"
                empty="—"
                onChange={(focusSites) => api.setSettings({ focusSites })}
              />
            </div>
          </section>

          <section id="settings-tabs">
            <h2>{t('settings.tabs')}</h2>
            <Row title={t('settings.sleep')} hint={t('settings.sleepHint')}>
              <select
                className="select"
                value={s.sleepAfterMinutes}
                onChange={(event) => api.setSettings({ sleepAfterMinutes: Number(event.target.value) })}
              >
                {[...new Set([...sleepOptions, s.sleepAfterMinutes])].sort((a, b) => a - b).map((n) => (
                  <option key={n} value={n}>
                    {sleepLabel(n)}
                  </option>
                ))}
              </select>
            </Row>
          </section>

          <section id="settings-downloads">
            <h2>{t('settings.downloads')}</h2>
            <Row title={t('settings.askSave')} hint={t('settings.downloadsFolder')}>
              <Toggle checked={s.askWhereToSave} label={t('settings.askSave')} onChange={(askWhereToSave) => api.setSettings({ askWhereToSave })} />
            </Row>
          </section>

          <section id="settings-permissions">
            <h2>{t('settings.permissions')}</h2>
            <Permissions />
          </section>

          <section id="settings-about">
            <h2>{t('settings.about')}</h2>
            <About />
          </section>
        </div>
      </div>
    </div>
  )
}
