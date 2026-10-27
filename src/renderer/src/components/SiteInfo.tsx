// Connection security in the address bar: the button at its left, the site panel
// it opens (connection, certificate, permissions, site data) and the certificate
// viewer. The state comes from Chromium's own certificate checks (security.ts).
import { useEffect, useState } from 'react'
import { ChevronRight, HardDrive, Info, Lock, ShieldAlert, TriangleAlert, X, type LucideIcon } from 'lucide-react'
import type { MessageKey } from '@shared/i18n'
import type { CertificateDetails } from '@shared/ipc'
import type { PermissionKind, SiteInfo, TabSecurity, TabState } from '@shared/types'
import { api, call } from '../api'
import { DoubleEye } from '../brand'
import { useUI } from '../context'
import { activeTab, useSnapshot, useT } from '../store'
import { Button, Icon, IconButton, Segmented } from '../ui'
import { Modal } from './BookmarkDialogs'
import { Popover } from './Panels'

const ICONS: Record<TabSecurity, LucideIcon | null> = {
  secure: Lock,
  unverified: Info,
  insecure: TriangleAlert,
  dangerous: ShieldAlert,
  local: HardDrive,
  internal: null,
}

const TITLES: Record<TabSecurity, MessageKey> = {
  secure: 'site.secure',
  unverified: 'site.unverified',
  insecure: 'site.insecure',
  dangerous: 'site.dangerous',
  local: 'site.local',
  internal: 'site.internal',
}

const BODIES: Record<TabSecurity, MessageKey> = {
  secure: 'site.secureBody',
  unverified: 'site.unverifiedBody',
  insecure: 'site.insecureBody',
  dangerous: 'site.dangerousBody',
  local: 'site.localBody',
  internal: 'site.internalBody',
}

const REASONS = new Set([
  'COMMON_NAME_INVALID',
  'DATE_INVALID',
  'AUTHORITY_INVALID',
  'REVOKED',
  'WEAK_SIGNATURE_ALGORITHM',
  'WEAK_KEY',
  'NAME_CONSTRAINT_VIOLATION',
  'VALIDITY_TOO_LONG',
  'CERTIFICATE_TRANSPARENCY_REQUIRED',
  'KNOWN_INTERCEPTION_BLOCKED',
])

/** "net::ERR_CERT_DATE_INVALID" or "ERR_CERT_DATE_INVALID" → a reason people can read. */
export function certificateReason(error: string | null): MessageKey {
  const name = (error ?? '').replace(/^net::/, '').replace(/^ERR_/, '').replace(/^CERT_/, '')
  return REASONS.has(name) ? (`cert.err.${name}` as MessageKey) : 'cert.err.generic'
}

/** The button at the left of the address bar. */
export function SiteButton({ tab }: { tab: TabState }) {
  const t = useT()
  const { togglePanel } = useUI()
  const security = tab.page ? 'internal' : tab.security
  const icon = ICONS[security]
  const warn = security === 'insecure' || security === 'dangerous'
  return (
    <button
      type="button"
      data-anchor="site"
      className={`omni-badge site-button is-${security}`}
      aria-label={t('site.title')}
      title={t(TITLES[security])}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => togglePanel({ kind: 'site', anchor: event.currentTarget.getBoundingClientRect() })}
    >
      {icon ? <Icon icon={icon} size={15} /> : <DoubleEye height={10} />}
      {warn && <span className="site-warning label">{t('omnibox.insecure')}</span>}
    </button>
  )
}

function PermissionRow({ origin, kind, allow, onChange }: { origin: string; kind: PermissionKind; allow: boolean; onChange: () => void }) {
  const t = useT()
  return (
    <div className="site-permission">
      <span>{t(`perm.${kind}` as MessageKey)}</span>
      <Segmented<'allow' | 'block' | 'ask'>
        label={t(`perm.${kind}` as MessageKey)}
        value={allow ? 'allow' : 'block'}
        options={[
          { value: 'allow', label: t('site.allow') },
          { value: 'block', label: t('site.block') },
          { value: 'ask', label: t('site.ask') },
        ]}
        onChange={(value) => {
          api.sitePermission(origin, kind, value === 'ask' ? null : value === 'allow')
          setTimeout(onChange, 80)
        }}
      />
    </div>
  )
}

