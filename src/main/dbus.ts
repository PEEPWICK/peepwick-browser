// A small D-Bus client, just enough to ask the desktop's Secret Service (GNOME
// Keyring, KWallet's Secret Service) for one secret: the key Chromium-based browsers
// encrypt their passwords and cookies with. The Secret Service ties a session to
// the connection that opened it, so opening the session and reading the secret has
// to happen on one connection; command-line tools that connect anew for every call
// can't do that, hence this.
import { connect, type Socket } from 'node:net'

type Value = unknown

// ── Marshalling (little-endian) ──────────────────────────────────────────────

/** Splits a signature into its complete types: "a{ss}ao" → ["a{ss}", "ao"]. */
export function splitSignature(signature: string): string[] {
  const out: string[] = []
  let i = 0
  const one = (): string => {
    const start = i
    const c = signature[i++]
    if (c === 'a') one()
    else if (c === '(' || c === '{') {
      const close = c === '(' ? ')' : '}'
      while (signature[i] !== close) {
        if (i >= signature.length) throw new Error(`Bad signature ${signature}`)
        one()
      }
      i++
    }
    return signature.slice(start, i)
  }
  while (i < signature.length) out.push(one())
  return out
}

function alignment(type: string): number {
  switch (type[0]) {
    case 'y':
    case 'g':
    case 'v':
      return 1
    case 'n':
    case 'q':
      return 2
    case 'x':
    case 't':
    case 'd':
    case '(':
    case '{':
      return 8
    default:
      return 4
  }
}

class Writer {
  private chunks: number[] = []
  get length(): number {
    return this.chunks.length
  }
  align(n: number): void {
    while (this.chunks.length % n !== 0) this.chunks.push(0)
  }
  bytes(data: Uint8Array | number[]): void {
    for (const b of data) this.chunks.push(b)
  }
  u32(value: number, at?: number): void {
    const bytes = [value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff]
    if (at === undefined) this.bytes(bytes)
    else bytes.forEach((b, i) => (this.chunks[at + i] = b))
  }
  buffer(): Buffer {
    return Buffer.from(this.chunks)
  }
}

export function writeValue(w: Writer, type: string, value: Value): void {
  w.align(alignment(type))
  switch (type[0]) {
    case 'y':
      w.bytes([Number(value) & 0xff])
      return
    case 'b':
      w.u32(value ? 1 : 0)
      return
    case 'u':
    case 'i':
      w.u32(Number(value) >>> 0)
      return
    case 's':
    case 'o': {
      const data = Buffer.from(String(value), 'utf8')
      w.u32(data.length)
      w.bytes(data)
      w.bytes([0])
      return
    }
    case 'g': {
      const data = Buffer.from(String(value), 'ascii')
      w.bytes([data.length])
      w.bytes(data)
      w.bytes([0])
      return
    }
    case 'v': {
      const [signature, inner] = value as [string, Value]
      writeValue(w, 'g', signature)
      writeValue(w, signature, inner)
      return
    }
    case 'a': {
      const element = type.slice(1)
      const lengthAt = w.length
      w.u32(0)
      w.align(alignment(element))
      const start = w.length
      if (element === 'y') w.bytes(value as Uint8Array)
      else if (element[0] === '{') {
        const [keyType, valueType] = splitSignature(element.slice(1, -1))
        const entries = value instanceof Map ? [...value.entries()] : Object.entries(value as Record<string, Value>)
        for (const [k, v] of entries) {
          w.align(8)
          writeValue(w, keyType, k)
          writeValue(w, valueType, v)
        }
      } else for (const item of value as Value[]) writeValue(w, element, item)
      w.u32(w.length - start, lengthAt)
      return
    }
    case '(': {
      const fields = splitSignature(type.slice(1, -1))
      fields.forEach((field, i) => writeValue(w, field, (value as Value[])[i]))
      return
    }
    default:
      throw new Error(`Can't write D-Bus type ${type}`)
  }
}

class Reader {
  offset = 0
  constructor(
    private readonly data: Buffer,
    private readonly little: boolean,
    private readonly base = 0,
  ) {}
  align(n: number): void {
    const rel = this.offset - this.base
    this.offset += (n - (rel % n)) % n
  }
  u8(): number {
    return this.data[this.offset++]
  }
  u32(): number {
    const v = this.little ? this.data.readUInt32LE(this.offset) : this.data.readUInt32BE(this.offset)
    this.offset += 4
    return v
  }
  i32(): number {
    const v = this.little ? this.data.readInt32LE(this.offset) : this.data.readInt32BE(this.offset)
    this.offset += 4
    return v
  }
  slice(length: number): Buffer {
    const out = this.data.subarray(this.offset, this.offset + length)
    this.offset += length
    return out
  }
}

