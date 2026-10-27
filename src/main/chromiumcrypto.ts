// Chromium's "OSCrypt" on Linux, for importing passwords and cookies. Values start
// with "v10" (key from the fixed password "peanuts", used when there is no keyring)
// or "v11" (key from a secret the browser keeps in the system keyring); both are
// AES-128-CBC with a key from PBKDF2-SHA1 and an IV of sixteen spaces.
import { createCipheriv, createDecipheriv, createHash, pbkdf2Sync } from 'node:crypto'
import { lookupSecret } from './dbus'

const IV = Buffer.alloc(16, 0x20)

export function chromiumKey(secret: string | Buffer): Buffer {
  return pbkdf2Sync(secret, 'saltysalt', 1, 16, 'sha1')
}

export interface ChromiumKeys {
  v10: Buffer
  /** Keys to try for v11 values: the keyring secret, then the empty one Chromium falls back to. */
  v11: Buffer[]
}

/** The keyring "application" each browser stores its secret under. */
export const KEYRING_APPS: Record<string, string> = {
  'Google Chrome': 'chrome',
  'Google Chrome Beta': 'chrome',
  'Google Chrome Dev': 'chrome',
  Chromium: 'chromium',
  Brave: 'brave',
  'Microsoft Edge': 'microsoft-edge',
  Vivaldi: 'vivaldi',
  Opera: 'opera',
}

/** Keys for a browser; reads its keyring secret, which the desktop may ask to unlock. */
export async function chromiumKeys(browser: string): Promise<ChromiumKeys> {
  const v11: Buffer[] = []
  const app = KEYRING_APPS[browser]
  if (app) {
    try {
      const secret = await lookupSecret({ application: app })
      if (secret) v11.push(chromiumKey(secret))
    } catch (error) {
      console.warn('[import] keyring unavailable', error instanceof Error ? error.message : error)
    }
  }
  v11.push(chromiumKey(''))
  return { v10: chromiumKey('peanuts'), v11 }
}

function decryptWith(key: Buffer, data: Buffer): Buffer | null {
  try {
    const decipher = createDecipheriv('aes-128-cbc', key, IV)
    return Buffer.concat([decipher.update(data), decipher.final()])
  } catch {
    return null
  }
}

/** Decrypts a password_value or encrypted_value; null when no key fits. */
export function decryptChromium(value: Uint8Array, keys: ChromiumKeys): Buffer | null {
  const data = Buffer.from(value)
  if (data.length === 0) return Buffer.alloc(0)
  const prefix = data.subarray(0, 3).toString('ascii')
  const body = data.subarray(3)
  if (prefix === 'v10') return decryptWith(keys.v10, body)
  if (prefix === 'v11') {
    for (const key of keys.v11) {
      const plain = decryptWith(key, body)
      if (plain) return plain
    }
    return null
  }
  // Very old profiles stored values in the clear.
  return data
}

/**
 * Cookies from Chromium 130 on (database version 24+) start with the SHA-256 of the
 * cookie's domain, to stop values being moved between domains.
 */
export function stripDomainHash(plain: Buffer, hostKey: string): Buffer {
  if (plain.length >= 32 && plain.subarray(0, 32).equals(createHash('sha256').update(hostKey).digest())) return plain.subarray(32)
  return plain
}

/** For tests: encrypts like Chromium does. */
export function encryptChromium(plain: string | Buffer, key: Buffer, prefix: 'v10' | 'v11'): Buffer {
  const cipher = createCipheriv('aes-128-cbc', key, IV)
  return Buffer.concat([Buffer.from(prefix), cipher.update(typeof plain === 'string' ? Buffer.from(plain, 'utf8') : plain), cipher.final()])
}
