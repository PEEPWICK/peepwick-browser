import { describe, expect, it } from 'vitest'
import { dictionariesForTest, dictionaryKeys, formatBytes, formatNumber, translate } from '@shared/i18n'
import { defaultSettings, hostMatches, normalizeDomain, sanitizeSettingsPatch } from '@shared/settings'
import { displayParts, internalPage, isImageDataURL, securityOf } from '@shared/url'

describe('i18n', () => {
  it('has every key in both languages', () => {
    for (const lang of ['tr', 'en'] as const) {
      for (const key of dictionaryKeys) {
        expect(dictionariesForTest[lang][key], `${lang}:${key}`).toBeTruthy()
      }
    }
  })

  it('keeps the same placeholders in both languages', () => {
    const vars = (text: string) => [...new Set([...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]))].sort()
    for (const key of dictionaryKeys) {
      const other = dictionariesForTest.en[key].split('|').at(-1) as string
      expect(vars(other), key).toEqual(vars(dictionariesForTest.tr[key]))
    }
  })

  it('follows the brand voice: no exclamation marks', () => {
    for (const lang of ['tr', 'en'] as const) {
      for (const key of dictionaryKeys) expect(dictionariesForTest[lang][key], key).not.toContain('!')
    }
  })

  it('formats numbers per locale', () => {
    expect(formatNumber('tr', 12480)).toBe('12.480')
    expect(formatNumber('en', 12480)).toBe('12,480')
    expect(translate('tr', 'tabs.count', { n: 1284 })).toBe('1.284 sekme')
    expect(translate('en', 'tabs.count', { n: 1 })).toBe('1 tab')
    expect(translate('en', 'tabs.count', { n: 2 })).toBe('2 tabs')
    expect(formatBytes('tr', 1536)).toBe('1,5 KB')
    expect(formatBytes('en', 1536)).toBe('1.5 KB')
  })
})

describe('settings', () => {
  it('drops unknown keys and bad values', () => {
    expect(
      sanitizeSettingsPatch({ theme: 'neon', language: 'tr', focusMinutes: 9999, evil: true, shieldLevel: 'strict' }),
    ).toEqual({ language: 'tr', focusMinutes: 240, shieldLevel: 'strict' })
    expect(sanitizeSettingsPatch(null)).toEqual({})
    expect(sanitizeSettingsPatch({ searchEngine: '__proto__' })).toEqual({})
  })

  it('normalises site lists', () => {
    expect(sanitizeSettingsPatch({ focusSites: ['https://www.YouTube.com/watch?v=1', 'x.com', 'x.com', 42, 'not a site'] })).toEqual({
      focusSites: ['youtube.com', 'x.com'],
    })
  })

  it('has calm, private defaults', () => {
    const s = defaultSettings('tr')
    expect(s.shieldLevel).toBe('standard')
    expect(s.blockThirdPartyCookies).toBe(true)
    expect(s.searchSuggestions).toBe(false)
    expect(s.searchEngine).toBe('duckduckgo')
  })

  it('matches subdomains of a site', () => {
    expect(hostMatches('m.youtube.com', 'youtube.com')).toBe(true)
    expect(hostMatches('youtube.com', 'youtube.com')).toBe(true)
    expect(hostMatches('notyoutube.com', 'youtube.com')).toBe(false)
    expect(normalizeDomain('  WWW.Example.COM ')).toBe('example.com')
    expect(normalizeDomain('no spaces allowed')).toBeNull()
  })
})

describe('url helpers', () => {
  it('recognises built-in pages', () => {
    expect(internalPage('peepwick://settings#privacy')).toBe('settings')
    expect(internalPage('peepwick://unknown')).toBeNull()
    expect(internalPage('https://settings')).toBeNull()
  })

  it('reports security', () => {
    expect(securityOf('https://a.com')).toBe('secure')
    expect(securityOf('http://a.com')).toBe('insecure')
    expect(securityOf('http://localhost:3000')).toBe('local')
    expect(securityOf('peepwick://newtab')).toBe('internal')
  })

  it('ignores empty data URL favicons', () => {
    expect(isImageDataURL('data:,')).toBe(false)
    expect(isImageDataURL('data:image/png;base64,')).toBe(false)
    expect(isImageDataURL('data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ')).toBe(true)
    expect(isImageDataURL('data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==AAAAAAAA')).toBe(false)
  })

  it('splits URLs for display', () => {
    expect(displayParts('https://www.example.com/')).toEqual({ prefix: '', host: 'example.com', rest: '' })
    expect(displayParts('http://example.com/a?b=1')).toEqual({ prefix: 'http://', host: 'example.com', rest: '/a?b=1' })
    expect(displayParts('https://tr.wikipedia.org/wiki/G%C3%B6z').rest).toBe('/wiki/Göz')
  })
})

describe('data URLs for site icons', () => {
  it('reads base64, parameters and URL-encoded text', async () => {
    const { parseDataURL } = await import('@shared/url')
    expect(parseDataURL('data:image/png;base64,iVBO')?.type).toBe('image/png')
    const svg = parseDataURL('data:image/svg+xml;charset=utf-8;base64,PHN2Zy8+')
    expect(svg?.type).toBe('image/svg+xml')
    expect(new TextDecoder().decode(svg!.body)).toBe('<svg/>')
    expect(new TextDecoder().decode(parseDataURL('data:image/svg+xml,%3Csvg%2F%3E')!.body)).toBe('<svg/>')
    expect(parseDataURL('data:,')).toBeNull()
    expect(parseDataURL('https://x/icon.png')).toBeNull()
  })
})
