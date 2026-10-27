import { resolve } from 'node:path'
import { defineConfig } from 'electron-vite'
import react from '@vitejs/plugin-react'

const shared = resolve(__dirname, 'src/shared')

export default defineConfig({
  main: {
    resolve: { alias: { '@shared': shared } },
    build: {
      rollupOptions: {
        input: { index: resolve(__dirname, 'src/main/index.ts') },
      },
    },
  },
  preload: {
    resolve: { alias: { '@shared': shared } },
    build: {
      // Sandboxed preloads cannot require() packages, so everything is bundled
      // into a single CommonJS file.
      externalizeDeps: false,
      rollupOptions: {
        input: { ui: resolve(__dirname, 'src/preload/ui.ts'), page: resolve(__dirname, 'src/preload/page.ts') },
        output: { format: 'cjs', entryFileNames: '[name].cjs' },
      },
    },
  },
  renderer: {
    root: resolve(__dirname, 'src/renderer'),
    resolve: { alias: { '@shared': shared, '@ui': resolve(__dirname, 'src/renderer/src') } },
    plugins: [react()],
    build: {
      rollupOptions: {
        input: { index: resolve(__dirname, 'src/renderer/index.html'), popup: resolve(__dirname, 'src/renderer/popup.html') },
      },
    },
  },
})
