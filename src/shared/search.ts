import type { SearchEngineId } from './types'

export interface SearchEngine {
  id: SearchEngineId
  name: string
  search: string
  /** OpenSearch-style suggestion endpoint returning [query, [suggestions…]]. */
  suggest: string | null
}

export const SEARCH_ENGINES: Record<SearchEngineId, SearchEngine> = {
  duckduckgo: {
    id: 'duckduckgo',
    name: 'DuckDuckGo',
    search: 'https://duckduckgo.com/?q={q}',
    suggest: 'https://duckduckgo.com/ac/?q={q}&type=list',
  },
  brave: {
    id: 'brave',
    name: 'Brave Search',
    search: 'https://search.brave.com/search?q={q}',
    suggest: 'https://search.brave.com/api/suggest?q={q}',
  },
  startpage: {
    id: 'startpage',
    name: 'Startpage',
    search: 'https://www.startpage.com/do/search?q={q}',
    suggest: null,
  },
  google: {
    id: 'google',
    name: 'Google',
    search: 'https://www.google.com/search?q={q}',
    suggest: 'https://suggestqueries.google.com/complete/search?client=firefox&q={q}',
  },
  ecosia: {
    id: 'ecosia',
    name: 'Ecosia',
    search: 'https://www.ecosia.org/search?q={q}',
    suggest: 'https://ac.ecosia.org/autocomplete?q={q}&type=list',
  },
  bing: {
    id: 'bing',
    name: 'Bing',
    search: 'https://www.bing.com/search?q={q}',
    suggest: 'https://api.bing.com/osjson.aspx?query={q}',
  },
}

export function searchURL(engine: SearchEngineId, query: string): string {
  return SEARCH_ENGINES[engine].search.replace('{q}', encodeURIComponent(query))
}

export function suggestURL(engine: SearchEngineId, query: string): string | null {
  const template = SEARCH_ENGINES[engine].suggest
  return template ? template.replace('{q}', encodeURIComponent(query)) : null
}
