import { createHash, randomBytes } from 'node:crypto'
import { describe, expect, it, vi } from 'vitest'
import { generatePassword, isWeakPassword, loginsFromCsv, loginsToCsv, originOf, parseCsv } from '@shared/passwordcsv'
import { decodeMessage, encodeMessage, splitSignature } from '../src/main/dbus'

vi.mock('electron', () => ({ safeStorage: {}, app: { isPackaged: false, getPath: () => '/tmp', getAppPath: () => '/tmp' } }))
const { chromiumKey, decryptChromium, encryptChromium, stripDomainHash } = await import('../src/main/chromiumcrypto')
const { decryptLoginField, encryptKey4Entry, encryptLoginField, loginKeys, PrimaryPasswordError } = await import('../src/main/firefoxcrypto')
const { looksLikeCard, rememberable } = await import('../src/main/formdata')

describe('Chromium passwords and cookies', () => {
  const keys = { v10: chromiumKey('peanuts'), v11: [chromiumKey('keyring-secret'), chromiumKey('')] }

  it('reads v10 (no keyring) and v11 (keyring) values', () => {
    expect(decryptChromium(encryptChromium('hunter2', keys.v10, 'v10'), keys)?.toString()).toBe('hunter2')
    expect(decryptChromium(encryptChromium('şifre-ğü', chromiumKey('keyring-secret'), 'v11'), keys)?.toString()).toBe('şifre-ğü')
  })

  it('gives up cleanly when the keyring secret is missing', () => {
    const value = encryptChromium('secret', chromiumKey('another-secret'), 'v11')
    expect(decryptChromium(value, { v10: keys.v10, v11: [chromiumKey('')] })).toBeNull()
  })

  it('drops the domain hash newer Chromium puts in front of cookie values', () => {
    const hash = createHash('sha256').update('.example.com').digest()
    expect(stripDomainHash(Buffer.concat([hash, Buffer.from('abc')]), '.example.com').toString()).toBe('abc')
    expect(stripDomainHash(Buffer.from('plain'), '.example.com').toString()).toBe('plain')
  })
})

describe('Firefox passwords', () => {
  const globalSalt = randomBytes(20)
  const loginKey = randomBytes(24)
  const keyId = randomBytes(16)
  const store = (primary: string) => ({
    meta: { item1: globalSalt, item2: encryptKey4Entry(Buffer.from('password-check\x02\x02', 'latin1').subarray(0, 14), globalSalt, primary, randomBytes(32), randomBytes(14)) },
    privates: [{ a11: encryptKey4Entry(loginKey, globalSalt, primary, randomBytes(32), randomBytes(14)), a102: keyId }],
  })

  it('decrypts logins encrypted with 3DES and with AES-256', () => {
    const { meta, privates } = store('')
    const keys = loginKeys(meta, privates)
    expect(decryptLoginField(encryptLoginField('ada@example.com', keyId, loginKey, randomBytes(8)), keys)).toBe('ada@example.com')
    const aesKey = randomBytes(32)
    const aesKeys = loginKeys(meta, [{ a11: encryptKey4Entry(aesKey, globalSalt, '', randomBytes(32), randomBytes(14)), a102: keyId }])
    expect(decryptLoginField(encryptLoginField('parolam', keyId, aesKey, randomBytes(16), true), aesKeys)).toBe('parolam')
  })

  it('asks for the primary password when there is one', () => {
    const { meta, privates } = store('ana-parola')
    expect(() => loginKeys(meta, privates, '')).toThrow(PrimaryPasswordError)
    const keys = loginKeys(meta, privates, 'ana-parola')
    expect(decryptLoginField(encryptLoginField('x', keyId, loginKey, randomBytes(8)), keys)).toBe('x')
  })
})

