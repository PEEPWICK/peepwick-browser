// Global Privacy Control (https://privacycg.github.io/gpc-spec/): pages that look for the
// signal in script find navigator.globalPrivacyControl, alongside the Sec-GPC header that goes
// with every request (webext/peepwick-webext.c). Consent tools read it here to opt people out
// of the sale and sharing of their data.
//
// Runs in the page's own world before any of its scripts, while the setting is on (a WebKit
// user script, from src/pagescripts.rs).
;(function () {
  'use strict'
  if ('globalPrivacyControl' in Navigator.prototype) return
  // A getter like the built-in ones: on the prototype, named, with nothing to set.
  var getter = Object.getOwnPropertyDescriptor(
    {
      get globalPrivacyControl() {
        return true
      },
    },
    'globalPrivacyControl',
  ).get
  Object.defineProperty(Navigator.prototype, 'globalPrivacyControl', {
    get: getter,
    enumerable: true,
    configurable: true,
  })
})()
