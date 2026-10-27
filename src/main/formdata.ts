// Autofill data: saved addresses and what was typed into form fields before, so
// fields can suggest it again. The whole file is encrypted with safeStorage like the
// passwords. Card numbers are never kept, whether typed or imported.
import { safeStorage } from 'electron'
import { randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import type { Address } from '@shared/types'
import { userPath } from './paths'
import { JsonFile } from './store'

export interface FormEntry {
  key: string
  value: string
  count: number
  lastUsed: number
}

interface FormFile {
  version: 1
  addresses: Address[]
  entries: FormEntry[]
}

const MAX_ENTRIES = 5000
const file = new JsonFile<{ data: string }>(userPath('formdata.json'), 1000)
let data: FormFile = { version: 1, addresses: [], entries: [] }

export function loadFormData(): void {
  try {
    const saved = JSON.parse(readFileSync(file.path, 'utf8')) as { data?: string }
    if (!saved.data) return
    const parsed = JSON.parse(safeStorage.decryptString(Buffer.from(saved.data, 'base64'))) as FormFile
    if (parsed.version === 1) data = { version: 1, addresses: parsed.addresses ?? [], entries: parsed.entries ?? [] }
  } catch {
    // Missing or unreadable: start empty.
  }
}

export function flushFormData(): void {
  file.flush()
}

function changed(): void {
  file.schedule({ data: safeStorage.encryptString(JSON.stringify(data)).toString('base64') })
}

/** A card number: 13–19 digits passing the Luhn check. Never stored. */
export function looksLikeCard(value: string): boolean {
  const digits = value.replace(/[\s-]/g, '')
  if (!/^\d{13,19}$/.test(digits)) return false
  let sum = 0
  for (let i = 0; i < digits.length; i++) {
    let d = Number(digits[digits.length - 1 - i])
    if (i % 2 === 1) {
      d *= 2
      if (d > 9) d -= 9
    }
    sum += d
  }
  return sum % 10 === 0
}

/** Values worth remembering for a field: short, not a card number, not a long token. */
export function rememberable(value: string): boolean {
  const v = value.trim()
  return v.length > 0 && v.length <= 200 && !looksLikeCard(v) && !/^[A-Za-z0-9_-]{32,}$/.test(v)
}

// ── Form entries ─────────────────────────────────────────────────────────────

export function suggestValues(key: string, prefix: string, limit = 6): string[] {
  const needle = prefix.trim().toLocaleLowerCase()
  return data.entries
    .filter((e) => e.key === key && e.value.toLocaleLowerCase().startsWith(needle) && e.value !== prefix)
    .sort((a, b) => b.count - a.count || b.lastUsed - a.lastUsed)
    .slice(0, limit)
    .map((e) => e.value)
}

export function rememberEntries(entries: { key: string; value: string }[], now = Date.now()): number {
  let added = 0
  for (const { key, value } of entries) {
    if (!key || !rememberable(value)) continue
    const clean = value.trim()
    const existing = data.entries.find((e) => e.key === key && e.value === clean)
    if (existing) {
      existing.count++
      existing.lastUsed = Math.max(existing.lastUsed, now)
    } else {
      data.entries.push({ key, value: clean, count: 1, lastUsed: now })
      added++
    }
  }
  if (data.entries.length > MAX_ENTRIES) data.entries = data.entries.sort((a, b) => b.lastUsed - a.lastUsed).slice(0, MAX_ENTRIES)
  changed()
  return added
}

/** Imported entries keep their counts and dates. */
export function importEntries(entries: FormEntry[]): number {
  let added = 0
  for (const entry of entries) {
    if (!entry.key || !rememberable(entry.value)) continue
    const existing = data.entries.find((e) => e.key === entry.key && e.value === entry.value.trim())
    if (existing) {
      existing.count = Math.max(existing.count, entry.count)
      existing.lastUsed = Math.max(existing.lastUsed, entry.lastUsed)
      continue
    }
    data.entries.push({ key: entry.key, value: entry.value.trim(), count: Math.max(1, entry.count), lastUsed: entry.lastUsed || Date.now() })
    added++
  }
  if (added > 0) changed()
  return added
}

export function countFormEntries(since = 0): number {
  return data.entries.filter((e) => e.lastUsed >= since).length
}

export function clearFormEntries(since = 0): void {
  data.entries = since > 0 ? data.entries.filter((e) => e.lastUsed < since) : []
  changed()
}

export function formEntries(): FormEntry[] {
  return data.entries.map((e) => ({ ...e }))
}

// ── Addresses ────────────────────────────────────────────────────────────────

export const ADDRESS_FIELDS: (keyof Omit<Address, 'id'>)[] = ['name', 'organization', 'street', 'city', 'state', 'postalCode', 'country', 'phone', 'email']

function cleanAddress(input: Partial<Address>): Omit<Address, 'id'> {
  const out = {} as Omit<Address, 'id'>
  for (const field of ADDRESS_FIELDS) out[field] = typeof input[field] === 'string' ? (input[field] as string).trim().slice(0, 300) : ''
  return out
}

export function listAddresses(): Address[] {
  return data.addresses.map((a) => ({ ...a }))
}

export function saveAddress(input: Partial<Address>): string | null {
  const clean = cleanAddress(input)
  if (ADDRESS_FIELDS.every((f) => !clean[f])) return null
  const existing = input.id ? data.addresses.find((a) => a.id === input.id) : undefined
  if (existing) Object.assign(existing, clean)
  else data.addresses.push({ id: randomUUID(), ...clean })
  changed()
  return existing?.id ?? data.addresses.at(-1)!.id
}

export function removeAddress(id: string): void {
  data.addresses = data.addresses.filter((a) => a.id !== id)
  changed()
}

/** Imported addresses; one that matches an existing address field for field is skipped. */
export function importAddresses(addresses: Partial<Address>[]): number {
  let added = 0
  for (const input of addresses) {
    const clean = cleanAddress(input)
    if (ADDRESS_FIELDS.every((f) => !clean[f])) continue
    if (data.addresses.some((a) => ADDRESS_FIELDS.every((f) => a[f] === clean[f]))) continue
    data.addresses.push({ id: randomUUID(), ...clean })
    added++
  }
  if (added > 0) changed()
  return added
}
