// Composites the captures written by the development harness (src/main/harness.ts)
// into one PNG per step: node scripts/shoot.mjs <dir>
import { readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { Resvg } from '@resvg/resvg-js'

const dir = process.argv[2]
if (!dir) {
  console.error('usage: node scripts/shoot.mjs <dir>')
  process.exit(1)
}

const png = (file) => `data:image/png;base64,${readFileSync(join(dir, file)).toString('base64')}`

for (const name of readdirSync(dir).filter((f) => f.endsWith('.json'))) {
  const base = name.slice(0, -5)
  const meta = JSON.parse(readFileSync(join(dir, name), 'utf8'))
  const { width, height } = meta.size
  const ui = `<image href="${png(`${base}.ui.png`)}" x="0" y="0" width="${width}" height="${height}"/>`
  const pages = meta.layers
    .map((l) => `<image href="${png(l.file)}" x="${l.x}" y="${l.y}" width="${l.width}" height="${l.height}"/>`)
    .join('')
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
    <rect width="100%" height="100%" fill="${meta.uiOnTop ? '#0B0B0C' : '#0B0B0C'}"/>
    ${meta.uiOnTop ? pages + ui : ui + pages}
  </svg>`
  const out = new Resvg(svg, { fitTo: { mode: 'original' } }).render().asPng()
  writeFileSync(join(dir, `${base}.png`), out)
  console.log('wrote', join(dir, `${base}.png`))
}
