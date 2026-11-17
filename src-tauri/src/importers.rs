//! Importing from other browsers on this computer, the Chromium family and the Firefox family
//! (Snap and Flatpak installs included): bookmarks, with the icons of the bookmarked sites,
//! history, saved passwords (browsercrypto.rs decrypts them, with the browser's key from the
//! desktop's keyring or Firefox's primary password), and form entries and addresses. Browsers
//! keep their databases locked while they run, so SQLite files are copied to a private temporary
//! folder and read from there. Only files on this computer are read; nothing is sent anywhere.
//! Cookies come with that feature.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use rusqlite::Connection;
use serde_json::{Value, json};
use webkit2gtk::glib;

use crate::bookmarks::{self, Draft, DraftTree, Parsed};
use crate::browsercrypto::{self, ChromiumKeys, FirefoxError};
use crate::formdata::{Address, FormEntry};
use crate::history::{self, Visit};
use crate::i18n::{Var, t, tv};
use crate::pages;
use crate::passwords::PlainLogin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Chromium,
    Firefox,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Family::Chromium => "chromium",
            Family::Firefox => "firefox",
        }
    }
}

/// Where each browser keeps its profiles, from the home folder.
const CHROMIUM: [(&str, &[&str]); 8] = [
    (
        "Google Chrome",
        &[
            ".config/google-chrome",
            ".var/app/com.google.Chrome/config/google-chrome",
        ],
    ),
    ("Google Chrome Beta", &[".config/google-chrome-beta"]),
    ("Google Chrome Dev", &[".config/google-chrome-unstable"]),
    (
        "Chromium",
        &[
            ".config/chromium",
            "snap/chromium/common/chromium",
            ".var/app/org.chromium.Chromium/config/chromium",
        ],
    ),
    (
        "Brave",
        &[
            ".config/BraveSoftware/Brave-Browser",
            ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser",
            "snap/brave/current/.config/BraveSoftware/Brave-Browser",
        ],
    ),
    (
        "Microsoft Edge",
        &[
            ".config/microsoft-edge",
            ".var/app/com.microsoft.Edge/config/microsoft-edge",
        ],
    ),
    (
        "Vivaldi",
        &[
            ".config/vivaldi",
            ".var/app/com.vivaldi.Vivaldi/config/vivaldi",
        ],
    ),
    (
        "Opera",
        &[
            ".config/opera",
            ".var/app/com.opera.Opera/config/opera",
            "snap/opera/current/.config/opera",
        ],
    ),
];

const FIREFOX: [(&str, &[&str]); 4] = [
    (
        "Firefox",
        &[
            ".mozilla/firefox",
            "snap/firefox/common/.mozilla/firefox",
            ".var/app/org.mozilla.firefox/.mozilla/firefox",
        ],
    ),
    (
        "LibreWolf",
        &[
            ".librewolf",
            ".var/app/io.gitlab.librewolf-community/.librewolf",
        ],
    ),
    ("Zen", &[".zen", ".var/app/app.zen_browser.zen/.zen"]),
    ("Floorp", &[".floorp", ".var/app/one.ablaze.floorp/.floorp"]),
];

/// Chromium counts time in microseconds since 1601; this is 1970 in its milliseconds.
const WEBKIT_EPOCH_MS: i64 = 11_644_473_600_000;
/// Visits read from one profile, newest first.
const MAX_VISITS: i64 = 100_000;
/// Folders nested deeper than this are left empty (a damaged file could loop).
const MAX_DEPTH: usize = 64;

/// A profile with something to import, and where it is.
#[derive(Clone, Debug)]
struct Found {
    id: String,
    browser: &'static str,
    profile: String,
    family: Family,
    links: usize,
    available: Vec<&'static str>,
    dir: PathBuf,
}

/// The last search, which the import dialog's choice refers to.
static FOUND: Mutex<Vec<Found>> = Mutex::new(Vec::new());

/// Titles for folders that come without one of their own.
struct Labels {
    folder: String,
    mobile: String,
    firefox_menu: String,
}

impl Labels {
    fn current() -> Labels {
        Labels {
            folder: t("bookmarks.folder"),
            mobile: t("bookmarks.mobile"),
            firefox_menu: t("bookmarks.firefoxMenu"),
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(glib::home_dir)
}

// ── Finding profiles ─────────────────────────────────────────────────────────

fn chromium_kinds(dir: &Path) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if dir.join("Bookmarks").is_file() {
        kinds.push("bookmarks");
    }
    if dir.join("Login Data").is_file() {
        kinds.push("passwords");
    }
    if dir.join("History").is_file() {
        kinds.push("history");
    }
    if dir.join("Web Data").is_file() {
        kinds.push("autofill");
    }
    kinds
}

fn firefox_kinds(dir: &Path) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if dir.join("places.sqlite").is_file() {
        kinds.push("bookmarks");
    }
    if dir.join("logins.json").is_file() && dir.join("key4.db").is_file() {
        kinds.push("passwords");
    }
    if dir.join("places.sqlite").is_file() {
        kinds.push("history");
    }
    if dir.join("formhistory.sqlite").is_file() || dir.join("autofill-profiles.json").is_file() {
        kinds.push("autofill");
    }
    kinds
}

fn kinds(family: Family, dir: &Path) -> Vec<&'static str> {
    match family {
        Family::Chromium => chromium_kinds(dir),
        Family::Firefox => firefox_kinds(dir),
    }
}

