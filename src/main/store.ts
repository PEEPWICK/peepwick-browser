import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

/**
 * A JSON file written atomically (temp file + rename) with optional debouncing,
 * so a crash mid-write never leaves a half-written file behind.
 */
export class JsonFile<T> {
  private timer: NodeJS.Timeout | null = null
  private pending: T | null = null

  constructor(
    readonly path: string,
    private readonly delay = 1000,
  ) {}

  read(): T | null {
    try {
      return JSON.parse(readFileSync(this.path, 'utf8')) as T
    } catch {
      return null
    }
  }

  schedule(data: T): void {
    this.pending = data
    if (this.timer) return
    this.timer = setTimeout(() => this.flush(), this.delay)
  }

  flush(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    if (this.pending === null) return
    const data = this.pending
    this.pending = null
    this.writeNow(data)
  }

  writeNow(data: T): void {
    try {
      mkdirSync(dirname(this.path), { recursive: true })
      const tmp = `${this.path}.tmp`
      writeFileSync(tmp, JSON.stringify(data))
      renameSync(tmp, this.path)
    } catch (error) {
      console.error('[store] could not write', this.path, error)
    }
  }
}
