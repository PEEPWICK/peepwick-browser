// Favicons are fetched once through the tab's own session and kept as data URLs,
// so the browser UI never loads remote images itself.
import type { Session } from 'electron'
import { hostOf } from '@shared/url'
import { getFavicon, setFavicon } from './history'

const MAX_BYTES = 150_000
const cache = new Map<string, string>()

function remember(key: string, value: string): void {
  cache.set(key, value)
  if (cache.size > 300) cache.delete(cache.keys().next().value as string)
}

export async function resolveFavicon(
  session: Session,
  candidates: string[],
  pageURL: string,
  persist: boolean,
): Promise<string | null> {
  const url = candidates.find((c) => /^(https?|data):/.test(c))
  if (!url) return null
  if (url.startsWith('data:')) return url.length <= MAX_BYTES ? url : null
  const cached = cache.get(url)
  if (cached) return cached
  try {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), 6000)
    const response = await session.fetch(url, { signal: controller.signal, credentials: 'omit' } as RequestInit)
    clearTimeout(timeout)
    const type = response.headers.get('content-type')?.split(';')[0]?.trim() ?? ''
    if (!response.ok || !(type.startsWith('image/') || type === 'application/octet-stream')) return null
    const body = new Uint8Array(await response.arrayBuffer())
    if (body.byteLength === 0 || body.byteLength > MAX_BYTES) return null
    const mime = type.startsWith('image/') ? type : sniff(body)
    if (!mime) return null
    const data = `data:${mime};base64,${Buffer.from(body).toString('base64')}`
    remember(url, data)
    if (persist) setFavicon(hostOf(pageURL), data)
    return data
  } catch {
    return null
  }
}

function sniff(body: Uint8Array): string | null {
  if (body[0] === 0x89 && body[1] === 0x50) return 'image/png'
  if (body[0] === 0x00 && body[1] === 0x00 && body[2] === 0x01) return 'image/x-icon'
  if (body[0] === 0x47 && body[1] === 0x49) return 'image/gif'
  if (body[0] === 0xff && body[1] === 0xd8) return 'image/jpeg'
  if (body[0] === 0x3c) return 'image/svg+xml'
  return null
}

export function storedFavicon(url: string): string | null {
  return getFavicon(hostOf(url))
}
