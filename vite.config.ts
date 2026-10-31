// The browser's own interface (React), served to the Tauri UI view. In development
// Tauri points the view at this dev server; for builds it loads dist-ui.
import { resolve } from 'node:path'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const root = import.meta.dirname

export default defineConfig({
  root: resolve(root, 'src/renderer'),
  resolve: { alias: { '@shared': resolve(root, 'src/shared'), '@ui': resolve(root, 'src/renderer/src') } },
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    outDir: resolve(root, 'dist-ui'),
    emptyOutDir: true,
    target: 'es2023',
    rollupOptions: {
      input: { index: resolve(root, 'src/renderer/index.html'), popup: resolve(root, 'src/renderer/popup.html') },
    },
  },
})
