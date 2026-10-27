// Imported first by index.ts: runs before any module reads app paths.
import { app } from 'electron'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

app.setName('PEEPWICK Browser')
if (!app.isPackaged) {
  // Keep development profiles away from the installed browser's data.
  app.setPath('userData', join(app.getPath('appData'), 'PEEPWICK Browser (dev)'))
}
if (process.env['PEEPWICK_PROFILE']) {
  app.setPath('userData', process.env['PEEPWICK_PROFILE'])
}

// System dialogs (a page's alert, "leave this page?", printing) are GTK windows and
// take the desktop's theme. When the browser has its own light or dark theme, give
// GTK the matching Adwaita variant. GTK reads this while starting up; index.ts
// removes it afterwards so programs the browser opens don't inherit it.
function savedSettings(): { theme?: unknown; hardwareAcceleration?: unknown } {
  try {
    return JSON.parse(readFileSync(join(app.getPath('userData'), 'settings.json'), 'utf8')) as { theme?: unknown }
  } catch {
    return {}
  }
}

const saved = savedSettings()
const theme = saved.theme ?? 'dark'
export const gtkThemeSet = process.platform === 'linux' && !process.env['GTK_THEME'] && (theme === 'dark' || theme === 'light')
if (gtkThemeSet) process.env['GTK_THEME'] = theme === 'dark' ? 'Adwaita:dark' : 'Adwaita'

// Read aloud uses the system's voices, which Chromium reaches through speech-dispatcher
// only when asked to.
if (process.platform === 'linux') app.commandLine.appendSwitch('enable-speech-dispatcher')

// Settings › General › Use hardware acceleration; only possible before the app is ready.
if (saved.hardwareAcceleration === false) app.disableHardwareAcceleration()
