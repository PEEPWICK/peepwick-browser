// What the page preload should switch on for a frame: fingerprinting protection
// (with a seed for the frame's site and this browser session) and cookie banner
// rejection. Both follow the shield, so a site with the shield off gets neither.
import { createHmac, randomBytes } from 'node:crypto'
import { ipcMain, type Session } from 'electron'
import { settings } from './settings'
import { shieldActiveFor, siteOf } from './shield'

/** A new key every launch and for every session, so seeds never repeat across them. */
const keys = new WeakMap<Session, Buffer>()

function keyFor(session: Session): Buffer {
  let key = keys.get(session)
  if (!key) {
    key = randomBytes(32)
    keys.set(session, key)
  }
  return key
}

export function seedFor(key: Buffer, site: string): number {
  return createHmac('sha256', key).update(site).digest().readUInt32LE(0)
}

export function installPageConfig(): void {
  ipcMain.on('peepwick:page-config', (event, href: unknown) => {
    let top: string | null = null
    try {
      top = event.senderFrame?.top?.url || null
    } catch {
      // The frame went away.
    }
    if (!top || top === 'about:blank') top = typeof href === 'string' ? href : null
    const prefs = settings()
    if (!top || !shieldActiveFor(top)) {
      event.returnValue = null
      return
    }
    const site = siteOf(top) ?? top
    event.returnValue = {
      seed: prefs.fingerprintProtection ? seedFor(keyFor(event.sender.session), site) : null,
      rejectCookieBanners: prefs.rejectCookieBanners,
    }
  })
}
