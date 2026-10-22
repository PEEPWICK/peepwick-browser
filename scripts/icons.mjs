// Renders the app icons from the Double-Eye W geometry in the PEEPWICK brand book
// (section 08). Sizes below 32 px use the heavier small-size symbol, as the book requires.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Resvg } from '@resvg/resvg-js'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const GECE = '#0B0B0C'
const BEYAZ = '#FFFFFF'

// Standard symbol: 121 × 90 units, 15-unit stroke, pupils with a 45° highlight.
const symbol = (fill) => `
  <clipPath id="c"><polygon points="0,0 15,15 106,15 121,0 121,110 0,110"/></clipPath>
  <path d="M7.5,-20 V56 A26.5,26.5 0 0 0 60.5,56 V30 M60.5,56 A26.5,26.5 0 0 0 113.5,56 V-20"
    fill="none" stroke="${fill}" stroke-width="15" stroke-linecap="butt" stroke-linejoin="round" clip-path="url(#c)"/>
  <path d="M22,51 A12,12 0 1 0 46,51 A12,12 0 1 0 22,51 Z M35.44,45.96 A3.6,3.6 0 1 0 42.64,45.96 A3.6,3.6 0 1 0 35.44,45.96 Z" fill="${fill}" fill-rule="evenodd"/>
  <path d="M75,51 A12,12 0 1 0 99,51 A12,12 0 1 0 75,51 Z M88.44,45.96 A3.6,3.6 0 1 0 95.64,45.96 A3.6,3.6 0 1 0 88.44,45.96 Z" fill="${fill}" fill-rule="evenodd"/>`

// Small-size symbol (16–31 px): 19-unit stroke, solid pupils, no highlight.
const smallSymbol = (fill) => `
  <clipPath id="c"><polygon points="0,0 19,12 102,12 121,0 121,110 0,110"/></clipPath>
  <path d="M9.5,-20 V55 A25.5,25.5 0 0 0 60.5,55 V34 M60.5,55 A25.5,25.5 0 0 0 111.5,55 V-20"
    fill="none" stroke="${fill}" stroke-width="19" stroke-linecap="butt" stroke-linejoin="round" clip-path="url(#c)"/>
  <path d="M21.5,51 A13.5,13.5 0 1 0 48.5,51 A13.5,13.5 0 1 0 21.5,51 Z" fill="${fill}"/>
  <path d="M72.5,51 A13.5,13.5 0 1 0 99.5,51 A13.5,13.5 0 1 0 72.5,51 Z" fill="${fill}"/>`

// Profile layout from the brand book: white symbol on a Gece circle, safe for round crops.
// The circle leaves a small margin so the icon sits well next to other desktop icons.
function appIcon(size) {
  const small = size < 32
  const margin = small ? 0 : 0.04
  const r = 0.5 - margin
  const symbolWidth = small ? 0.7 : 0.56
  const scale = symbolWidth / 121
  const x = (1 - symbolWidth) / 2
  const y = (1 - 90 * scale) / 2 + (small ? 0 : 0.012)
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1" width="${size}" height="${size}">
    <circle cx="0.5" cy="0.5" r="${r}" fill="${GECE}"/>
    <g transform="translate(${x} ${y}) scale(${scale})">${small ? smallSymbol(BEYAZ) : symbol(BEYAZ)}</g>
  </svg>`
}

function render(svg, size) {
  return new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng()
}

function write(path, data) {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, data)
  console.log('wrote', path.replace(root + '/', ''))
}

for (const size of [16, 24, 32, 48, 64, 128, 256, 512]) {
  write(join(root, 'build/icons', `${size}x${size}.png`), render(appIcon(size), size))
}
write(join(root, 'build/icon.png'), render(appIcon(512), 512))
write(join(root, 'resources/icon.png'), render(appIcon(256), 256))
write(join(root, 'build/icon.svg'), appIcon(512))
