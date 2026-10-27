// Browsing history and favicons in SQLite, using Node's built-in node:sqlite so the
// app ships without native modules.
import { DatabaseSync } from 'node:sqlite'
import { displayHost, hostOf } from '@shared/url'
import type { HistoryEntry, TopSite } from '@shared/types'
import { userPath } from './paths'

let db: DatabaseSync | null = null

export function openHistory(): void {
  db = new DatabaseSync(userPath('history.db'))
  db.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS visits (
      id INTEGER PRIMARY KEY,
      url TEXT NOT NULL,
      title TEXT NOT NULL DEFAULT '',
      visited_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS visits_time ON visits (visited_at DESC);
    CREATE INDEX IF NOT EXISTS visits_url ON visits (url);
    CREATE TABLE IF NOT EXISTS pages (
      url TEXT PRIMARY KEY,
      title TEXT NOT NULL DEFAULT '',
      visits INTEGER NOT NULL DEFAULT 0,
      last_visit INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS favicons (
      host TEXT PRIMARY KEY,
      data TEXT NOT NULL,
      updated_at INTEGER NOT NULL
    );
  `)
}

export function closeHistory(): void {
  db?.close()
  db = null
}

function recordable(url: string): boolean {
  return /^https?:\/\//.test(url) && url.length < 4096
}

/** Same page within a short window counts once (reloads, redirects back). */
const recent = new Map<string, number>()

export function addVisit(url: string, title: string): void {
  if (!db || !recordable(url)) return
  const now = Date.now()
  const last = recent.get(url)
  recent.set(url, now)
  if (recent.size > 200) recent.delete(recent.keys().next().value as string)
  if (last && now - last < 30_000) return
  db.prepare('INSERT INTO visits (url, title, visited_at) VALUES (?, ?, ?)').run(url, title, now)
  db.prepare(
    `INSERT INTO pages (url, title, visits, last_visit) VALUES (?, ?, 1, ?)
     ON CONFLICT(url) DO UPDATE SET visits = visits + 1, last_visit = excluded.last_visit,
       title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE pages.title END`,
  ).run(url, title, now)
}

export function updateTitle(url: string, title: string): void {
  if (!db || !recordable(url) || !title) return
  db.prepare('UPDATE pages SET title = ? WHERE url = ?').run(title, url)
  db.prepare(
    `UPDATE visits SET title = ? WHERE id = (SELECT id FROM visits WHERE url = ? ORDER BY visited_at DESC LIMIT 1)`,
  ).run(title, url)
}

export function setFavicon(host: string, data: string): void {
  if (!db || !host || data.length > 200_000) return
  db.prepare(
    `INSERT INTO favicons (host, data, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(host) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at`,
  ).run(host, data, Date.now())
}

export function getFavicon(host: string): string | null {
  if (!db || !host) return null
  const row = db.prepare('SELECT data FROM favicons WHERE host = ?').get(host) as { data: string } | undefined
  return row?.data ?? null
}

function escapeLike(text: string): string {
  return text.replace(/[\\%_]/g, (c) => `\\${c}`)
}

export function queryHistory(text: string, before = Date.now() + 1, limit = 100): HistoryEntry[] {
  if (!db) return []
  const like = `%${escapeLike(text.trim())}%`
  const rows = db
    .prepare(
      `SELECT id, url, title, visited_at FROM visits
       WHERE visited_at < ? AND (? = '%%' OR url LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\')
       ORDER BY visited_at DESC LIMIT ?`,
    )
    .all(before, like, like, like, Math.min(500, limit)) as {
    id: number
    url: string
    title: string
    visited_at: number
  }[]
  return rows.map((row) => ({
    id: row.id,
    url: row.url,
    title: row.title,
    visitedAt: row.visited_at,
    favicon: getFavicon(hostOf(row.url)),
  }))
}

export interface PageMatch {
  url: string
  title: string
  visits: number
  lastVisit: number
}

/** Candidates for address bar suggestions, ranked by the caller. */
export function matchPages(text: string, limit = 40): PageMatch[] {
  if (!db || !text.trim()) return []
  const like = `%${escapeLike(text.trim())}%`
  return (
    db
      .prepare(
        `SELECT url, title, visits, last_visit FROM pages
         WHERE url LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\'
         ORDER BY visits DESC, last_visit DESC LIMIT ?`,
      )
      .all(like, like, limit) as { url: string; title: string; visits: number; last_visit: number }[]
  ).map((row) => ({ url: row.url, title: row.title, visits: row.visits, lastVisit: row.last_visit }))
}

/** Most-visited sites, weighted towards recent visits and grouped by host. */
export function topSites(limit = 8): TopSite[] {
  if (!db) return []
  const now = Date.now()
  const rows = db
    .prepare(`SELECT url, title, visits, last_visit FROM pages ORDER BY last_visit DESC LIMIT 2000`)
    .all() as { url: string; title: string; visits: number; last_visit: number }[]
  const byHost = new Map<string, { score: number; url: string; title: string; best: number }>()
  for (const row of rows) {
    const host = hostOf(row.url)
    if (!host) continue
    const ageDays = (now - row.last_visit) / 86_400_000
    const score = row.visits / (1 + ageDays / 7)
    const entry = byHost.get(host)
    if (!entry) {
      byHost.set(host, { score, url: new URL(row.url).origin, title: row.title, best: score })
    } else {
      entry.score += score
      if (score > entry.best && new URL(row.url).pathname === '/') {
        entry.best = score
        entry.title = row.title
      }
    }
  }
  return [...byHost.entries()]
    .sort((a, b) => b[1].score - a[1].score)
    .slice(0, limit)
    .map(([host, entry]) => ({ url: entry.url, title: displayHost(entry.url) || entry.title, favicon: getFavicon(host) }))
}

export function removeVisit(id: number): void {
  if (!db) return
  const row = db.prepare('SELECT url FROM visits WHERE id = ?').get(id) as { url: string } | undefined
  db.prepare('DELETE FROM visits WHERE id = ?').run(id)
  if (row) {
    const left = db.prepare('SELECT COUNT(*) AS n FROM visits WHERE url = ?').get(row.url) as { n: number }
    if (left.n === 0) db.prepare('DELETE FROM pages WHERE url = ?').run(row.url)
    else db.prepare('UPDATE pages SET visits = ? WHERE url = ?').run(left.n, row.url)
  }
}

export function clearHistory(): void {
  if (!db) return
  db.exec('DELETE FROM visits; DELETE FROM pages; DELETE FROM favicons; VACUUM;')
  recent.clear()
}

/** Pages whose visits changed: recount them, drop the ones left without visits. */
function refreshPages(urls: string[]): void {
  if (!db) return
  const count = db.prepare('SELECT COUNT(*) AS n, MAX(visited_at) AS last FROM visits WHERE url = ?')
  for (const url of urls) {
    const row = count.get(url) as { n: number; last: number | null }
    if (row.n === 0) db.prepare('DELETE FROM pages WHERE url = ?').run(url)
    else db.prepare('UPDATE pages SET visits = ?, last_visit = ? WHERE url = ?').run(row.n, row.last, url)
  }
}

export function countVisits(since = 0): number {
  if (!db) return 0
  return (db.prepare('SELECT COUNT(*) AS n FROM visits WHERE visited_at >= ?').get(since) as { n: number }).n
}

/** Deletes visits from `since` on; with 0, the whole history (and stored site icons). */
export function deleteVisitsSince(since: number): number {
  if (!db) return 0
  if (since <= 0) {
    const n = countVisits(0)
    clearHistory()
    return n
  }
  const urls = (db.prepare('SELECT DISTINCT url FROM visits WHERE visited_at >= ?').all(since) as { url: string }[]).map((r) => r.url)
  const { changes } = db.prepare('DELETE FROM visits WHERE visited_at >= ?').run(since)
  refreshPages(urls)
  recent.clear()
  return Number(changes)
}

export function removeVisits(ids: number[]): void {
  if (!db || ids.length === 0) return
  const urls = new Set<string>()
  const find = db.prepare('SELECT url FROM visits WHERE id = ?')
  const remove = db.prepare('DELETE FROM visits WHERE id = ?')
  for (const id of ids) {
    const row = find.get(id) as { url: string } | undefined
    if (!row) continue
    urls.add(row.url)
    remove.run(id)
  }
  refreshPages([...urls])
}

/** Every visit to a host (and its www. twin). */
export function deleteHost(host: string): number {
  if (!db || !host) return 0
  const bare = host.replace(/^www\./, '')
  const urls = (db.prepare('SELECT url FROM pages').all() as { url: string }[])
    .map((r) => r.url)
    .filter((url) => hostOf(url).replace(/^www\./, '') === bare)
  let removed = 0
  const remove = db.prepare('DELETE FROM visits WHERE url = ?')
  for (const url of urls) removed += Number(remove.run(url).changes)
  refreshPages(urls)
  recent.clear()
  return removed
}

// ── Backup ────────────────────────────────────────────────────────────────────

export interface VisitRow {
  url: string
  title: string
  visitedAt: number
}

export function exportVisits(): VisitRow[] {
  if (!db) return []
  return (db.prepare('SELECT url, title, visited_at FROM visits ORDER BY visited_at').all() as { url: string; title: string; visited_at: number }[]).map((r) => ({
    url: r.url,
    title: r.title,
    visitedAt: r.visited_at,
  }))
}

/** Adds visits that aren't here yet (same address at the same moment); returns how many. */
export function importVisits(rows: VisitRow[]): number {
  if (!db) return 0
  const exists = db.prepare('SELECT 1 FROM visits WHERE url = ? AND visited_at = ?')
  const insert = db.prepare('INSERT INTO visits (url, title, visited_at) VALUES (?, ?, ?)')
  const touched = new Map<string, string>()
  let added = 0
  db.exec('BEGIN')
  try {
    for (const row of rows) {
      if (!recordable(row.url) || !Number.isFinite(row.visitedAt)) continue
      if (exists.get(row.url, row.visitedAt)) continue
      insert.run(row.url, row.title.slice(0, 1000), row.visitedAt)
      touched.set(row.url, row.title)
      added++
    }
    for (const [url, title] of touched) {
      db.prepare(
        `INSERT INTO pages (url, title, visits, last_visit) VALUES (?, ?, 0, 0)
         ON CONFLICT(url) DO UPDATE SET title = CASE WHEN pages.title = '' THEN excluded.title ELSE pages.title END`,
      ).run(url, title)
    }
    refreshPages([...touched.keys()])
    db.exec('COMMIT')
  } catch (error) {
    db.exec('ROLLBACK')
    throw error
  }
  return added
}
