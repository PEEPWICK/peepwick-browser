/// <reference lib="dom" />
// Runs in every web page, in an isolated world the page's own scripts can't reach.
// It notices sign-in and other form fields, asks the browser for suggestions when
// one gets focus, fills in what the person picks from the browser's own popup, and
// reports a sign-in so the browser can offer to save the password. It never decides
// what to fill: the main process checks the page's origin and sends the values.
import { ipcRenderer, webFrame } from 'electron'

const CHANNEL = 'peepwick:autofill'
const TEXT_TYPES = new Set(['text', 'email', 'tel', 'url', 'number', 'search'])

type FieldKind = 'username' | 'password' | 'new-password' | 'text'
type AddressField = 'name' | 'organization' | 'street' | 'city' | 'state' | 'postalCode' | 'country' | 'phone' | 'email'

function visible(el: HTMLElement): boolean {
  const r = el.getBoundingClientRect()
  return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden'
}

function inputsIn(scope: ParentNode): HTMLInputElement[] {
  return [...scope.querySelectorAll('input')].filter((i) => !i.disabled && !i.readOnly && visible(i))
}

const passwordFields = (scope: ParentNode) => inputsIn(scope).filter((i) => i.type === 'password')
const scopeOf = (input: HTMLInputElement): ParentNode => input.form ?? document

function usernameFor(password: HTMLInputElement): HTMLInputElement | null {
  const inputs = inputsIn(scopeOf(password)).filter((i) => TEXT_TYPES.has(i.type))
  const tagged = inputs.find((i) => /\b(username|email)\b/.test(i.autocomplete))
  if (tagged) return tagged
  // Otherwise the last text field before the password.
  return inputs.filter((i) => i.compareDocumentPosition(password) & Node.DOCUMENT_POSITION_FOLLOWING).at(-1) ?? null
}

function classify(input: HTMLInputElement): FieldKind | null {
  if (input.type === 'password') {
    const several = passwordFields(scopeOf(input)).length >= 2
    return /new-password/.test(input.autocomplete) || several ? 'new-password' : 'password'
  }
  if (!TEXT_TYPES.has(input.type) || input.autocomplete === 'off' || /one-time-code|cc-/.test(input.autocomplete)) return null
  const passwords = passwordFields(scopeOf(input))
  if (/\busername\b/.test(input.autocomplete)) return 'username'
  if (passwords.length > 0 && usernameFor(passwords[0]) === input) return 'username'
  return 'text'
}

/** What a field is called, for remembering its values: autocomplete token, name or id. */
function fieldKey(input: HTMLInputElement): string {
  const token = input.autocomplete.split(/\s+/).filter((t) => t && !['on', 'off'].includes(t) && !t.startsWith('section-')).at(-1)
  return (token || input.name || input.id || '').toLowerCase().slice(0, 100)
}

const AUTOCOMPLETE: Record<string, AddressField> = {
  name: 'name',
  organization: 'organization',
  'street-address': 'street',
  'address-line1': 'street',
  'address-level2': 'city',
  'address-level1': 'state',
  'postal-code': 'postalCode',
  country: 'country',
  'country-name': 'country',
  tel: 'phone',
  'tel-national': 'phone',
  email: 'email',
}

const PATTERNS: [AddressField, RegExp][] = [
  ['email', /e-?mail|e-?posta/],
  ['phone', /phone|tel|telefon|gsm|\bcep\b/],
  ['postalCode', /zip|postal|posta.?kod|postcode/],
  ['country', /country|ülke|ulke/],
  ['state', /state|province|region|county|\bil\b|eyalet/],
  ['city', /city|town|şehir|sehir|ilçe|ilce/],
  ['street', /street|address|adres|addr/],
  ['organization', /company|organi[sz]ation|firma|şirket|sirket/],
  ['name', /full.?name|\bname\b|ad.?soyad|isim|\bad\b/],
]

function addressField(input: HTMLInputElement): AddressField | null {
  const token = input.autocomplete.split(/\s+/).at(-1) ?? ''
  if (AUTOCOMPLETE[token]) return AUTOCOMPLETE[token]
  if (/user|login|search|captcha|code|coupon/i.test(`${input.name} ${input.id}`)) return null
  const text = `${input.name} ${input.id} ${input.placeholder} ${input.getAttribute('aria-label') ?? ''}`.toLocaleLowerCase('tr')
  return PATTERNS.find(([, pattern]) => pattern.test(text))?.[0] ?? null
}

function setValue(input: HTMLInputElement, value: string): void {
  // The prototype's setter, so frameworks that track the value see a real change.
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(input, value)
  input.dispatchEvent(new Event('input', { bubbles: true }))
  input.dispatchEvent(new Event('change', { bubbles: true }))
}

