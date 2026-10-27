// The error log behind "Report a bug". It holds errors from the browser itself:
// crashes, failed processes, errors in our own code, and page load failures as
// bare Chromium error names. Addresses, page titles, file paths and user names are
// scrubbed before anything is stored. It lives in memory; with the setting on, also
// in a file on this computer so a crash's errors survive a restart. Nothing is sent
// anywhere: the person copies the report and decides where it goes.
import { app, type WebContents } from 'electron'
import { appendFileSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { release } from 'node:os'
import { dirname } from 'node:path'
import type { ErrorRecord, Report } from '@shared/types'
import { bus } from './events'
import { releaseStage, shownVersion, userPath } from './paths'
import { scrub } from './scrub'
import { settings } from './settings'

const MAX = 200
const FILE_LIMIT = 256 * 1024
const logFile = () => userPath('logs', 'errors.jsonl')

let records: ErrorRecord[] = []
const loadErrors = new Map<string, number>()

function persist(record: ErrorRecord): void {
  if (!settings().errorLog) return
  try {
    const file = logFile()
    mkdirSync(dirname(file), { recursive: true })
    appendFileSync(file, `${JSON.stringify(record)}\n`)
    if (statSync(file).size > FILE_LIMIT) writeFileSync(file, records.map((r) => JSON.stringify(r)).join('\n') + '\n')
  } catch {
    // The log must never take the browser down with it.
  }
}

export function recordError(source: string, message: string): void {
  const clean = scrub(message.trim())
  if (!clean) return
  const now = Date.now()
  const same = records.find((r) => r.source === source && r.message === clean)
  if (same) {
    same.count++
    same.time = now
    records = [same, ...records.filter((r) => r !== same)]
  } else {
    records = [{ time: now, source, message: clean, count: 1 }, ...records].slice(0, MAX)
  }
  persist({ time: now, source, message: clean, count: 1 })
}

/** A page didn't load: only the Chromium error name is kept, never the address. */
export function recordLoadError(name: string): void {
  const clean = /^ERR_[A-Z0-9_]+$/.test(name) ? name : 'ERR_OTHER'
  loadErrors.set(clean, (loadErrors.get(clean) ?? 0) + 1)
}

function readLogFile(): ErrorRecord[] {
  try {
    return readFileSync(logFile(), 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line) => JSON.parse(line) as ErrorRecord)
      .filter((r) => typeof r.time === 'number' && typeof r.message === 'string' && typeof r.source === 'string')
  } catch {
    return []
  }
}

export function deleteLogFile(): void {
  rmSync(logFile(), { force: true })
}

export function clearErrorLog(): void {
  records = []
  loadErrors.clear()
  deleteLogFile()
}

/** Starts listening for errors. Call once, early. */
export function startDiagnostics(): void {
  // Earlier sessions' errors, when the log file is kept.
  if (settings().errorLog) {
    const merged = new Map<string, ErrorRecord>()
    for (const r of readLogFile()) {
      const key = `${r.source}\u0000${r.message}`
      const seen = merged.get(key)
      if (seen) {
        seen.count += r.count || 1
        seen.time = Math.max(seen.time, r.time)
      } else merged.set(key, { ...r, count: r.count || 1 })
    }
    records = [...merged.values()].sort((a, b) => b.time - a.time).slice(0, MAX)
  }
  let keep = settings().errorLog
  bus.on('settings', () => {
    if (keep && !settings().errorLog) deleteLogFile()
    keep = settings().errorLog
  })

  process.on('uncaughtException', (error) => {
    console.error(error)
    recordError('main', error.stack ?? String(error))
  })
  process.on('unhandledRejection', (reason) => {
    console.error('Unhandled rejection:', reason)
    recordError('main', reason instanceof Error ? (reason.stack ?? reason.message) : String(reason))
  })
  const error = console.error.bind(console)
  console.error = (...args: unknown[]) => {
    error(...args)
    recordError('main', args.map((a) => (a instanceof Error ? (a.stack ?? a.message) : typeof a === 'string' ? a : safeJSON(a))).join(' '))
  }

  app.on('render-process-gone', (_event, contents, details) => {
    if (details.reason === 'clean-exit') return
    recordError(isUI(contents) ? 'interface' : 'page', `Renderer gone: ${details.reason} (exit ${details.exitCode})`)
  })
  app.on('child-process-gone', (_event, details) => {
    if (details.reason === 'clean-exit') return
    const name = details.serviceName ? ` · ${details.serviceName}` : ''
    recordError(details.type.toLowerCase(), `${details.type} process gone: ${details.reason} (exit ${details.exitCode})${name}`)
  })
}

