// The translate bar: pick the languages, agree to download a model the first time,
// watch the page change, and go back to the original.
import { useEffect, useMemo, useState } from 'react'
import { ArrowRight, Languages, X } from 'lucide-react'
import { formatBytes } from '@shared/i18n'
import type { Lang, TabState, TranslationState } from '@shared/types'
import { api, call } from '../api'
import { useSnapshot, useT } from '../store'
import { Button, Icon, IconButton } from '../ui'

export function useLanguageName(lang: Lang): (code: string) => string {
  const names = useMemo(() => new Intl.DisplayNames([lang === 'tr' ? 'tr' : 'en'], { type: 'language' }), [lang])
  return (code) => {
    try {
      const name = names.of(code) ?? code
      return name.charAt(0).toLocaleUpperCase(lang) + name.slice(1)
    } catch {
      return code
    }
  }
}

/** The omnibox shows the translate button for pages in another language. */
export function canTranslate(tab: TabState | null | undefined, lang: Lang): boolean {
  return !!tab && !tab.page && !tab.interstitial && (!!tab.translation || (!!tab.pageLang && tab.pageLang !== lang && /^https?:/.test(tab.url)))
}

const percent = (fraction: number) => `%${Math.round(fraction * 100)}`

function LanguageSelect({ value, options, label, onChange }: { value: string; options: string[]; label: string; onChange: (code: string) => void }) {
  const { settings } = useSnapshot()
  const name = useLanguageName(settings.language)
  const sorted = useMemo(() => [...new Set([...options, value])].sort((a, b) => name(a).localeCompare(name(b), settings.language)), [options, value, settings.language])
  return (
    <select className="select" aria-label={label} value={value} onChange={(event) => onChange(event.target.value)}>
      {sorted.map((code) => (
        <option key={code} value={code}>
          {name(code)}
        </option>
      ))}
    </select>
  )
}

function status(t: ReturnType<typeof useT>, state: TranslationState, name: (code: string) => string, lang: Lang): string {
  if (state.status === 'downloading') return t('translate.downloading', { percent: percent(state.progress) })
  if (state.status === 'translating') return t('translate.translating', { percent: percent(state.progress) })
  if (state.status === 'done') return t('translate.done', { from: name(state.from), to: name(state.to) })
  if (state.error) return t(`translate.error.${state.error}`)
  if (state.from === state.to) return t('translate.same')
  if (state.download > 0) return t('translate.download', { size: formatBytes(lang, state.download) })
  if (state.download === 0) return t('translate.ready')
  return ''
}

export function TranslateBar({ tab }: { tab: TabState }) {
  const { settings } = useSnapshot()
  const t = useT()
  const name = useLanguageName(settings.language)
  const [languages, setLanguages] = useState<string[]>([])
  useEffect(() => {
    void call.translationLanguages().then(setLanguages)
  }, [])
  const state = tab.translation
  if (!state) return null
  const busy = state.status === 'downloading' || state.status === 'translating'
  const choose = (from: string, to: string) => api.translation(tab.id, 'choose', from, to)

  return (
    <div className="bar translate-bar" role="region" aria-label={t('cmd.translate')}>
      <Icon icon={Languages} size={16} className="muted" />
      {state.status === 'offer' || state.status === 'error' ? (
        <>
          <LanguageSelect value={state.from} options={languages} label={t('translate.from')} onChange={(from) => choose(from, state.to)} />
          <Icon icon={ArrowRight} size={14} className="muted" />
          <LanguageSelect value={state.to} options={languages} label={t('translate.to')} onChange={(to) => choose(state.from, to)} />
          <Button
            variant="primary"
            disabled={state.from === state.to || state.error === 'unsupported' || state.download < 0}
            onClick={() => api.translation(tab.id, 'start', state.from, state.to)}
          >
            {state.status === 'error' ? t('translate.retry') : state.download > 0 ? t('translate.downloadRun') : t('translate.run')}
          </Button>
          <span className="translate-note muted">{status(t, state, name, settings.language)}</span>
        </>
      ) : (
        <>
          <span className="translate-status">{status(t, state, name, settings.language)}</span>
          {busy && (
            <span className="translate-progress" aria-hidden>
              <span style={{ width: `${Math.round(state.progress * 100)}%` }} />
            </span>
          )}
          {state.status === 'done' && (
            <Button variant="quiet" onClick={() => api.translation(tab.id, 'revert')}>
              {t('translate.original')}
            </Button>
          )}
        </>
      )}
      <span className="spacer" />
      <IconButton icon={X} label={busy ? t('translate.cancel') : t('common.close')} onClick={() => api.translation(tab.id, 'close')} />
    </div>
  )
}
