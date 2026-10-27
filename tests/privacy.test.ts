import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { translate } from '@shared/i18n'
import { defaultSettings, sanitizeSettingsPatch } from '@shared/settings'
import type { DownloadState } from '@shared/types'
import { scrub } from '../src/main/scrub'
import { downloadStatus } from '../src/renderer/src/downloads'

const dir = mkdtempSync(join(tmpdir(), 'peepwick-test-'))
vi.mock('../src/main/paths', () => ({ userPath: (...parts: string[]) => join(dir, ...parts) }))
const history = await import('../src/main/history')
const now = Date.now()

describe('the bug report never carries personal data', () => {
  it('removes addresses, the home folder, the user name, e-mail and IP addresses and tokens', () => {
    const text =
      'Failed to load https://bank.example/account?id=42 from /home/emir/Belgeler/secret.pdf for emir, ' +
      'mail emir@example.com, peer 192.168.1.20, key 4f3c2a1b9e8d7c6b5a4f3c2a1b9e8d7c6b, file:///home/emir/x'
    const out = scrub(text, '/home/emir', 'emir')
    for (const leak of ['bank.example', 'Belgeler', 'secret.pdf', 'emir', '192.168', '4f3c2a1b', 'example.com']) expect(out).not.toContain(leak)
    expect(out).toContain('<url>')
    expect(out).toContain('<path>')
  })

  it('keeps error names and the browser’s own code locations', () => {
    expect(
      scrub("TypeError: Cannot read properties of undefined (reading 'x') at Tab.load (/tmp/.mount_PEEPWIx/resources/app.asar/out/main/index.js:120:5)", '/home/u', 'someone'),
    ).toBe("TypeError: Cannot read properties of undefined (reading 'x') at Tab.load (out/main/index.js:120:5)")
    expect(scrub('[store] could not write /home/u/.config/PEEPWICK Browser/settings.json', '/home/u', 'someone')).toBe('[store] could not write <path> Browser/settings.json')
    expect(scrub('downloaded 50 / 100 parts', '/home/u', 'someone')).toBe('downloaded 50 / 100 parts')
  })
})

describe('settings added in 0.1.6', () => {
  it('defaults: telemetry off, nothing cleared on exit, automatic secure DNS', () => {
    const s = defaultSettings('tr')
    expect(s.errorLog).toBe(false)
    expect(s.clearOnExit).toEqual([])
    expect(s.secureDns).toBe('automatic')
    expect(s.hardwareAcceleration).toBe(true)
    expect(s.downloadDir).toBe('')
  })

  it('accepts only known values', () => {
    expect(sanitizeSettingsPatch({ clearOnExit: ['history', 'passwords', 'cache', 3] }).clearOnExit).toEqual(['history', 'cache'])
    expect(sanitizeSettingsPatch({ secureDns: 'quad9' }).secureDns).toBe('quad9')
    expect(sanitizeSettingsPatch({ secureDns: 'evil' }).secureDns).toBeUndefined()
    expect(sanitizeSettingsPatch({ downloadDir: 'relative/path' }).downloadDir).toBeUndefined()
    expect(sanitizeSettingsPatch({ downloadDir: '/home/u/İndirilenler' }).downloadDir).toBe('/home/u/İndirilenler')
    expect(sanitizeSettingsPatch({ errorLog: 'yes' }).errorLog).toBeUndefined()
  })
})

describe('download status line', () => {
  const t = (key: Parameters<typeof translate>[1], vars?: Record<string, string | number>) => translate('tr', key, vars)
  const base: DownloadState = { id: '1', filename: 'a.zip', path: '/tmp/a.zip', url: 'https://x/a.zip', received: 0, total: 0, status: 'progressing', startedAt: 0 }

  it('shows progress, speed and time left', () => {
    expect(downloadStatus({ ...base, received: 5 * 1024 * 1024, total: 10 * 1024 * 1024, speed: 1024 * 1024 }, 'tr', t)).toBe('5,0 MB / 10,0 MB · 1,0 MB/sn · 5 sn kaldı')
  })

  it('says how a download ended', () => {
    expect(downloadStatus({ ...base, status: 'completed', total: 2048 }, 'tr', t)).toBe('2,0 KB')
    expect(downloadStatus({ ...base, status: 'completed', total: 2048, missing: true }, 'tr', t)).toBe('Dosya silinmiş ya da taşınmış')
    expect(downloadStatus({ ...base, status: 'interrupted' }, 'tr', t)).toBe('Yarıda kaldı')
  })
})

describe('history clearing', () => {
  beforeAll(() => {
    history.openHistory()
    history.importVisits([
      { url: 'https://a.example/1', title: 'A1', visitedAt: now - 3 * 3_600_000 },
      { url: 'https://a.example/2', title: 'A2', visitedAt: now - 30 * 60_000 },
      { url: 'https://www.b.example/', title: 'B', visitedAt: now - 10 * 60_000 },
      { url: 'https://b.example/x', title: 'Bx', visitedAt: now - 5 * 24 * 3_600_000 },
    ])
  })
  afterAll(() => {
    history.closeHistory()
    rmSync(dir, { recursive: true, force: true })
  })

  it('imports each visit once', () => {
    expect(history.importVisits([{ url: 'https://a.example/1', title: 'A1', visitedAt: now - 3 * 3_600_000 }])).toBe(0)
    expect(history.countVisits(0)).toBe(4)
  })

  it('clears only the chosen time range', () => {
    expect(history.countVisits(Date.now() - 3_600_000)).toBe(2)
    expect(history.deleteVisitsSince(Date.now() - 3_600_000)).toBe(2)
    expect(history.countVisits(0)).toBe(2)
    expect(history.matchPages('A2')).toHaveLength(0)
    expect(history.matchPages('A1')).toHaveLength(1)
  })

  it('deletes one site, with or without www.', () => {
    expect(history.deleteHost('b.example')).toBe(1)
    expect(history.queryHistory('').map((e) => e.title)).toEqual(['A1'])
  })
})
