// Development-only screenshot harness. With PEEPWICK_HARNESS=<file.json> in an
// unpackaged build, plays a list of steps against the first window and saves
// captures of the UI and the visible pages (see scripts/shoot.mjs). Never active
// in packaged builds.
import { app } from 'electron'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { countNodes } from '@shared/bookmarks'
import type { CommandId, InternalPage, UICommand, WorkspaceIcon } from '@shared/types'
import { installExtension } from 'electron-chrome-web-store'
import { openPage, runCommand } from './actions'
import { bookmarkTree } from './bookmarks'
import { detectSources, importBookmarks, importFrom } from './importers'
import { lifecycle } from './lifecycle'
import { BACKUP_CATEGORIES } from '@shared/types'
import { collectBackup, holdBackup, restoreBackup, unlockBackup } from './backup'
import { encryptBackup } from './backupfile'
import { confirm } from './popup'
import { browsingSession } from './sessions'
import { updateSettings } from './settings'
import { BrowserWin } from './window'

type Step =
  | { do: 'wait'; ms: number }
  | { do: 'navigate'; url: string }
  | { do: 'newTab'; url?: string }
  | { do: 'command'; id: CommandId }
  | { do: 'ui'; command: UICommand }
  | { do: 'settings'; patch: Record<string, unknown> }
  | { do: 'eval'; js: string }
  | { do: 'resize'; width: number; height: number }
  | { do: 'capture'; name: string; window?: number }
  | { do: 'evalTab'; js: string }
  | { do: 'workspace'; name: string; icon: WorkspaceIcon }
  | { do: 'state' }
  | { do: 'installExtension'; id: string }
  | { do: 'type'; text: string }
  | { do: 'key'; key: string }
  | { do: 'page'; page: InternalPage; sub?: string }
  | { do: 'importDetected'; index: number }
  | { do: 'bookmarks' }
  | { do: 'mouse'; target?: 'ui' | 'tab'; x: number; y: number; button?: 'left' | 'right' | 'middle'; move?: boolean }
  | { do: 'confirm'; title: string; body?: string; lines?: string[]; danger?: boolean }
  | { do: 'download'; url: string }
  | { do: 'importAll'; index: number; primaryPassword?: string }
  | { do: 'tabKey'; key: string }
  | { do: 'backup'; password: string; restore?: boolean; dialog?: boolean }
  | { do: 'quit' }

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

/** capturePage never settles for a view that has not painted yet. */
function grab(wc: Electron.WebContents): Promise<Electron.NativeImage | null> {
  return Promise.race([wc.capturePage(), sleep(3000).then(() => null)])
}

async function capture(win: BrowserWin, dir: string, name: string): Promise<void> {
  const ui = await grab(win.ui.webContents)
  if (!ui) return
  const layers: { file: string; x: number; y: number; width: number; height: number }[] = []
  writeFileSync(join(dir, `${name}.ui.png`), ui.toPNG())
  let index = 0
  for (const child of win.base.contentView.children) {
    if (child === win.ui || !('webContents' in child)) continue
    const view = child as Electron.WebContentsView
    const bounds = view.getBounds()
    const image = await grab(view.webContents)
    if (!image) continue
    const file = `${name}.layer${index++}.png`
    writeFileSync(join(dir, file), image.toPNG())
    layers.push({ file, ...bounds })
  }
  const order = win.base.contentView.children.map((child) => (child === win.ui ? 'ui' : 'page'))
  const size = win.base.getContentBounds()
  writeFileSync(join(dir, `${name}.json`), JSON.stringify({ size, layers, uiOnTop: order.at(-1) === 'ui' }))
}

