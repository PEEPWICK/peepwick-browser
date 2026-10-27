// What the address bar and the command bar ("Bak", Ctrl+K) understand: URLs,
// searches, open tabs, history and commands.
import { net } from 'electron'
import { translate, type MessageKey } from '@shared/i18n'
import { resolveInput } from '@shared/input'
import { SEARCH_ENGINES, searchURL, suggestURL } from '@shared/search'
import type { CommandId, Lang, SearchEngineId, Suggestion } from '@shared/types'

export interface SuggestContext {
  lang: Lang
  engine: SearchEngineId
  remoteSuggestions: boolean
  tabs: { id: number; title: string; url: string; favicon: string | null }[]
  bookmarks: { title: string; url: string; favicon: string | null }[]
  pages: { url: string; title: string; visits: number; lastVisit: number; favicon: string | null }[]
}

function normalizeForMatch(text: string): string {
  return text.toLocaleLowerCase('tr').replace(/^https?:\/\/(www\.)?/, '')
}

export async function suggest(input: string, context: SuggestContext): Promise<Suggestion[]> {
  const text = input.trim()
  if (!text) return []
  const needle = normalizeForMatch(text)
  const out: Suggestion[] = []
  const resolved = resolveInput(text, context.engine)
  const isSearch = resolved === searchURL(context.engine, text)

  // Inline-style best match: a visited page whose host starts with the input.
  const hostMatch = !isSearch || text.includes(' ')
    ? null
    : context.pages.find((page) => normalizeForMatch(page.url).startsWith(needle))
  if (hostMatch) out.push({ kind: 'history', title: hostMatch.title || hostMatch.url, value: hostMatch.url, favicon: hostMatch.favicon })

  if (isSearch) {
    out.push({
      kind: 'search',
      title: text,
      value: text,
    })
  } else if (!hostMatch || hostMatch.url !== resolved) {
    out.push({ kind: 'url', title: resolved, value: resolved })
  }

  for (const bookmark of context.bookmarks) {
    if (out.length >= 6) break
    if (out.some((s) => s.value === bookmark.url)) continue
    out.push({ kind: 'bookmark', title: bookmark.title, value: bookmark.url, favicon: bookmark.favicon })
  }

  for (const tab of context.tabs) {
    if (out.length >= 9) break
    if (normalizeForMatch(tab.title).includes(needle) || normalizeForMatch(tab.url).includes(needle)) {
      out.push({ kind: 'tab', title: tab.title, value: tab.url, favicon: tab.favicon, tabId: tab.id })
    }
  }

  const now = Date.now()
  const ranked = context.pages
    .filter((page) => page !== hostMatch)
    .map((page) => {
      const url = normalizeForMatch(page.url)
      const prefix = url.startsWith(needle) ? 4 : 0
      const recency = 1 / (1 + (now - page.lastVisit) / 86_400_000 / 3)
      return { page, score: prefix + Math.log2(1 + page.visits) + recency }
    })
    .sort((a, b) => b.score - a.score)
  for (const { page } of ranked) {
    if (out.length >= 8) break
    if (out.some((s) => s.value === page.url)) continue
    out.push({ kind: 'history', title: page.title || page.url, value: page.url, favicon: page.favicon })
  }

  if (context.remoteSuggestions && isSearch) {
    const remote = await fetchSuggestions(context.engine, text)
    for (const phrase of remote) {
      if (out.length >= 10) break
      if (phrase.toLocaleLowerCase('tr') === text.toLocaleLowerCase('tr')) continue
      out.push({ kind: 'search', title: phrase, value: phrase })
    }
  }
  return out
}

async function fetchSuggestions(engine: SearchEngineId, query: string): Promise<string[]> {
  const url = suggestURL(engine, query)
  if (!url) return []
  try {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), 1200)
    const response = await net.fetch(url, { signal: controller.signal, credentials: 'omit' } as RequestInit)
    clearTimeout(timeout)
    if (!response.ok) return []
    const data = (await response.json()) as unknown
    if (Array.isArray(data) && Array.isArray(data[1])) {
      return (data[1] as unknown[]).filter((s): s is string => typeof s === 'string').slice(0, 5)
    }
  } catch {
    // Offline or slow: suggestions are a nicety.
  }
  return []
}

export const COMMANDS: CommandId[] = [
  'new-tab',
  'new-window',
  'new-incognito',
  'focus-start',
  'focus-stop',
  'reader',
  'split',
  'unsplit',
  'pin',
  'toggle-sidebar',
  'downloads',
  'history',
  'extensions',
  'settings',
  'bookmark-page',
  'bookmark-tabs',
  'bookmarks',
  'toggle-bookmarks-bar',
  'import-bookmarks',
  'clear-data',
  'passwords',
  'reading-add',
  'reading-list',
  'translate',
  'backup',
  'report',
  'task-manager',
  'check-updates',
]

export function commandSuggestions(input: string, lang: Lang, available: (id: CommandId) => boolean): Suggestion[] {
  const needle = input.trim().toLocaleLowerCase(lang === 'tr' ? 'tr' : 'en')
  return COMMANDS.filter(available)
    .map((id) => ({ id, title: translate(lang, `cmd.${id}` as MessageKey) }))
    .filter(({ title }) => !needle || title.toLocaleLowerCase(lang === 'tr' ? 'tr' : 'en').includes(needle))
    .map(({ id, title }) => ({ kind: 'command' as const, title, value: id, command: id }))
}

export function engineName(engine: SearchEngineId): string {
  return SEARCH_ENGINES[engine].name
}
