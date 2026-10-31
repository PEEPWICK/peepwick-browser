//! Downloads: the list (saved for normal windows, kept in memory for private ones), progress
//! with a smoothed speed, and what the downloads page, the panel and the menu offer. WebKitGTK
//! can't pause a download, so "retry" starts it over.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use webkit2gtk::{
    Download, DownloadError, DownloadExt, URIRequestExt, URIResponseExt, WebContext, WebContextExt,
};

use crate::i18n::{Var, t, tv};
use crate::menu::{self, MenuSpec, item, separator};
use crate::menus::{copy_text, on_window};
use crate::tab::now_ms;
use crate::window::{self, BrowserWindow};
use crate::{notify, paths, settings, store};

const KEEP: usize = 500;
/// How many every window's state carries, for the panel; the page asks for the rest.
const RECENT: usize = 30;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub url: String,
    pub received: u64,
    pub total: u64,
    /// progressing, completed, cancelled or interrupted.
    pub status: String,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Bytes per second, smoothed; 0 when not moving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// The finished file is no longer where it was saved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    /// From a private window: listed while the browser runs, never saved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub incognito: bool,
}

impl DownloadState {
    fn active(&self) -> bool {
        self.status == "progressing"
    }

    fn checked(mut self) -> DownloadState {
        if self.status == "completed" {
            self.missing = !Path::new(&self.path).exists();
        }
        self
    }
}

thread_local! {
    static LIST: RefCell<Vec<DownloadState>> = const { RefCell::new(Vec::new()) };
    static ACTIVE: RefCell<HashMap<String, Download>> = RefCell::new(HashMap::new());
    static BROADCAST_PENDING: Cell<bool> = const { Cell::new(false) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> PathBuf {
    paths::user_path("downloads.json")
}

pub fn load() {
    let Some(saved) = store::read_json::<Vec<DownloadState>>(&file()) else {
        return;
    };
    // Downloads that were running when the browser closed didn't finish.
    let list = saved
        .into_iter()
        .map(|mut d| {
            if d.active() || d.status == "paused" {
                d.status = "interrupted".into();
                d.speed = Some(0.0);
            }
            d
        })
        .collect();
    LIST.with(|cell| cell.replace(list));
}

/// The most recent downloads, for the panel and every window's state.
pub fn recent() -> Vec<Value> {
    LIST.with(|cell| {
        cell.borrow()
            .iter()
            .take(RECENT)
            .cloned()
            .map(DownloadState::checked)
            .collect::<Vec<_>>()
    })
    .into_iter()
    .filter_map(|d| serde_json::to_value(d).ok())
    .collect()
}

/// Every download, newest first; `text` matches the file name or the address.
pub fn all(text: &str) -> Vec<DownloadState> {
    let needle = text.trim().to_lowercase();
    LIST.with(|cell| {
        cell.borrow()
            .iter()
            .filter(|d| {
                needle.is_empty()
                    || d.filename.to_lowercase().contains(&needle)
                    || d.url.to_lowercase().contains(&needle)
            })
            .cloned()
            .map(DownloadState::checked)
            .collect()
    })
}

pub fn active_count() -> usize {
    LIST.with(|cell| cell.borrow().iter().filter(|d| d.active()).count())
}

/// The folder chosen in Settings, or the system's Downloads folder.
pub fn folder() -> PathBuf {
    let chosen = settings::get().download_dir;
    if !chosen.is_empty() && Path::new(&chosen).is_dir() {
        return PathBuf::from(chosen);
    }
    glib::user_special_dir(glib::UserDirectory::Downloads)
        .unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

fn save() {
    let list: Vec<DownloadState> = LIST.with(|cell| {
        cell.borrow()
            .iter()
            .filter(|d| !d.incognito)
            .take(KEEP)
            .cloned()
            .collect()
    });
    match serde_json::to_vec(&list) {
        Ok(json) => {
            if let Err(error) = store::write_atomic(&file(), &json) {
                eprintln!("[downloads] could not save: {error}");
            }
        }
        Err(error) => eprintln!("[downloads] could not serialize: {error}"),
    }
}

pub fn flush() {
    if SAVE_PENDING.with(|pending| pending.replace(false)) {
        save();
    }
}

/// Saves a moment later, and shows the change: right away, or at most four times a second
/// while bytes come in.
fn changed(immediate: bool) {
    if !SAVE_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_secs(1), flush);
    }
    if immediate {
        BROADCAST_PENDING.with(|pending| pending.set(false));
        window::global_changed();
        return;
    }
    if !BROADCAST_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_millis(250), || {
            if BROADCAST_PENDING.with(|pending| pending.replace(false)) {
                window::global_changed();
            }
        });
    }
}