export function readValue(r: Reader, type: string): Value {
  r.align(alignment(type))
  switch (type[0]) {
    case 'y':
      return r.u8()
    case 'b':
      return r.u32() !== 0
    case 'u':
      return r.u32()
    case 'i':
      return r.i32()
    case 'n':
    case 'q': {
      const v = r.slice(2)
      return v[0] | (v[1] << 8)
    }
    case 'x':
    case 't':
    case 'd':
      return r.slice(8)
    case 's':
    case 'o': {
      const length = r.u32()
      const text = r.slice(length).toString('utf8')
      r.offset++
      return text
    }
    case 'g': {
      const length = r.u8()
      const text = r.slice(length).toString('ascii')
      r.offset++
      return text
    }
    case 'v': {
      const signature = readValue(r, 'g') as string
      return [signature, readValue(r, signature)]
    }
    case 'a': {
      const element = type.slice(1)
      const length = r.u32()
      r.align(alignment(element))
      const end = r.offset + length
      if (element === 'y') return Buffer.from(r.slice(length))
      const out: Value[] = []
      while (r.offset < end) {
        if (element[0] === '{') {
          r.align(8)
          const [keyType, valueType] = splitSignature(element.slice(1, -1))
          out.push([readValue(r, keyType), readValue(r, valueType)])
        } else out.push(readValue(r, element))
      }
      return out
    }
    case '(': {
      return splitSignature(type.slice(1, -1)).map((field) => readValue(r, field))
    }
    default:
      throw new Error(`Can't read D-Bus type ${type}`)
  }
}

// ── Messages ─────────────────────────────────────────────────────────────────

export interface Message {
  type: number
  serial: number
  replySerial?: number
  path?: string
  interface?: string
  member?: string
  errorName?: string
  signature: string
  body: Value[]
}

export function encodeMessage(call: { serial: number; destination: string; path: string; interface: string; member: string; signature?: string; body?: Value[] }): Buffer {
  const body = new Writer()
  const signature = call.signature ?? ''
  splitSignature(signature).forEach((type, i) => writeValue(body, type, call.body?.[i]))
  const fields: [number, [string, Value]][] = [
    [1, ['o', call.path]],
    [2, ['s', call.interface]],
    [3, ['s', call.member]],
    [6, ['s', call.destination]],
  ]
  if (signature) fields.push([8, ['g', signature]])
  const head = new Writer()
  head.bytes([0x6c, 1, 0, 1]) // little-endian, method call, no flags, version 1
  head.u32(body.length)
  head.u32(call.serial)
  writeValue(head, 'a(yv)', fields)
  head.align(8)
  return Buffer.concat([head.buffer(), body.buffer()])
}

/** One complete message from the start of `data`, or null if more bytes are needed. */
export function decodeMessage(data: Buffer): { message: Message; size: number } | null {
  if (data.length < 16) return null
  const little = data[0] === 0x6c
  const read32 = (at: number) => (little ? data.readUInt32LE(at) : data.readUInt32BE(at))
  const bodyLength = read32(4)
  const fieldsLength = read32(12)
  const headerEnd = 16 + fieldsLength + ((8 - ((16 + fieldsLength) % 8)) % 8)
  const size = headerEnd + bodyLength
  if (data.length < size) return null
  const r = new Reader(data, little)
  r.offset = 12
  const fields = readValue(r, 'a(yv)') as [number, [string, Value]][]
  const message: Message = { type: data[1], serial: read32(8), signature: '', body: [] }
  for (const [code, [, value]] of fields) {
    if (code === 1) message.path = value as string
    else if (code === 2) message.interface = value as string
    else if (code === 3) message.member = value as string
    else if (code === 4) message.errorName = value as string
    else if (code === 5) message.replySerial = value as number
    else if (code === 8) message.signature = value as string
  }
  const body = new Reader(data, little, headerEnd)
  body.offset = headerEnd
  message.body = splitSignature(message.signature).map((type) => readValue(body, type))
  return { message, size }
}

// ── Connection ───────────────────────────────────────────────────────────────

function sessionBusPath(): string {
  const address = process.env['DBUS_SESSION_BUS_ADDRESS'] ?? ''
  for (const part of address.split(';')) {
    const path = /unix:(?:.*,)?path=([^,]+)/.exec(part)?.[1]
    if (path) return decodeURIComponent(path)
    const abstract = /unix:(?:.*,)?abstract=([^,]+)/.exec(part)?.[1]
    if (abstract) return `\0${decodeURIComponent(abstract)}`
  }
  return `/run/user/${process.getuid?.() ?? 1000}/bus`
}

export class Bus {
  private serial = 1
  private buffer = Buffer.alloc(0)
  private readonly waiting = new Map<number, { resolve: (m: Message) => void; reject: (e: Error) => void }>()
  private readonly signals: ((m: Message) => void)[] = []

  private constructor(private readonly socket: Socket) {}