export function SitePanel({ anchor }: { anchor: DOMRect }) {
  const { state } = useSnapshot()
  const { openPanel, closePanel, ask, showToast } = useUI()
  const t = useT()
  const [info, setInfo] = useState<SiteInfo | null>(null)
  const tab = activeTab(state)
  const load = () => void call.siteInfo().then(setInfo)
  useEffect(load, [tab?.url, tab?.security])

  if (!info) return null
  const security = tab?.page ? 'internal' : info.security
  const web = /^https?:/.test(info.url)
  const icon = ICONS[security]
  const certificate = info.certificate

  return (
    <Popover anchor={anchor} label={t('site.title')} width={360}>
      <div className="panel-head">
        <span className="label">{t('site.title')}</span>
        {web && <span className="mono muted panel-host">{info.host}</span>}
      </div>
      <div className={`site-state is-${security}`}>
        <span className="site-state-icon">{icon ? <Icon icon={icon} size={18} /> : <DoubleEye height={12} />}</span>
        <div>
          <div className="site-state-title">{t(TITLES[security])}</div>
          <p className="panel-text muted">{t(BODIES[security])}</p>
        </div>
      </div>

      {certificate && (
        <button
          type="button"
          className="site-link"
          onClick={() => openPanel({ kind: 'certificate', host: info.host })}
        >
          <span className="site-link-text">
            <span>{t('site.certificate')}</span>
            <span className="muted">
              {info.errorCode === 0
                ? t('site.certificateValid', { issuer: certificate.issuerOrganization || certificate.issuer })
                : t('site.certificateInvalid', { reason: t(certificateReason(info.verification)) })}
            </span>
          </span>
          <Icon icon={ChevronRight} size={15} />
        </button>
      )}
      {certificate && info.errorCode === 0 && !info.knownRoot && <p className="panel-hint muted">{t('site.localRoot')}</p>}
      {security === 'dangerous' && info.exception && (
        <Button
          onClick={() => {
            api.certificateRevoke()
            closePanel()
          }}
        >
          {t('site.restoreWarning')}
        </Button>
      )}
      {security === 'unverified' && tab && (
        <Button
          onClick={() => {
            api.reload(tab.id)
            closePanel()
          }}
        >
          {t('site.reload')}
        </Button>
      )}

      {web && !state.incognito && (
        <>
          <div className="panel-section">
            <span className="label">{t('site.permissions')}</span>
            {info.permissions.length === 0 ? (
              <p className="panel-text muted">{t('site.noPermissions')}</p>
            ) : (
              info.permissions.map((p) => <PermissionRow key={p.kind} origin={info.origin} kind={p.kind} allow={p.allow} onChange={load} />)
            )}
            <button
              type="button"
              className="link-button"
              onClick={() => {
                closePanel()
                api.openPage('settings', 'permissions')
              }}
            >
              {t('site.allPermissions')}
            </button>
          </div>
          <div className="panel-section">
            <span className="label">{t('site.data')}</span>
            <div className="panel-row">
              <span className="muted">{t('site.cookies', { n: info.cookies })}</span>
              <Button
                variant="quiet"
                onClick={() =>
                  void ask({
                    title: t('site.clearConfirm', { site: info.host.replace(/^www\./, '') }),
                    body: t('site.clearBody'),
                    confirm: t('site.clearData'),
                    cancel: t('common.cancel'),
                    danger: true,
                  }).then((ok) => {
                    if (ok) void call.siteClearData().then(() => showToast({ text: t('site.cleared') }))
                  })
                }
              >
                {t('site.clearData')}
              </Button>
            </div>
          </div>
        </>
      )}
    </Popover>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt className="label">{label}</dt>
      <dd>{children}</dd>
    </>
  )
}

