// Runs Bergamot (the translation engine Firefox uses, compiled to WebAssembly) in a
// worker thread. Started from source text by translation.ts, so it is plain
// CommonJS and receives the engine's glue code and binary in workerData.
// After bergamot-translator's worker/translator-worker.js (MPL-2.0).
;(() => {
  const { parentPort, workerData } = require('node:worker_threads')
  const vm = require('node:vm')

  const GEMM = {
    int8_prepare_a: 'int8PrepareAFallback',
    int8_prepare_b: 'int8PrepareBFallback',
    int8_prepare_b_from_transposed: 'int8PrepareBFromTransposedFallback',
    int8_prepare_b_from_quantized_transposed: 'int8PrepareBFromQuantizedTransposedFallback',
    int8_prepare_bias: 'int8PrepareBiasFallback',
    int8_multiply_and_add_bias: 'int8MultiplyAndAddBiasFallback',
    int8_select_columns_of_b: 'int8SelectColumnsOfBFallback',
  }

  // The glue code declares its own top-level names, so ours stay inside this function
  // and only Module is shared, through the global object.
  const Module = {}
  globalThis.Module = Module
  // The glue code expects to run in a web worker.
  globalThis.importScripts = () => {}
  globalThis.self = globalThis
  globalThis.self.location = { href: 'file:///bergamot/' }

  const ready = new Promise((resolve, reject) => {
    Module.instantiateWasm = (info, accept) => {
      const gemm = Object.fromEntries(Object.entries(GEMM).map(([name, fallback]) => [name, (...args) => Module.asm[fallback](...args)]))
      WebAssembly.instantiate(workerData.wasm, { ...info, wasm_gemm: gemm })
        .then(({ instance }) => accept(instance))
        .catch(reject)
      return {}
    }
    Module.onRuntimeInitialized = () => resolve()
    Module.onAbort = (what) => reject(new Error(String(what)))
    vm.runInThisContext(workerData.glue, { filename: 'bergamot-translator-worker.js' })
  })

  let service = null
  const models = new Map()

  function yaml(config) {
    return Object.entries(config)
      .map(([key, value]) => `${key}: ${value}`)
      .join('\n')
  }

  function aligned(buffer, alignment) {
    const bytes = new Int8Array(buffer)
    const memory = new Module.AlignedMemory(bytes.byteLength, alignment)
    memory.getByteArrayView().set(bytes)
    return memory
  }

  function load({ from, to, model, lex, vocabs: files }) {
    const key = `${from}-${to}`
    if (models.has(key)) return
    // One vocabulary shared by both languages, or one each (source, then target).
    const vocabs = new Module.AlignedMemoryList()
    for (const file of files) vocabs.push_back(aligned(file, 64))
    const config = yaml({
      'beam-size': 1,
      normalize: 1.0,
      'word-penalty': 0,
      'cpu-threads': 0,
      'gemm-precision': 'int8shiftAlphaAll',
      'skip-cost': true,
      alignment: 'soft',
      quiet: true,
      'quiet-translation': true,
      'max-length-break': 128,
      'mini-batch-words': 1024,
      workspace: 128,
      'max-length-factor': 2.0,
    })
    models.set(key, new Module.TranslationModel(config, aligned(model, 256), aligned(lex, 64), vocabs, null))
  }

  function translate({ pairs, texts, html }) {
    const input = new Module.VectorString()
    const options = new Module.VectorResponseOptions()
    for (const text of texts) {
      input.push_back(text)
      options.push_back({ alignment: false, html: !!html, qualityScores: false })
    }
    const chosen = pairs.map(({ from, to }) => {
      const model = models.get(`${from}-${to}`)
      if (!model) throw new Error(`model ${from}-${to} is not loaded`)
      return model
    })
    const responses = chosen.length > 1 ? service.translateViaPivoting(chosen[0], chosen[1], input, options) : service.translate(chosen[0], input, options)
    const result = texts.map((_, i) => responses.get(i).getTranslatedText())
    input.delete()
    options.delete()
    responses.delete()
    return result
  }

  parentPort.on('message', async ({ id, type, ...args }) => {
    try {
      await ready
      if (!service) service = new Module.BlockingService({ cacheSize: 1 << 13 })
      let result = null
      if (type === 'load') load(args)
      else if (type === 'translate') result = translate(args)
      else if (type === 'loaded') result = [...models.keys()]
      parentPort.postMessage({ id, result })
    } catch (error) {
      parentPort.postMessage({ id, error: error instanceof Error ? error.message : String(error) })
    }
  })
})()