/// "Profile 3" sorts as 3, after Default.
fn profile_number(name: &str) -> Option<u32> {
    if name == "Default" {
        return Some(0);
    }
    name.strip_prefix("Profile ")
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| n.saturating_add(1))
}

fn chromium_profiles(root: &Path) -> Vec<(PathBuf, String)> {
    // The names people gave their profiles, from Local State.
    let names: HashMap<String, String> = std::fs::read_to_string(root.join("Local State"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|state| state["profile"]["info_cache"].as_object().cloned())
        .map(|cache| {
            cache
                .iter()
                .filter_map(|(dir, info)| {
                    let name = info["name"].as_str().filter(|n| !n.is_empty())?;
                    Some((dir.clone(), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let name_of = |dir: &str| names.get(dir).cloned().unwrap_or_else(|| dir.to_string());
    let mut out = Vec::new();
    // Opera keeps a single profile in the root folder.
    if !chromium_kinds(root).is_empty() {
        out.push((root.to_path_buf(), name_of("Default")));
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<(u32, String)> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| Some((profile_number(&name)?, name)))
        .collect();
    dirs.sort();
    for (_, name) in dirs {
        let dir = root.join(&name);
        if !chromium_kinds(&dir).is_empty() {
            out.push((dir, name_of(&name)));
        }
    }
    out
}

/// The profiles a Firefox profiles.ini lists: (path, whether it is relative, name).
fn parse_profiles_ini(ini: &str) -> Vec<(String, bool, Option<String>)> {
    let mut out = Vec::new();
    for section in ini.split("\n[").skip(usize::from(!ini.starts_with('['))) {
        let section = section.strip_prefix('[').unwrap_or(section);
        if !section.to_ascii_lowercase().starts_with("profile") {
            continue;
        }
        let value = |key: &str| {
            section.lines().skip(1).find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
        };
        if let Some(path) = value("Path").filter(|p| !p.is_empty()) {
            out.push((
                path,
                value("IsRelative").as_deref() != Some("0"),
                value("Name"),
            ));
        }
    }
    out
}

fn firefox_profiles(root: &Path) -> Vec<(PathBuf, String)> {
    let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    parse_profiles_ini(&ini)
        .into_iter()
        .filter_map(|(path, relative, name)| {
            let dir = if relative {
                root.join(&path)
            } else {
                PathBuf::from(&path)
            };
            (!firefox_kinds(&dir).is_empty()).then(|| (dir, name.unwrap_or(path)))
        })
        .collect()
}

fn links_in(drafts: &[Draft]) -> usize {
    drafts
        .iter()
        .map(|draft| match draft {
            Draft::Link { .. } => 1,
            Draft::Folder { children, .. } => links_in(children),
        })
        .sum()
}

/// Looks for other browsers' profiles under a home folder.
fn find_in(home: &Path, labels: &Labels) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    // The same folder reached twice, through a link, counts once.
    let mut seen = HashSet::new();
    let families = CHROMIUM
        .iter()
        .map(|(browser, roots)| (Family::Chromium, *browser, *roots))
        .chain(
            FIREFOX
                .iter()
                .map(|(browser, roots)| (Family::Firefox, *browser, *roots)),
        );
    for (family, browser, roots) in families {
        for root in roots {
            let root = home.join(root);
            let profiles = match family {
                Family::Chromium => chromium_profiles(&root),
                Family::Firefox => firefox_profiles(&root),
            };
            for (dir, profile) in profiles {
                if let Ok(meta) = std::fs::metadata(&dir)
                    && !seen.insert((meta.dev(), meta.ino()))
                {
                    continue;
                }
                let mut available = kinds(family, &dir);
                let links = if available.contains(&"bookmarks") {
                    read_bookmarks(family, &dir, labels)
                        .map(|parsed| links_in(&parsed.tree.bar) + links_in(&parsed.tree.other))
                        .unwrap_or(0)
                } else {
                    0
                };
                if links == 0 {
                    available.retain(|kind| *kind != "bookmarks");
                }
                if available.is_empty() {
                    continue;
                }
                found.push(Found {
                    id: format!("{}:{}", family.name(), found.len()),
                    browser,
                    profile,
                    family,
                    links,
                    available,
                    dir,
                });
            }
        }
    }
    found
}

/// The profiles the import dialog offers. Takes a moment: not on the main thread.
pub fn detect_sources() -> Value {
    let found = find_in(&home(), &Labels::current());
    let list: Vec<Value> = found
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "browser": f.browser,
                "profile": f.profile,
                "kind": f.family.name(),
                "links": f.links,
                "available": f.available,
            })
        })
        .collect();
    if let Ok(mut slot) = FOUND.lock() {
        *slot = found;
    }
    json!(list)
}

// ── Reading ──────────────────────────────────────────────────────────────────

/// Copies a SQLite database, with its write-ahead log, to a private temporary folder and
/// reads it there.
fn with_copy<T>(
    dir: &Path,
    name: &str,
    read: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> Result<T, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = std::env::temp_dir().join(format!(
        "peepwick-import-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        std::fs::copy(dir.join(name), temp.join(name)).map_err(|e| e.to_string())?;
        for suffix in ["-wal", "-shm"] {
            let file = format!("{name}{suffix}");
            if dir.join(&file).is_file() {
                std::fs::copy(dir.join(&file), temp.join(&file)).map_err(|e| e.to_string())?;
            }
        }
        let db = Connection::open(temp.join(name)).map_err(|e| e.to_string())?;
        read(&db).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(&temp);
    result
}

/// Chromium writes times as strings of microseconds since 1601.
fn chromium_time(value: &Value) -> Option<u64> {
    let micros: i64 = value.as_str()?.parse().ok()?;
    let ms = micros / 1000 - WEBKIT_EPOCH_MS;
    (micros > 0 && ms > 0).then_some(ms as u64)
}

fn chromium_nodes(
    parent: Option<&Value>,
    labels: &Labels,
    skipped: &mut usize,
    depth: usize,
) -> Vec<Draft> {
    let mut out = Vec::new();
    let Some(children) = parent.and_then(|p| p["children"].as_array()) else {
        return out;
    };
    for node in children {
        let name = node["name"].as_str().filter(|n| !n.is_empty());
        match node["type"].as_str() {
            Some("folder") => out.push(Draft::Folder {
                title: name.map_or_else(|| labels.folder.clone(), str::to_string),
                added_at: chromium_time(&node["date_added"]),
                children: if depth < MAX_DEPTH {
                    chromium_nodes(Some(node), labels, skipped, depth + 1)
                } else {
                    Vec::new()
                },
            }),
            Some("url") => {
                let Some(url) = node["url"].as_str().filter(|u| !u.is_empty()) else {
                    continue;
                };
                if !bookmarks::bookmarkable(url) {
                    *skipped += 1;
                    continue;
                }
                out.push(Draft::Link {
                    title: name.unwrap_or(url).to_string(),
                    url: url.to_string(),
                    added_at: chromium_time(&node["date_added"]),
                });
            }
            _ => {}
        }
    }
    out
}

/// A Chromium Bookmarks file: the bar, other bookmarks, and the phone's in a folder of its own.
fn from_chromium(json: &Value, labels: &Labels) -> Result<Parsed, String> {
    let roots = json
        .get("roots")
        .filter(|r| r.is_object())
        .ok_or("not a Chromium bookmarks file")?;
    let mut skipped = 0;
    let bar = chromium_nodes(roots.get("bookmark_bar"), labels, &mut skipped, 0);
    let mut other = chromium_nodes(roots.get("other"), labels, &mut skipped, 0);
    let mobile = chromium_nodes(roots.get("synced"), labels, &mut skipped, 0);
    if !mobile.is_empty() {
        other.push(Draft::Folder {
            title: labels.mobile.clone(),
            added_at: None,
            children: mobile,
        });
    }
    Ok(Parsed {
        tree: DraftTree { bar, other },
        skipped,
    })
}

/// A row of Firefox's moz_bookmarks, with its address from moz_places.
#[derive(Debug)]
struct PlacesRow {
    id: i64,
    kind: i64,
    parent: i64,
    position: i64,
    title: Option<String>,
    url: Option<String>,
    /// Microseconds since 1970.
    date_added: Option<i64>,
    guid: String,
}

/// Firefox's bookmarks: the toolbar becomes the bar; the menu (in a folder of its own), the
/// unsorted ones and the phone's go to other bookmarks.
fn from_places(rows: &[PlacesRow], labels: &Labels) -> Parsed {
    let mut by_parent: HashMap<i64, Vec<&PlacesRow>> = HashMap::new();
    for row in rows {
        by_parent.entry(row.parent).or_default().push(row);
    }
    for list in by_parent.values_mut() {
        list.sort_by_key(|row| row.position);
    }
    fn build(
        parent: i64,
        by_parent: &HashMap<i64, Vec<&PlacesRow>>,
        labels: &Labels,
        skipped: &mut usize,
        depth: usize,
    ) -> Vec<Draft> {
        let mut out = Vec::new();
        for row in by_parent
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let added_at = row
                .date_added
                .filter(|d| *d > 0)
                .map(|d| ((d + 500) / 1000) as u64);
            let title = row.title.as_deref().filter(|t| !t.is_empty());
            match (row.kind, row.url.as_deref()) {
                (2, _) => out.push(Draft::Folder {
                    title: title.map_or_else(|| labels.folder.clone(), str::to_string),
                    added_at,
                    children: if depth < MAX_DEPTH {
                        build(row.id, by_parent, labels, skipped, depth + 1)
                    } else {
                        Vec::new()
                    },
                }),
                (1, Some(url)) if !url.is_empty() => {
                    if !bookmarks::bookmarkable(url) {
                        *skipped += 1;
                        continue;
                    }
                    out.push(Draft::Link {
                        title: title.unwrap_or(url).to_string(),
                        url: url.to_string(),
                        added_at,
                    });
                }
                _ => {}
            }
        }
        out
    }
    let mut skipped = 0;
    let mut root = |guid: &str| {
        rows.iter()
            .find(|row| row.guid == guid)
            .map(|row| build(row.id, &by_parent, labels, &mut skipped, 0))
            .unwrap_or_default()
    };
    let bar = root("toolbar_____");
    let menu = root("menu________");
    let unfiled = root("unfiled_____");
    let mobile = root("mobile______");
    let mut other = Vec::new();
    if !menu.is_empty() {
        other.push(Draft::Folder {
            title: labels.firefox_menu.clone(),
            added_at: None,
            children: menu,
        });
    }
    other.extend(unfiled);
    if !mobile.is_empty() {
        other.push(Draft::Folder {
            title: labels.mobile.clone(),
            added_at: None,
            children: mobile,
        });
    }
    Parsed {
        tree: DraftTree { bar, other },
        skipped,
    }
}

fn read_bookmarks(family: Family, dir: &Path, labels: &Labels) -> Result<Parsed, String> {
    match family {
        Family::Chromium => {
            let text = std::fs::read_to_string(dir.join("Bookmarks")).map_err(|e| e.to_string())?;
            let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            from_chromium(&json, labels)
        }
        Family::Firefox => {
            let rows: Vec<PlacesRow> = with_copy(dir, "places.sqlite", |db| {
                db.prepare(
                    "SELECT b.id, b.type, b.parent, b.position, b.title, p.url, b.dateAdded, b.guid
                     FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk",
                )?
                .query_map([], |row| {
                    Ok(PlacesRow {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        parent: row.get(2)?,
                        position: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        title: row.get(4)?,
                        url: row.get(5)?,
                        date_added: row.get(6)?,
                        guid: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })?
                .collect()
            })?;
            Ok(from_places(&rows, labels))
        }
    }
}

fn read_history(family: Family, dir: &Path) -> Result<Vec<Visit>, String> {
    let (file, sql) = match family {
        Family::Chromium => (
            "History",
            "SELECT u.url, u.title, v.visit_time FROM visits v JOIN urls u ON u.id = v.url
             ORDER BY v.visit_time DESC LIMIT ?1",
        ),
        Family::Firefox => (
            "places.sqlite",
            "SELECT p.url, p.title, v.visit_date FROM moz_historyvisits v
             JOIN moz_places p ON p.id = v.place_id ORDER BY v.visit_date DESC LIMIT ?1",
        ),
    };
    with_copy(dir, file, |db| {
        let mut visits = Vec::new();
        let mut statement = db.prepare(sql)?;
        let rows = statement.query_map([MAX_VISITS], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        for row in rows {
            let (Some(url), title, Some(time)) = row? else {
                continue;
            };
            let visited_at = match family {
                Family::Chromium => time / 1000 - WEBKIT_EPOCH_MS,
                Family::Firefox => (time + 500) / 1000,
            };
            visits.push(Visit {
                url,
                title: title.unwrap_or_default(),
                visited_at,
            });
        }
        Ok(visits)
    })
}

fn mime_of(data: &[u8]) -> Option<&'static str> {
    match data {
        [0x89, b'P', ..] => Some("image/png"),
        [0xff, 0xd8, ..] => Some("image/jpeg"),
        [b'G', b'I', ..] => Some("image/gif"),
        [0, 0, 1, ..] => Some("image/x-icon"),
        [b'<', ..] => Some("image/svg+xml"),
        _ => None,
    }
}

fn hosts_of(drafts: &[Draft], hosts: &mut HashSet<String>) {
    for draft in drafts {
        match draft {
            Draft::Link { url, .. } => {
                hosts.insert(pages::host_of(url));
            }
            Draft::Folder { children, .. } => hosts_of(children, hosts),
        }
    }
}

/// Copies the icons of bookmarked sites we have no icon for yet; best effort.
fn import_favicons(family: Family, dir: &Path, hosts: &HashSet<String>) -> usize {
    let wanted: HashSet<&String> = hosts
        .iter()
        .filter(|host| !host.is_empty() && history::favicon(host).is_none())
        .collect();
    let (file, sql) = match family {
        Family::Chromium => (
            "Favicons",
            "SELECT m.page_url, b.image_data, b.width FROM icon_mapping m
             JOIN favicon_bitmaps b ON b.icon_id = m.icon_id WHERE b.width <= 64",
        ),
        Family::Firefox => (
            "favicons.sqlite",
            "SELECT p.page_url, i.data, i.width FROM moz_pages_w_icons p
             JOIN moz_icons_to_pages ip ON ip.page_id = p.id
             JOIN moz_icons i ON i.id = ip.icon_id WHERE i.width <= 64",
        ),
    };
    if wanted.is_empty() || !dir.join(file).is_file() {
        return 0;
    }
    let mut best: HashMap<String, (i64, Vec<u8>)> = HashMap::new();
    let read = with_copy(dir, file, |db| {
        let mut statement = db.prepare(sql)?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        for row in rows {
            let (Some(url), Some(data), width) = row? else {
                continue;
            };
            let host = pages::host_of(&url);
            if !wanted.contains(&host) || data.is_empty() || data.len() > 100_000 {
                continue;
            }
            let width = width.unwrap_or(0);
            // 32 pixels, or the closest to it.
            if best
                .get(&host)
                .is_none_or(|(current, _)| (width - 32).abs() < (current - 32).abs())
            {
                best.insert(host, (width, data));
            }
        }
        Ok(())
    });
    if let Err(error) = read {
        crate::log_error!("[import] icons skipped: {error}");
    }
    best.into_iter()
        .filter(|(host, (_, data))| {
            mime_of(data).is_some_and(|mime| {
                let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                history::set_favicon(host, &format!("data:{mime};base64,{encoded}"))
            })
        })
        .count()
}

/// Chromium counts time in microseconds since 1601: milliseconds since 1970, if set.
fn chromium_ms(micros: Option<i64>) -> Option<u64> {
    micros
        .filter(|m| *m > 0)
        .and_then(|m| u64::try_from(m / 1000 - WEBKIT_EPOCH_MS).ok())
}

/// Chromium's saved logins: Login Data, and Login Data For Account (a signed-in Chrome's account
/// passwords). The logins, the sites never to save for, and how many couldn't be decrypted.
fn chromium_passwords(
    dir: &Path,
    keys: &ChromiumKeys,
) -> Result<(Vec<PlainLogin>, Vec<String>, usize), String> {
    type Row = (
        String,
        Option<String>,
        Option<Vec<u8>>,
        Option<i64>,
        Option<i64>,
        i64,
    );
    let mut logins = Vec::new();
    let mut never = Vec::new();
    let mut undecrypted = 0;
    for name in ["Login Data", "Login Data For Account"] {
        if !dir.join(name).is_file() {
            continue;
        }
        let rows: Vec<Row> = with_copy(dir, name, |db| {
            let columns: HashSet<String> = db
                .prepare("PRAGMA table_info(logins)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<_>>()?;
            let modified = if columns.contains("date_password_modified") {
                "date_password_modified"
            } else {
                "0"
            };
            let blocked = if columns.contains("blacklisted_by_user") {
                "blacklisted_by_user"
            } else {
                "0"
            };
            let mut statement = db.prepare(&format!(
                "SELECT origin_url, username_value, password_value, date_created, {modified}, {blocked} FROM logins"
            ))?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    ))
                })?
                .collect()
        })?;
        for (origin, username, secret, created, modified, blocked) in rows {
            if !(origin.starts_with("https:") || origin.starts_with("http:")) {
                continue;
            }
            if blocked != 0 {
                never.push(origin);
                continue;
            }
            let Some(plain) = browsercrypto::decrypt_chromium(&secret.unwrap_or_default(), keys)
            else {
                undecrypted += 1;
                continue;
            };
            let created = chromium_ms(created);
            logins.push(PlainLogin {
                origin,
                username: username.unwrap_or_default(),
                password: String::from_utf8_lossy(&plain).into_owned(),
                note: None,
                created,
                updated: chromium_ms(modified).or(created),
            });
        }
    }
    Ok((logins, never, undecrypted))
}

/// Firefox's saved logins, or that the profile's primary password is needed.
enum FirefoxLogins {
    Read(Vec<PlainLogin>, Vec<String>),
    NeedsPrimary,
}

fn firefox_passwords(dir: &Path, primary: &str) -> Result<FirefoxLogins, String> {
    type Keys = (Vec<u8>, Vec<u8>, Vec<(Vec<u8>, Vec<u8>)>);
    let (item1, item2, privates): Keys = with_copy(dir, "key4.db", |db| {
        let (item1, item2) = db.query_row(
            "SELECT item1, item2 FROM metadata WHERE id = 'password'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let privates = db
            .prepare("SELECT a11, a102 FROM nssPrivate")?
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(a11, a102)| Some((a11?, a102?)))
            .collect();
        Ok((item1, item2, privates))
    })?;
    let keys = match browsercrypto::login_keys(&item1, &item2, &privates, primary) {
        Ok(keys) => keys,
        Err(FirefoxError::PrimaryPassword) => return Ok(FirefoxLogins::NeedsPrimary),
    };
    let text = std::fs::read_to_string(dir.join("logins.json")).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut logins = Vec::new();
    for login in json["logins"].as_array().into_iter().flatten() {
        let origin = login["hostname"].as_str().unwrap_or_default();
        if !(origin.starts_with("https:") || origin.starts_with("http:")) {
            continue;
        }
        let field = |key: &str| {
            browsercrypto::decrypt_login_field(login[key].as_str().unwrap_or_default(), &keys)
        };
        // One unreadable login doesn't stop the rest.
        let (Some(username), Some(password)) =
            (field("encryptedUsername"), field("encryptedPassword"))
        else {
            continue;
        };
        let ms = |key: &str| login[key].as_f64().filter(|n| *n > 0.0).map(|n| n as u64);
        logins.push(PlainLogin {
            origin: origin.to_string(),
            username,
            password,
            note: None,
            created: ms("timeCreated"),
            updated: ms("timePasswordChanged").or(ms("timeCreated")),
        });
    }
    let never = json["disabledHosts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| h.as_str().map(str::to_string))
        .collect();
    Ok(FirefoxLogins::Read(logins, never))
}

// ── Form entries and addresses ──────────────────────────────────────────────────

/// Chromium's address field types (its server field types) that we keep.
fn chromium_address_part(kind: i64) -> Option<&'static str> {
    Some(match kind {
        7 => "name",
        9 => "email",
        14 => "phone",
        60 => "organization",
        // The whole street address; 30 is its first line.
        77 | 30 => "street",
        33 => "city",
        34 => "state",
        35 => "postalCode",
        36 => "country",
        _ => return None,
    })
}

/// Chromium's Web Data: what was typed into fields (seconds since 1970) and saved addresses,
/// one row per field in the current schema, one row per address in the old one.
fn chromium_autofill(dir: &Path) -> Result<(Vec<Address>, Vec<FormEntry>), String> {
    with_copy(dir, "Web Data", |db| {
        let entries = (|| -> rusqlite::Result<Vec<FormEntry>> {
            let mut select =
                db.prepare("SELECT name, value, count, date_last_used FROM autofill")?;
            let rows = select.query_map([], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            })?;
            Ok(rows
                .filter_map(Result::ok)
                .filter_map(|(name, value, count, used)| {
                    Some(FormEntry {
                        key: name?.to_lowercase(),
                        value: value?,
                        count: count.unwrap_or(1).max(0) as u64,
                        last_used: used.unwrap_or(0).max(0) as u64 * 1000,
                    })
                })
                .collect())
        })()
        .unwrap_or_default();
        let tokens = |table: &str| -> rusqlite::Result<Vec<Address>> {
            let mut select = db.prepare(&format!("SELECT guid, type, value FROM {table}"))?;
            let rows = select.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            let mut by_guid: Vec<(String, Value)> = Vec::new();
            for (guid, kind, value) in rows.filter_map(Result::ok) {
                let (Some(part), Some(value)) = (chromium_address_part(kind), value) else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                let at = match by_guid.iter().position(|(g, _)| *g == guid) {
                    Some(at) => at,
                    None => {
                        by_guid.push((guid, json!({})));
                        by_guid.len() - 1
                    }
                };
                let address = &mut by_guid[at].1;
                // A whole street address beats a first line.
                if part != "street" || kind == 77 || address.get("street").is_none() {
                    address[part] = json!(value);
                }
            }
            Ok(by_guid
                .iter()
                .map(|(_, a)| Address::from_value(a))
                .collect())
        };
        let addresses = tokens("address_type_tokens")
            .or_else(|_| tokens("local_addresses_type_tokens"))
            .or_else(|_| {
                let mut select = db.prepare(
                    "SELECT p.company_name, p.street_address, p.city, p.state, p.zipcode, p.country_code,
                       (SELECT full_name FROM autofill_profile_names n WHERE n.guid = p.guid LIMIT 1),
                       (SELECT email FROM autofill_profile_emails e WHERE e.guid = p.guid LIMIT 1),
                       (SELECT number FROM autofill_profile_phones f WHERE f.guid = p.guid LIMIT 1)
                     FROM autofill_profiles p",
                )?;
                let rows = select.query_map([], |row| {
                    let text = |i: usize| row.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
                    Ok(Address::from_value(&json!({
                        "organization": text(0)?,
                        "street": text(1)?,
                        "city": text(2)?,
                        "state": text(3)?,
                        "postalCode": text(4)?,
                        "country": text(5)?,
                        "name": text(6)?,
                        "email": text(7)?,
                        "phone": text(8)?,
                    })))
                })?;
                Ok::<_, rusqlite::Error>(rows.filter_map(Result::ok).collect())
            })
            .unwrap_or_default();
        Ok((addresses, entries))
    })
}

/// Firefox's formhistory.sqlite (microseconds) and its saved addresses (autofill-profiles.json).
fn firefox_autofill(dir: &Path) -> Result<(Vec<Address>, Vec<FormEntry>), String> {
    let entries = if dir.join("formhistory.sqlite").is_file() {
        with_copy(dir, "formhistory.sqlite", |db| {
            let mut select =
                db.prepare("SELECT fieldname, value, timesUsed, lastUsed FROM moz_formhistory")?;
            let rows = select.query_map([], |row| {
                Ok(FormEntry {
                    key: row.get::<_, String>(0)?.to_lowercase(),
                    value: row.get(1)?,
                    count: row.get::<_, Option<i64>>(2)?.unwrap_or(1).max(0) as u64,
                    last_used: (row.get::<_, Option<i64>>(3)?.unwrap_or(0).max(0) / 1000) as u64,
                })
            })?;
            Ok(rows.filter_map(Result::ok).collect())
        })?
    } else {
        Vec::new()
    };
    let json: Value = std::fs::read(dir.join("autofill-profiles.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let addresses = json["addresses"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let text = |key: &str| a[key].as_str().unwrap_or("").trim().to_string();
            let mut name = text("name");
            if name.is_empty() {
                name = ["given-name", "additional-name", "family-name"]
                    .into_iter()
                    .map(text)
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            Address::from_value(&json!({
                "name": name,
                "organization": text("organization"),
                "street": text("street-address"),
                "city": text("address-level2"),
                "state": text("address-level1"),
                "postalCode": text("postal-code"),
                "country": text("country"),
                "phone": text("tel"),
                "email": text("email"),
            }))
        })
        .collect();
    Ok((addresses, entries))
}

/// What was read from a profile, for the main thread to put in place.
#[derive(Clone, Debug)]
pub struct Imported {
    browser: &'static str,
    bookmarks: Option<(DraftTree, usize)>,
    history: Option<usize>,
    favicons: usize,
    /// Logins in the clear and the sites never to save for: they stay in memory, on their way
    /// into ours.
    passwords: Option<(Vec<PlainLogin>, Vec<String>)>,
    /// Addresses and form entries, which are kept with the passwords' key too.
    autofill: Option<(Vec<Address>, Vec<FormEntry>)>,
    needs_primary: bool,
    failed: Vec<&'static str>,
}

impl Imported {
    /// Whether putting it in place needs the passwords' key: passwords, addresses and form
    /// entries are kept with it.
    pub fn needs_key(&self) -> bool {
        self.passwords.is_some() || self.autofill.is_some()
    }
}

/// Reads the chosen kinds from a profile the last search found. History goes into ours
/// straight away; bookmarks and passwords wait for `finish` on the main thread. Not on the main
/// thread: a Chromium browser's keyring secret may need the keyring unlocked.
pub fn read(source_id: &str, kinds: &[String], primary: &str) -> Option<Imported> {
    let source = FOUND
        .lock()
        .ok()?
        .iter()
        .find(|s| s.id == source_id)
        .cloned()?;
    let wants = |kind: &str| kinds.iter().any(|k| k == kind) && source.available.contains(&kind);
    let mut imported = Imported {
        browser: source.browser,
        bookmarks: None,
        history: None,
        favicons: 0,
        passwords: None,
        autofill: None,
        needs_primary: false,
        failed: Vec::new(),
    };
    if wants("bookmarks") {
        match read_bookmarks(source.family, &source.dir, &Labels::current()) {
            Ok(parsed) => {
                let mut hosts = HashSet::new();
                hosts_of(&parsed.tree.bar, &mut hosts);
                hosts_of(&parsed.tree.other, &mut hosts);
                imported.favicons = import_favicons(source.family, &source.dir, &hosts);
                imported.bookmarks = Some((parsed.tree, parsed.skipped));
            }
            Err(error) => {
                crate::log_error!("[import] bookmarks failed: {error}");
                imported.failed.push("bookmarks");
            }
        }
    }
    if wants("passwords") {
        match source.family {
            Family::Chromium => {
                let keys = browsercrypto::chromium_keys(source.browser);
                match chromium_passwords(&source.dir, &keys) {
                    Ok((logins, never, undecrypted)) => {
                        if undecrypted > 0 && logins.is_empty() {
                            imported.failed.push("passwords");
                        }
                        imported.passwords = Some((logins, never));
                    }
                    Err(error) => {
                        crate::log_error!("[import] passwords failed: {error}");
                        imported.failed.push("passwords");
                    }
                }
            }
            Family::Firefox => match firefox_passwords(&source.dir, primary) {
                Ok(FirefoxLogins::Read(logins, never)) => {
                    imported.passwords = Some((logins, never));
                }
                Ok(FirefoxLogins::NeedsPrimary) => imported.needs_primary = true,
                Err(error) => {
                    crate::log_error!("[import] passwords failed: {error}");
                    imported.failed.push("passwords");
                }
            },
        }
    }
    if wants("history") {
        match read_history(source.family, &source.dir) {
            Ok(visits) => imported.history = Some(history::import_visits(&visits)),
            Err(error) => {
                crate::log_error!("[import] history failed: {error}");
                imported.failed.push("history");
            }
        }
    }
    if wants("autofill") {
        let read = match source.family {
            Family::Chromium => chromium_autofill(&source.dir),
            Family::Firefox => firefox_autofill(&source.dir),
        };
        match read {
            Ok(autofill) => imported.autofill = Some(autofill),
            Err(error) => {
                crate::log_error!("[import] form data failed: {error}");
                imported.failed.push("autofill");
            }
        }
    }
    Some(imported)
}

/// Adds the bookmarks read and says what the import brought, for the dialog.
pub fn finish(mut imported: Imported, skip_duplicates: bool) -> Value {
    let mut result = match imported.bookmarks {
        Some((tree, skipped)) => bookmarks::import_parsed(
            Parsed { tree, skipped },
            skip_duplicates,
            &tv("import.folder", &[("browser", Var::Text(imported.browser))]),
        ),
        None => json!({ "links": 0, "folders": 0, "duplicates": 0, "skipped": 0 }),
    };
    result["favicons"] = json!(imported.favicons);
    if let Some(history) = imported.history {
        result["history"] = json!(history);
    }
    if let Some((logins, never)) = &imported.passwords {
        let (added, duplicates) = crate::passwords::import_logins(logins, never);
        result["passwords"] = json!(added);
        result["passwordDuplicates"] = json!(duplicates);
        // Read but not kept: the keyring stayed locked.
        if added + duplicates == 0 && !logins.is_empty() && !imported.failed.contains(&"passwords")
        {
            imported.failed.push("passwords");
        }
    }
    if let Some((addresses, entries)) = imported.autofill.take() {
        // Kept only with a key to seal them: a keyring that stayed locked keeps them out.
        if crate::passwords::can_seal() && crate::formdata::ensure_open() {
            let (addresses, entries) = crate::formdata::import(addresses, entries);
            result["addresses"] = json!(addresses);
            result["formEntries"] = json!(entries);
        } else if !imported.failed.contains(&"autofill") {
            imported.failed.push("autofill");
        }
    }
    if imported.needs_primary {
        result["needsPrimaryPassword"] = json!(true);
    }
    if !imported.failed.is_empty() {
        result["failed"] = json!(imported.failed);
    }
    if imported.favicons > 0 {
        crate::window::favicons_changed();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Labels {
        Labels {
            folder: "Folder".into(),
            mobile: "Mobile".into(),
            firefox_menu: "Menu".into(),
        }
    }

    fn titles(drafts: &[Draft]) -> Vec<String> {
        drafts
            .iter()
            .map(|d| match d {
                Draft::Link { title, .. } => title.clone(),
                Draft::Folder {
                    title, children, ..
                } => format!("{title}/{}", titles(children).join(",")),
            })
            .collect()
    }

    #[test]
    fn reads_chromium_bookmarks() {
        let json = json!({ "roots": {
            "bookmark_bar": { "children": [
                { "type": "url", "name": "Docs", "url": "https://docs.example/", "date_added": "13300000000000000" },
                { "type": "folder", "name": "", "children": [
                    { "type": "url", "name": "", "url": "https://untitled.example/" },
                    { "type": "url", "name": "Tool", "url": "javascript:alert(1)" }
                ]}
            ]},
            "other": { "children": [ { "type": "url", "name": "Other", "url": "http://other.example/" } ] },
            "synced": { "children": [ { "type": "url", "name": "Phone", "url": "https://phone.example/" } ] }
        }});
        let parsed = from_chromium(&json, &labels()).unwrap();
        assert_eq!(
            titles(&parsed.tree.bar),
            ["Docs", "Folder/https://untitled.example/"]
        );
        assert_eq!(titles(&parsed.tree.other), ["Other", "Mobile/Phone"]);
        assert_eq!(parsed.skipped, 1);
        let Draft::Link { added_at, .. } = &parsed.tree.bar[0] else {
            panic!()
        };
        // 13300000000000000 µs after 1601 is 2022-06-18.
        assert_eq!(*added_at, Some(1_655_526_400_000));
        assert!(from_chromium(&json!({ "nope": 1 }), &labels()).is_err());
    }

    #[test]
    fn reads_firefox_bookmarks() {
        let row =
            |id, kind, parent, position, title: &str, url: Option<&str>, guid: &str| PlacesRow {
                id,
                kind,
                parent,
                position,
                title: (!title.is_empty()).then(|| title.to_string()),
                url: url.map(str::to_string),
                date_added: Some(1_700_000_000_000_000),
                guid: guid.into(),
            };
        let rows = vec![
            row(1, 2, 0, 0, "", None, "root________"),
            row(2, 2, 1, 0, "menu", None, "menu________"),
            row(3, 2, 1, 1, "toolbar", None, "toolbar_____"),
            row(4, 2, 1, 2, "unfiled", None, "unfiled_____"),
            row(5, 2, 1, 3, "mobile", None, "mobile______"),
            row(10, 1, 3, 1, "Second", Some("https://b.example/"), "a"),
            row(11, 1, 3, 0, "First", Some("https://a.example/"), "b"),
            row(12, 2, 3, 2, "Work", None, "c"),
            row(13, 1, 12, 0, "", Some("https://work.example/"), "d"),
            row(14, 1, 2, 0, "In menu", Some("https://menu.example/"), "e"),
            row(15, 1, 4, 0, "Smart", Some("place:sort=8"), "f"),
            row(16, 1, 4, 1, "Loose", Some("https://loose.example/"), "g"),
        ];
        let parsed = from_places(&rows, &labels());
        assert_eq!(
            titles(&parsed.tree.bar),
            ["First", "Second", "Work/https://work.example/"]
        );
        assert_eq!(titles(&parsed.tree.other), ["Menu/In menu", "Loose"]);
        assert_eq!(parsed.skipped, 1);
        let Draft::Link { added_at, .. } = &parsed.tree.bar[0] else {
            panic!()
        };
        assert_eq!(*added_at, Some(1_700_000_000_000));
    }

    #[test]
    fn reads_firefox_profile_lists() {
        let ini = "[Install4F96D1932A9F858E]\nDefault=abc.default-release\n\n[Profile1]\nName=default\nIsRelative=1\nPath=xyz.default\n\n[Profile0]\nName=default-release\nIsRelative=0\nPath=/data/ff/abc\nDefault=1\n\n[General]\nStartWithLastProfile=1\n";
        assert_eq!(
            parse_profiles_ini(ini),
            [
                ("xyz.default".to_string(), true, Some("default".to_string())),
                (
                    "/data/ff/abc".to_string(),
                    false,
                    Some("default-release".to_string())
                ),
            ]
        );
    }

    #[test]
    fn finds_profiles_under_a_home_folder() {
        let home =
            std::env::temp_dir().join(format!("peepwick-import-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let brave = home.join(".config/BraveSoftware/Brave-Browser");
        std::fs::create_dir_all(brave.join("Default")).unwrap();
        std::fs::create_dir_all(brave.join("Profile 2")).unwrap();
        std::fs::create_dir_all(brave.join("Profile 1")).unwrap();
        std::fs::create_dir_all(brave.join("System Profile")).unwrap();
        std::fs::write(
            brave.join("Local State"),
            r#"{"profile":{"info_cache":{"Default":{"name":"Personal"},"Profile 2":{"name":"Work"}}}}"#,
        )
        .unwrap();
        std::fs::write(
            brave.join("Default/Bookmarks"),
            r#"{"roots":{"bookmark_bar":{"children":[{"type":"url","name":"A","url":"https://a.example/"}]}}}"#,
        )
        .unwrap();
        // Bookmarks without links count as none; history alone is enough.
        std::fs::write(brave.join("Profile 2/Bookmarks"), r#"{"roots":{}}"#).unwrap();
        std::fs::write(brave.join("Profile 2/History"), b"").unwrap();
        std::fs::write(brave.join("System Profile/History"), b"").unwrap();
        // A second way to the same folder, looked at after Brave's own.
        std::os::unix::fs::symlink(&brave, home.join(".config/vivaldi")).unwrap();

        let found = find_in(&home, &labels());
        let summary: Vec<(String, String, usize, Vec<&str>)> = found
            .iter()
            .map(|f| {
                (
                    f.browser.to_string(),
                    f.profile.clone(),
                    f.links,
                    f.available.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "Brave".to_string(),
                    "Personal".to_string(),
                    1,
                    vec!["bookmarks"]
                ),
                ("Brave".to_string(), "Work".to_string(), 0, vec!["history"]),
            ]
        );
        assert_eq!(found[1].id, "chromium:1");
        let _ = std::fs::remove_dir_all(&home);
    }
}
