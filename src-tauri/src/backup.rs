//! Encrypted backups, as the Electron version made them (its backup.ts): settings, bookmarks,
//! spaces (and optionally their tabs), focus history, site permissions, addresses and form
//! entries, and optionally saved passwords and browsing history, in one password-protected file
//! the person keeps wherever they like (the format is backupfile.rs). No cloud: moving it to
//! another computer is up to them. Restoring merges into what is already here and never deletes
//! anything.
//!
//! A backup made with 0.1.x opens here, so it is also the way to bring 0.1.x's data over. Its
//! list of Chrome extensions has nothing to go to here and is left out.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Map, Value, json};

use crate::backupfile;
use crate::formdata::{self, Address, FormEntry};
use crate::i18n::t;
use crate::tab::{SavedTab, now_ms};
use crate::window::{self, BrowserWindow, OpenOptions, WORKSPACE_ICONS};
use crate::{bookmarks, history, menus, pages, passwords, permissions, settings, stats};

/// Settings that belong to one computer and never travel.
const LOCAL_SETTINGS: [&str; 3] = ["onboarded", "downloadDir", "hardwareAcceleration"];

/// How long an opened file waits between "choose file", "password" and "restore".
const HOLD: Duration = Duration::from_secs(10 * 60);

/// The largest file taken for a backup; real ones are a few megabytes.
const MAX_FILE: u64 = 256 * 1024 * 1024;

/// What a restore brings back at most, as the Electron version did.
const MAX_WORKSPACES: usize = 12;
const MAX_TABS: usize = 200;

/// A file chosen for restoring, and once unlocked what it holds.
struct Opened {
    data: Vec<u8>,
    payload: Option<Value>,
    since: Instant,
}

static OPENED: LazyLock<Mutex<HashMap<String, Opened>>> = LazyLock::new(Default::default);

/// The opened files, without those kept too long.
fn opened() -> MutexGuard<'static, HashMap<String, Opened>> {
    let mut opened = OPENED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    opened.retain(|_, file| file.since.elapsed() < HOLD);
    opened
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn wanted(categories: &[String], category: &str) -> bool {
    categories.iter().any(|c| c == category)
}

/// Pages a backup keeps: those that can be opened again elsewhere.
fn restorable(url: &str) -> bool {
    ["http:", "https:", "file:"]
        .iter()
        .any(|s| url.starts_with(s))
}

// ── Making a backup ─────────────────────────────────────────────────────────────

/// Every normal window's spaces, with their web pages when `with_tabs`.
fn saved_workspaces(with_tabs: bool) -> Vec<Value> {
    window::all()
        .into_iter()
        .filter(|win| !win.incognito)
        .flat_map(|win| win.serialize().workspaces)
        .map(|workspace| {
            let mut entry = json!({ "name": workspace.name, "icon": workspace.icon });
            if with_tabs {
                entry["tabs"] = workspace
                    .tabs
                    .iter()
                    .filter(|tab| restorable(&tab.url))
                    .map(|tab| json!({ "url": tab.url, "title": tab.title, "pinned": tab.pinned }))
                    .collect();
            }
            entry
        })
        .collect()
}

/// How much of each kind this computer has, for the export dialog. Kinds missing here aren't
/// offered.
pub fn local_counts() -> Value {
    let workspaces = saved_workspaces(true);
    let tabs: usize = workspaces.iter().map(|w| list(&w["tabs"]).len()).sum();
    json!({
        "settings": 1,
        "bookmarks": bookmarks::links().len(),
        "workspaces": workspaces.len(),
        "tabs": tabs,
        "focus": stats::active_days(),
        "permissions": permissions::count(),
        "autofill": formdata::addresses().len() + formdata::count_entries(0),
        "passwords": passwords::count(),
        "history": history::count_visits(0),
    })
}

