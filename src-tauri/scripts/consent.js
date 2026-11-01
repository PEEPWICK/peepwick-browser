// Rejecting cookie banners: when a consent notice shows up, press its "reject" (or "only
// necessary") button. Known consent tools are matched by their buttons; other notices only
// when a visible dialog or fixed bar talks about cookies and has a button whose whole label
// is a refusal, so nothing else on a page is ever pressed.
//
// Runs in a world of its own in every frame (a WebKit user script, from src/pagescripts.rs).
// The browser puts its settings in place of __CONFIG__: { paused: sites with the shield off }.
;(function (config) {
  'use strict'

  var top = location.hostname
  try {
    var origins = location.ancestorOrigins
    if (origins && origins.length) top = new URL(origins[origins.length - 1]).hostname
  } catch (e) {}
  for (var p = 0; p < config.paused.length; p++) {
    var site = config.paused[p]
    if (top === site || top.slice(-site.length - 1) === '.' + site) return
  }

  // Reject buttons of common consent tools; pressed even when the shield hides the banner.
  var KNOWN = [
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

  // Shadow-DOM consent tools: host element and the reject button inside it.
  var SHADOW = [
    { host: '#usercentrics-root', button: '[data-testid="uc-deny-all-button"]' },
    { host: '#cmpwrapper', button: '.cmpboxbtnno' },
  ]

  var REJECT_LABEL = new RegExp(
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

  var ABOUT_COOKIES = /cookie|çerez|consent|gdpr|kvkk|tracking|izleme|datenschutz|confidentialité/i
  var BANNER_NAME = /cookie|consent|gdpr|kvkk|cmp|privacy|banner|notice/i

  function visible(el) {
    var r = el.getBoundingClientRect()
    if (r.width === 0 || r.height === 0) return false
    var style = getComputedStyle(el)
    return style.visibility !== 'hidden' && style.display !== 'none' && Number(style.opacity) > 0.05
  }

  function labelOf(el) {
    var text = el instanceof HTMLInputElement ? el.value : el.innerText || el.getAttribute('aria-label') || ''
    return text.replace(/\s+/g, ' ').trim()
  }

  // The banner around a button: a dialog, a fixed or sticky bar, or something named like one.
  function bannerOf(button) {
    for (var el = button.parentElement; el && el !== document.documentElement; el = el.parentElement) {
      var role = el.getAttribute('role')
      var position = getComputedStyle(el).position
      if (role === 'dialog' || role === 'alertdialog' || el.getAttribute('aria-modal') === 'true' || position === 'fixed' || position === 'sticky') return el
      if (BANNER_NAME.test(el.id + ' ' + (typeof el.className === 'string' ? el.className : ''))) return el
    }
    // Consent tools that load in their own frame fill it with the banner.
    return window.top !== window ? document.body : null
  }

  function findKnown() {
    for (var i = 0; i < KNOWN.length; i++) {
      var el = document.querySelector(KNOWN[i])
      if (el && el.isConnected) return el
    }
    for (var j = 0; j < SHADOW.length; j++) {
      var host = document.querySelector(SHADOW[j].host)
      var button = host && host.shadowRoot && host.shadowRoot.querySelector(SHADOW[j].button)
      if (button) return button
    }
    return null
  }

  function findByLabel() {
    var candidates = document.querySelectorAll('button, [role="button"], a, input[type="button"], input[type="submit"]')
    for (var i = 0; i < candidates.length; i++) {
      var el = candidates[i]
      var label = labelOf(el)
      if (label.length > 60 || !REJECT_LABEL.test(label) || !visible(el)) continue
      var banner = bannerOf(el)
      if (banner && ABOUT_COOKIES.test(banner.innerText || '')) return el
    }
    return null
  }

  var done = false
  var scheduled = false
  var observer = null

  function look() {
    scheduled = false
    if (done || !document.body) return
    var button = findKnown() || findByLabel()
    if (!button) return
    done = true
    if (observer) observer.disconnect()
    button.click()
  }

  function start() {
    look()
    if (done) return
    // Banners often arrive a moment after the page; watch the first half minute.
    observer = new MutationObserver(function () {
      if (scheduled) return
      scheduled = true
      setTimeout(look, 300)
    })
    observer.observe(document.documentElement, { childList: true, subtree: true })
    setTimeout(function () {
      if (observer) observer.disconnect()
    }, 30000)
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, { once: true })
  else start()
})(__CONFIG__)
