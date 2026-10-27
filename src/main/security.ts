// Connection security for the site panel in the address bar. Every TLS certificate
// Chromium verifies passes through here on its way: we note the result for the host
// and keep Chromium's own verdict (callback -3), so what the panel shows is what
// the connection actually got, not a guess from the URL scheme.
import { X509Certificate } from 'node:crypto'
import type { Certificate, Session } from 'electron'
import type { CertificateInfo, SiteInfo, TabSecurity } from '@shared/types'
import { hostOf, originOf, securityOf } from '@shared/url'
import { bus } from './events'
import { listPermissions } from './permissions'
import { siteOf } from './shield'

interface Verification {
  errorCode: number
  verification: string
  knownRoot: boolean
  certificate: Certificate
}

const LIMIT = 500
const results = new Map<string, Verification>()
/** Hosts whose certificate warning the person chose to skip, this session only. */
const exceptions = new Set<string>()

export function watchCertificates(session: Session): void {
  session.setCertificateVerifyProc((request, callback) => {
    const host = request.hostname.toLowerCase()
    const first = !results.has(host)
    results.delete(host)
    results.set(host, {
      errorCode: request.errorCode,
      verification: request.verificationResult,
      knownRoot: request.isIssuedByKnownRoot,
      certificate: request.validatedCertificate ?? request.certificate,
    })
    if (results.size > LIMIT) results.delete(results.keys().next().value as string)
    // A tab may be waiting on this to stop showing "unverified".
    if (first) bus.emit('security')
    callback(-3)
  })
}

export function allowCertificate(host: string): void {
  exceptions.add(host)
}

export function certificateAllowed(host: string): boolean {
  return exceptions.has(host)
}

export function revokeCertificateException(host: string): void {
  exceptions.delete(host)
}

export function verificationFor(host: string): Verification | null {
  return results.get(host.toLowerCase()) ?? null
}

export function securityFor(url: string): TabSecurity {
  const base = securityOf(url)
  if (base !== 'secure') return base
  const host = hostOf(url)
  const result = verificationFor(host)
  if (exceptions.has(host) || (result && result.errorCode !== 0)) return 'dangerous'
  return result ? 'secure' : 'unverified'
}

// ── Reading certificates ──────────────────────────────────────────────────────

const colonHex = (hex: string): string => hex.replace(/:/g, '').toUpperCase().match(/.{1,2}/g)?.join(':') ?? hex

function parse(pem: string): X509Certificate | null {
  try {
    return new X509Certificate(pem)
  } catch {
    return null
  }
}

function keyType(x509: X509Certificate | null): string {
  const key = x509?.publicKey
  if (!key) return ''
  const details = key.asymmetricKeyDetails
  switch (key.asymmetricKeyType) {
    case 'rsa':
      return details?.modulusLength ? `RSA ${details.modulusLength}` : 'RSA'
    case 'ec':
      return details?.namedCurve ? `ECDSA ${details.namedCurve}` : 'ECDSA'
    default:
      return (key.asymmetricKeyType ?? '').toUpperCase()
  }
}

/** subjectAltName "DNS:a.com, DNS:*.a.com, IP Address:1.2.3.4" → names. */
export function altNames(value: string | undefined): string[] {
  if (!value) return []
  return value
    .split(/,\s*/)
    .map((part) => part.replace(/^(DNS|IP Address):/, '').trim())
    .filter(Boolean)
    .slice(0, 200)
}

export function certificateInfo(certificate: Certificate): CertificateInfo {
  const x509 = parse(certificate.data)
  const chain: CertificateInfo['chain'] = []
  let issuer = certificate.issuerCert
  for (let depth = 0; issuer && depth < 6; depth++) {
    chain.push({
      subject: issuer.subject.commonName || issuer.subjectName,
      issuer: issuer.issuer.commonName || issuer.issuerName,
      validTo: issuer.validExpiry * 1000,
    })
    // A self-signed root points at itself.
    if (issuer.issuerCert === issuer || issuer.subjectName === issuer.issuerName) break
    issuer = issuer.issuerCert
  }
  return {
    subject: certificate.subject.commonName || certificate.subjectName,
    subjectOrganization: certificate.subject.organizations.join(', '),
    issuer: certificate.issuer.commonName || certificate.issuerName,
    issuerOrganization: certificate.issuer.organizations.join(', '),
    validFrom: certificate.validStart * 1000,
    validTo: certificate.validExpiry * 1000,
    names: altNames(x509?.subjectAltName),
    serialNumber: colonHex(x509?.serialNumber ?? certificate.serialNumber),
    sha256: x509?.fingerprint256 ?? '',
    sha1: x509?.fingerprint ?? '',
    keyType: keyType(x509),
    signatureAlgorithm: (x509 as (X509Certificate & { signatureAlgorithm?: string }) | null)?.signatureAlgorithm ?? '',
    chain,
  }
}

// ── The site panel ───────────────────────────────────────────────────────────

export async function siteInfo(url: string, session: Session): Promise<SiteInfo> {
  const host = hostOf(url)
  const origin = originOf(url)
  const result = /^https:/.test(url) ? verificationFor(host) : null
  let cookies = 0
  if (/^https?:/.test(url)) {
    try {
      cookies = (await session.cookies.get({ url })).length
    } catch {
      cookies = 0
    }
  }
  return {
    url,
    host,
    origin,
    security: securityFor(url),
    verification: result?.verification ?? null,
    errorCode: result?.errorCode ?? 0,
    knownRoot: result?.knownRoot ?? false,
    exception: exceptions.has(host),
    certificate: result ? certificateInfo(result.certificate) : null,
    permissions: listPermissions().filter((p) => p.origin === origin),
    cookies,
  }
}

/** Deletes the cookies and storage of the site (registrable domain) the URL is on. */
export async function clearSiteData(url: string, session: Session): Promise<void> {
  const site = siteOf(url)
  if (!site) return
  const cookies = await session.cookies.get({ domain: site })
  await Promise.all(
    cookies.map((cookie) => {
      const domain = (cookie.domain ?? site).replace(/^\./, '')
      return session.cookies.remove(`${cookie.secure ? 'https' : 'http'}://${domain}${cookie.path ?? '/'}`, cookie.name).catch(() => {})
    }),
  )
  const origins = new Set([originOf(url), `https://${site}`, `https://www.${site}`])
  for (const origin of origins) await session.clearStorageData({ origin }).catch(() => {})
}