/// A new backup's contents: the chosen kinds, in the Electron version's layout. Passwords,
/// addresses and form entries need the key (`passwords::ready`) first.
pub fn collect(categories: &[String]) -> Value {
    let has = |category: &str| wanted(categories, category);
    let mut payload = json!({
        "format": "peepwick-backup",
        "version": 1,
        "app": window::display_version(),
        "createdAt": now_ms(),
    });
    if has("settings") {
        let mut saved = serde_json::to_value(settings::get()).unwrap_or_default();
        if let Some(saved) = saved.as_object_mut() {
            for key in LOCAL_SETTINGS {
                saved.remove(key);
            }
        }
        payload["settings"] = saved;
    }
    if has("bookmarks") {
        payload["bookmarks"] = bookmarks::backup_draft();
    }
    if has("workspaces") {
        payload["workspaces"] = Value::Array(saved_workspaces(has("tabs")));
    }
    if has("focus") {
        payload["focus"] = stats::export();
    }
    if has("permissions") {
        payload["permissions"] = permissions::export();
    }
    if has("autofill") && formdata::ensure_open() {
        payload["autofill"] = json!({
            "addresses": formdata::addresses(),
            "entries": formdata::entries(),
        });
    }
    if has("history") {
        payload["history"] = history::export_visits()
            .into_iter()
            .map(|v| json!({ "url": v.url, "title": v.title, "visitedAt": v.visited_at }))
            .collect();
    }
    if has("passwords") {
        let logins: Vec<Value> = passwords::plain_logins()
            .into_iter()
            .map(|l| {
                json!({
                    "origin": l.origin,
                    "username": l.username,
                    "password": l.password,
                    "note": l.note.unwrap_or_default(),
                    "created": l.created,
                    "updated": l.updated,
                })
            })
            .collect();
        payload["passwords"] = json!({ "logins": logins, "never": passwords::never_list() });
    }
    payload
}

/// Seals the contents with the password into `path`, readable by its owner only. Takes about
/// half a second: off the main thread.
pub fn write(path: &Path, payload: &Value, password: &str) -> bool {
    let sealed = serde_json::to_vec(payload)
        .ok()
        .and_then(|json| backupfile::encrypt(&json, password, backupfile::DEFAULT_KDF));
    let Some(sealed) = sealed else {
        crate::log_error!("[backup] could not encrypt");
        return false;
    };
    match passwords::write_private(path, &sealed) {
        Ok(()) => true,
        Err(error) => {
            crate::log_error!("[backup] could not save: {error}");
            false
        }
    }
}

fn file_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&t("backup.fileType")));
    filter.add_pattern(&format!("*.{}", backupfile::EXTENSION));
    filter
}

/// Where to save a new backup; None when the person cancels.
pub fn choose_save_path(win: &BrowserWindow) -> Option<PathBuf> {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("backup.export")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Save,
        None,
        None,
    );
    dialog.set_do_overwrite_confirmation(true);
    dialog.add_filter(file_filter());
    let day = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|d| d.to_string())
        .unwrap_or_default();
    dialog.set_current_name(&format!(
        "{}-{day}.{}",
        t("backup.fileName"),
        backupfile::EXTENSION
    ));
    if let Some(documents) = glib::user_special_dir(glib::UserDirectory::Documents) {
        let _ = dialog.set_current_folder(documents);
    }
    if dialog.run() != gtk::ResponseType::Accept {
        return None;
    }
    let path = dialog.filename()?;
    let extension = format!(".{}", backupfile::EXTENSION);
    if path.to_string_lossy().ends_with(&extension) {
        return Some(path);
    }
    let mut named = path.into_os_string();
    named.push(&extension);
    Some(named.into())
}

// ── Restoring ───────────────────────────────────────────────────────────────────

/// The backup file to restore; None when the person cancels.
pub fn choose_backup_file(win: &BrowserWindow) -> Option<PathBuf> {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("backup.import")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Open,
        None,
        None,
    );
    dialog.add_filter(file_filter());
    if dialog.run() != gtk::ResponseType::Accept {
        return None;
    }
    dialog.filename()
}

/// Reads a chosen file: `{ token, name }` to unlock it with, or `{ error: "format" }`.
pub fn open_path(path: &Path) -> Value {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let data = match std::fs::metadata(path) {
        Ok(meta) if meta.len() <= MAX_FILE => std::fs::read(path).ok(),
        _ => None,
    };
    match data {
        Some(data) if backupfile::looks_like_backup(&data) => {
            json!({ "token": hold(data), "name": name })
        }
        _ => json!({ "error": "format" }),
    }
}

/// Keeps an opened file until it's unlocked and restored, or ten minutes pass.
pub fn hold(data: Vec<u8>) -> String {
    let token = bookmarks::new_id();
    opened().insert(
        token.clone(),
        Opened {
            data,
            payload: None,
            since: Instant::now(),
        },
    );
    // Forgotten after ten minutes even if nobody asks again, with what it unlocked.
    let later = token.clone();
    glib::timeout_add_seconds_once(HOLD.as_secs() as u32, move || forget(&later));
    token
}

