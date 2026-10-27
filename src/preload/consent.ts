/// <reference lib="dom" />
// Rejecting cookie banners: when a consent notice shows up, press its "reject" (or
// "only necessary") button. Known consent tools are matched by their buttons; other
// notices only when a visible dialog or fixed bar talks about cookies and has a
// button whose whole label is a refusal, so nothing else on a page is ever pressed.

/** Reject buttons of common consent tools; pressed even when the shield hides the banner. */
const KNOWN = [
  '#onetrust-reject-all-handler',
  '#CybotCookiebotDialogBodyButtonDecline',
  '#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll',
  '#didomi-notice-disagree-button',
  '#truste-consent-required',
  '.cky-btn-reject',
  '.cmplz-btn.cmplz-deny',
  '.osano-cm-denyAll',
  '.iubenda-cs-reject-btn',
  '.klaro .cm-btn-decline',
  '#cn-refuse-cookie',
  '[data-tid="banner-decline"]',
  'button.sp_choice_type_REJECT_ALL',
  '.fc-cta-do-not-consent',
  '#axeptio_btn_dismiss',
  '#ccc-notify-reject',
  '.cmpboxbtnno',
  '.moove-gdpr-infobar-reject-btn',
  '#shopify-pc__banner__btn-decline',
  '#_evidon-decline-button',
  '._brlbs-refuse-btn',
  '.cc-banner .cc-deny',
  '#cookiescript_reject',
  '.js-cookie-consent-reject',
]

/** Shadow-DOM consent tools: host element and the reject button inside it. */
const SHADOW = [{ host: '#usercentrics-root', button: '[data-testid="uc-deny-all-button"]' }, { host: '#cmpwrapper', button: '.cmpboxbtnno' }]

const REJECT_LABEL = new RegExp(
  '^(' +
    [
      // Turkish
      'reddet',
      'tümünü reddet',
      'hepsini reddet',
      'reddediyorum',
      'kabul etmiyorum',
      'kabul etme',
      'sadece gerekli(ler)?',
      'yalnızca gerekli(ler)?',
      'sadece zorunlu(lar)?',
      'yalnızca zorunlu(lar)?',
      '(sadece|yalnızca) (gerekli|zorunlu) çerezler(i kabul et)?',
      '(zorunlu|gerekli) olmayan(lar)?ı reddet',
      'isteğe bağlı çerezleri reddet',
      'çerezleri reddet',
      'tüm çerezleri reddet',
      // English
      'reject',
      'reject all',
      'reject all cookies',
      'reject cookies',
      'reject optional cookies',
      'reject non-essential( cookies)?',
      'decline',
      'decline all',
      'decline cookies',
      'decline optional cookies',
      'deny',
      'deny all',
      'refuse',
      'refuse all',
      'disagree',
      'i disagree',
      'i do not accept',
      'do not accept',
      '(use )?(only|strictly) (necessary|essential)( cookies)?( only)?',
      '(necessary|essential) (cookies )?only',
      'allow (only )?(necessary|essential)( cookies)?( only)?',
      'continue without accepting',
      // A few other common languages
      'alle ablehnen',
      'ablehnen',
      'tout refuser',
      'refuser',
      'rechazar( todo| todas)?',
      'rifiuta( tutto)?',
      'weigeren',
      'alles weigeren',
    ].join('|') +
    ')[.!]?$',
  'i',
)

const ABOUT_COOKIES = /cookie|çerez|consent|gdpr|kvkk|tracking|izleme|datenschutz|confidentialité/i
const BANNER_NAME = /cookie|consent|gdpr|kvkk|cmp|privacy|banner|notice/i

function visible(el: Element): boolean {
  const r = el.getBoundingClientRect()
  if (r.width === 0 || r.height === 0) return false
  const style = getComputedStyle(el)
  return style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) > 0.05
}

function labelOf(el: Element): string {
  const text = el instanceof HTMLInputElement ? el.value : (el as HTMLElement).innerText || el.getAttribute('aria-label') || ''
  return text.replace(/\s+/g, ' ').trim()
}

/** The banner around a button: a dialog, a fixed or sticky bar, or something named like one. */
function bannerOf(button: Element): Element | null {
  for (let el = button.parentElement; el && el !== document.documentElement; el = el.parentElement) {
    const role = el.getAttribute('role')
    const position = getComputedStyle(el).position
    if (role === 'dialog' || role === 'alertdialog' || el.getAttribute('aria-modal') === 'true' || position === 'fixed' || position === 'sticky') return el
    if (BANNER_NAME.test(`${el.id} ${typeof el.className === 'string' ? el.className : ''}`)) return el
  }
  // Consent tools that load in their own frame fill it with the banner.
  return window.top !== window ? document.body : null
}

function findKnown(): HTMLElement | null {
  for (const selector of KNOWN) {
    const el = document.querySelector<HTMLElement>(selector)
    if (el?.isConnected) return el
  }
  for (const { host, button } of SHADOW) {
    const el = document.querySelector(host)?.shadowRoot?.querySelector<HTMLElement>(button)
    if (el) return el
  }
  return null
}

function findByLabel(): HTMLElement | null {
  const candidates = document.querySelectorAll<HTMLElement>('button, [role="button"], a, input[type="button"], input[type="submit"]')
  for (const el of candidates) {
    const label = labelOf(el)
    if (label.length > 60 || !REJECT_LABEL.test(label) || !visible(el)) continue
    const banner = bannerOf(el)
    if (banner && ABOUT_COOKIES.test((banner as HTMLElement).innerText ?? '')) return el
  }
  return null
}

export function rejectCookieBanners(): void {
  let done = false
  let scheduled = false
  let observer: MutationObserver | null = null

  const look = () => {
    scheduled = false
    if (done || !document.body) return
    const button = findKnown() ?? findByLabel()
    if (!button) return
    done = true
    observer?.disconnect()
    button.click()
  }

  const start = () => {
    look()
    if (done) return
    // Banners often arrive a moment after the page; watch the first half minute.
    observer = new MutationObserver(() => {
      if (scheduled) return
      scheduled = true
      setTimeout(look, 300)
    })
    observer.observe(document.documentElement, { childList: true, subtree: true })
    setTimeout(() => observer?.disconnect(), 30_000)
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true })
  else start()
}
