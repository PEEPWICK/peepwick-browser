//! Browsing history and stored site icons in SQLite (history.db in the profile), in the
//! same tables the Electron version used. Private windows record nothing.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use base64::Engine;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::pages::host_of;
use crate::paths;
use crate::tab::now_ms;

const SCHEMA: &str = r"
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
";

/// Icons are kept as data URLs; larger ones aren't worth keeping.
const MAX_ICON: usize = 200_000;

/// Also read by the icon protocol, which WebKit may call on another thread.
static DB: Mutex<Option<Connection>> = Mutex::new(None);

thread_local! {
    /// The same page within a short time counts once (reloads, redirects back).
    static RECENT: RefCell<VecDeque<(String, i64)>> = const { RefCell::new(VecDeque::new()) };
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
    pub favicon: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PageMatch {
    pub url: String,
    pub title: String,
    pub visits: i64,
    pub last_visit: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TopSite {
    pub url: String,
    pub title: String,
    pub favicon: Option<String>,
}

pub fn open() {
    let path = paths::user_path("history.db");
    match Connection::open(&path).and_then(|db| db.execute_batch(SCHEMA).map(|()| db)) {
        Ok(db) => {
            if let Ok(mut slot) = DB.lock() {
                *slot = Some(db);
            }
        }
        Err(error) => eprintln!("[history] could not open {}: {error}", path.display()),
    }
}

fn with_db<R>(f: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Option<R> {
    let slot = DB.lock().ok()?;
    let db = slot.as_ref()?;
    match f(db) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[history] {error}");
            None
        }
    }
}

fn recordable(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() < 4096
}

fn now() -> i64 {
    now_ms() as i64
}

pub fn add_visit(url: &str, title: &str) {
    if !recordable(url) {
        return;
    }
    let now = now();
    let seen_recently = RECENT.with(|recent| {
        let mut recent = recent.borrow_mut();
        let last = recent
            .iter()
            .position(|(u, _)| u == url)
            .and_then(|i| recent.remove(i))
            .map(|(_, at)| at);
        recent.push_back((url.to_string(), now));
        if recent.len() > 200 {
            recent.pop_front();
        }
        last.is_some_and(|at| now - at < 30_000)
    });
    if seen_recently {
        return;
    }
    with_db(|db| {
        db.prepare_cached("INSERT INTO visits (url, title, visited_at) VALUES (?1, ?2, ?3)")?
            .execute(params![url, title, now])?;
        db.prepare_cached(
            "INSERT INTO pages (url, title, visits, last_visit) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET visits = visits + 1, last_visit = excluded.last_visit,
               title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE pages.title END",
        )?
        .execute(params![url, title, now])?;
        Ok(())
    });
}

pub fn update_title(url: &str, title: &str) {
    if !recordable(url) || title.is_empty() {
        return;
    }
    with_db(|db| {
        db.prepare_cached("UPDATE pages SET title = ?1 WHERE url = ?2")?
            .execute(params![title, url])?;
        db.prepare_cached(
            "UPDATE visits SET title = ?1
             WHERE id = (SELECT id FROM visits WHERE url = ?2 ORDER BY visited_at DESC LIMIT 1)",
        )?
        .execute(params![title, url])?;
        Ok(())
    });
}

/// Stores a site's icon; true when it changed.
pub fn set_favicon(host: &str, data: &str) -> bool {
    if host.is_empty() || data.len() > MAX_ICON {
        return false;
    }
    with_db(|db| {
        let current: Option<String> = db
            .prepare_cached("SELECT data FROM favicons WHERE host = ?1")?
            .query_row([host], |row| row.get(0))
            .optional()?;
        if current.as_deref() == Some(data) {
            return Ok(false);
        }
        db.prepare_cached(
            "INSERT INTO favicons (host, data, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(host) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
        )?
        .execute(params![host, data, now()])?;
        Ok(true)
    })
    .unwrap_or(false)
}

/// The icon for a host, or for its www. twin (a bookmark and the page it lands on often differ).
pub fn favicon(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    let twin = match host.strip_prefix("www.") {
        Some(bare) => bare.to_string(),
        None => format!("www.{host}"),
    };
    with_db(|db| {
        db.prepare_cached(
            "SELECT data FROM favicons WHERE host IN (?1, ?2) ORDER BY host = ?1 DESC LIMIT 1",
        )?
        .query_row(params![host, twin], |row| row.get(0))
        .optional()
    })
    .flatten()
}

pub fn favicon_for_url(url: &str) -> Option<String> {
    favicon(&host_of(url))
}

/// A stored icon as a file: its type and bytes.
pub fn favicon_file(host: &str) -> Option<(String, Vec<u8>)> {
    let data = favicon(host)?;
    let (meta, payload) = data.strip_prefix("data:")?.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    if !mime.starts_with("image/") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    Some((mime.to_string(), bytes))
}

fn like(text: &str) -> String {
    let escaped: String = text
        .trim()
        .chars()
        .flat_map(|c| {
            if matches!(c, '\\' | '%' | '_') {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect();
    format!("%{escaped}%")
}

pub fn query(text: &str, before: Option<i64>, limit: Option<i64>) -> Vec<HistoryEntry> {
    let pattern = like(text);
    let before = before.unwrap_or_else(|| now() + 1);
    let limit = limit.unwrap_or(100).clamp(1, 500);
    let rows = with_db(|db| {
        let mut statement = db.prepare_cached(
            r"SELECT id, url, title, visited_at FROM visits
              WHERE visited_at < ?1 AND (?2 = '%%' OR url LIKE ?2 ESCAPE '\' OR title LIKE ?2 ESCAPE '\')
              ORDER BY visited_at DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![before, pattern, limit], |row| {
            Ok(HistoryEntry { id: row.get(0)?, url: row.get(1)?, title: row.get(2)?, visited_at: row.get(3)?, favicon: None })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default();
    with_icons(rows, |entry| &entry.url, |entry, icon| entry.favicon = icon)
}

/// Adds stored icons, looking each host up once.
fn with_icons<T>(
    mut items: Vec<T>,
    url: impl Fn(&T) -> &str,
    mut set: impl FnMut(&mut T, Option<String>),
) -> Vec<T> {
    let mut icons: HashMap<String, Option<String>> = HashMap::new();
    for item in &mut items {
        let host = host_of(url(item));
        let icon = icons
            .entry(host.clone())
            .or_insert_with(|| favicon(&host))
            .clone();
        set(item, icon);
    }
    items
}

/// Candidates for address bar suggestions, ranked by the caller.
pub fn match_pages(text: &str, limit: i64) -> Vec<PageMatch> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let pattern = like(text);
    with_db(|db| {
        let mut statement = db.prepare_cached(
            r"SELECT url, title, visits, last_visit FROM pages
              WHERE url LIKE ?1 ESCAPE '\' OR title LIKE ?1 ESCAPE '\'
              ORDER BY visits DESC, last_visit DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![pattern, limit], |row| {
            Ok(PageMatch {
                url: row.get(0)?,
                title: row.get(1)?,
                visits: row.get(2)?,
                last_visit: row.get(3)?,
            })
        })?;
        rows.collect()
    })
    .unwrap_or_default()
}

/// Most-visited sites, weighted towards recent visits and grouped by host.
pub fn top_sites(limit: usize) -> Vec<TopSite> {
    let rows = with_db(|db| {
        let mut statement = db.prepare_cached(
            "SELECT url, title, visits, last_visit FROM pages ORDER BY last_visit DESC LIMIT 2000",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PageMatch {
                url: row.get(0)?,
                title: row.get(1)?,
                visits: row.get(2)?,
                last_visit: row.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default();
    let now = now() as f64;
    // host → (score, origin, title, best page score)
    let mut by_host: Vec<(String, f64, String, String, f64)> = Vec::new();
    for row in rows {
        let Ok(url) = url::Url::parse(&row.url) else {
            continue;
        };
        let Some(host) = url.host_str().map(str::to_string) else {
            continue;
        };
        let age_days = (now - row.last_visit as f64) / 86_400_000.0;
        let score = row.visits as f64 / (1.0 + age_days / 7.0);
        match by_host.iter_mut().find(|entry| entry.0 == host) {
            None => by_host.push((
                host,
                score,
                url.origin().ascii_serialization(),
                row.title,
                score,
            )),
            Some(entry) => {
                entry.1 += score;
                if score > entry.4 && url.path() == "/" {
                    entry.4 = score;
                    entry.3 = row.title;
                }
            }
        }
    }
    by_host.sort_by(|a, b| b.1.total_cmp(&a.1));
    by_host
        .into_iter()
        .take(limit)
        .map(|(host, _, origin, title, _)| {
            let shown = host.strip_prefix("www.").unwrap_or(&host).to_string();
            TopSite {
                title: if shown.is_empty() { title } else { shown },
                favicon: favicon(&host),
                url: origin,
            }
        })
        .collect()
}

/// Pages whose visits changed: recount them, drop the ones left without visits.
fn refresh_pages(db: &Connection, urls: &[String]) -> rusqlite::Result<()> {
    for url in urls {
        let (count, last): (i64, Option<i64>) = db
            .prepare_cached("SELECT COUNT(*), MAX(visited_at) FROM visits WHERE url = ?1")?
            .query_row([url], |row| Ok((row.get(0)?, row.get(1)?)))?;
        if count == 0 {
            db.prepare_cached("DELETE FROM pages WHERE url = ?1")?
                .execute([url])?;
        } else {
            db.prepare_cached("UPDATE pages SET visits = ?1, last_visit = ?2 WHERE url = ?3")?
                .execute(params![count, last, url])?;
        }
    }
    Ok(())
}

pub fn remove_visits(ids: &[i64]) {
    with_db(|db| {
        let mut urls = Vec::new();
        for id in ids {
            let url: Option<String> = db
                .prepare_cached("SELECT url FROM visits WHERE id = ?1")?
                .query_row([id], |row| row.get(0))
                .optional()?;
            let Some(url) = url else { continue };
            db.prepare_cached("DELETE FROM visits WHERE id = ?1")?
                .execute([id])?;
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        refresh_pages(db, &urls)
    });
}

/// Every visit to a host (and its www. twin); returns how many went.
pub fn delete_host(host: &str) -> usize {
    let bare = host.strip_prefix("www.").unwrap_or(host).to_string();
    if bare.is_empty() {
        return 0;
    }
    RECENT.with(|recent| recent.borrow_mut().clear());
    with_db(|db| {
        let urls: Vec<String> = db
            .prepare_cached("SELECT url FROM pages")?
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .filter(|url| host_of(url).strip_prefix("www.").unwrap_or(&host_of(url)) == bare)
            .collect();
        let mut removed = 0;
        for url in &urls {
            removed += db
                .prepare_cached("DELETE FROM visits WHERE url = ?1")?
                .execute([url])?;
        }
        refresh_pages(db, &urls)?;
        Ok(removed)
    })
    .unwrap_or(0)
}

pub fn count_visits(since: i64) -> i64 {
    with_db(|db| {
        db.prepare_cached("SELECT COUNT(*) FROM visits WHERE visited_at >= ?1")?
            .query_row([since], |row| row.get(0))
    })
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_patterns() {
        assert_eq!(like(" 100%_a\\b "), "%100\\%\\_a\\\\b%");
        assert_eq!(like(""), "%%");
    }
}
