import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it, vi } from 'vitest'

const dir = mkdtempSync(join(tmpdir(), 'peepwick-desktop-'))
const icon = join(dir, 'icon.png')
writeFileSync(icon, 'png')
vi.mock('electron', () => ({ app: { isPackaged: true, isDefaultProtocolClient: () => false } }))
vi.mock('../src/main/paths', () => ({ resourcePath: () => icon }))
process.env.XDG_DATA_HOME = join(dir, 'share')
const { syncAppImageDesktopFile } = await import('../src/main/defaultbrowser')
const file = join(dir, 'share', 'applications', 'peepwick-browser.desktop')

afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe('AppImage desktop file', () => {
  it('is only written when asked (making the browser the default)', () => {
    process.env.APPIMAGE = '/home/u/Masaüstü/peepwick-browser-0.1.6a-x86_64.AppImage'
    expect(syncAppImageDesktopFile()).toBe(false)
    expect(existsSync(file)).toBe(false)
    expect(syncAppImageDesktopFile(true)).toBe(true)
    const text = readFileSync(file, 'utf8')
    expect(text).toContain('Exec="/home/u/Masaüstü/peepwick-browser-0.1.6a-x86_64.AppImage" %U')
    expect(text).toContain('MimeType=x-scheme-handler/http;x-scheme-handler/https;text/html;application/xhtml+xml;')
    expect(existsSync(join(dir, 'share', 'icons', 'hicolor', '256x256', 'apps', 'peepwick-browser.png'))).toBe(true)
  })

  it('follows the AppImage when it moves or updates', () => {
    process.env.APPIMAGE = '/opt/apps/PEEPWICK "new".AppImage'
    expect(syncAppImageDesktopFile()).toBe(true)
    expect(readFileSync(file, 'utf8')).toContain('Exec="/opt/apps/PEEPWICK \\"new\\".AppImage" %U')
  })

  it('leaves a desktop file that isn’t ours alone', () => {
    writeFileSync(file, '[Desktop Entry]\nName=Something else\n')
    expect(syncAppImageDesktopFile(true)).toBe(false)
    expect(readFileSync(file, 'utf8')).toContain('Something else')
  })
})
