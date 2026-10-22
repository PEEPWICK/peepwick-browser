// Imported first by index.ts: runs before any module reads app paths.
import { app } from 'electron'
import { join } from 'node:path'

app.setName('PEEPWICK Browser')
if (!app.isPackaged) {
  // Keep development profiles away from the installed browser's data.
  app.setPath('userData', join(app.getPath('appData'), 'PEEPWICK Browser (dev)'))
}
if (process.env['PEEPWICK_PROFILE']) {
  app.setPath('userData', process.env['PEEPWICK_PROFILE'])
}
