import { describe, expect, it, vi } from 'vitest'
import { articleParagraphs, decodeEntities, speechChunks } from '@shared/readaloud'
import { defaultSettings, sanitizeSettingsPatch } from '@shared/settings'
import { isLocalHost, securityOf } from '@shared/url'

vi.mock('electron', () => ({ net: {}, app: { isPackaged: false, getPath: () => '/tmp', getAppPath: () => '/tmp' } }))
const { listedThreat, parseHostList } = await import('../src/main/safebrowsing')
const { canTranslate, defaultTarget, normalizeLang, pickPairs, route } = await import('../src/main/translation')

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

describe('dangerous site lists', () => {
  const lists = new Map([
    ['phishing' as const, parseHostList('# Title: test\n! comment\nlogin-verify.example\n\nEvil.Example.org\n')],
    ['malware' as const, parseHostList('# Title: test\n203.0.113.7\nbad.test\n')],
  ])

  it('finds a listed host and the subdomains under it', () => {
    expect(listedThreat('login-verify.example', lists)).toBe('phishing')
    expect(listedThreat('www.evil.example.org', lists)).toBe('phishing')
    expect(listedThreat('cdn.bad.test.', lists)).toBe('malware')
    expect(listedThreat('203.0.113.7', lists)).toBe('malware')
  })

  it('never matches a parent domain or a bare top-level domain', () => {
    expect(listedThreat('example.org', lists)).toBeNull()
    expect(listedThreat('113.7', lists)).toBeNull()
    expect(listedThreat('good.example', lists)).toBeNull()
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

describe('translation', () => {
  it('maps page language tags to model codes', () => {
    expect(normalizeLang('en-US')).toBe('en')
    expect(normalizeLang('tr_TR')).toBe('tr')
    expect(normalizeLang('zh-CN')).toBe('zh')
    expect(normalizeLang('no')).toBe('nb')
    expect(normalizeLang('')).toBeNull()
    expect(normalizeLang('x-klingon')).toBeNull()
  })

  it('goes through English between two other languages', () => {
    expect(route('de', 'tr')).toEqual([
      { from: 'de', to: 'en' },
      { from: 'en', to: 'tr' },
    ])
    expect(route('en', 'tr')).toEqual([{ from: 'en', to: 'tr' }])
    expect(route('tr', 'tr')).toEqual([])
  })

  it('offers only what the model store can do', () => {
    expect(defaultTarget('tr')).toBe('en')
    expect(defaultTarget('de')).toBe('de')
    expect(canTranslate('de', 'en')).toBe(true)
    expect(canTranslate('is', 'de')).toBe(true)
    expect(canTranslate('en', 'tr')).toBe(false)
    expect(canTranslate('tr', 'en')).toBe(false)
    expect(canTranslate('en', 'en')).toBe(false)
  })

  it('reads the model store registry and skips incomplete or odd entries', () => {
    const file = (name: string, size = 10) => ({ name, size, estimatedCompressedSize: size / 2, expectedSha256Hash: 'AB'.repeat(32) })
    const pairs = pickPairs({
      deen: { model: file('model.deen.intgemm.alphas.bin'), lex: file('lex.50.50.deen.s2t.bin'), vocab: file('vocab.deen.spm'), qualityModel: file('q.bin') },
      enfa: { model: file('model.enfa.bin'), lex: file('lex.enfa.bin'), srcvocab: file('srcvocab.enfa.spm'), trgvocab: file('trgvocab.enfa.spm') },
      enes: { model: file('model.enes.bin'), vocab: file('vocab.enes.spm') },
      ende: { model: file('../escape'), lex: file('lex.ende.bin'), vocab: file('vocab.ende.spm') },
      notes: {},
    })
    expect(pairs.map((p) => `${p.from}-${p.to}`)).toEqual(['de-en', 'en-fa'])
    expect(pairs[0].files.map((f) => f.path)).toEqual(['deen/model.deen.intgemm.alphas.bin', 'deen/lex.50.50.deen.s2t.bin', 'deen/vocab.deen.spm'])
    expect(pairs[0].files[0]).toMatchObject({ size: 10, download: 5, sha256: 'ab'.repeat(32) })
    expect(pairs[1].files.map((f) => f.name)).toEqual(['model', 'lex', 'srcvocab', 'trgvocab'])
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
