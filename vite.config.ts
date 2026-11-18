// The browser's own interface (React), served to the Tauri UI view. In development
// Tauri points the view at this dev server; for builds it loads dist-ui.
import { createReadStream, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import react from '@vitejs/plugin-react'
import { defineConfig, type Plugin } from 'vite'

const root = import.meta.dirname

// Bergamot, the translation engine (MPL-2.0), runs in a worker of translator.html. Its three
// files load each other by name, so they go next to each other under /bergamot/ unchanged.
const BERGAMOT = resolve(root, 'node_modules/@browsermt/bergamot-translator/worker')
const ENGINE_FILES = ['translator-worker.js', 'bergamot-translator-worker.js', 'bergamot-translator-worker.wasm']

function bergamot(): Plugin {
  return {
    name: 'bergamot',
    configureServer(server) {
      server.middlewares.use('/bergamot/', (request, response, next) => {
        const name = (request.url ?? '').slice(1).split('?')[0]
        if (!ENGINE_FILES.includes(name)) return next()
        response.setHeader('Content-Type', name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript')
        createReadStream(join(BERGAMOT, name)).pipe(response)
      })
    },
    generateBundle() {
      for (const name of ENGINE_FILES) {
        this.emitFile({ type: 'asset', fileName: `bergamot/${name}`, source: readFileSync(join(BERGAMOT, name)) })
      }
    },
  }
}

export default defineConfig({
  root: resolve(root, 'src/renderer'),
  resolve: { alias: { '@shared': resolve(root, 'src/shared'), '@ui': resolve(root, 'src/renderer/src') } },
  plugins: [react(), bergamot()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    outDir: resolve(root, 'dist-ui'),
    emptyOutDir: true,
    target: 'es2023',
    rollupOptions: {
      input: {
        index: resolve(root, 'src/renderer/index.html'),
        popup: resolve(root, 'src/renderer/popup.html'),
        translator: resolve(root, 'src/renderer/translator.html'),
      },
    },
  },
})
