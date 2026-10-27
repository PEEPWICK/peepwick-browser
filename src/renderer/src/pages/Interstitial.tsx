// Screens drawn in place of a page: load errors, certificate errors, crashes and
// the focus gate.
import { useEffect, useState } from 'react'
import type { MessageKey } from '@shared/i18n'
import type { Interstitial as InterstitialState, TabState } from '@shared/types'
import { hostOf } from '@shared/url'
import { api } from '../api'
import { certificateReason } from '../components/SiteInfo'
import { useUI } from '../context'
import { DoubleEye, LookMark } from '../brand'
import { useGlobal, useT } from '../store'
import { Button, remaining, useNow } from '../ui'

const GATE_SECONDS = 15

function errorText(code: number): MessageKey {
  if (code === -105 || code === -137) return 'error.dns'
  if (code === -106 || code === -21) return 'error.offline'
  if (code === -102) return 'error.refused'
  if (code === -118 || code === -7) return 'error.timeout'
  if (code === -20 || code === -27) return 'error.blocked'
  return 'error.generic'
}

function FocusGate({ tab, host }: { tab: TabState; host: string }) {
  const t = useT()
  const focus = useGlobal().focus
  const now = useNow(1000)
  const [wait, setWait] = useState(GATE_SECONDS)
  useEffect(() => {
    setWait(GATE_SECONDS)
    const id = setInterval(() => setWait((w) => Math.max(0, w - 1)), 1000)
    return () => clearInterval(id)
  }, [tab.id, host])

  return (
    <div className="page interstitial is-focus">
      <div className="interstitial-body">
        <DoubleEye height={40} mode="look" />
        <span className="label interstitial-code">
          <LookMark className="accent" /> {t('focus.title')} · {t('focus.remaining', { time: remaining(focus.endsAt - now) })}
        </span>
        <h1 className="display small">{t('focus.gate.title')}</h1>
        <p className="lead">{t('focus.gate.body', { host })}</p>
        <div className="interstitial-actions">
          <Button variant="primary" onClick={() => api.interstitial(tab.id, 'back')}>
            {t('focus.gate.back')}
          </Button>
          <Button variant="quiet" disabled={wait > 0} onClick={() => api.interstitial(tab.id, 'open-anyway')}>
            {wait > 0 ? t('focus.gate.wait', { n: wait }) : t('focus.gate.open')}
          </Button>
        </div>
      </div>
    </div>
  )
}

export function Interstitial({ tab, interstitial }: { tab: TabState; interstitial: InterstitialState }) {
  const t = useT()
  const { openPanel } = useUI()

  if (interstitial.kind === 'focus') return <FocusGate tab={tab} host={interstitial.host} />

  if (interstitial.kind === 'crashed') {
    return (
      <div className="page interstitial">
        <div className="interstitial-body">
          <DoubleEye height={40} mode="down" />
          <h1 className="display small">{t('error.crashed.title')}</h1>
          <p className="lead">{t('error.crashed.body')}</p>
          <div className="interstitial-actions">
            <Button variant="primary" onClick={() => api.interstitial(tab.id, 'retry')}>
              {t('nav.reload')}
            </Button>
          </div>
        </div>
      </div>
    )
  }

  const host = hostOf(interstitial.url)
  if (interstitial.certificate) {
    return (
      <div className="page interstitial is-warning">
        <div className="interstitial-body">
          <DoubleEye height={40} mode="down" />
          <span className="label interstitial-code">
            {interstitial.code} · {interstitial.description}
          </span>
          <h1 className="display small">{t('error.cert.title')}</h1>
          <p className="lead">{t('error.cert.body', { host })}</p>
          <p className="lead">{t('error.cert.reason', { reason: t(certificateReason(interstitial.description)) })}</p>
          <div className="interstitial-actions">
            <Button variant="primary" onClick={() => api.interstitial(tab.id, 'back')}>
              {t('focus.gate.back')}
            </Button>
            <Button variant="quiet" onClick={() => openPanel({ kind: 'certificate', host })}>
              {t('cert.view')}
            </Button>
            <Button variant="quiet" onClick={() => api.interstitial(tab.id, 'proceed')}>
              {t('error.cert.proceed')}
            </Button>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="page interstitial">
      <div className="interstitial-body">
        <DoubleEye height={40} mode="down" />
        <span className="label interstitial-code">
          {interstitial.code} · {interstitial.description}
        </span>
        <h1 className="display small">{t('error.title')}</h1>
        <p className="lead">{t(errorText(interstitial.code))}</p>
        <p className="mono muted interstitial-url">{interstitial.url}</p>
        <div className="interstitial-actions">
          <Button variant="primary" onClick={() => api.interstitial(tab.id, 'retry')}>
            {t('error.retry')}
          </Button>
        </div>
      </div>
    </div>
  )
}
