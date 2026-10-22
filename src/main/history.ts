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
