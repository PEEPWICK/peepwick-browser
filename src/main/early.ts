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
function savedTheme(): unknown {
  try {
    return (JSON.parse(readFileSync(join(app.getPath('userData'), 'settings.json'), 'utf8')) as { theme?: unknown }).theme
  } catch {
    return undefined
  }
}

const theme = savedTheme() ?? 'dark'
export const gtkThemeSet = process.platform === 'linux' && !process.env['GTK_THEME'] && (theme === 'dark' || theme === 'light')
if (gtkThemeSet) process.env['GTK_THEME'] = theme === 'dark' ? 'Adwaita:dark' : 'Adwaita'
