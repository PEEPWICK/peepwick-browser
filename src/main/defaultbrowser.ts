// Making PEEPWICK Browser the system's web browser. The .deb installs a desktop
// file; an AppImage has none, so "make default" had nothing to point the system at
// and did nothing. For an AppImage we write our own desktop file (and icon) under
// ~/.local/share, then set it with xdg-settings and xdg-mime.
import { app } from 'electron'
import { execFile, execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { resourcePath } from './paths'

export const DESKTOP = 'peepwick-browser.desktop'
const MIME = ['x-scheme-handler/http', 'x-scheme-handler/https', 'text/html', 'application/xhtml+xml']

const dataHome = () => process.env['XDG_DATA_HOME'] || join(homedir(), '.local', 'share')
const userDesktopFile = () => join(dataHome(), 'applications', DESKTOP)

function run(command: string, args: string[]): Promise<boolean> {
  return new Promise((resolve) => execFile(command, args, { timeout: 8000 }, (error) => resolve(!error)))
}

function desktopEntry(appImage: string): string {
  const exec = `"${appImage.replace(/(["`$\\])/g, '\\$1')}"`
  return [
    '[Desktop Entry]',
    'Type=Application',
    'Name=PEEPWICK Browser',
    'GenericName=Web Browser',
    'GenericName[tr]=Web Tarayıcısı',
    'Comment=Browse the web, calmly and privately',
    "Comment[tr]=Web'de sakin ve gizli gezinin",
    `Exec=${exec} %U`,
    'Icon=peepwick-browser',
    'Terminal=false',
    'StartupWMClass=peepwick-browser',
    'Categories=Network;WebBrowser;',
    `MimeType=${MIME.join(';')};`,
    'Keywords=browser;web;internet;privacy;peepwick;',
    '',
  ].join('\n')
}

/**
 * For an AppImage: keeps a desktop file that starts this very AppImage. Called at
 * startup too, so moving or updating the AppImage doesn't leave the system pointing
 * at a file that is gone. Only rewrites a file that is already ours.
 */
export function syncAppImageDesktopFile(create = false): boolean {
  const appImage = process.env['APPIMAGE']
  if (!appImage) return false
  const file = userDesktopFile()
  const wanted = desktopEntry(appImage)
  const current = existsSync(file) ? readFileSync(file, 'utf8') : null
  if (current === wanted) return true
  if (current === null && !create) return false
  if (current !== null && !current.includes('Name=PEEPWICK Browser')) return false
  try {
    mkdirSync(join(dataHome(), 'applications'), { recursive: true })
    writeFileSync(file, wanted)
    const icons = join(dataHome(), 'icons', 'hicolor', '256x256', 'apps')
    mkdirSync(icons, { recursive: true })
    copyFileSync(resourcePath('icon.png'), join(icons, 'peepwick-browser.png'))
    return true
  } catch (error) {
    console.error('[default-browser] could not write the desktop file', error)
    return false
  }
}

export function isDefaultBrowser(): boolean {
  try {
    if (execFileSync('xdg-settings', ['get', 'default-web-browser'], { timeout: 3000, encoding: 'utf8' }).trim() === DESKTOP) return true
  } catch {
    // No xdg-settings: ask Electron instead.
  }
  return app.isDefaultProtocolClient('https') && app.isDefaultProtocolClient('http')
}

export type DefaultBrowserResult = 'done' | 'development' | 'failed'

export async function makeDefaultBrowser(): Promise<DefaultBrowserResult> {
  if (!app.isPackaged) return 'development'
  if (process.env['APPIMAGE']) {
    if (!syncAppImageDesktopFile(true)) return 'failed'
    await run('update-desktop-database', [join(dataHome(), 'applications')])
  }
  await run('xdg-settings', ['set', 'default-web-browser', DESKTOP])
  await run('xdg-mime', ['default', DESKTOP, ...MIME])
  return isDefaultBrowser() ? 'done' : 'failed'
}
