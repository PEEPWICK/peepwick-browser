// The forms script: runs in a tab's main frame, in a script world of its own that the page's
// scripts can't reach (the "forms" world of peepwick-webext.c, which embeds this file at build
// time). It tells the browser when a sign-in or other form field gets focus or loses it, what
// is typed into a field while the browser's list is under it, the keys that move through that
// list, and forms being sent (a password the browser may offer to save, and the values it may
// remember). It fills in what the browser sends back. It never decides what to fill: the
// browser checks the page's address, which comes from WebKit, and sends the values.
;(() => {
  const send = globalThis.__peepwickFormsSend
  delete globalThis.__peepwickFormsSend
  const tell = (type, payload) => send(type, JSON.stringify(payload === undefined ? null : payload))
  const TEXT_TYPES = new Set(['text', 'email', 'tel', 'url', 'number', 'search'])

  const visible = (el) => {
    const r = el.getBoundingClientRect()
    return r.width > 0 && r.height > 0 && getComputedStyle(el).visibility !== 'hidden'
  }
  const inputsIn = (scope) => [...scope.querySelectorAll('input')].filter((i) => !i.disabled && !i.readOnly && visible(i))
  const passwordFields = (scope) => inputsIn(scope).filter((i) => i.type === 'password')
  const scopeOf = (input) => input.form || document

  const usernameFor = (password) => {
    const inputs = inputsIn(scopeOf(password)).filter((i) => TEXT_TYPES.has(i.type))
    const tagged = inputs.find((i) => /\b(username|email)\b/.test(i.autocomplete))
    if (tagged) return tagged
    // Otherwise the last text field before the password.
    return inputs.filter((i) => i.compareDocumentPosition(password) & Node.DOCUMENT_POSITION_FOLLOWING).at(-1) || null
  }

  const classify = (input) => {
    if (input.type === 'password') {
      const several = passwordFields(scopeOf(input)).length >= 2
      return /new-password/.test(input.autocomplete) || several ? 'new-password' : 'password'
    }
    if (!TEXT_TYPES.has(input.type) || input.autocomplete === 'off' || /one-time-code|cc-/.test(input.autocomplete)) return null
    if (/\busername\b/.test(input.autocomplete)) return 'username'
    const passwords = passwordFields(scopeOf(input))
    if (passwords.length > 0 && usernameFor(passwords[0]) === input) return 'username'
    return 'text'
  }

  // What a field is called, for remembering its values: its autocomplete token, name or id.
  const fieldKey = (input) => {
    const token = input.autocomplete
      .split(/\s+/)
      .filter((t) => t && t !== 'on' && t !== 'off' && !t.startsWith('section-'))
      .at(-1)
    return (token || input.name || input.id || '').toLowerCase().slice(0, 100)
  }

  // Which part of an address a field asks for: its autocomplete token, else its name, id,
  // placeholder or label, in English or Turkish.
  const AUTOCOMPLETE = {
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
  const PATTERNS = [
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
  const addressField = (input) => {
    const token = input.autocomplete.split(/\s+/).at(-1) || ''
    if (AUTOCOMPLETE[token]) return AUTOCOMPLETE[token]
    if (/user|login|search|captcha|code|coupon/i.test(`${input.name} ${input.id}`)) return null
    const text = `${input.name} ${input.id} ${input.placeholder} ${input.getAttribute('aria-label') || ''}`.toLocaleLowerCase('tr')
    const found = PATTERNS.find(([, pattern]) => pattern.test(text))
    return found ? found[0] : null
  }

  const setValue = (input, value) => {
    // The prototype's setter, so frameworks that track the value see a real change.
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
  }
  const rectOf = (input) => {
    const r = input.getBoundingClientRect()
    return { x: r.left, y: r.top, width: r.width, height: r.height }
  }

  let active = null
  let popupOpen = false
  let popupSelected = false
  let inputTimer = 0
  let lastSubmit = ''

  document.addEventListener(
    'focusin',
    (event) => {
      const input = event.target
      if (!(input instanceof HTMLInputElement)) return
      const kind = classify(input)
      if (!kind) return
      active = input
      tell('focus', {
        kind,
        rect: rectOf(input),
        key: fieldKey(input),
        value: input.value.slice(0, 200),
        address: kind === 'text' ? addressField(input) : null,
      })
    },
    true,
  )
  document.addEventListener(
    'focusout',
    (event) => {
      if (event.target === active) tell('blur')
    },
    true,
  )
  // Typing narrows the list; what the browser fills in doesn't count.
  document.addEventListener(
    'input',
    (event) => {
      if (!active || event.target !== active || !event.isTrusted) return
      clearTimeout(inputTimer)
      const field = active
      inputTimer = setTimeout(() => tell('input', { value: field.value.slice(0, 200), rect: rectOf(field) }), 80)
    },
    true,
  )

  // A form being sent: a sign-in or a new password the browser may offer to save, and the
  // values of its text fields, which it may remember.
  const capture = (scope) => {
    const passwords = passwordFields(scope).filter((p) => p.value)
    const many = passwords.length >= 2
    // In change-password forms the new password is the second field.
    const password = passwords.length === 0 ? '' : many ? passwords[1].value : passwords[0].value
    const user = passwords.length > 0 ? usernameFor(passwords[0]) : null
    const username = user ? user.value : ''
    const entries = inputsIn(scope)
      .filter((i) => TEXT_TYPES.has(i.type) && i.type !== 'search' && i.value.trim() && i.autocomplete !== 'off' && !/cc-|one-time-code/.test(i.autocomplete))
      .map((i) => ({ key: fieldKey(i), value: i.value.slice(0, 500) }))
      .filter((e) => e.key)
      .slice(0, 50)
    const signature = [username, password, ...entries.map((e) => e.value)].join('\u0000')
    if ((!password && entries.length === 0) || signature === lastSubmit) return
    lastSubmit = signature
    setTimeout(() => {
      lastSubmit = ''
    }, 3000)
    tell('submit', { username, password, newPassword: many || passwords.some((p) => /new-password/.test(p.autocomplete)), entries })
  }

  document.addEventListener(
    'keydown',
    (event) => {
      if (!active || event.target !== active) return
      if (popupOpen && ['ArrowDown', 'ArrowUp', 'Escape', 'Tab'].includes(event.key)) {
        if (event.key !== 'Tab') event.preventDefault()
        tell('key', event.key)
        return
      }
      if (popupOpen && popupSelected && event.key === 'Enter') {
        event.preventDefault()
        tell('key', 'Enter')
        return
      }
      if (event.key === 'Enter') capture(scopeOf(active))
    },
    true,
  )
  document.addEventListener(
    'submit',
    (event) => {
      if (event.target instanceof HTMLFormElement) capture(event.target)
    },
    true,
  )
  document.addEventListener(
    'click',
    (event) => {
      const target = event.target instanceof Element ? event.target.closest('button, input[type=submit], input[type=button], [role=button]') : null
      if (!target) return
      const scope = target.closest('form') || document
      if (passwordFields(scope).some((p) => p.value)) capture(scope)
    },
    true,
  )
  // The list under a field follows it; when it moves, the browser closes the list.
  addEventListener('scroll', () => popupOpen && tell('blur'), true)
  addEventListener('resize', () => popupOpen && tell('blur'))

  globalThis.__peepwickFormsReceive = (json) => {
    const message = JSON.parse(json)
    if (message.type === 'popup') {
      popupOpen = message.open === true
      popupSelected = message.selected === true
      return
    }
    const field = active
    if (!field || !field.isConnected) return
    const scope = scopeOf(field)
    if (message.type === 'login') {
      const password = field.type === 'password' ? field : passwordFields(scope)[0]
      const user = password ? usernameFor(password) : field
      if (user && message.username) setValue(user, message.username)
      if (password && typeof message.password === 'string') setValue(password, message.password)
    } else if (message.type === 'generated' && typeof message.password === 'string') {
      // Every password field of a sign-up form: the password and its confirmation.
      for (const p of passwordFields(scope).filter((p) => !/current-password/.test(p.autocomplete))) setValue(p, message.password)
    } else if (message.type === 'value' && typeof message.value === 'string') {
      setValue(field, message.value)
    } else if (message.type === 'address' && message.address && typeof message.address === 'object') {
      // Each field of the form that asks for part of an address, unless it's filled already.
      for (const input of inputsIn(scope).filter((i) => TEXT_TYPES.has(i.type))) {
        const part = addressField(input)
        const value = part && message.address[part]
        if (typeof value === 'string' && value && (!input.value || input === field)) setValue(input, value)
      }
    }
    field.focus()
  }
})()