export function startHarness(): void {
  const file = process.env['PEEPWICK_HARNESS']
  if (!file || app.isPackaged) return
  const steps = JSON.parse(readFileSync(file, 'utf8')) as Step[]
  const dir = process.env['PEEPWICK_HARNESS_OUT'] ?? join(app.getPath('temp'), 'peepwick-shots')
  mkdirSync(dir, { recursive: true })
  app.setPath('downloads', dir)

  void (async () => {
    while (BrowserWin.all.size === 0 || ![...BrowserWin.all.values()][0].isUIReady) await sleep(100)
    const win = [...BrowserWin.all.values()][0]
    await sleep(800)
    for (const step of steps) {
      try {
        switch (step.do) {
          case 'wait':
            await sleep(step.ms)
            break
          case 'navigate':
            win.activeTab()?.load(step.url)
            break
          case 'newTab':
            win.openTab({ url: step.url })
            break
          case 'command':
            runCommand(win, step.id)
            break
          case 'ui':
            win.command(step.command)
            break
          case 'settings':
            updateSettings(step.patch)
            break
          case 'eval': {
            const result: unknown = await win.ui.webContents.executeJavaScript(step.js)
            if (result !== undefined) console.log('[harness] eval', JSON.stringify(result))
            break
          }
          case 'resize':
            win.base.setContentSize(step.width, step.height)
            break
          case 'capture':
            await capture([...BrowserWin.all.values()][step.window ?? 0] ?? win, dir, step.name)
            console.log('[harness] captured', step.name)
            break
          case 'evalTab': {
            const result: unknown = await win.activeTab()?.webContents?.executeJavaScript(step.js, true).catch(() => undefined)
            if (result !== undefined) console.log('[harness] evalTab', JSON.stringify(result))
            break
          }
          case 'tabKey': {
            const wc = win.activeTab()?.webContents
            if (wc) for (const type of ['keyDown', 'char', 'keyUp'] as const) wc.sendInputEvent({ type, keyCode: step.key })
            break
          }
          case 'importAll': {
            const sources = detectSources()
            console.log('[harness] sources', JSON.stringify(sources.map((s) => [s.browser, s.available])))
            const source = sources[step.index]
            if (source) console.log('[harness] importAll', JSON.stringify(await importFrom(win, source.id, { kinds: source.available, skipDuplicates: true, primaryPassword: step.primaryPassword })))
            break
          }
          case 'workspace':
            win.createWorkspace(step.name, step.icon)
            break
          case 'state':
            console.log('[harness] state', JSON.stringify(Object.values(win.toState().tabs).map((t) => ({ id: t.id, url: t.url, loading: t.loading, title: t.title, blocked: t.blocked }))))
            break
          case 'type':
            win.ui.webContents.insertText(step.text)
            break
          case 'key':
            for (const type of ['keyDown', 'keyUp'] as const) win.ui.webContents.sendInputEvent({ type, keyCode: step.key })
            break
          case 'page':
            openPage(win, step.page, step.sub)
            break
          case 'importDetected': {
            const sources = detectSources()
            console.log('[harness] sources', JSON.stringify(sources))
            const source = sources[step.index]
            if (source) console.log('[harness] imported', JSON.stringify(await importBookmarks(win, source.id, true)))
            break
          }
          case 'bookmarks': {
            const tree = bookmarkTree()
            const summary = (nodes: typeof tree.bar.children): unknown =>
              nodes.map((n) => (n.type === 'url' ? n.title : { [n.title]: summary(n.children) }))
            console.log('[harness] bookmarks', JSON.stringify({ bar: summary(tree.bar.children), other: summary(tree.other.children), counts: countNodes([tree.bar, tree.other]) }))
            break
          }
          case 'installExtension':
            await installExtension(step.id, { session: browsingSession() })
            console.log('[harness] installed', step.id)
            break
          case 'mouse': {
            const wc = step.target === 'tab' ? win.activeTab()?.webContents : win.ui.webContents
            if (!wc) break
            const { x, y } = step
            if (step.move) wc.sendInputEvent({ type: 'mouseMove', x, y })
            else
              for (const type of ['mouseDown', 'mouseUp'] as const)
                wc.sendInputEvent({ type, x, y, button: step.button ?? 'left', clickCount: 1 })
            break
          }
          case 'download':
            win.session.downloadURL(step.url)
            break
          case 'backup': {
            // Round trip without the file dialogs: encrypt, open, unlock, restore.
            const data = await encryptBackup(collectBackup(BACKUP_CATEGORIES), step.password)
            writeFileSync(join(dir, 'harness.pwbackup'), data)
            console.log('[harness] backup bytes', data.length)
            const token = holdBackup(data, 'harness.pwbackup')
            if (step.dialog) {
              win.command({ type: 'backup-restore', token, name: 'harness.pwbackup' })
              break
            }
            console.log('[harness] wrong password', JSON.stringify(await unlockBackup(token, `${step.password}x`)))
            const summary = await unlockBackup(token, step.password)
            console.log('[harness] unlocked', JSON.stringify(summary))
            if (step.restore) console.log('[harness] restored', JSON.stringify(await restoreBackup(win, token, BACKUP_CATEGORIES, [])))
            break
          }
          case 'confirm':
            void confirm(win, { title: step.title, body: step.body, lines: step.lines, danger: step.danger, confirm: 'OK', cancel: 'Cancel' }).then((ok) =>
              console.log('[harness] confirm', ok),
            )
            break
          case 'quit':
            lifecycle.forceQuit = true
            app.quit()
            return
        }
      } catch (error) {
        console.error('[harness] step failed', step, error)
      }
    }
  })()
}
