import { describe, expect, it } from 'vitest'
import { looksLikeHost, resolveInput } from '@shared/input'

const ddg = (q: string) => `https://duckduckgo.com/?q=${encodeURIComponent(q)}`

describe('resolveInput', () => {
  it('treats domains as https URLs', () => {
    expect(resolveInput('example.com', 'duckduckgo')).toBe('https://example.com')
    expect(resolveInput('peepwick.com.tr/hakkimizda', 'duckduckgo')).toBe('https://peepwick.com.tr/hakkimizda')
  })

  it('keeps explicit schemes', () => {
    expect(resolveInput('http://example.com', 'duckduckgo')).toBe('http://example.com')
    expect(resolveInput('https://example.com/a b', 'duckduckgo')).toBe('https://example.com/a b')
  })

  it('uses http for local hosts', () => {
    expect(resolveInput('localhost:5173', 'duckduckgo')).toBe('http://localhost:5173')
    expect(resolveInput('192.168.1.10', 'duckduckgo')).toBe('http://192.168.1.10')
    expect(resolveInput('printer.local', 'duckduckgo')).toBe('http://printer.local')
  })

  it('searches for phrases and unknown words', () => {
    expect(resolveInput('önce bakarız', 'duckduckgo')).toBe(ddg('önce bakarız'))
    expect(resolveInput('peepwick', 'duckduckgo')).toBe(ddg('peepwick'))
    expect(resolveInput('file.notatld', 'duckduckgo')).toBe(ddg('file.notatld'))
  })

  it('never navigates to javascript: URLs', () => {
    expect(resolveInput('javascript:alert(1)', 'duckduckgo')).toBe(ddg('javascript:alert(1)'))
  })

  it('opens built-in pages', () => {
    expect(resolveInput('peepwick://settings', 'duckduckgo')).toBe('peepwick://settings')
  })

  it('uses the chosen engine', () => {
    expect(resolveInput('hava durumu', 'brave')).toBe('https://search.brave.com/search?q=hava%20durumu')
  })
})

describe('looksLikeHost', () => {
  it('recognises real suffixes, including Turkish ones', () => {
    expect(looksLikeHost('example.com')).toBe(true)
    expect(looksLikeHost('gov.tr')).toBe(false)
    expect(looksLikeHost('turkiye.gov.tr')).toBe(true)
    expect(looksLikeHost('example.com:8080/path')).toBe(true)
  })

  it('rejects things that are not hosts', () => {
    expect(looksLikeHost('.com')).toBe(false)
    expect(looksLikeHost('hello')).toBe(false)
    expect(looksLikeHost('a..b')).toBe(false)
  })
})