const uiContents = new WeakSet<WebContents>()

/** The browser's own interface: its console errors are our bugs. */
export function watchInterface(contents: WebContents): void {
  uiContents.add(contents)
  contents.on('console-message', (event) => {
    if (event.level !== 'error') return
    const source = event.sourceId ? ` (${event.sourceId.split('/').pop()}:${event.lineNumber})` : ''
    recordError('interface', `${event.message}${source}`)
  })
}

function isUI(contents: WebContents): boolean {
  return uiContents.has(contents)
}

function safeJSON(value: unknown): string {
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}

// ── The report ────────────────────────────────────────────────────────────────

function packageKind(): string {
  if (!app.isPackaged) return 'source'
  return process.env['APPIMAGE'] ? 'AppImage' : '.deb'
}

function systemLine(): string {
  // What Chromium actually runs on, which can differ from the desktop session.
  const session = app.commandLine.getSwitchValue('ozone-platform') || process.env['XDG_SESSION_TYPE'] || (process.env['WAYLAND_DISPLAY'] ? 'wayland' : 'x11')
  const desktop = process.env['XDG_CURRENT_DESKTOP'] || 'unknown desktop'
  const gpu = app.getGPUFeatureStatus().gpu_compositing ?? 'unknown'
  return `Linux ${release()} · ${process.arch} · ${desktop} · ${session} · GPU compositing ${gpu}${settings().hardwareAcceleration ? '' : ' (acceleration off)'}`
}

const stamp = (time: number) => new Date(time).toISOString().replace('T', ' ').slice(0, 16)

export function buildReport(extensionCount: number): Report {
  const s = settings()
  const version = `${shownVersion()} (${releaseStage()}) · ${packageKind()}`
  const system = systemLine()
  const loads = [...loadErrors.entries()].map(([name, count]) => ({ name, count })).sort((a, b) => b.count - a.count)
  const lines = [
    `PEEPWICK Browser ${version}`,
    `Electron ${process.versions.electron} · Chromium ${process.versions.chrome} · Node ${process.versions.node}`,
    system,
    `Language ${s.language} · Theme ${s.theme} · Shield ${s.shieldLevel} · Secure DNS ${s.secureDns} · Extensions ${extensionCount}`,
    '',
    `Errors (${records.length})`,
    ...(records.length === 0
      ? ['none']
      : records.slice(0, 60).map((r) => `${stamp(r.time)} · ${r.source} · ${r.message.split('\n').slice(0, 4).join(' ⏎ ')}${r.count > 1 ? ` ×${r.count}` : ''}`)),
    '',
    `Page load errors (${loads.reduce((n, l) => n + l.count, 0)})`,
    ...(loads.length === 0 ? ['none'] : loads.slice(0, 20).map((l) => `${l.name} ×${l.count}`)),
  ]
  return { text: lines.join('\n'), version: shownVersion(), system, errors: records.slice(0, 60), loadErrors: loads, logFile: s.errorLog }
}

/** The GitHub bug form with the version, system, package and report filled in. */
export function issueURL(report: Report): string {
  const params = new URLSearchParams({ template: 'bug_report.yml', version: report.version, system: report.system })
  const kind = packageKind()
  params.set('package', kind === 'source' ? 'Kaynaktan · From source' : kind)
  // Long reports are cut to keep the address short; the full text is on the clipboard.
  params.set('report', report.text.length > 3500 ? `${report.text.slice(0, 3500)}\n…` : report.text)
  return `https://github.com/PEEPWICK/peepwick-browser/issues/new?${params.toString()}`
}
