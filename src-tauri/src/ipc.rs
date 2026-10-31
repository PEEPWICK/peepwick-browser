//! The UI's three entry points (invoke, send and the pw:event channel), routed by method
//! name as the Electron main process did. GTK lives on the main thread, so every call is
//! carried out there.

use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::actions;
use crate::input::resolve_input;
use crate::menu::{self, ClickEvent};
use crate::settings;
use crate::state::PaneRect;
use crate::window::{self, BrowserWindow, OpenOptions};
use crate::{bookmarks, downloads, history, menus, omnibox, permissions, zoom};

/// Runs `f` on the main thread with the window whose UI called. The UI may ask before Rust
/// has finished setting its window up; then it waits a little.
async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    label: &str,
    f: impl Fn(&Rc<BrowserWindow>) -> T + Send + Clone + 'static,
) -> Result<T, String> {
    for _ in 0..100 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (label, f) = (label.to_string(), f.clone());
        app.run_on_main_thread(move || {
            let _ = tx.send(window::find(&label).map(|win| f(&win)));
        })
        .map_err(|e| e.to_string())?;
        match rx.await.map_err(|e| e.to_string())? {
            Some(value) => return Ok(value),
            None => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    Err("the window isn't ready".into())
}

#[tauri::command]
pub async fn pw_invoke(
    app: AppHandle,
    webview: tauri::Webview,
    method: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    let label = webview.label().to_string();
    if method == "suggest" {
        // The search engine's suggestions are fetched here, off the main thread.
        let input = short(args.first(), 500);
        let local = on_main(&app, &label, move |win| omnibox::suggest_local(win, &input)).await?;
        return to_json(omnibox::finish(local).await);
    }
    on_main(&app, &label, move |win| invoke(win, &method, &args)).await?
}

#[tauri::command]
pub fn pw_send(
    app: AppHandle,
    webview: tauri::Webview,
    method: String,
    args: Vec<Value>,
) -> Result<(), String> {
    let label = webview.label().to_string();
    app.run_on_main_thread(move || {
        if let Some(win) = window::find(&label) {
            send(&win, &method, &args);
        }
    })
    .map_err(|e| e.to_string())
}

fn num(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn float(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}

fn flag(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

fn text(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// A list of ids from the UI, at most 5000 of them.
fn ids(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .take(5000)
                .map(|id| id.chars().take(64).collect())
                .collect()
        })
        .unwrap_or_default()
}

/// At most `max` characters of a string from the UI.
fn short(value: Option<&Value>, max: usize) -> String {
    text(value).chars().take(max).collect()
}

fn to_json(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

pub(crate) fn invoke(
    win: &Rc<BrowserWindow>,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    let value = match method {
        "ready" => {
            // Shown once the UI has had a moment to draw its first state.
            let weak = Rc::downgrade(win);
            glib::timeout_add_local_once(Duration::from_millis(50), move || {
                if let Some(win) = weak.upgrade() {
                    win.ui_ready();
                }
            });
            to_json(win.initial_data())?
        }
        "appInfo" => to_json(window::app_info())?,
        "siteZooms" => to_json(zoom::site_zooms())?,
        "palette" => to_json(omnibox::palette(win, &short(args.first(), 200)))?,
        "historyQuery" => {
            let query = |key: &str| args.first().and_then(|q| q.get(key));
            let text: String = query("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            let before = query("before").and_then(Value::as_i64);
            let limit = query("limit").and_then(Value::as_i64);
            if win.incognito {
                json!([])
            } else {
                to_json(history::query(&text, before, limit))?
            }
        }
        "topSites" => {
            if win.incognito {
                json!([])
            } else {
                to_json(history::top_sites(8))?
            }
        }
        "stats" => {
            json!({ "blockedToday": 0, "blockedWeek": 0, "blockedTotal": 0, "focusMinutesToday": 0, "focusMinutesWeek": 0 })
        }
        "passwordsStatus" => json!({ "count": 0, "backend": "", "never": [] }),
        "clearDataCounts" => {
            json!({ "history": 0, "downloads": 0, "cookies": 0, "permissions": 0, "forms": 0, "passwords": 0 })
        }
        "formEntryCount" => json!(0),
        "bookmarkCreate" => {
            let input = |key: &str| args.first().and_then(|i| i.get(key));
            let index = input("index").and_then(Value::as_u64).map(|i| i as usize);
            let title: String = input("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(500)
                .collect();
            let url = input("url")
                .and_then(Value::as_str)
                .map(|u| u.chars().take(8192).collect::<String>());
            let parent = input("parentId").and_then(Value::as_str).unwrap_or("");
            json!(bookmarks::create(parent, index, &title, url.as_deref()))
        }
        "bookmarkPage" => match actions::bookmark_current_page(win) {
            Some((id, created)) => json!({ "id": id, "created": created }),
            None => Value::Null,
        },
        // Reading other browsers' profiles hasn't moved over yet; HTML files have.
        "importSources" => json!([]),
        "importBookmarks" => match text(args.first()) {
            "html" if !win.incognito => {
                bookmarks::import_html(win, args.get(1).and_then(Value::as_bool) != Some(false))
            }
            _ => Value::Null,
        },
        "exportBookmarks" => json!(!win.incognito && bookmarks::export(win)),
        "downloadsList" => to_json(downloads::all(&short(args.first(), 200)))?,
        "chooseDownloadFolder" => downloads::choose_folder(win),
        "permissionsList" => permissions::list(),
        "extensionsList"
        | "shieldReport"
        | "passwordsList"
        | "addressesList"
        | "tasks"
        | "readingList"
        | "translationLanguages"
        | "translationModels" => json!([]),
        _ => {
            eprintln!("[ipc] not ported yet: invoke {method}");
            Value::Null
        }
    };
    Ok(value)
}

pub(crate) fn send(win: &Rc<BrowserWindow>, method: &str, args: &[Value]) {
    let arg = |i: usize| args.get(i);
    let tab = |i: usize| num(arg(i)).and_then(|id| win.tab(id));
    match method {
        "windowControl" => win.window_control(text(arg(0))),
        "setPanes" => {
            let rects = arg(0)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .take(4)
                        .filter_map(|r| serde_json::from_value::<PaneRect>(r.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            win.set_panes(rects);
        }
        "setOverlay" => win.set_overlay(flag(arg(0))),
        "setRaised" => win.set_raised(flag(arg(0))),
        "menuSelect" => {
            let event: ClickEvent = arg(2)
                .and_then(|e| serde_json::from_value(e.clone()).ok())
                .unwrap_or_default();
            if let (Some(menu_id), Some(item_id)) = (num(arg(0)), num(arg(1))) {
                menu::select(win, menu_id, item_id, event);
            }
        }
        "menuClosed" => {
            if let Some(id) = num(arg(0)) {
                menu::closed(win, id);
            }
        }
        "menuContext" => {
            if let (Some(menu_id), Some(item_id)) = (num(arg(0)), num(arg(1))) {
                menu::context(win, menu_id, item_id, float(arg(2)), float(arg(3)));
            }
        }
        "confirmReply" => {
            if let Some(id) = num(arg(0)) {
                menu::confirm_reply(id, flag(arg(1)));
            }
        }
        "appMenu" => menus::app_menu(win, float(arg(0)), float(arg(1))),
        "tabMenu" => {
            if let Some(id) = num(arg(0)) {
                menus::tab_menu(win, id, float(arg(1)), float(arg(2)));
            }
        }
        "newTab" => {
            let option = |key: &str| arg(0).and_then(|o| o.get(key));
            let url = option("url")
                .and_then(Value::as_str)
                .map(|url| resolve_input(url, settings::get().search_engine))
                .filter(|url| !url.is_empty());
            match url {
                Some(url) => {
                    let background = flag(option("background"));
                    win.open_tab(OpenOptions {
                        url: Some(url),
                        background,
                        ..OpenOptions::default()
                    });
                }
                None => actions::new_tab(win),
            }
        }
        "closeTab" => {
            if let Some(id) = num(arg(0)) {
                win.close_tab(id, true);
            }
        }
        "activateTab" => {
            if let Some(id) = num(arg(0)) {
                win.activate_tab(id);
            }
        }
        "focusTab" => {
            if let Some(tab) = tab(0) {
                win.pane_focused(tab.id);
                if let Some(view) = tab.view() {
                    view.grab_focus();
                }
            }
        }
        "navigate" => {
            let url = resolve_input(&short(arg(1), 8192), settings::get().search_engine);
            if let Some(tab) = tab(0)
                && !url.is_empty()
            {
                tab.load(&url);
                win.focus_active();
            }
        }
        "back" => tab(0).iter().for_each(|tab| tab.back()),
        "forward" => tab(0).iter().for_each(|tab| tab.forward()),
        "reload" => tab(0).iter().for_each(|tab| tab.reload(flag(arg(1)))),
        "stop" => tab(0).iter().for_each(|tab| tab.stop()),
        "togglePin" => {
            if let Some(id) = num(arg(0)) {
                win.toggle_pin(id);
            }
        }
        "toggleMute" => tab(0).iter().for_each(|tab| tab.set_muted(!tab.is_muted())),
        "moveTab" => {
            if let Some(id) = num(arg(0)) {
                let index = num(arg(2)).unwrap_or(0) as usize;
                win.move_tab(
                    id,
                    &short(arg(1), 64),
                    index,
                    arg(3).and_then(Value::as_bool),
                );
            }
        }
        "openPage" => actions::open_page(win, &short(arg(0), 40), &short(arg(1), 40)),
        "openURL" => {
            let url = text(arg(0));
            if !["http:", "https:", "peepwick:"]
                .iter()
                .any(|s| url.starts_with(s))
            {
                return;
            }
            if flag(arg(1)) {
                win.open_tab(OpenOptions {
                    url: Some(url.to_string()),
                    ..OpenOptions::default()
                });
            } else if let Some(tab) = win.active_tab() {
                tab.load(url);
            }
        }
        "split" => {
            if let (Some(left), Some(right)) = (num(arg(0)), num(arg(1))) {
                win.split(left, right);
            }
        }
        "unsplit" => win.unsplit(),
        "splitRatio" => win.set_split_ratio(float(arg(0))),
        "createWorkspace" => win.create_workspace(&short(arg(0), 40), text(arg(1))),
        "updateWorkspace" => {
            let patch = |key: &str| arg(1).and_then(|p| p.get(key)).and_then(Value::as_str);
            win.update_workspace(&short(arg(0), 64), patch("name"), patch("icon"));
        }
        "deleteWorkspace" => win.delete_workspace(&short(arg(0), 64)),
        "activateWorkspace" => win.activate_workspace(&short(arg(0), 64)),
        "find" => {
            if let Some(tab) = win.active_tab() {
                let forward = arg(1).and_then(Value::as_bool) != Some(false);
                tab.find_in_page(&short(arg(0), 500), forward, flag(arg(2)));
            }
        }
        "stopFind" => win.active_tab().iter().for_each(|tab| tab.stop_find()),
        "zoom" => {
            let direction = text(arg(0));
            if matches!(direction, "in" | "out" | "reset")
                && let Some(tab) = win.active_tab()
            {
                tab.zoom(direction);
            }
        }
        "interstitial" => {
            let Some(tab) = tab(0) else { return };
            match text(arg(1)) {
                "retry" => tab.reload(false),
                "back" => tab.back(),
                other => eprintln!("[ipc] not ported yet: interstitial {other}"),
            }
        }
        "command" => {
            let id = text(arg(0));
            if actions::is_available(win, id) {
                actions::run_command(win, id);
            }
        }
        "setSettings" => {
            if let Some(patch) = arg(0).filter(|p| p.is_object()) {
                settings::update(patch);
                window::settings_changed();
                if patch.get("defaultZoom").is_some() {
                    window::zoom_changed();
                }
            }
        }
        "onboardingDone" => {
            settings::update(&json!({ "onboarded": true }));
            window::settings_changed();
        }
        "historyRemove" => {
            if win.incognito {
                return;
            }
            let ids: Vec<i64> = arg(0)
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_i64).take(5000).collect())
                .unwrap_or_default();
            history::remove_visits(&ids);
        }
        "historyMenu" => {
            if !win.incognito
                && let Some(id) = arg(0).and_then(Value::as_i64)
            {
                menus::history_menu(win, id, text(arg(1)), float(arg(2)), float(arg(3)));
            }
        }
        "bookmarkUpdate" => {
            let patch = |key: &str| arg(1).and_then(|p| p.get(key)).and_then(Value::as_str);
            bookmarks::update(&short(arg(0), 64), patch("title"), patch("url"));
        }
        "bookmarkMove" => bookmarks::move_nodes(
            &ids(arg(0)),
            &short(arg(1), 64),
            num(arg(2)).unwrap_or(0) as usize,
        ),
        "bookmarkRemove" => bookmarks::remove_and_tell(win, &ids(arg(0))),
        "bookmarkUndo" => bookmarks::undo_remove(),
        "bookmarkSort" => bookmarks::sort_folder(&short(arg(0), 64)),
        "bookmarkOpen" => {
            let how = text(arg(1));
            if matches!(
                how,
                "current" | "tab" | "background" | "window" | "incognito"
            ) {
                bookmarks::open(win, &ids(arg(0)), how);
            }
        }
        "bookmarkMenu" => {
            bookmarks::bookmark_menu(win, &short(arg(0), 64), float(arg(1)), float(arg(2)))
        }
        "bookmarkFolderMenu" => {
            let from = num(arg(3)).unwrap_or(0) as usize;
            bookmarks::folder_menu(win, &short(arg(0), 64), float(arg(1)), float(arg(2)), from);
        }
        "bookmarkTabs" => actions::bookmark_space_tabs(win),
        "download" => downloads::action(&short(arg(0), 64), text(arg(1))),
        "downloadsClear" => downloads::clear(),
        "downloadsOpenFolder" => downloads::open_folder(),
        "downloadMenu" => downloads::menu(win, &short(arg(0), 64), float(arg(1)), float(arg(2))),
        "permission" => {
            if let Some(id) = num(arg(0)) {
                permissions::respond(id, flag(arg(1)), flag(arg(2)));
            }
        }
        "permissionRemove" => permissions::set(&short(arg(0), 500), text(arg(1)), None),
        "sitePermission" => permissions::set(
            &short(arg(0), 500),
            text(arg(1)),
            arg(2).and_then(Value::as_bool),
        ),
        "updateCheck" => crate::updater::check(&win.app_handle()),
        "updateInstall" => crate::updater::install_and_restart(&win.app_handle()),
        "forgetZoom" => {
            zoom::forget(arg(0).and_then(Value::as_str));
            window::zoom_changed();
        }
        _ => eprintln!("[ipc] not ported yet: send {method}"),
    }
}