export function CertificateDialog({ host }: { host: string }) {
  const { closePanel } = useUI()
  const { settings } = useSnapshot()
  const t = useT()
  const [details, setDetails] = useState<CertificateDetails | null | undefined>(undefined)
  useEffect(() => {
    void call.certificate(host).then(setDetails)
  }, [host])

  const date = new Intl.DateTimeFormat(settings.language === 'tr' ? 'tr-TR' : 'en-US', { dateStyle: 'long', timeStyle: 'short' })
  const cert = details?.certificate
  const now = Date.now()
  const days = cert ? Math.floor((cert.validTo - now) / 86_400_000) : 0

  return (
    <Modal label={t('cert.title')} width={560} onClose={closePanel}>
      <div className="modal-head">
        <h2>
          {t('cert.title')} <span className="mono muted cert-host">{host}</span>
        </h2>
        <IconButton icon={X} label={t('common.close')} onClick={closePanel} />
      </div>
      <div className="modal-body cert">
        {details === null && <p className="muted">{t('cert.none')}</p>}
        {details && cert && (
          <>
            <div className={`site-state ${details.errorCode === 0 ? 'is-secure' : 'is-dangerous'}`}>
              <span className="site-state-icon">
                <Icon icon={details.errorCode === 0 ? Lock : ShieldAlert} size={18} />
              </span>
              <div>
                <div className="site-state-title">{details.errorCode === 0 ? t('cert.ok') : t(certificateReason(details.verification))}</div>
                <p className="panel-text muted">{details.knownRoot ? t('cert.knownRoot') : t('cert.addedRoot')}</p>
              </div>
            </div>
            <dl className="cert-fields">
              <Field label={t('cert.issuedTo')}>
                <span className="mono">{cert.subject}</span>
                {cert.subjectOrganization && <span className="muted">{cert.subjectOrganization}</span>}
              </Field>
              <Field label={t('cert.issuedBy')}>
                <span className="mono">{cert.issuer}</span>
                {cert.issuerOrganization && <span className="muted">{cert.issuerOrganization}</span>}
              </Field>
              <Field label={t('cert.validity')}>
                <span>
                  {date.format(cert.validFrom)} – {date.format(cert.validTo)}
                </span>
                <span className={`cert-days ${days < 0 || cert.validFrom > now ? 'is-bad' : ''}`}>
                  {cert.validFrom > now ? t('cert.notYet') : days < 0 ? t('cert.expired') : t('cert.daysLeft', { n: days })}
                </span>
              </Field>
              {cert.names.length > 0 && (
                <Field label={t('cert.names')}>
                  <span className="cert-names">
                    {cert.names.map((name) => (
                      <span key={name} className="mono">
                        {name}
                      </span>
                    ))}
                  </span>
                </Field>
              )}
              {cert.chain.length > 0 && (
                <Field label={t('cert.chain')}>
                  <ol className="cert-chain mono">
                    <li>{cert.subject}</li>
                    {cert.chain.map((link, index) => (
                      <li key={`${link.subject}-${index}`}>{link.subject}</li>
                    ))}
                  </ol>
                </Field>
              )}
            </dl>
            <details className="cert-details">
              <summary className="label">{t('cert.details')}</summary>
              <dl className="cert-fields">
                <Field label={t('cert.serial')}>
                  <span className="mono cert-hex">{cert.serialNumber}</span>
                </Field>
                {cert.keyType && (
                  <Field label={t('cert.key')}>
                    <span className="mono">{cert.keyType}</span>
                  </Field>
                )}
                {cert.signatureAlgorithm && (
                  <Field label={t('cert.signature')}>
                    <span className="mono">{cert.signatureAlgorithm}</span>
                  </Field>
                )}
                <Field label={t('cert.sha256')}>
                  <span className="mono cert-hex">{cert.sha256}</span>
                </Field>
                <Field label={t('cert.sha1')}>
                  <span className="mono cert-hex">{cert.sha1}</span>
                </Field>
              </dl>
            </details>
          </>
        )}
      </div>
    </Modal>
  )
}