/// The restore dialog closed early.
pub fn forget(token: &str) {
    opened().remove(token);
}

fn is_payload(payload: &Value) -> bool {
    payload["format"] == "peepwick-backup"
        && payload["version"] == 1
        && payload["createdAt"].is_number()
}

/// How much of each kind a backup holds, for the restore dialog.
fn counts(payload: &Value) -> Map<String, Value> {
    let mut counts = Map::new();
    if payload["settings"].is_object() {
        counts.insert("settings".into(), 1.into());
    }
    if let Some(draft) = bookmarks::draft_from_backup(&payload["bookmarks"]) {
        counts.insert("bookmarks".into(), bookmarks::draft_links(&draft).into());
    }
    if let Some(workspaces) = payload["workspaces"].as_array() {
        counts.insert("workspaces".into(), workspaces.len().into());
        let tabs: usize = workspaces.iter().map(|w| list(&w["tabs"]).len()).sum();
        if tabs > 0 {
            counts.insert("tabs".into(), tabs.into());
        }
    }
    if let Some(days) = payload["focus"]["days"].as_object() {
        counts.insert("focus".into(), days.len().into());
    }
    if let Some(sites) = payload["permissions"].as_object() {
        let answers: usize = sites
            .values()
            .map(|kinds| kinds.as_object().map_or(0, Map::len))
            .sum();
        counts.insert("permissions".into(), answers.into());
    }
    if payload["autofill"].is_object() {
        let entries = list(&payload["autofill"]["addresses"]).len()
            + list(&payload["autofill"]["entries"]).len();
        counts.insert("autofill".into(), entries.into());
    }
    if let Some(visits) = payload["history"].as_array() {
        counts.insert("history".into(), visits.len().into());
    }
    if let Some(logins) = payload["passwords"]["logins"].as_array() {
        counts.insert("passwords".into(), logins.len().into());
    }
    counts
}

/// Unlocks a held file with the password: what it holds, or `{ error }` ("password",
/// "format", "expired"). Scrypt takes about half a second: off the main thread.
pub fn unlock(token: &str, password: &str) -> Value {
    let Some(data) = opened().get(token).map(|file| file.data.clone()) else {
        return json!({ "error": "expired" });
    };
    let plain = match backupfile::decrypt(&data, password) {
        Ok(plain) => plain,
        Err(error) => return json!({ "error": error.reason() }),
    };
    let payload = match serde_json::from_slice::<Value>(&plain) {
        Ok(payload) if is_payload(&payload) => payload,
        _ => return json!({ "error": "format" }),
    };
    let summary = json!({
        "token": token,
        "createdAt": payload["createdAt"],
        "version": payload["app"].as_str().unwrap_or("").chars().take(20).collect::<String>(),
        "counts": counts(&payload),
        // Chrome extensions (0.1.x) have nothing to go to here.
        "extensions": [],
    });
    match opened().get_mut(token) {
        Some(file) => file.payload = Some(payload),
        None => return json!({ "error": "expired" }),
    }
    summary
}

/// Whether restoring needs the passwords' key: the person chose passwords, or addresses and
/// form entries, and there are some.
pub fn needs_key(token: &str, categories: &[String]) -> bool {
    opened()
        .get(token)
        .and_then(|file| file.payload.as_ref())
        .is_some_and(|payload| {
            (wanted(categories, "passwords") && !list(&payload["passwords"]["logins"]).is_empty())
                || (wanted(categories, "autofill") && payload["autofill"].is_object())
        })
}

/// Brings the chosen kinds in: what happened, or null when the file was forgotten meanwhile.
pub fn restore(win: &Rc<BrowserWindow>, token: &str, categories: &[String]) -> Value {
    let Some(payload) = opened().remove(token).and_then(|file| file.payload) else {
        return Value::Null;
    };
    let has = |category: &str| wanted(categories, category);
    let settings = has("settings") && restore_settings(&payload["settings"]);
    let bookmarks = match bookmarks::draft_from_backup(&payload["bookmarks"]) {
        Some(draft) if has("bookmarks") => bookmarks::merge_draft(draft),
        _ => 0,
    };
    let (workspaces, tabs) = match payload["workspaces"].as_array() {
        Some(saved) if has("workspaces") && !win.incognito => {
            restore_workspaces(win, saved, has("tabs"))
        }
        _ => (0, 0),
    };
    let focus_days = if has("focus") {
        stats::merge(&payload["focus"])
    } else {
        0
    };
    let permissions = if has("permissions") {
        permissions::merge(&payload["permissions"])
    } else {
        0
    };
    let history = if has("history") {
        restore_history(&payload["history"])
    } else {
        0
    };
    let passwords = if has("passwords") {
        restore_passwords(&payload["passwords"])
    } else {
        0
    };
    let autofill = if has("autofill") {
        restore_autofill(&payload["autofill"])
    } else {
        0
    };
    json!({
        "settings": settings,
        "bookmarks": bookmarks,
        "workspaces": workspaces,
        "tabs": tabs,
        "focusDays": focus_days,
        "permissions": permissions,
        "extensions": 0,
        "history": history,
        "passwords": passwords,
        "autofill": autofill,
    })
}

