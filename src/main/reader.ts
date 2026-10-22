// Reader view: Mozilla Readability extracts the article inside an isolated world,
// and the result is shown in a separate view with JavaScript disabled, a strict CSP
// and its own in-memory session. Article HTML never reaches a privileged page.
import { WebContentsView, session as electronSession, type Session, type WebContents } from 'electron'
import { readFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import readabilitySource from '@mozilla/readability/Readability.js?raw'
import readerableSource from '@mozilla/readability/Readability-readerable.js?raw'
import { translate } from '@shared/i18n'
import { displayHost } from '@shared/url'
import type { Lang } from '@shared/types'
import { installNetwork } from './network'
import { resourcePath } from './paths'

const WORLD = 1001
const pages = new Map<string, string>()
let readerSession: Session | null = null

export interface Article {
  title: string
  content: string
  byline: string | null
  siteName: string | null
  length: number
  lang: string | null
  dir: string | null
}

export function setupReaderSession(): Session {
  if (readerSession) return readerSession
  const ses = electronSession.fromPartition('peepwick-reader')
  installNetwork(ses, 'reader')
  ses.setPermissionRequestHandler((_wc, _permission, callback) => callback(false))
  ses.protocol.handle('peepwick', async (request) => {
    const url = new URL(request.url)
    if (url.host !== 'reader') return new Response('Not found', { status: 404 })
    const font = /^\/fonts\/([\w.-]+\.woff2)$/.exec(url.pathname)
    if (font) {
      try {
        const data = await readFile(resourcePath('fonts', font[1]))
        return new Response(data, { headers: { 'content-type': 'font/woff2', 'cache-control': 'max-age=31536000' } })
      } catch {
        return new Response('Not found', { status: 404 })
      }
    }
    const html = pages.get(url.pathname.slice(1))
    if (!html) return new Response('Not found', { status: 404 })
    return new Response(html, {
      headers: {
        'content-type': 'text/html; charset=utf-8',
        'content-security-policy':
          "default-src 'none'; img-src https: http: data:; media-src https: http:; style-src 'unsafe-inline'; font-src 'self'; form-action 'none'; base-uri 'none'",
      },
    })
  })
  readerSession = ses
  return ses
}

export async function isReaderable(wc: WebContents): Promise<boolean> {
  try {
    const result = await wc.executeJavaScriptInIsolatedWorld(WORLD, [
      { code: `${readerableSource}\n;isProbablyReaderable(document, { minContentLength: 180, minScore: 30 })` },
    ])
    return result === true
  } catch {
    return false
  }
}

export async function extractArticle(wc: WebContents): Promise<Article | null> {
  try {
    const result = (await wc.executeJavaScriptInIsolatedWorld(WORLD, [
      {
        code: `${readabilitySource}
;(() => {
  const article = new Readability(document.cloneNode(true), { charThreshold: 300 }).parse()
  if (!article) return null
  return {
    title: article.title || document.title || '',
    content: article.content || '',
    byline: article.byline || null,
    siteName: article.siteName || null,
    length: (article.textContent || '').trim().split(/\\s+/).length,
    lang: article.lang || document.documentElement.lang || null,
    dir: article.dir || null,
  }
})()`,
      },
    ])) as Article | null
    return result && result.content ? result : null
  } catch {
    return null
  }
}

function escape(text: string): string {
  return text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`)
}

export function renderArticle(article: Article, sourceURL: string, uiLang: Lang, dark: boolean): string {
  const minutes = Math.max(1, Math.round(article.length / 200))
  const site = article.siteName || displayHost(sourceURL)
  const lang = article.lang && /^[a-z]{2,3}(-[a-z0-9]+)*$/i.test(article.lang) ? article.lang : uiLang
  const dir = article.dir === 'rtl' ? 'rtl' : 'ltr'
  const meta = `${escape(site)} · ${escape(translate(uiLang, 'reader.minutes', { n: minutes }))}`
  return `<!doctype html>
<html lang="${escape(lang)}" dir="${dir}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>${escape(article.title)}</title>
<style>
@font-face { font-family: Newsreader; src: url(/fonts/newsreader-latin-ext-standard-normal.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0100-02BA, U+1E00-1EFF; }
@font-face { font-family: Newsreader; src: url(/fonts/newsreader-latin-standard-normal.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }
@font-face { font-family: Newsreader; font-style: italic; src: url(/fonts/newsreader-latin-ext-standard-italic.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0100-02BA, U+1E00-1EFF; }
@font-face { font-family: Newsreader; font-style: italic; src: url(/fonts/newsreader-latin-standard-italic.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }
@font-face { font-family: Archivo; src: url(/fonts/archivo-latin-ext-standard-normal.woff2) format('woff2'); font-weight: 100 900; font-stretch: 62% 125%; unicode-range: U+0100-02BA, U+1E00-1EFF; }
@font-face { font-family: Archivo; src: url(/fonts/archivo-latin-standard-normal.woff2) format('woff2'); font-weight: 100 900; font-stretch: 62% 125%; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }
@font-face { font-family: 'Plex Mono'; src: url(/fonts/ibm-plex-mono-latin-ext-400-normal.woff2) format('woff2'); unicode-range: U+0100-02BA, U+1E00-1EFF; }
@font-face { font-family: 'Plex Mono'; src: url(/fonts/ibm-plex-mono-latin-400-normal.woff2) format('woff2'); unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }
:root {
  --bg: ${dark ? '#0B0B0C' : '#FFFFFF'};
  --text: ${dark ? '#F4F3F0' : '#0B0B0C'};
  --muted: ${dark ? '#8E8981' : '#3A3B3F'};
  --line: ${dark ? '#1D1E20' : '#E7E6E2'};
  --surface: ${dark ? '#1D1E20' : '#E7E6E2'};
  --fitil: #F2A93B;
  color-scheme: ${dark ? 'dark' : 'light'};
}
* { box-sizing: border-box; }
html { background: var(--bg); color: var(--text); -webkit-font-smoothing: antialiased; }
body { margin: 0; padding: 72px 24px 128px; font: 400 20px/1.65 Newsreader, Georgia, serif; }
main { max-width: 680px; margin: 0 auto; }
.meta { font: 400 12px/1.3 'Plex Mono', monospace; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); margin: 0 0 24px; display: flex; align-items: center; gap: 12px; }
.meta::before { content: ''; width: 14px; height: 6px; flex: none; background: radial-gradient(circle at 3px 3px, var(--fitil) 2.5px, transparent 3px), radial-gradient(circle at 11px 3px, var(--fitil) 2.5px, transparent 3px); }
h1.title { font-family: Archivo, sans-serif; font-weight: 700; font-stretch: 112%; font-size: 44px; line-height: 1.05; letter-spacing: -0.015em; margin: 0 0 16px; }
.byline { font-style: italic; color: var(--muted); margin: 0 0 48px; font-size: 18px; }
article h1, article h2, article h3, article h4 { font-family: Archivo, sans-serif; line-height: 1.2; letter-spacing: -0.01em; margin: 48px 0 16px; }
article h1 { font-size: 32px; font-weight: 700; font-stretch: 108%; }
article h2 { font-size: 28px; font-weight: 650; font-stretch: 108%; }
article h3 { font-size: 22px; font-weight: 600; }
article h4 { font-size: 19px; font-weight: 600; }
article p { margin: 0 0 24px; }
article a { color: inherit; text-decoration: underline; text-decoration-color: var(--muted); text-underline-offset: 3px; }
article a:hover { text-decoration-color: var(--fitil); }
article img, article video, article svg { max-width: 100%; height: auto; display: block; margin: 32px auto; }
article figure { margin: 32px 0; }
article figcaption { font: 400 14px/1.5 Archivo, sans-serif; color: var(--muted); margin-top: -16px; }
article blockquote { margin: 32px 0; padding: 0 0 0 24px; border-left: 2px solid var(--text); font-style: italic; font-size: 22px; line-height: 1.5; }
article pre, article code { font-family: 'Plex Mono', monospace; font-size: 15px; }
article pre { background: var(--surface); padding: 16px; overflow-x: auto; line-height: 1.5; border-radius: 4px; }
article :not(pre) > code { background: var(--surface); padding: 1px 4px; border-radius: 4px; }
article hr { border: 0; border-top: 1px solid var(--line); margin: 48px 0; }
article table { border-collapse: collapse; width: 100%; font: 400 15px/1.5 Archivo, sans-serif; margin: 32px 0; }
article th, article td { border-bottom: 1px solid var(--line); padding: 8px; text-align: start; }
article ul, article ol { padding-inline-start: 24px; margin: 0 0 24px; }
article li { margin: 0 0 8px; }
footer { margin-top: 64px; padding-top: 24px; border-top: 1px solid var(--line); font: 400 12px/1.3 'Plex Mono', monospace; letter-spacing: .08em; text-transform: uppercase; }
footer a { color: var(--muted); text-decoration: none; }
footer a:hover { color: var(--text); }
::selection { background: var(--fitil); color: #0B0B0C; }
</style>
</head>
<body>
<main>
<p class="meta">${meta}</p>
<h1 class="title">${escape(article.title)}</h1>
${article.byline ? `<p class="byline">${escape(article.byline)}</p>` : ''}
<article>${article.content}</article>
<footer><a href="${escape(sourceURL)}">${escape(displayHost(sourceURL))} →</a></footer>
</main>
</body>
</html>`
}

/** A reader view laid over one tab. */
export class ReaderView {
  readonly view: WebContentsView
  private readonly token = randomUUID()

  constructor(html: string, onNavigate: (url: string, newTab: boolean) => void, zoom: number) {
    pages.set(this.token, html)
    this.view = new WebContentsView({
      webPreferences: {
        session: setupReaderSession(),
        javascript: false,
        sandbox: true,
        contextIsolation: true,
        spellcheck: false,
        navigateOnDragDrop: false,
      },
    })
    const wc = this.view.webContents
    wc.on('will-navigate', (event, url) => {
      event.preventDefault()
      if (/^https?:/.test(url)) onNavigate(url, false)
    })
    wc.setWindowOpenHandler(({ url }) => {
      if (/^https?:/.test(url)) onNavigate(url, true)
      return { action: 'deny' }
    })
    wc.setZoomFactor(zoom)
    void wc.loadURL(`peepwick://reader/${this.token}`).catch(() => {})
  }

  get webContents(): WebContents {
    return this.view.webContents
  }

  destroy(): void {
    pages.delete(this.token)
    if (!this.view.webContents.isDestroyed()) this.view.webContents.close()
  }
}
