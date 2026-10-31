//! Restoring the last session at startup and saving it as it changes (session.json in the
//! profile). Private windows are never saved.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::window::{self, BrowserWindow, SavedWindow};
use crate::{paths, settings, store};

/// 2: the Tauri format (WebKit back/forward history); Electron's sessions aren't read.
const VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    windows: Vec<SavedWindow>,
}

thread_local! {
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
    static QUITTING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> std::path::PathBuf {
    paths::user_path("session.json")
}

/// Opens the windows of the last session (when that setting is on), or one new window. Addresses
/// given on the command line open in the last window.
pub fn restore(app: &AppHandle, urls: Vec<String>) -> tauri::Result<()> {
    let saved = if settings::get().restore_session {
        store::read_json::<SessionFile>(&file())
    } else {
        None
    };
    let windows: Vec<SavedWindow> = saved
        .filter(|s| s.version == VERSION)
        .map(|s| {
            s.windows
                .into_iter()
                .filter(|w| !w.workspaces.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if windows.is_empty() {
        return window::open(app, false, urls, None);
    }
    let last = windows.len() - 1;
    for (index, saved) in windows.into_iter().enumerate() {
        window::open(
            app,
            false,
            if index == last {
                urls.clone()
            } else {
                Vec::new()
            },
            Some(saved),
        )?;
    }
    Ok(())
}

/// Saves a few seconds after the last change.
pub fn schedule_save() {
    if SAVE_PENDING.with(|p| p.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(Duration::from_secs(3), || {
        SAVE_PENDING.with(|p| p.set(false));
        if !QUITTING.with(Cell::get) {
            save_now();
        }
    });
}

pub fn save_now() {
    let windows: Vec<SavedWindow> = window::all()
        .iter()
        .filter(|w| !w.incognito)
        .map(|w| w.serialize())
        .collect();
    // With no window left the last saved session stays, to come back next time.
    if windows.is_empty() && settings::get().restore_session {
        return;
    }
    match serde_json::to_vec(&SessionFile {
        version: VERSION,
        windows,
    }) {
        Ok(json) => {
            if let Err(error) = store::write_atomic(&file(), &json) {
                eprintln!("[session] could not save: {error}");
            }
        }
        Err(error) => eprintln!("[session] could not serialize: {error}"),
    }
}

/// Closing the last normal window ends the session: remember it as it was, this window included.
pub fn window_closing(win: &Rc<BrowserWindow>) {
    if win.incognito || QUITTING.with(Cell::get) {
        return;
    }
    let others = window::all().iter().any(|w| w.id != win.id && !w.incognito);
    if !others {
        save_now();
    }
}

/// Saves the session as it is before the browser quits or restarts; the windows closing
/// afterwards don't change it.
pub fn save_before_exit() {
    if !QUITTING.with(|q| q.replace(true)) {
        save_now();
    }
    crate::bookmarks::flush();
    crate::downloads::flush();
    crate::permissions::flush();
}

/// Before the browser quits (Ctrl+Q, the app menu).
pub fn quit(app: &AppHandle) {
    save_before_exit();
    app.exit(0);
}
