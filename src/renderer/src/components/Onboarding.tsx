// First run: the eyes open once (the brand's opening blink), then three small choices.
import { useState } from 'react'
import { SEARCH_ENGINES } from '@shared/search'
import type { SearchEngineId, ThemePref } from '@shared/types'
import { api, call } from '../api'
import { DoubleEye } from '../brand'
import { useSettings, useSnapshot, useT } from '../store'
import { Button, Segmented, useOverlay } from '../ui'

export function Onboarding() {
  useOverlay(true)
  const t = useT()
  const settings = useSettings()
  const { app } = useSnapshot()
  const [makeDefault, setMakeDefault] = useState(true)

  const finish = async () => {
    if (makeDefault) await call.setDefaultBrowser().catch(() => false)
    api.onboardingDone()
  }

  return (
    <div className="onboarding" role="dialog" aria-labelledby="onboard-title">
      <div className="onboarding-inner">
        <DoubleEye height={54} mode="blink" className="onboarding-mark" />
        {app.stage !== 'stable' && (
          <span className="onboarding-stage">
            <span className="label">{t(`stage.${app.stage}`)}</span>
            <span className="mono muted">{app.version}</span>
          </span>
        )}
        <h1 id="onboard-title" className="display">
          {t('onboard.title')}
        </h1>
        <p className="lead">{t('onboard.lead')}</p>

        <div className="onboarding-choices">
          <div className="field">
            <span className="label">{t('settings.language')}</span>
            <Segmented
              label={t('settings.language')}
              value={settings.language}
              options={[
                { value: 'tr', label: 'Türkçe' },
                { value: 'en', label: 'English' },
              ]}
              onChange={(language) => api.setSettings({ language })}
            />
          </div>
          <div className="field">
            <span className="label">{t('onboard.theme')}</span>
            <Segmented<ThemePref>
              label={t('onboard.theme')}
              value={settings.theme}
              options={[
                { value: 'dark', label: t('settings.theme.dark') },
                { value: 'light', label: t('settings.theme.light') },
                { value: 'system', label: t('settings.theme.system') },
              ]}
              onChange={(theme) => api.setSettings({ theme })}
            />
          </div>
          <div className="field">
            <span className="label">{t('onboard.engine')}</span>
            <div className="chips">
              {(Object.keys(SEARCH_ENGINES) as SearchEngineId[]).map((id) => (
                <button
                  key={id}
                  type="button"
                  className={`chip ${settings.searchEngine === id ? 'is-active' : ''}`}
                  onClick={() => api.setSettings({ searchEngine: id })}
                >
                  {SEARCH_ENGINES[id].name}
                </button>
              ))}
            </div>
          </div>
          <label className="check">
            <input type="checkbox" checked={makeDefault} onChange={(event) => setMakeDefault(event.target.checked)} />
            <span>{t('onboard.default')}</span>
          </label>
        </div>

        <Button variant="accent" className="onboarding-start" onClick={() => void finish()}>
          {t('onboard.start')}
        </Button>
      </div>
    </div>
  )
}