fn with_entry(id: &str, change: impl FnOnce(&mut DownloadState)) {
    LIST.with(|cell| {
        if let Some(entry) = cell.borrow_mut().iter_mut().find(|d| d.id == id) {
            change(entry);
        }
    });
}

fn new_id() -> String {
    let mut bytes = [0u8; 12];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        bytes[..8].copy_from_slice(&now_ms().to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `name` in `dir`, or `name (1)` and so on when that's taken.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let safe: String = Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().replace(['/', '\\', '\0'], "_"))
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "download".into());
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (safe.clone(), String::new()),
    };
    (1..1000)
        .map(|i| dir.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(candidate)
}

fn file_uri(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

/// Opens a file or folder with the desktop's default app. An AppImage's own library and
/// plugin paths are left out of the app's environment, as they would break it.
pub fn open_with_default(uri: &str) {
    let context = gio::AppLaunchContext::new();
    if std::env::var_os("APPIMAGE").is_some() {
        for name in [
            "LD_LIBRARY_PATH",
            "GTK_PATH",
            "GTK_EXE_PREFIX",
            "GTK_DATA_PREFIX",
            "GTK_IM_MODULE_FILE",
            "GDK_PIXBUF_MODULE_FILE",
            "GIO_MODULE_DIR",
            "GST_PLUGIN_PATH",
            "GST_PLUGIN_PATH_1_0",
            "GST_PLUGIN_SYSTEM_PATH",
            "GST_PLUGIN_SYSTEM_PATH_1_0",
            "GST_PLUGIN_SCANNER",
            "GST_PLUGIN_SCANNER_1_0",
            "GST_REGISTRY",
            "GST_REGISTRY_1_0",
        ] {
            context.unsetenv(name);
        }
    }
    if let Err(error) = gio::AppInfo::launch_default_for_uri(uri, Some(&context)) {
        eprintln!("[downloads] could not open {uri}: {error}");
    }
}

/// Shows the file selected in the file manager, or opens its folder.
fn show_in_folder(path: &Path) {
    let uri = file_uri(path);
    let folder = path
        .parent()
        .map(file_uri)
        .unwrap_or_else(|| file_uri(&folder()));
    gio::bus_get(
        gio::BusType::Session,
        None::<&gio::Cancellable>,
        move |bus| {
            let Ok(bus) = bus else {
                open_with_default(&folder);
                return;
            };
            bus.call(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&(vec![uri], "").to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                None::<&gio::Cancellable>,
                move |result| {
                    if result.is_err() {
                        open_with_default(&folder);
                    }
                },
            );
        },
    );
}

// ── Following downloads ───────────────────────────────────────────────────────

/// Every download a browsing context starts lands here.
pub fn attach(context: &WebContext, incognito: bool) {
    context.connect_download_started(move |_, download| started(download, incognito));
}

fn started(download: &Download, incognito: bool) {
    let id = new_id();
    let url = download
        .request()
        .and_then(|r| r.uri())
        .map(|u| u.to_string())
        .unwrap_or_default();
    // A tab that never showed a page (a link opened only to download) goes away, as in Chrome.
    if let Some(view) = download.web_view()
        && let Some((win, tab)) = window::tab_for_view(&view)
        && tab.page().is_none()
        && matches!(tab.url().as_str(), "" | "about:blank")
    {
        glib::idle_add_local_once(move || {
            if tab.opener_id.get().is_some() {
                win.close_tab(tab.id, false);
            } else {
                tab.show_page("newtab", "");
            }
        });
    }
    let entry = DownloadState {
        id: id.clone(),
        filename: String::new(),
        path: String::new(),
        url,
        received: 0,
        total: 0,
        status: "progressing".into(),
        started_at: now_ms(),
        ended_at: None,
        speed: Some(0.0),
        missing: false,
        incognito,
    };
    LIST.with(|cell| {
        let mut list = cell.borrow_mut();
        list.insert(0, entry);
        list.truncate(KEEP);
    });
    ACTIVE.with(|active| active.borrow_mut().insert(id.clone(), download.clone()));
    changed(true);

    let entry_id = id.clone();
    download.connect_decide_destination(move |download, suggested| {
        let dir = folder();
        let _ = std::fs::create_dir_all(&dir);
        let path = if settings::get().ask_where_to_save {
            match ask_where(download, &dir, suggested) {
                Some(path) => path,
                None => {
                    download.cancel();
                    return true;
                }
            }
        } else {
            unique_path(&dir, suggested)
        };
        download.set_destination(&file_uri(&path));
        with_entry(&entry_id, |d| {
            d.path = path.to_string_lossy().into_owned();
            d.filename = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
        });
        changed(true);
        true
    });

    // Speed from the bytes received between samples, smoothed so the time left doesn't jump.
    let (sample_at, sample_bytes) = (Rc::new(Cell::new(now_ms())), Rc::new(Cell::new(0u64)));
    let entry_id = id.clone();
    download.connect_received_data(move |download, _| {
        let received = download.received_data_length();
        let total = download.response().map(|r| r.content_length()).unwrap_or(0);
        let now = now_ms();
        with_entry(&entry_id, |d| {
            if now.saturating_sub(sample_at.get()) >= 500 {
                let current = (received.saturating_sub(sample_bytes.get())) as f64 * 1000.0
                    / (now - sample_at.get()) as f64;
                let speed = d.speed.unwrap_or(0.0);
                d.speed = Some(if speed > 0.0 {
                    speed * 0.7 + current * 0.3
                } else {
                    current
                });
                sample_at.set(now);
                sample_bytes.set(received);
            }
            d.received = received;
            d.total = total;
        });
        changed(false);
    });

    let entry_id = id.clone();
    download.connect_failed(move |download, error| {
        let cancelled = error.matches(DownloadError::CancelledByUser);
        let chosen = download.destination().is_some();
        LIST.with(|cell| {
            let mut list = cell.borrow_mut();
            // Cancelled before a file was chosen: nothing to list.
            if cancelled && !chosen {
                list.retain(|d| d.id != entry_id);
            } else if let Some(d) = list.iter_mut().find(|d| d.id == entry_id) {
                d.status = if cancelled {
                    "cancelled"
                } else {
                    "interrupted"
                }
                .into();
                d.speed = Some(0.0);
                d.ended_at = Some(now_ms());
            }
        });
        changed(true);
    });

    download.connect_finished(move |download| {
        ACTIVE.with(|active| active.borrow_mut().remove(&id));
        let mut done = None;
        with_entry(&id, |d| {
            // `finished` also follows `failed`.
            if d.active() {
                d.status = "completed".into();
                d.received = download.received_data_length().max(d.received);
                d.total = d.total.max(d.received);
                d.speed = Some(0.0);
                d.ended_at = Some(now_ms());
                done = Some(d.filename.clone());
            }
        });
        changed(true);
        let focused = window::all()
            .iter()
            .any(|w| w.tauri_window().is_focused().unwrap_or(false));
        if let Some(name) = done
            && !focused
        {
            notify::show(&t("downloads.done"), &name);
        }
    });
}

fn ask_where(download: &Download, dir: &Path, suggested: &str) -> Option<PathBuf> {
    let parent = download
        .web_view()
        .and_then(|view| window::tab_for_view(&view))
        .and_then(|(win, _)| win.gtk_window());
    let dialog = gtk::FileChooserNative::new(
        Some(&t("downloads.title")),
        parent.as_ref(),
        gtk::FileChooserAction::Save,
        None,
        None,
    );
    dialog.set_do_overwrite_confirmation(true);
    let _ = dialog.set_current_folder(dir);
    dialog.set_current_name(suggested);
    if dialog.run() == gtk::ResponseType::Accept {
        dialog.filename()
    } else {
        None
    }
}

// ── What the page, the panel and the menu do ──────────────────────────────────

pub fn action(id: &str, action: &str) {
    let Some(entry) = LIST.with(|cell| cell.borrow().iter().find(|d| d.id == id).cloned()) else {
        return;
    };
    let download = ACTIVE.with(|active| active.borrow().get(id).cloned());
    let path = PathBuf::from(&entry.path);
    match action {
        "open" if entry.status == "completed" => open_with_default(&file_uri(&path)),
        "show" => {
            if path.exists() {
                show_in_folder(&path);
            } else {
                open_with_default(&file_uri(&folder()));
            }
        }
        "cancel" => {
            if let Some(download) = download {
                download.cancel();
            }
        }
        "retry" => {
            if let Some(download) = download {
                download.cancel();
            }
            LIST.with(|cell| cell.borrow_mut().retain(|d| d.id != id));
            changed(true);
            if let Some(context) = window::web_context(entry.incognito) {
                context.download_uri(&entry.url);
            }
        }
        "copy" => copy_text(&entry.url),
        "trash" if entry.status == "completed" && path.exists() => {
            let id = id.to_string();
            gio::File::for_path(&path).trash_async(
                glib::Priority::DEFAULT,
                None::<&gio::Cancellable>,
                move |result| match result {
                    Ok(()) => {
                        LIST.with(|cell| cell.borrow_mut().retain(|d| d.id != id));
                        changed(true);
                    }
                    Err(error) => eprintln!("[downloads] could not move to the trash: {error}"),
                },
            );
        }
        "remove" => {
            if let Some(download) = download {
                download.cancel();
            }
            LIST.with(|cell| cell.borrow_mut().retain(|d| d.id != id));
            changed(true);
        }
        _ => {}
    }
}

pub fn open_folder() {
    open_with_default(&file_uri(&folder()));
}

/// Clears finished downloads from the list; the files stay.
pub fn clear() {
    LIST.with(|cell| cell.borrow_mut().retain(DownloadState::active));
    changed(true);
}

pub fn menu(win: &Rc<BrowserWindow>, id: &str, x: f64, y: f64) {
    let Some(entry) = LIST
        .with(|cell| cell.borrow().iter().find(|d| d.id == id).cloned())
        .map(DownloadState::checked)
    else {
        return;
    };
    let active = entry.active();
    let done = entry.status == "completed" && !entry.missing;
    let run = |action_name: &'static str| {
        let id = id.to_string();
        move |_: menu::ClickEvent| action(&id, action_name)
    };
    let mut items: Vec<MenuSpec> = vec![
        item(t("downloads.open"), run("open")).enabled(done),
        item(t("downloads.show"), run("show")).enabled(done),
        separator(),
    ];
    if active {
        items.push(item(t("downloads.cancel"), run("cancel")));
    } else {
        let label = if entry.status == "completed" {
            t("downloads.again")
        } else {
            t("downloads.retry")
        };
        items.push(item(label, run("retry")));
    }
    items.push(item(t("downloads.copyLink"), run("copy")));
    items.push(separator());
    let (trash_id, name) = (id.to_string(), entry.filename.clone());
    items.push(
        item(
            t("downloads.trash"),
            on_window(win, move |win| {
                let question = menu::Question {
                    title: tv("downloads.trashConfirm", &[("name", Var::Text(&name))]),
                    body: Some(t("downloads.trashBody")),
                    confirm: t("downloads.trash"),
                    cancel: t("common.cancel"),
                    danger: true,
                };
                let id = trash_id.clone();
                menu::confirm(win, question, move |ok| {
                    if ok {
                        action(&id, "trash");
                    }
                });
            }),
        )
        .enabled(done),
    );
    items.push(item(t("downloads.remove"), run("remove")).enabled(!active));
    menu::popup(win, items, x, y, false, None);
}

/// Settings › Downloads: picks the folder downloads go to.
pub fn choose_folder(win: &BrowserWindow) -> Value {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("settings.downloadFolder")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::SelectFolder,
        None,
        None,
    );
    let _ = dialog.set_current_folder(folder());
    if dialog.run() != gtk::ResponseType::Accept {
        return Value::Null;
    }
    let Some(path) = dialog.filename() else {
        return Value::Null;
    };
    let path = path.to_string_lossy().into_owned();
    settings::update(&json!({ "downloadDir": path }));
    window::settings_changed();
    json!(path)
}

#[cfg(test)]
mod tests {
    use super::unique_path;

    #[test]
    fn picks_free_names() {
        let dir = std::env::temp_dir().join(format!("peepwick-downloads-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report.pdf"));
        std::fs::write(dir.join("report.pdf"), b"").unwrap();
        assert_eq!(unique_path(&dir, "report.pdf"), dir.join("report (1).pdf"));
        assert_eq!(unique_path(&dir, "../../etc/passwd"), dir.join("passwd"));
        assert_eq!(unique_path(&dir, ""), dir.join("download"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