/// The backup's settings over these, except the ones that belong to this computer. Lists are
/// joined rather than replaced: restoring never takes anything away. True if anything changed.
fn restore_settings(incoming: &Value) -> bool {
    let Some(incoming) = incoming.as_object() else {
        return false;
    };
    let mut patch = incoming.clone();
    for key in LOCAL_SETTINGS {
        patch.remove(key);
    }
    let current = settings::get();
    let before = serde_json::to_value(&current).unwrap_or_default();
    for key in ["focusSites", "shieldAllowlist"] {
        if let Some(items) = patch.get(key).and_then(Value::as_array) {
            let mut joined = list(&before[key]).to_vec();
            for item in items {
                if !joined.contains(item) {
                    joined.push(item.clone());
                }
            }
            patch.insert(key.to_string(), Value::Array(joined));
        }
    }
    let patch = Value::Object(patch);
    let mut after = current;
    after.apply_patch(&patch);
    if serde_json::to_value(&after).unwrap_or_default() == before {
        return false;
    }
    settings::update(&patch);
    window::settings_changed();
    if patch.get("defaultZoom").is_some() {
        window::zoom_changed();
    }
    true
}

/// The backup's spaces, into this window: a space with the same name gets the pages it lacks,
/// others are added. (spaces added, tabs opened)
fn restore_workspaces(win: &Rc<BrowserWindow>, saved: &[Value], with_tabs: bool) -> (usize, usize) {
    let started_in = win.active_workspace_id();
    let (mut created, mut opened) = (0, 0);
    for space in saved.iter().take(MAX_WORKSPACES) {
        let name: String = space["name"]
            .as_str()
            .unwrap_or("")
            .trim()
            .chars()
            .take(40)
            .collect();
        if name.is_empty() {
            continue;
        }
        let existing = win
            .workspace_list()
            .into_iter()
            .find(|(_, other, _)| *other == name)
            .map(|(id, ..)| id);
        let (id, fresh) = match existing {
            Some(id) => (id, false),
            None => {
                let icon = space["icon"]
                    .as_str()
                    .filter(|icon| WORKSPACE_ICONS.contains(icon))
                    .unwrap_or("circle");
                let before = win.workspace_list().len();
                win.create_workspace(&name, icon);
                let after = win.workspace_list();
                if after.len() == before {
                    break;
                }
                created += 1;
                (
                    after.last().map(|(id, ..)| id.clone()).unwrap_or_default(),
                    true,
                )
            }
        };
        let tabs = list(&space["tabs"]);
        if !with_tabs || tabs.is_empty() {
            continue;
        }
        let mut open: HashSet<String> = win
            .tabs_in_order(Some(&id))
            .iter()
            .map(|tab| tab.url())
            .collect();
        let mut added = 0;
        for tab in tabs.iter().take(MAX_TABS) {
            let Some(url) = tab["url"].as_str() else {
                continue;
            };
            if !restorable(url) || !open.insert(url.to_string()) {
                continue;
            }
            win.open_tab(OpenOptions {
                saved: Some(SavedTab {
                    url: url.to_string(),
                    title: tab["title"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(1000)
                        .collect(),
                    pinned: tab["pinned"] == true,
                    ..SavedTab::default()
                }),
                background: true,
                workspace_id: Some(id.clone()),
                ..OpenOptions::default()
            });
            added += 1;
        }
        opened += added;
        // A new space starts with a new tab page; with restored tabs it isn't needed.
        if fresh
            && added > 0
            && let Some(first) = win
                .tabs_in_order(Some(&id))
                .into_iter()
                .find(|tab| tab.url() == pages::NEW_TAB_URL)
        {
            win.close_tab(first.id, false);
        }
    }
    win.activate_workspace(&started_in);
    (created, opened)
}

fn restore_history(saved: &Value) -> usize {
    let mut visits: Vec<history::Visit> = list(saved)
        .iter()
        .filter_map(|visit| {
            let at = &visit["visitedAt"];
            Some(history::Visit {
                url: visit["url"].as_str()?.to_string(),
                title: visit["title"].as_str().unwrap_or("").to_string(),
                visited_at: at
                    .as_i64()
                    .or_else(|| at.as_f64().filter(|t| t.is_finite()).map(|t| t as i64))?,
            })
        })
        .collect();
    // Newest first, as import_visits takes them: each page keeps its latest title.
    visits.sort_by_key(|visit| std::cmp::Reverse(visit.visited_at));
    let added = history::import_visits(&visits);
    if added > 0 {
        menus::history_changed();
    }
    added
}

/// The backup's addresses and form entries, as an import: what is here already is left out.
fn restore_autofill(saved: &Value) -> usize {
    if !saved.is_object() || !passwords::can_seal() || !formdata::ensure_open() {
        return 0;
    }
    let addresses: Vec<Address> = list(&saved["addresses"])
        .iter()
        .map(Address::from_value)
        .collect();
    let entries: Vec<FormEntry> = list(&saved["entries"])
        .iter()
        .filter_map(FormEntry::from_value)
        .collect();
    let (addresses, entries) = formdata::import(addresses, entries);
    addresses + entries
}

/// The backup's logins, as an import: a login already here keeps the newer password.
fn restore_passwords(saved: &Value) -> usize {
    let text = |value: &Value| value.as_str().map(str::to_string);
    let logins: Vec<passwords::PlainLogin> = list(&saved["logins"])
        .iter()
        .filter_map(|login| {
            Some(passwords::PlainLogin {
                origin: text(&login["origin"])?,
                username: text(&login["username"])?,
                password: text(&login["password"])?,
                note: text(&login["note"]),
                created: login["created"].as_u64(),
                updated: login["updated"].as_u64(),
            })
        })
        .collect();
    let never: Vec<String> = list(&saved["never"]).iter().filter_map(text).collect();
    passwords::import_logins(&logins, &never).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_what_a_backup_from_0_1_x_holds() {
        let payload = json!({
            "format": "peepwick-backup",
            "version": 1,
            "app": "0.1.9a",
            "createdAt": 1,
            "settings": { "theme": "light" },
            "bookmarks": {
                "bar": [
                    { "type": "url", "title": "A", "url": "https://a.example/" },
                    { "type": "folder", "title": "F", "children": [
                        { "type": "url", "title": "B", "url": "https://b.example/" }
                    ] }
                ],
                "other": []
            },
            "workspaces": [
                { "name": "Kişisel", "icon": "circle", "tabs": [{ "url": "https://a.example/" }] },
                { "name": "İş", "icon": "briefcase" }
            ],
            "focus": { "days": { "2026-11-01": { "blocked": 3, "focus": 25 } }, "blockedTotal": 3 },
            "permissions": { "https://a.example": { "camera": true, "geolocation": false } },
            "extensions": [{ "id": "cjpalhdlnbpafiamejdnhcphjbkeiagm", "name": "uBlock Origin" }],
            "history": [{ "url": "https://a.example/", "title": "A", "visitedAt": 5 }],
            "passwords": { "logins": [], "never": [] },
            "autofill": { "addresses": [{ "name": "Ada" }], "entries": [] },
        });
        assert!(is_payload(&payload));
        assert_eq!(
            Value::Object(counts(&payload)),
            json!({
                "settings": 1,
                "bookmarks": 2,
                "workspaces": 2,
                "tabs": 1,
                "focus": 1,
                "permissions": 2,
                "autofill": 1,
                "history": 1,
                "passwords": 0,
            })
        );
    }

    #[test]
    fn knows_a_payload_from_other_json() {
        assert!(!is_payload(
            &json!({ "format": "peepwick-backup", "version": 2, "createdAt": 1 })
        ));
        assert!(!is_payload(
            &json!({ "format": "other", "version": 1, "createdAt": 1 })
        ));
        assert!(!is_payload(&json!([1, 2, 3])));
    }

    #[test]
    fn keeps_web_pages_only() {
        assert!(restorable("https://example.com/"));
        assert!(restorable("file:///home/u/a.html"));
        assert!(!restorable("peepwick://newtab"));
        assert!(!restorable("javascript:alert(1)"));
    }
}
