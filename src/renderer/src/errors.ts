// Errors in the browser's own interface go to the error log behind "Report a bug", as
// 0.1.x took them from the interface's console. The interface's own addresses lose their
// origin, so the place in its code stays; the Rust side scrubs every other address.
import { api } from './api'

/** At most this many a session: a fault that repeats shouldn't flood the log. */
const LIMIT = 100
let reported = 0

/** "fn@tauri://localhost/assets/index-abc.js:1:2" → "fn@assets/index-abc.js:1:2". */
const shorten = (text: string) => text.split(`${location.origin}/`).join('')

function describe(value: unknown): string {
  if (value instanceof Error) return value.stack ? `${value.name}: ${value.message}\n${value.stack}` : `${value.name}: ${value.message}`
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value) ?? String(value)
  } catch {
    return String(value)
  }
}

function report(message: string): void {
  // Sending the error itself failed: telling about that would only fail again.
  if (!message || reported >= LIMIT || message.startsWith('[bridge] uiError')) return
  reported++
  api.uiError(shorten(message).slice(0, 2000))
}

export function watchErrors(): void {
  window.addEventListener('error', (event) => {
    const file = shorten(event.filename ?? '')
    const place = file ? ` (${file}:${event.lineno})` : ''
    report(`${event.error instanceof Error ? describe(event.error) : event.message}${place}`)
  })
  window.addEventListener('unhandledrejection', (event) => report(`Unhandled rejection: ${describe(event.reason)}`))
  const consoleError = console.error.bind(console)
  console.error = (...args: unknown[]) => {
    consoleError(...args)
    report(args.map(describe).join(' '))
  }
}
