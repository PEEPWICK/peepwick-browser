// Firefox's saved logins: logins.json holds usernames and passwords encrypted with
// a key that key4.db keeps, itself encrypted under the profile's primary password
// (empty unless the person set one). Everything is DER: a small reader below.
//
// key4.db entries use PBES2 (PBKDF2-SHA256 + AES-256-CBC) in current Firefox, or
// NSS's older SHA-1 + 3DES scheme; logins themselves are 3DES or AES-256-CBC.
import { createCipheriv, createDecipheriv, createHash, createHmac, pbkdf2Sync } from 'node:crypto'

// ── DER ──────────────────────────────────────────────────────────────────────

export interface Der {
  tag: number
  value: Buffer
  children: Der[]
}

export function parseDer(data: Buffer, start = 0, end = data.length): Der[] {
  const out: Der[] = []
  let i = start
  while (i < end) {
    const tag = data[i++]
    let length = data[i++]
    if (length & 0x80) {
      const bytes = length & 0x7f
      length = 0
      for (let k = 0; k < bytes; k++) length = length * 256 + data[i++]
    }
    const value = data.subarray(i, i + length)
    // Constructed types (SEQUENCE, SET, context-specific constructed) hold children.
    const children = tag & 0x20 ? parseDer(data, i, i + length) : []
    out.push({ tag, value, children })
    i += length
  }
  return out
}

export function oid(node: Der): string {
  const bytes = node.value
  const parts = [Math.floor(bytes[0] / 40), bytes[0] % 40]
  let n = 0
  for (const b of bytes.subarray(1)) {
    n = n * 128 + (b & 0x7f)
    if (!(b & 0x80)) {
      parts.push(n)
      n = 0
    }
  }
  return parts.join('.')
}

const integer = (node: Der): number => node.value.reduce((n, b) => n * 256 + b, 0)

export const OID = {
  pbes2: '1.2.840.113549.1.5.13',
  pbkdf2: '1.2.840.113549.1.5.12',
  sha1TripleDes: '1.2.840.113549.1.12.5.1.3',
  aes256cbc: '2.16.840.1.101.3.4.1.42',
  tripleDesCbc: '1.2.840.113549.3.7',
  hmacSha256: '1.2.840.113549.2.9',
}

// ── key4.db entries ──────────────────────────────────────────────────────────

function unpad(data: Buffer): Buffer {
  const pad = data[data.length - 1]
  return pad > 0 && pad <= 16 ? data.subarray(0, data.length - pad) : data
}

/** Decrypts one key4.db entry (the password check, or the key itself). */
export function decryptKey4Entry(der: Buffer, globalSalt: Buffer, primaryPassword: string): Buffer {
  const [root] = parseDer(der)
  const [algorithm, encrypted] = root.children
  const [algorithmOid, params] = algorithm.children
  const id = oid(algorithmOid)
  const password = Buffer.from(primaryPassword, 'utf8')
  if (id === OID.pbes2) {
    const [kdf, cipher] = params.children
    const [, kdfParams] = kdf.children
    const [salt, iterations, keyLength] = kdfParams.children
    const [, ivNode] = cipher.children
    const base = createHash('sha1').update(Buffer.concat([globalSalt, password])).digest()
    const length = keyLength && keyLength.tag === 0x02 ? integer(keyLength) : 32
    const key = pbkdf2Sync(base, salt.value, integer(iterations), length, 'sha256')
    // NSS stores the 14-byte IV DER-encoded; the real IV is that encoding (04 0e + bytes).
    const iv = Buffer.concat([Buffer.from([0x04, 0x0e]), ivNode.value])
    const decipher = createDecipheriv('aes-256-cbc', key, iv)
    decipher.setAutoPadding(false)
    return unpad(Buffer.concat([decipher.update(encrypted.value), decipher.final()]))
  }
  if (id === OID.sha1TripleDes) {
    const [entrySalt] = params.children
    const es = entrySalt.value
    const hp = createHash('sha1').update(Buffer.concat([globalSalt, password])).digest()
    const pes = Buffer.concat([es, Buffer.alloc(Math.max(0, 20 - es.length))])
    const chp = createHash('sha1').update(Buffer.concat([hp, es])).digest()
    const k1 = createHmac('sha1', chp).update(Buffer.concat([pes, es])).digest()
    const tk = createHmac('sha1', chp).update(pes).digest()
    const k2 = createHmac('sha1', chp).update(Buffer.concat([tk, es])).digest()
    const k = Buffer.concat([k1, k2])
    const decipher = createDecipheriv('des-ede3-cbc', k.subarray(0, 24), k.subarray(k.length - 8))
    decipher.setAutoPadding(false)
    return unpad(Buffer.concat([decipher.update(encrypted.value), decipher.final()]))
  }
  throw new Error(`Unknown key4.db algorithm ${id}`)
}

export class PrimaryPasswordError extends Error {
  constructor() {
    super('Firefox primary password needed')
  }
}

/**
 * The login keys from key4.db's rows, by their id (a102, matching the key id in
 * each encrypted login). Throws PrimaryPasswordError when the password is wrong.
 */
