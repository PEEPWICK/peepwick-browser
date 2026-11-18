// The translation engine's page. It is never shown: the browser (src-tauri/src/translation.rs)
// keeps it in a view of its own, sends requests through window.__pwTranslator and gets the
// answers through the peepwickTranslator message handler. Bergamot runs in a worker; model
// files come from the browser's peepwick-model:// scheme. Nothing here touches the network.

interface Request {
  id: number
  op: 'load' | 'translate'
  from?: string
  to?: string
  /** For "load": the model's files, the model first, then the lexical shortlist, then the vocabularies. */
  urls?: string[]
  /** For "translate": one model, or two when going through English. */
  pairs?: { from: string; to: string }[]
  texts?: string[]
  /** For "translate": the texts are markup (a page's runs, with numbered elements), not plain text. */
  html?: boolean
}

interface Handler {
  postMessage(message: string): void
}

const tell = (message: unknown) =>
  (window as unknown as { webkit: { messageHandlers: { peepwickTranslator: Handler } } }).webkit.messageHandlers.peepwickTranslator.postMessage(
    JSON.stringify(message),
  )

const worker = new Worker(new URL('/bergamot/translator-worker.js', location.href))
let next = 1
const waiting = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>()

worker.addEventListener('message', ({ data }: MessageEvent<{ id: number; result?: unknown; error?: { message?: string } }>) => {
  const call = waiting.get(data.id)
  waiting.delete(data.id)
  if (data.error) call?.reject(new Error(data.error.message ?? 'engine error'))
  else call?.resolve(data.result)
})
worker.addEventListener('error', (event) => {
  for (const call of waiting.values()) call.reject(new Error(event.message || 'engine stopped'))
  waiting.clear()
})

function call<T>(name: string, ...args: unknown[]): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const id = next++
    waiting.set(id, { resolve: resolve as (value: unknown) => void, reject })
    worker.postMessage({ id, name, args })
  })
}

let started: Promise<unknown> | null = null
const loaded = new Set<string>()

async function file(url: string): Promise<ArrayBuffer> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`model file ${response.status}`)
  return response.arrayBuffer()
}

async function handle(request: Request): Promise<unknown> {
  started ??= call('initialize', { useNativeIntGemm: false, cacheSize: 1 << 13 })
  await started
  if (request.op === 'load') {
    const key = `${request.from}-${request.to}`
    if (loaded.has(key)) return null
    const [model, shortlist, ...vocabs] = await Promise.all((request.urls ?? []).map(file))
    await call('loadTranslationModel', { from: request.from, to: request.to }, { model, shortlist, vocabs })
    loaded.add(key)
    return null
  }
  const texts = request.texts ?? []
  if (texts.length === 0) return []
  const out = await call<{ target: { text: string } }[]>('translate', {
    models: request.pairs ?? [],
    texts: texts.map((text) => ({ text, html: request.html === true })),
  })
  return out.map((response) => response.target.text)
}

;(window as unknown as { __pwTranslator: (request: Request) => void }).__pwTranslator = (request) => {
  handle(request).then(
    (result) => tell({ id: request.id, result }),
    (error: unknown) => tell({ id: request.id, error: error instanceof Error ? error.message : String(error) }),
  )
}
tell({ ready: true })
