// The backup file format. Kept free of Electron so the tests can use it directly.
//
//   "PWBK" | version (1) | log2 N, r, p (scrypt) | salt (16) | IV (12) | ciphertext | tag (16)
//
// The payload is gzipped JSON, sealed with AES-256-GCM under a key that scrypt
// derives from the password. The header is authenticated too (associated data), so
// a file whose parameters were tampered with fails like a wrong password does.
import { createCipheriv, createDecipheriv, randomBytes, scrypt } from 'node:crypto'
import { gunzipSync, gzipSync } from 'node:zlib'

export const BACKUP_EXTENSION = 'pwbackup'
const MAGIC = Buffer.from('PWBK', 'ascii')
const VERSION = 1
const HEADER = 4 + 1 + 3 + 16 + 12
const TAG = 16

export interface KdfParams {
  logN: number
  r: number
  p: number
}

/** About half a second and 128 MB on a laptop: slow enough to make guessing expensive. */
export const DEFAULT_KDF: KdfParams = { logN: 17, r: 8, p: 1 }

export class BackupError extends Error {
  constructor(readonly reason: 'format' | 'password') {
    super(reason === 'format' ? 'Not a PEEPWICK Browser backup' : 'Wrong password or damaged file')
  }
}

function deriveKey(password: string, salt: Buffer, { logN, r, p }: KdfParams): Promise<Buffer> {
  const N = 2 ** logN
  return new Promise((resolve, reject) =>
    scrypt(password.normalize('NFC'), salt, 32, { N, r, p, maxmem: 128 * N * r * 2 }, (error, key) => (error ? reject(error) : resolve(key))),
  )
}

export async function encryptBackup(payload: unknown, password: string, kdf: KdfParams = DEFAULT_KDF): Promise<Buffer> {
  const salt = randomBytes(16)
  const iv = randomBytes(12)
  const header = Buffer.concat([MAGIC, Buffer.from([VERSION, kdf.logN, kdf.r, kdf.p]), salt, iv])
  const key = await deriveKey(password, salt, kdf)
  const cipher = createCipheriv('aes-256-gcm', key, iv)
  cipher.setAAD(header)
  const body = Buffer.concat([cipher.update(gzipSync(Buffer.from(JSON.stringify(payload), 'utf8'))), cipher.final()])
  return Buffer.concat([header, body, cipher.getAuthTag()])
}

/** True when the bytes start like a backup file, before asking for the password. */
export function looksLikeBackup(data: Buffer): boolean {
  return data.length > HEADER + TAG && data.subarray(0, 4).equals(MAGIC) && data[4] === VERSION
}

export async function decryptBackup(data: Buffer, password: string): Promise<unknown> {
  if (!looksLikeBackup(data)) throw new BackupError('format')
  const header = data.subarray(0, HEADER)
  const kdf = { logN: data[5], r: data[6], p: data[7] }
  // Refuse parameters no file of ours would have; they could only make us hang.
  if (kdf.logN < 10 || kdf.logN > 20 || kdf.r < 1 || kdf.r > 16 || kdf.p < 1 || kdf.p > 4) throw new BackupError('format')
  const salt = data.subarray(8, 24)
  const iv = data.subarray(24, HEADER)
  const key = await deriveKey(password, salt, kdf)
  const decipher = createDecipheriv('aes-256-gcm', key, iv)
  decipher.setAAD(header)
  decipher.setAuthTag(data.subarray(data.length - TAG))
  let plain: Buffer
  try {
    plain = Buffer.concat([decipher.update(data.subarray(HEADER, data.length - TAG)), decipher.final()])
  } catch {
    throw new BackupError('password')
  }
  try {
    return JSON.parse(gunzipSync(plain).toString('utf8'))
  } catch {
    throw new BackupError('format')
  }
}
