import { randomBytes } from 'node:crypto'
import { describe, expect, it } from 'vitest'
import { generatePassword, isWeakPassword, loginsFromCsv, loginsToCsv, originOf, parseCsv } from '@shared/passwordcsv'

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
