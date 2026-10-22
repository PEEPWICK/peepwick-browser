// Renders the 1280×640 social preview card with the brand typefaces, using Electron's
// own renderer: npx electron scripts/social.cjs
// Layout: descriptor sub-brand lockup (brand book §15) over the crowd pattern (§11).
const { app, BrowserWindow } = require('electron')
const { readFileSync, writeFileSync } = require('node:fs')
const { join } = require('node:path')

const root = join(__dirname, '..')
const font = (file) => `data:font/woff2;base64,${readFileSync(join(root, 'resources/fonts', file)).toString('base64')}`
const logo = readFileSync(join(root, 'build/brand/peepwick-logo-yatay-beyaz.svg'), 'utf8')
  .replace(/<\?xml[^>]*>/, '')
  .replace(/<!--.*?-->/gs, '')

const html = `<!doctype html><html><head><meta charset="utf-8"><style>
@font-face { font-family: Archivo; src: url(${font('archivo-latin-standard-normal.woff2')}); font-weight: 100 900; font-stretch: 62% 125%; }
@font-face { font-family: Archivo; src: url(${font('archivo-latin-ext-standard-normal.woff2')}); font-weight: 100 900; font-stretch: 62% 125%; unicode-range: U+0100-02BA; }
@font-face { font-family: Plex; src: url(${font('ibm-plex-mono-latin-400-normal.woff2')}); }
html, body { margin: 0; width: 1280px; height: 640px; background: #0B0B0C; overflow: hidden; }
body { position: relative; font-family: Archivo; -webkit-font-smoothing: antialiased;
  background-image: radial-gradient(circle, #1D1E20 2px, transparent 2.5px); background-size: 40px 40px; background-position: 0 0; }
.veil { position: absolute; inset: 0; background: radial-gradient(ellipse 70% 80% at 28% 55%, #0B0B0C 45%, transparent 75%); }
.eye { position: absolute; left: 1028px; top: 128px; width: 24px; height: 24px; border-radius: 50%; background: #F2A93B; }
.eye::after { content: ''; position: absolute; left: 10px; top: 5px; width: 10px; height: 10px; border-radius: 50%; background: #0B0B0C; }
.lockup { position: absolute; left: 96px; top: 228px; display: flex; align-items: center; gap: 28px; }
.lockup svg { width: 520px; height: auto; display: block; }
.rule { width: 2px; height: 72px; background: #3A3B3F; }
.descriptor { font-family: Plex; font-size: 30px; letter-spacing: 0.08em; color: #F4F3F0; }
.line { position: absolute; left: 96px; top: 388px; font-size: 30px; color: #8E8981; letter-spacing: -0.005em; }
.line b { color: #F4F3F0; font-weight: 700; font-stretch: 112%; }
</style></head><body>
<div class="veil"></div><div class="eye"></div>
<div class="lockup">${logo}<div class="rule"></div><div class="descriptor">BROWSER</div></div>
<div class="line"><b>Önce bakarız.</b> First, we look.</div>
</body></html>`

app.whenReady().then(async () => {
  const win = new BrowserWindow({ width: 1280, height: 640, show: false, useContentSize: true, webPreferences: { offscreen: true } })
  await win.loadURL(`data:text/html;base64,${Buffer.from(html).toString('base64')}`)
  await win.webContents.executeJavaScript('document.fonts.ready.then(() => true)')
  await new Promise((r) => setTimeout(r, 400))
  const image = await win.webContents.capturePage({ x: 0, y: 0, width: 1280, height: 640 })
  writeFileSync(join(root, 'docs/social-preview.png'), image.resize({ width: 1280, height: 640 }).toPNG())
  console.log('wrote docs/social-preview.png')
  app.quit()
})