describe('password CSV files', () => {
  it('parses quotes, commas, newlines and a BOM', () => {
    expect(parseCsv('﻿a,"b,c","d ""e"""\r\n1,"2\n3",4\n')).toEqual([
      ['a', 'b,c', 'd "e"'],
      ['1', '2\n3', '4'],
    ])
  })

  it('reads Chrome, Firefox and Bitwarden exports', () => {
    const chrome = loginsFromCsv('name,url,username,password,note\nGitHub,https://github.com/login,ada,pw1,\n')
    expect(chrome.logins).toEqual([{ url: 'https://github.com/login', username: 'ada', password: 'pw1', note: '' }])
    const firefox = loginsFromCsv('"url","username","password","httpRealm"\n"https://example.com","bob","pw2",""\n')
    expect(firefox.logins[0]).toMatchObject({ url: 'https://example.com', username: 'bob', password: 'pw2' })
    const bitwarden = loginsFromCsv('folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,X,n,,0,example.org,cem,pw3,\n,,note,Y,,,0,,,,\n')
    expect(bitwarden.logins).toEqual([{ url: 'example.org', username: 'cem', password: 'pw3', note: 'n' }])
    expect(bitwarden.skipped).toBe(1)
  })

  it('writes what it reads', () => {
    const csv = loginsToCsv([{ name: 'a, "b"', url: 'https://a.example', username: 'u', password: 'p,"q"', note: 'line1\nline2' }])
    expect(loginsFromCsv(csv).logins[0]).toEqual({ url: 'https://a.example', username: 'u', password: 'p,"q"', note: 'line1\nline2' })
  })

  it('turns addresses into origins', () => {
    expect(originOf('github.com')).toBe('https://github.com')
    expect(originOf('http://localhost:8080/login')).toBe('http://localhost:8080')
    expect(originOf('ftp://x')).toBeNull()
    expect(originOf('android://abc@com.app/')).toBeNull()
  })
})

describe('password strength and generation', () => {
  it('flags short, common and one-kind passwords', () => {
    expect(isWeakPassword('123456')).toBe(true)
    expect(isWeakPassword('galatasaray')).toBe(true)
    expect(isWeakPassword('abcdefghij')).toBe(true)
    expect(isWeakPassword('Kq7#vT9!mZ2x')).toBe(false)
    expect(isWeakPassword('correcthorsebatterystaple')).toBe(false)
  })

  it('generates 16 characters with every kind', () => {
    for (let i = 0; i < 50; i++) {
      const password = generatePassword((n) => randomBytes(n))
      expect(password).toHaveLength(16)
      expect(isWeakPassword(password)).toBe(false)
      expect(password).toMatch(/[a-z]/)
      expect(password).toMatch(/[A-Z]/)
      expect(password).toMatch(/\d/)
    }
  })
})

describe('form data', () => {
  it('never keeps card numbers', () => {
    expect(looksLikeCard('4111 1111 1111 1111')).toBe(true)
    expect(looksLikeCard('5500-0000-0000-0004')).toBe(true)
    expect(looksLikeCard('4111 1111 1111 1112')).toBe(false)
    expect(rememberable('4111111111111111')).toBe(false)
    expect(rememberable('Ankara')).toBe(true)
  })
})

describe('D-Bus messages', () => {
  it('splits signatures into complete types', () => {
    expect(splitSignature('a{ss}aoo(oayays)')).toEqual(['a{ss}', 'ao', 'o', '(oayays)'])
  })

  it('round-trips a method call with dictionaries, arrays and bytes', () => {
    const data = encodeMessage({
      serial: 7,
      destination: 'org.freedesktop.secrets',
      path: '/org/freedesktop/secrets',
      interface: 'org.freedesktop.Secret.Service',
      member: 'Test',
      signature: 'a{ss}ao(oayays)',
      body: [{ application: 'chrome', x: 'ğ' }, ['/a', '/b'], ['/s', Buffer.alloc(0), Buffer.from('secret'), 'text/plain']],
    })
    const decoded = decodeMessage(data)!
    expect(decoded.size).toBe(data.length)
    expect(decoded.message.member).toBe('Test')
    expect(decoded.message.body[0]).toEqual([
      ['application', 'chrome'],
      ['x', 'ğ'],
    ])
    expect(decoded.message.body[1]).toEqual(['/a', '/b'])
    expect((decoded.message.body[2] as unknown[])[2]).toEqual(Buffer.from('secret'))
  })
})