function main(): void {
  if (window.top !== window) return
  let active: HTMLInputElement | null = null
  let popupOpen = false
  let popupSelected = false
  let inputTimer: ReturnType<typeof setTimeout> | undefined
  let lastSubmit = ''

  const send = (type: string, payload?: unknown) => ipcRenderer.send(CHANNEL, type, payload)

  const rectOf = (input: HTMLInputElement) => {
    const r = input.getBoundingClientRect()
    return { x: r.left, y: r.top, width: r.width, height: r.height }
  }

  document.addEventListener(
    'focusin',
    (event) => {
      const input = event.target
      if (!(input instanceof HTMLInputElement)) return
      const kind = classify(input)
      if (!kind) return
      active = input
      send('focus', {
        kind,
        rect: rectOf(input),
        zoom: webFrame.getZoomFactor(),
        key: fieldKey(input),
        value: input.value,
        address: kind === 'text' ? addressField(input) : null,
      })
    },
    true,
  )

  document.addEventListener(
    'focusout',
    (event) => {
      if (event.target === active) send('blur')
    },
    true,
  )

  document.addEventListener(
    'input',
    (event) => {
      if (event.target !== active || !active || !event.isTrusted) return
      clearTimeout(inputTimer)
      const field = active
      inputTimer = setTimeout(() => send('input', { value: field.value, rect: rectOf(field) }), 80)
    },
    true,
  )

  document.addEventListener(
    'keydown',
    (event) => {
      if (event.target !== active) return
      if (popupOpen && ['ArrowDown', 'ArrowUp', 'Escape', 'Tab'].includes(event.key)) {
        if (event.key !== 'Tab') event.preventDefault()
        send('key', event.key)
        return
      }
      if (popupOpen && popupSelected && event.key === 'Enter') {
        event.preventDefault()
        send('key', 'Enter')
        return
      }
      if (event.key === 'Enter') capture(scopeOf(active as HTMLInputElement))
    },
    true,
  )

  // Popups follow their field; when it moves, the browser closes them.
  window.addEventListener('scroll', () => popupOpen && send('blur'), true)
  window.addEventListener('resize', () => popupOpen && send('blur'))

  /** A sign-in or any form being sent: report the password (if any) and the field values. */
  function capture(scope: ParentNode): void {
    const passwords = passwordFields(scope).filter((p) => p.value)
    const many = passwords.length >= 2
    // In change-password forms the new password is the second field.
    const password = passwords.length === 0 ? '' : many ? passwords[1].value : passwords[0].value
    const username = passwords.length > 0 ? (usernameFor(passwords[0])?.value ?? '') : ''
    const entries = inputsIn(scope)
      .filter((i) => TEXT_TYPES.has(i.type) && i.type !== 'search' && i.value.trim() && i.autocomplete !== 'off' && !/cc-|one-time-code/.test(i.autocomplete))
      .map((i) => ({ key: fieldKey(i), value: i.value }))
      .filter((e) => e.key)
    const signature = `${username}\u0000${password}\u0000${entries.map((e) => e.value).join('\u0000')}`
    if ((!password && entries.length === 0) || signature === lastSubmit) return
    lastSubmit = signature
    setTimeout(() => (lastSubmit = ''), 3000)
    send('submit', { username, password, newPassword: many || passwords.some((p) => /new-password/.test(p.autocomplete)), entries })
  }

  document.addEventListener('submit', (event) => event.target instanceof HTMLFormElement && capture(event.target), true)
  document.addEventListener(
    'click',
    (event) => {
      const target = event.target instanceof Element ? event.target.closest('button, input[type=submit], input[type=button], [role=button]') : null
      if (!target) return
      const form = target.closest('form')
      const scope: ParentNode = form ?? document
      if (passwordFields(scope).some((p) => p.value)) capture(scope)
    },
    true,
  )

  ipcRenderer.on(CHANNEL, (_event, message: { type: string; [key: string]: unknown }) => {
    if (message.type === 'popup') {
      popupOpen = message.open === true
      popupSelected = message.selected === true
      return
    }
    const field = active
    if (!field || !field.isConnected) return
    const scope = scopeOf(field)
    if (message.type === 'login') {
      const passwords = passwordFields(scope)
      const password = field.type === 'password' ? field : passwords[0]
      const user = password ? usernameFor(password) : field.type !== 'password' ? field : null
      if (user && typeof message.username === 'string' && message.username) setValue(user, message.username)
      if (password && typeof message.password === 'string') setValue(password, message.password)
    } else if (message.type === 'generated' && typeof message.password === 'string') {
      // Every password field of a sign-up form: the password and its confirmation.
      for (const p of passwordFields(scope).filter((p) => !/current-password/.test(p.autocomplete))) setValue(p, message.password)
    } else if (message.type === 'value' && typeof message.value === 'string') {
      setValue(field, message.value)
    } else if (message.type === 'address' && message.address && typeof message.address === 'object') {
      const address = message.address as Record<AddressField, string>
      for (const input of inputsIn(scope).filter((i) => TEXT_TYPES.has(i.type))) {
        const kind = addressField(input)
        if (kind && address[kind] && (!input.value || input === field)) setValue(input, address[kind])
      }
    }
    field.focus()
  })
}

main()
