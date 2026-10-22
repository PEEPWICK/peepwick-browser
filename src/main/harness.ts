// Development-only screenshot harness. With PEEPWICK_HARNESS=<file.json> in an
// unpackaged build, plays a list of steps against the first window and saves
// captures of the UI and the visible pages (see scripts/shoot.mjs). Never active
// in packaged builds.
import { app } from 'electron'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { CommandId, UICommand, WorkspaceIcon } from '@shared/types'
import { installExtension } from 'electron-chrome-web-store'
import { runCommand } from './actions'
import { lifecycle } from './lifecycle'
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
          case 'eval':
            await win.ui.webContents.executeJavaScript(step.js)
            break
          case 'resize':
            win.base.setContentSize(step.width, step.height)
            break
          case 'capture':
            await capture([...BrowserWin.all.values()][step.window ?? 0] ?? win, dir, step.name)
            console.log('[harness] captured', step.name)
            break
          case 'evalTab':
            await win.activeTab()?.webContents?.executeJavaScript(step.js, true).catch(() => {})
            break
          case 'workspace':
            win.createWorkspace(step.name, step.icon)
            break
          case 'state':
            console.log('[harness] state', JSON.stringify(Object.values(win.toState().tabs).map((t) => ({ id: t.id, url: t.url, loading: t.loading, title: t.title, blocked: t.blocked }))))
            break
          case 'installExtension':
            await installExtension(step.id, { session: browsingSession() })
            console.log('[harness] installed', step.id)
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