export function loginKeys(
  meta: { item1: Uint8Array; item2: Uint8Array },
  privates: { a11: Uint8Array; a102: Uint8Array }[],
  primaryPassword = '',
): Map<string, Buffer> {
  const globalSalt = Buffer.from(meta.item1)
  let check: Buffer
  try {
    check = decryptKey4Entry(Buffer.from(meta.item2), globalSalt, primaryPassword)
  } catch {
    throw new PrimaryPasswordError()
  }
  if (!check.toString('latin1').startsWith('password-check')) throw new PrimaryPasswordError()
  const keys = new Map<string, Buffer>()
  for (const row of privates) {
    if (!row.a11 || !row.a102) continue
    try {
      keys.set(Buffer.from(row.a102).toString('hex'), decryptKey4Entry(Buffer.from(row.a11), globalSalt, primaryPassword))
    } catch {
      // A key we can't read is skipped; logins that need it are skipped too.
    }
  }
  return keys
}

/** Decrypts an encryptedUsername / encryptedPassword field (base64 DER). */
export function decryptLoginField(base64: string, keys: Map<string, Buffer>): string {
  const [root] = parseDer(Buffer.from(base64, 'base64'))
  const [keyId, algorithm, encrypted] = root.children
  const [cipherOid, iv] = algorithm.children
  const key = keys.get(keyId.value.toString('hex')) ?? [...keys.values()][0]
  if (!key) throw new Error('No key for this login')
  const id = oid(cipherOid)
  const decipher =
    id === OID.aes256cbc
      ? createDecipheriv('aes-256-cbc', key.subarray(0, 32), iv.value)
      : createDecipheriv('des-ede3-cbc', key.subarray(0, 24), iv.value)
  return Buffer.concat([decipher.update(encrypted.value), decipher.final()]).toString('utf8')
}

// ── Writing, for tests ───────────────────────────────────────────────────────

function derLength(length: number): Buffer {
  if (length < 0x80) return Buffer.from([length])
  const bytes: number[] = []
  for (let n = length; n > 0; n = Math.floor(n / 256)) bytes.unshift(n & 0xff)
  return Buffer.from([0x80 | bytes.length, ...bytes])
}

export const der = {
  node: (tag: number, value: Buffer) => Buffer.concat([Buffer.from([tag]), derLength(value.length), value]),
  seq: (...items: Buffer[]) => der.node(0x30, Buffer.concat(items)),
  octets: (value: Buffer) => der.node(0x04, value),
  int: (n: number) => {
    const bytes: number[] = []
    for (let v = n; v > 0; v = Math.floor(v / 256)) bytes.unshift(v & 0xff)
    if (bytes.length === 0 || bytes[0] & 0x80) bytes.unshift(0)
    return der.node(0x02, Buffer.from(bytes))
  },
  oid: (dotted: string) => {
    const [a, b, ...rest] = dotted.split('.').map(Number)
    const bytes = [a * 40 + b]
    for (const n of rest) {
      const chunk: number[] = [n & 0x7f]
      for (let v = Math.floor(n / 128); v > 0; v = Math.floor(v / 128)) chunk.unshift((v & 0x7f) | 0x80)
      bytes.push(...chunk)
    }
    return der.node(0x06, Buffer.from(bytes))
  },
}

/** Builds a PBES2 key4.db entry the way current Firefox does. */
export function encryptKey4Entry(plain: Buffer, globalSalt: Buffer, primaryPassword: string, salt: Buffer, iv14: Buffer, iterations = 1000): Buffer {
  const base = createHash('sha1').update(Buffer.concat([globalSalt, Buffer.from(primaryPassword, 'utf8')])).digest()
  const key = pbkdf2Sync(base, salt, iterations, 32, 'sha256')
  const cipher = createCipheriv('aes-256-cbc', key, Buffer.concat([Buffer.from([0x04, 0x0e]), iv14]))
  const encrypted = Buffer.concat([cipher.update(plain), cipher.final()])
  return der.seq(
    der.seq(
      der.oid(OID.pbes2),
      der.seq(
        der.seq(der.oid(OID.pbkdf2), der.seq(der.octets(salt), der.int(iterations), der.int(32), der.seq(der.oid(OID.hmacSha256)))),
        der.seq(der.oid(OID.aes256cbc), der.octets(iv14)),
      ),
    ),
    der.octets(encrypted),
  )
}

/** Encrypts a login field the way NSS's SDR does (3DES or AES-256-CBC). */
export function encryptLoginField(text: string, keyId: Buffer, key: Buffer, iv: Buffer, aes = false): string {
  const cipher = aes ? createCipheriv('aes-256-cbc', key.subarray(0, 32), iv) : createCipheriv('des-ede3-cbc', key.subarray(0, 24), iv)
  const encrypted = Buffer.concat([cipher.update(text, 'utf8'), cipher.final()])
  return der.seq(der.octets(keyId), der.seq(der.oid(aes ? OID.aes256cbc : OID.tripleDesCbc), der.octets(iv)), der.octets(encrypted)).toString('base64')
}
