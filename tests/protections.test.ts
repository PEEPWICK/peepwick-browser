import { describe, expect, it } from 'vitest'
import { articleParagraphs, decodeEntities, speechChunks } from '@shared/readaloud'
import { defaultSettings, sanitizeSettingsPatch } from '@shared/settings'
import { isLocalHost, securityOf } from '@shared/url'

describe('HTTPS-Only mode', () => {
  it('leaves this computer and the local network on HTTP', () => {
    for (const host of ['localhost', 'app.localhost', 'printer', 'nas.local', 'router.lan', 'site.test', 'api.internal', '127.0.0.1', '10.0.0.8', '172.20.1.1', '192.168.1.1', '169.254.3.4', '[::1]', 'fd12:3456::1']) {
      expect(isLocalHost(host), host).toBe(true)
    }
    for (const host of ['example.com', 'neverssl.com', '8.8.8.8', '172.32.0.1', 'local.com', 'testing.org']) {
      expect(isLocalHost(host), host).toBe(false)
    }
  })

  it('calls local addresses local, not insecure', () => {
    expect(securityOf('http://192.168.1.1/')).toBe('local')
    expect(securityOf('http://example.com/')).toBe('insecure')
  })
})

describe('read aloud', () => {
  it('reads paragraphs in order and skips code, tables and figures', () => {
    const html = '<h2>Başlık</h2><p>Birinci &amp; <a href="#">bağlantı</a>.</p><pre>code()</pre><figure><img><figcaption>x</figcaption></figure><ul><li>Bir</li><li>İki&nbsp;üç</li></ul><table><tr><td>1</td></tr></table><p>Son&#8230;</p>'
    expect(articleParagraphs(html)).toEqual(['Başlık', 'Birinci & bağlantı.', 'Bir', 'İki üç', 'Son…'])
  })

  it('decodes entities without touching unknown ones', () => {
    expect(decodeEntities('&lt;b&gt; &#x131; &rsquo; &unknown;')).toBe('<b> ı ’ &unknown;')
  })

  it('splits long paragraphs at sentence ends', () => {
    const paragraph = Array.from({ length: 30 }, (_, i) => `Cümle ${i} biraz uzun bir cümledir.`).join(' ')
    const chunks = speechChunks(paragraph, 200)
    expect(chunks.length).toBeGreaterThan(3)
    expect(chunks.every((c) => c.length <= 200)).toBe(true)
    expect(chunks.join(' ')).toBe(paragraph)
    expect(speechChunks('Kısa.')).toEqual(['Kısa.'])
  })
})

describe('new settings', () => {
  it('turns the protections on by default and keeps sizes to the offered steps', () => {
    const s = defaultSettings('tr')
    expect([s.httpsOnly, s.safeBrowsing, s.fingerprintProtection, s.rejectCookieBanners]).toEqual([true, true, true, true])
    expect(sanitizeSettingsPatch({ defaultZoom: 1.25, fontSize: 20 })).toEqual({ defaultZoom: 1.25, fontSize: 20 })
    expect(sanitizeSettingsPatch({ defaultZoom: 7, fontSize: 13, httpsOnly: 'yes' })).toEqual({})
  })
})
