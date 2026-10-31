import { describe, expect, it } from 'vitest'
import { translate } from '@shared/i18n'
import { defaultSettings, sanitizeSettingsPatch } from '@shared/settings'
import type { DownloadState } from '@shared/types'
import { downloadStatus } from '../src/renderer/src/downloads'

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
