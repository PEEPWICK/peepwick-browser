// Turning an article's HTML into the text read aloud: paragraphs in reading order,
// without code blocks, tables or figures, split where speech engines would stumble.

const ENTITIES: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
  mdash: '—',
  ndash: '–',
  hellip: '…',
  lsquo: '‘',
  rsquo: '’',
  ldquo: '“',
  rdquo: '”',
  laquo: '«',
  raquo: '»',
}

export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (match, code: string) => {
    if (code.startsWith('#')) {
      const n = code[1].toLowerCase() === 'x' ? parseInt(code.slice(2), 16) : parseInt(code.slice(1), 10)
      return Number.isFinite(n) && n > 0 && n < 0x110000 ? String.fromCodePoint(n) : match
    }
    return ENTITIES[code.toLowerCase()] ?? match
  })
}

export function articleParagraphs(html: string): string[] {
  const text = html
    .replace(/<(script|style|noscript|svg|math|figure|table|pre)\b[\s\S]*?<\/\1>/gi, '\n')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/(p|div|h[1-6]|li|blockquote|figcaption|dd|dt|section|article|header|footer|tr)>/gi, '\n')
    .replace(/<[^>]*>/g, '')
  return decodeEntities(text)
    .split(/\n+/)
    .map((line) => line.replace(/\s+/g, ' ').trim())
    .filter((line) => /[\p{L}\p{N}]/u.test(line))
}

/** Long paragraphs in sentence-sized pieces; some engines stop partway through long text. */
export function speechChunks(paragraph: string, max = 400): string[] {
  if (paragraph.length <= max) return [paragraph]
  const chunks: string[] = []
  let current = ''
  for (const sentence of paragraph.split(/(?<=[.!?…;:])\s+/)) {
    if (current && current.length + sentence.length + 1 > max) {
      chunks.push(current)
      current = sentence
    } else {
      current = current ? `${current} ${sentence}` : sentence
    }
  }
  if (current) chunks.push(current)
  return chunks
}