  static open(timeout = 10_000): Promise<Bus> {
    return new Promise((resolve, reject) => {
      const socket = connect({ path: sessionBusPath() })
      const timer = setTimeout(() => {
        socket.destroy()
        reject(new Error('D-Bus timeout'))
      }, timeout)
      let auth = ''
      socket.once('error', (error) => {
        clearTimeout(timer)
        reject(error)
      })
      socket.on('connect', () => {
        const uid = String(process.getuid?.() ?? 0)
        socket.write(Buffer.concat([Buffer.from([0]), Buffer.from(`AUTH EXTERNAL ${Buffer.from(uid).toString('hex')}\r\n`)]))
      })
      const onAuth = (chunk: Buffer) => {
        auth += chunk.toString('ascii')
        if (!auth.includes('\r\n')) return
        socket.off('data', onAuth)
        if (!auth.startsWith('OK')) {
          clearTimeout(timer)
          socket.destroy()
          reject(new Error('D-Bus authentication failed'))
          return
        }
        socket.write('BEGIN\r\n')
        const bus = new Bus(socket)
        socket.on('data', (data) => bus.receive(data))
        socket.on('close', () => bus.fail(new Error('D-Bus connection closed')))
        void bus
          .call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'Hello')
          .then(() => {
            clearTimeout(timer)
            resolve(bus)
          }, reject)
      }
      socket.on('data', onAuth)
    })
  }

  private receive(data: Buffer): void {
    this.buffer = Buffer.concat([this.buffer, data])
    for (;;) {
      const decoded = decodeMessage(this.buffer)
      if (!decoded) return
      this.buffer = this.buffer.subarray(decoded.size)
      const { message } = decoded
      if (message.type === 4) this.signals.forEach((listener) => listener(message))
      const waiter = message.replySerial !== undefined ? this.waiting.get(message.replySerial) : undefined
      if (!waiter) continue
      this.waiting.delete(message.replySerial!)
      if (message.type === 3) waiter.reject(new Error(`${message.errorName}: ${String(message.body[0] ?? '')}`))
      else waiter.resolve(message)
    }
  }

  private fail(error: Error): void {
    for (const waiter of this.waiting.values()) waiter.reject(error)
    this.waiting.clear()
  }

  call(destination: string, path: string, iface: string, member: string, signature = '', body: Value[] = [], timeout = 10_000): Promise<Value[]> {
    const serial = this.serial++
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(serial)
        reject(new Error(`D-Bus ${member} timed out`))
      }, timeout)
      this.waiting.set(serial, {
        resolve: (message) => {
          clearTimeout(timer)
          resolve(message.body)
        },
        reject: (error) => {
          clearTimeout(timer)
          reject(error)
        },
      })
      this.socket.write(encodeMessage({ serial, destination, path, interface: iface, member, signature, body }))
    })
  }

  onSignal(listener: (message: Message) => void): void {
    this.signals.push(listener)
  }

  close(): void {
    this.socket.destroy()
  }
}

// ── Secret Service ───────────────────────────────────────────────────────────

const SECRETS = 'org.freedesktop.secrets'
const SERVICE = 'org.freedesktop.Secret.Service'

/**
 * The secret stored under these attributes (the first matching item), or null when
 * there is none. A locked keyring is unlocked through the desktop's own prompt.
 */
export async function lookupSecret(attributes: Record<string, string>): Promise<Buffer | null> {
  const bus = await Bus.open()
  try {
    const [, session] = (await bus.call(SECRETS, '/org/freedesktop/secrets', SERVICE, 'OpenSession', 'sv', ['plain', ['s', '']])) as [Value, string]
    const [unlocked, locked] = (await bus.call(SECRETS, '/org/freedesktop/secrets', SERVICE, 'SearchItems', 'a{ss}', [attributes])) as [string[], string[]]
    let items = unlocked
    if (items.length === 0 && locked.length > 0) {
      const [now, prompt] = (await bus.call(SECRETS, '/org/freedesktop/secrets', SERVICE, 'Unlock', 'ao', [locked])) as [string[], string]
      items = now
      if (items.length === 0 && prompt !== '/') {
        await bus.call('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'AddMatch', 's', [
          `type='signal',interface='org.freedesktop.Secret.Prompt',path='${prompt}'`,
        ])
        const done = new Promise<boolean>((resolve) => bus.onSignal((m) => m.path === prompt && m.member === 'Completed' && resolve(!m.body[0])))
        await bus.call(SECRETS, prompt, 'org.freedesktop.Secret.Prompt', 'Prompt', 's', [''])
        const ok = await Promise.race([done, new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 120_000))])
        if (!ok) return null
        const [again] = (await bus.call(SECRETS, '/org/freedesktop/secrets', SERVICE, 'SearchItems', 'a{ss}', [attributes])) as [string[]]
        items = again
      }
    }
    if (items.length === 0) return null
    const [secrets] = (await bus.call(SECRETS, '/org/freedesktop/secrets', SERVICE, 'GetSecrets', 'aoo', [items.slice(0, 1), session])) as [[string, [string, Buffer, Buffer, string]][]]
    return secrets[0]?.[1][2] ?? null
  } finally {
    bus.close()
  }
}
