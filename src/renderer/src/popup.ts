// The autofill popup: a small view of its own placed under a form field, so the
// page stays clickable and keeps keyboard focus. The main process sends the items
// and the highlighted one; a click picks an item. No React here: it opens often.
import '@fontsource-variable/archivo/standard.css'
import '@fontsource/ibm-plex-mono/400.css'
import './styles/tokens.css'
import './styles/popup.css'
import type { AutofillPopupState } from '@shared/types'

const bridge = window.peepwick
const list = document.getElementById('list')!

const ICONS: Record<string, string> = {
  login: '<path d="M2.6 18.4A2 2 0 0 0 2 19.8V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.2a2 2 0 0 0 1.4-.6l.8-.8a6.5 6.5 0 1 0-4-4Z"/><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"/>',
  generate: '<path d="m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3Z"/>',
  value: '<path d="M12 8v4l3 3"/><circle cx="12" cy="12" r="9"/>',
  address: '<path d="M20 10c0 4.99-5.54 10.19-7.4 11.8a1 1 0 0 1-1.2 0C9.54 20.19 4 14.99 4 10a8 8 0 0 1 16 0"/><circle cx="12" cy="10" r="3"/>',
  manage: '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
}

function render(state: AutofillPopupState): void {
  document.documentElement.dataset.theme = state.dark ? 'dark' : 'light'
  document.documentElement.lang = state.lang
  list.replaceChildren(
    ...state.items.map((item, index) => {
      const li = document.createElement('li')
      li.className = `autofill-item is-${item.kind}${index === state.selected ? ' is-active' : ''}`
      li.setAttribute('role', 'option')
      li.setAttribute('aria-selected', String(index === state.selected))
      const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
      icon.setAttribute('viewBox', '0 0 24 24')
      icon.setAttribute('class', 'autofill-icon')
      icon.innerHTML = ICONS[item.kind] ?? ''
      const text = document.createElement('span')
      text.className = 'autofill-text'
      const label = document.createElement('span')
      label.className = 'autofill-label'
      label.textContent = item.label
      text.append(label)
      if (item.detail) {
        const detail = document.createElement('span')
        detail.className = 'autofill-detail'
        detail.textContent = item.detail
        text.append(detail)
      }
      li.append(icon, text)
      li.addEventListener('mouseenter', () => bridge.send('autofillHover', index))
      // mousedown, not click: picking must not wait for the page to lose focus.
      li.addEventListener('mousedown', (event) => {
        event.preventDefault()
        bridge.send('autofillPick', index)
      })
      return li
    }),
  )
}

bridge.on('autofill', (payload) => render(payload as AutofillPopupState))
