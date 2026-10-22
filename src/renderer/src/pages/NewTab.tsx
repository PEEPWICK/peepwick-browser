import { useEffect, useState } from 'react'
import { Search } from 'lucide-react'
import type { Stats, TopSite } from '@shared/types'
import { api, call } from '../api'
import { Crowd, DoubleEye, LookMark } from '../brand'
import { useUI } from '../context'
import { useSnapshot, useT } from '../store'
import { Favicon, Icon } from '../ui'

function today(lang: string): string {
  return new Intl.DateTimeFormat(lang === 'tr' ? 'tr-TR' : 'en-US', { weekday: 'long', day: 'numeric', month: 'long' }).format(new Date())
}

export function NewTab() {
  const { state, settings } = useSnapshot()
  const t = useT()
  const { focusOmnibox } = useUI()
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
          <button type="button" className="fakebox" onClick={() => focusOmnibox()}>
            <Icon icon={Search} size={18} />
            <span>{t('newtab.search')}</span>
          </button>
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
        <button
          type="button"
          className="fakebox"
          onClick={() => focusOmnibox()}
          onKeyDown={(event) => {
            if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
              event.preventDefault()
              focusOmnibox(event.key)
            }
          }}
        >
          <Icon icon={Search} size={18} />
          <span>{t('newtab.search')}</span>
        </button>
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
