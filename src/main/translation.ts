// Page translation with Bergamot, the open translation engine Firefox's translations
// grew out of, on this computer. Models come from the Bergamot project's public
// model store, are downloaded once per language pair after the person agrees, and are
// checked against the SHA-256 the store lists for them. The page's text never leaves
// the browser. Languages without a direct model go through English.
import { net, type WebContents } from 'electron'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { mkdir, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { Worker } from 'node:worker_threads'
import type { TranslationModel } from '@shared/types'
import workerSource from './translation-worker.js?raw'
import { userPath } from './paths'

const STORE_URL = 'https://storage.googleapis.com/bergamot-models-sandbox/0.3.3/'
const REGISTRY_MAX_AGE = 7 * 24 * 60 * 60 * 1000
const PIVOT = 'en'
/** An isolated world for our own scripts in the page, away from the page's scripts. */
const WORLD = 1002
const IDLE_MS = 5 * 60 * 1000

type FileName = 'model' | 'lex' | 'vocab' | 'srcvocab' | 'trgvocab'
interface RemoteFile {
  name: FileName
  /** The file's name in the store, under the pair's folder. */
  path: string
  size: number
  /** The store keeps files gzip-compressed; this is about what goes over the wire. */
  download: number
  sha256: string
}
interface Pair {
  from: string
  to: string
  files: RemoteFile[]
}

const resolveModule = createRequire(import.meta.url).resolve
const root = () => userPath('translation')
const pairDir = (from: string, to: string) => join(root(), `${from}-${to}`)

// ── Model list ───────────────────────────────────────────────────────────────

let pairs: Promise<Pair[]> | null = null

/** The pairs of the store's registry with every file the engine needs. */
export function pickPairs(registry: unknown): Pair[] {
  const result: Pair[] = []
  if (!registry || typeof registry !== 'object') return result
  for (const [code, entry] of Object.entries(registry as Record<string, Record<string, unknown>>)) {
    if (!/^[a-z]{4}$/.test(code) || !entry || typeof entry !== 'object') continue
    const file = (name: FileName): RemoteFile | null => {
      const raw = entry[name] as { name?: unknown; size?: unknown; estimatedCompressedSize?: unknown; expectedSha256Hash?: unknown } | undefined
      if (!raw || typeof raw.name !== 'string' || !/^[\w.-]+$/.test(raw.name) || typeof raw.size !== 'number' || typeof raw.expectedSha256Hash !== 'string') return null
      const download = typeof raw.estimatedCompressedSize === 'number' ? raw.estimatedCompressedSize : raw.size
      return { name, path: `${code}/${raw.name}`, size: raw.size, download, sha256: raw.expectedSha256Hash.toLowerCase() }
    }
    const model = file('model')
    const lex = file('lex')
    const vocab = file('vocab')
    const src = file('srcvocab')
    const trg = file('trgvocab')
    if (!model || !lex || !(vocab || (src && trg))) continue
    const vocabs = vocab ? [vocab] : [src!, trg!]
    result.push({ from: code.slice(0, 2), to: code.slice(2), files: [model, lex, ...vocabs] })
  }
  return result.sort((a, b) => `${a.from}${a.to}`.localeCompare(`${b.from}${b.to}`))
}

async function loadPairs(): Promise<Pair[]> {
  const cache = join(root(), 'registry.json')
  const age = await stat(cache).then((s) => Date.now() - s.mtimeMs, () => Infinity)
  const cached = async () => pickPairs(JSON.parse(await readFile(cache, 'utf8')))
  if (age < REGISTRY_MAX_AGE) {
    try {
      return await cached()
    } catch {
      // Fetch again below.
    }
  }
  try {
    const response = await net.fetch(`${STORE_URL}registry.json`, { credentials: 'omit' } as RequestInit)
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const registry: unknown = await response.json()
    await mkdir(root(), { recursive: true })
    await writeFile(cache, JSON.stringify(registry))
    return pickPairs(registry)
  } catch (error) {
    // Offline: an old list is better than none.
    try {
      return await cached()
    } catch {
      throw error
    }
  }
}

function availablePairs(): Promise<Pair[]> {
  if (!pairs) {
    pairs = loadPairs()
    pairs.catch(() => (pairs = null))
  }
  return pairs
}

async function pairFor(from: string, to: string): Promise<Pair> {
  const pair = (await availablePairs()).find((p) => p.from === from && p.to === to)
  if (!pair) throw new Error('unsupported')
  return pair
}

/** Languages that can be translated from and into (through English when needed). */
export async function translationLanguages(): Promise<string[]> {
  const list = await availablePairs().catch(() => [] as Pair[])
  const langs = new Set<string>()
  for (const { from, to } of list) langs.add(from).add(to)
  for (const model of await downloadedModels()) langs.add(model.from).add(model.to)
  return [...langs].sort()
}

// The store's registry (0.3.3) no longer changes. Knowing its languages up front lets
// the address bar offer translation without contacting the store on every page.
const INTO_ENGLISH = ['bg', 'cs', 'de', 'es', 'et', 'fa', 'fr', 'is', 'it', 'nb', 'nl', 'nn', 'pl', 'pt', 'ru', 'uk']
const FROM_ENGLISH = ['bg', 'cs', 'de', 'es', 'et', 'fa', 'fr', 'it', 'nl', 'pl', 'pt', 'ru', 'uk']

/** The language to translate into: the interface language when there are models for it, else English. */
export function defaultTarget(ui: string): string {
  return FROM_ENGLISH.includes(ui) ? ui : PIVOT
}

export function canTranslate(from: string, to: string): boolean {
  const steps = route(from, to)
  return steps.length > 0 && steps.every((step) => (step.to === PIVOT ? INTO_ENGLISH.includes(step.from) : FROM_ENGLISH.includes(step.to)))
}

/** The models needed to go from one language to another. */
export function route(from: string, to: string): { from: string; to: string }[] {
  if (from === to) return []
  if (from === PIVOT || to === PIVOT) return [{ from, to }]
  return [
    { from, to: PIVOT },
    { from: PIVOT, to },
  ]
}

// ── Downloaded models ────────────────────────────────────────────────────────

/** The files of a downloaded model, from the list written when its download finished. */
async function modelFiles(from: string, to: string): Promise<string[] | null> {
  try {
    const files = JSON.parse(await readFile(join(pairDir(from, to), 'files.json'), 'utf8')) as string[]
    await Promise.all(files.map((file) => stat(join(pairDir(from, to), file))))
    return files
  } catch {
    return null
  }
}

export async function downloadedModels(): Promise<TranslationModel[]> {
  const names = await readdir(root()).catch(() => [] as string[])
  const models: TranslationModel[] = []
  for (const name of names) {
    const match = /^([a-z]{2,3})-([a-z]{2,3})$/.exec(name)
    const files = match ? await modelFiles(match[1], match[2]) : null
    if (!match || !files) continue
    let size = 0
    for (const file of files) size += (await stat(join(root(), name, file))).size
    models.push({ from: match[1], to: match[2], size })
  }
  return models
}

export async function removeModel(from: string, to: string): Promise<void> {
  if (!/^[a-z]{2,3}$/.test(from) || !/^[a-z]{2,3}$/.test(to)) return
  unloadAll()
  await rm(pairDir(from, to), { recursive: true, force: true })
}

/** The models a translation still needs; throws 'unsupported' when a pair has none. */
async function missing(from: string, to: string): Promise<Pair[]> {
  const needed: Pair[] = []
  for (const step of route(from, to)) {
    if (!(await modelFiles(step.from, step.to))) needed.push(await pairFor(step.from, step.to))
  }
  return needed
}

/** About how many bytes a translation would download first. */
export async function downloadSize(from: string, to: string): Promise<number> {
  return (await missing(from, to)).reduce((sum, pair) => sum + pair.files.reduce((n, f) => n + f.download, 0), 0)
}

async function downloadFile(file: RemoteFile, target: string, onBytes: (n: number) => void): Promise<void> {
  // The store sends files gzip-encoded; fetch hands us the original bytes.
  const response = await net.fetch(STORE_URL + file.path, { credentials: 'omit' } as RequestInit)
  if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`)
  const hash = createHash('sha256')
  const chunks: Uint8Array[] = []
  const reader = response.body.getReader()
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    hash.update(value)
    chunks.push(value)
    onBytes(value.byteLength)
  }
  if (hash.digest('hex') !== file.sha256) throw new Error('checksum')
  await writeFile(`${target}.part`, Buffer.concat(chunks))
  await rename(`${target}.part`, target)
}

/** Downloads what a translation needs, reporting progress from 0 to 1. */
export async function ensureModels(from: string, to: string, onProgress: (fraction: number) => void): Promise<void> {
  const needed = await missing(from, to)
  const total = needed.reduce((sum, pair) => sum + pair.files.reduce((n, f) => n + f.size, 0), 0)
  let received = 0
  for (const pair of needed) {
    const dir = pairDir(pair.from, pair.to)
    await mkdir(dir, { recursive: true })
    for (const file of pair.files) {
      await downloadFile(file, join(dir, file.name), (n) => {
        received += n
        onProgress(total > 0 ? Math.min(1, received / total) : 1)
      })
    }
    // Written last: a model counts as downloaded only once every file is in place.
    await writeFile(join(dir, 'files.json'), JSON.stringify(pair.files.map((f) => f.name)))
  }
}

// ── Engine ───────────────────────────────────────────────────────────────────

let worker: Worker | null = null
let loaded = new Set<string>()
let nextId = 0
const pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>()
let idleTimer: NodeJS.Timeout | null = null

function unloadAll(): void {
  if (worker) void worker.terminate()
  worker = null
  loaded = new Set()
}

function engine(): Worker {
  if (worker) return worker
  const dir = dirname(resolveModule('@browsermt/bergamot-translator/worker/translator-worker.js'))
  const glue = readFileSync(join(dir, 'bergamot-translator-worker.js'), 'utf8')
  const wasm = readFileSync(join(dir, 'bergamot-translator-worker.wasm'))
  const started = new Worker(workerSource, { eval: true, workerData: { glue, wasm } })
  started.on('message', ({ id, result, error }: { id: number; result?: unknown; error?: string }) => {
    const call = pending.get(id)
    pending.delete(id)
    if (error) call?.reject(new Error(error))
    else call?.resolve(result)
  })
  const fail = (error: Error) => {
    for (const call of pending.values()) call.reject(error)
    pending.clear()
    if (worker === started) {
      worker = null
      loaded = new Set()
    }
  }
  started.on('error', fail)
  started.on('exit', () => fail(new Error('translation engine stopped')))
  worker = started
  return started
}

function call<T>(type: string, args: Record<string, unknown>, transfer: ArrayBuffer[] = []): Promise<T> {
  const target = engine()
  if (idleTimer) clearTimeout(idleTimer)
  // The engine holds a few hundred megabytes; let it go when translation isn't in use.
  idleTimer = setTimeout(() => pending.size === 0 && unloadAll(), IDLE_MS)
  return new Promise<T>((resolve, reject) => {
    const id = ++nextId
    pending.set(id, { resolve: resolve as (value: unknown) => void, reject })
    target.postMessage({ id, type, ...args }, transfer)
  })
}

async function loadModel(from: string, to: string): Promise<void> {
  const key = `${from}-${to}`
  if (loaded.has(key)) return
  const files = await modelFiles(from, to)
  if (!files) throw new Error('unsupported')
  const read = async (name: string) => {
    const data = await readFile(join(pairDir(from, to), name))
    return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as ArrayBuffer
  }
  const model = await read('model')
  const lex = await read('lex')
  const vocabs = await Promise.all(files.filter((f) => f.endsWith('vocab')).map(read))
  await call('load', { from, to, model, lex, vocabs }, [model, lex, ...vocabs])
  loaded.add(key)
}

export async function translateTexts(from: string, to: string, texts: string[]): Promise<string[]> {
  const steps = route(from, to)
  if (steps.length === 0 || texts.length === 0) return texts
  for (const step of steps) await loadModel(step.from, step.to)
  return call<string[]>('translate', { pairs: steps, texts })
}

// ── The page ─────────────────────────────────────────────────────────────────

/** Language tags as pages write them, mapped to the model server's codes. */
export function normalizeLang(tag: string | null | undefined): string | null {
  const value = (tag ?? '').trim().replace(/_/g, '-').toLowerCase()
  if (!value) return null
  const [base] = value.split('-')
  if (base === 'zh') return 'zh'
  if (base === 'no') return 'nb'
  if (base === 'iw') return 'he'
  if (base === 'in') return 'id'
  return /^[a-z]{2,3}$/.test(base) ? base : null
}

const DETECT = `(() => document.documentElement.lang || document.querySelector('meta[http-equiv="content-language" i]')?.content || '')()`

export async function pageLanguage(contents: WebContents): Promise<string | null> {
  try {
    return normalizeLang((await contents.executeJavaScriptInIsolatedWorld(WORLD, [{ code: DETECT }])) as string)
  } catch {
    return null
  }
}

// Collects the page's visible text, one entry per text node, and keeps the nodes so
// translations can be put in place and taken out again.
const COLLECT = `(() => {
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEXTAREA', 'INPUT', 'SELECT', 'OPTION', 'CODE', 'PRE', 'KBD', 'SAMP', 'VAR', 'SVG', 'MATH', 'IFRAME', 'CANVAS', 'TEMPLATE'])
  const old = window.__pwTranslation
  if (old) old.nodes.forEach((node, i) => { node.data = old.originals[i] })
  const state = { nodes: [], originals: [], title: document.title }
  window.__pwTranslation = state
  const skip = new WeakMap()
  const skipped = (el) => {
    if (!el) return false
    if (skip.has(el)) return skip.get(el)
    const value = SKIP.has(el.tagName) || el.isContentEditable || el.getAttribute('translate') === 'no' || el.classList.contains('notranslate') || skipped(el.parentElement)
    skip.set(el, value)
    return value
  }
  const walker = document.createTreeWalker(document.body || document.documentElement, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => (/\\p{L}/u.test(node.data) && !skipped(node.parentElement) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT),
  })
  while (walker.nextNode()) {
    state.nodes.push(walker.currentNode)
    state.originals.push(walker.currentNode.data)
  }
  return { texts: state.originals, title: document.title }
})()`

const apply = (start: number, texts: (string | null)[]) => `((start, texts) => {
  const state = window.__pwTranslation
  if (!state) return
  texts.forEach((text, i) => {
    const node = state.nodes[start + i]
    const original = state.originals[start + i]
    if (!node || text === null) return
    node.data = original.match(/^\\s*/)[0] + text + original.match(/\\s*$/)[0]
  })
})(${start}, ${JSON.stringify(texts)})`

const setTitle = (title: string) => `(() => { if (window.__pwTranslation) document.title = ${JSON.stringify(title)} })()`

const RESTORE = `(() => {
  const state = window.__pwTranslation
  if (!state) return
  state.nodes.forEach((node, i) => { node.data = state.originals[i] })
  document.title = state.title
  delete window.__pwTranslation
})()`

/**
 * Translates a page in place, a block of text at a time so it changes while the
 * person watches. Returns false when `cancelled` turned true partway.
 */
export async function translatePage(contents: WebContents, from: string, to: string, onProgress: (fraction: number) => void, cancelled: () => boolean): Promise<boolean> {
  const run = (code: string) => contents.executeJavaScriptInIsolatedWorld(WORLD, [{ code }])
  const { texts, title } = (await run(COLLECT)) as { texts: string[]; title: string }
  const done = new Map<string, string>()
  if (title.trim()) {
    const [translated] = await translateTexts(from, to, [title.trim()])
    if (cancelled()) return false
    await run(setTitle(translated))
  }
  const BLOCK = 60
  for (let start = 0; start < texts.length; start += BLOCK) {
    const block = texts.slice(start, start + BLOCK).map((text) => text.replace(/\s+/g, ' ').trim())
    const missing = [...new Set(block.filter((text) => text && !done.has(text)))]
    if (missing.length > 0) {
      const translated = await translateTexts(from, to, missing)
      missing.forEach((text, i) => done.set(text, translated[i]))
    }
    if (cancelled() || contents.isDestroyed()) return false
    await run(apply(start, block.map((text) => (text ? (done.get(text) ?? null) : null))))
    onProgress(Math.min(1, (start + BLOCK) / Math.max(1, texts.length)))
  }
  return true
}

export async function restorePage(contents: WebContents): Promise<void> {
  await contents.executeJavaScriptInIsolatedWorld(WORLD, [{ code: RESTORE }]).catch(() => {})
}
