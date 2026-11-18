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
use crate::{
    bookmarks, cleardata, downloads, focus, history, importers, menus, omnibox, permissions,
    reader, readinglist, stats, zoom,
};

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
    // Saved addresses and form entries: sealed with the passwords' key, read (or made) first.
    if let Some(create) = crate::formdata::invoke_needs_key(&method) {
        crate::passwords::ready(create).await;
        return on_main(&app, &label, move |_| {
            crate::formdata::invoke(&method, &args)
        })
        .await;
    }
    // Saved passwords: the keyring's key is read, or made, off the main thread first.
    if let Some(create) = crate::passwords::invoke_needs_key(&method) {
        crate::passwords::ready(create).await;
        return on_main(&app, &label, move |win| {
            crate::passwords::invoke(win, &method, &args)
        })
        .await;
    }
    if matches!(
        method.as_str(),
        "siteInfo" | "siteClearData" | "clearData" | "clearDataCounts"
    ) {
        // Form entries are counted, or cleared for a range, with the key.
        if method.starts_with("clearData")
            && on_main(&app, &label, |_| crate::formdata::needs_key()).await?
        {
            crate::passwords::ready(false).await;
        }
        // WebKit answers these later (cookies, site data): the reply waits for it.
        let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        on_main(&app, &label, move |win| {
            let tx = tx.clone();
            let done = move |value: Value| {
                if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                    let _ = tx.send(value);
                }
            };
            match method.as_str() {
                "siteInfo" => crate::security::site_info(win, done),
                "siteClearData" => crate::security::clear_site_data(win, done),
                "clearData" => cleardata::clear_request(cleardata::request(args.first()), done),
                _ => cleardata::counts(cleardata::range(args.first()), done),
            }
        })
        .await?;
        return rx.await.map_err(|e| e.to_string());
    }
    // Other browsers' profiles are read off the main thread; their bookmarks go in on it.
    if method == "importSources" {
        return tauri::async_runtime::spawn_blocking(importers::detect_sources)
            .await
            .map_err(|e| e.to_string());
    }
    if method == "importFrom" {
        if on_main(&app, &label, |win| win.incognito).await? {
            return Ok(Value::Null);
        }
        let source = short(args.first(), 100);
        let kinds: Vec<String> = args
            .get(1)
            .and_then(Value::as_array)
            .map(|kinds| {
                kinds
                    .iter()
                    .filter_map(|k| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let skip_duplicates = args.get(2).and_then(Value::as_bool) != Some(false);
        // Firefox's primary password, when the profile has one.
        let primary = short(args.get(3), 1000);
        let read = tauri::async_runtime::spawn_blocking(move || {
            importers::read(&source, &kinds, &primary)
        })
        .await
        .map_err(|e| e.to_string())?;
        let Some(mut read) = read else {
            return Ok(Value::Null);
        };
        if read.needs_key() {
            crate::passwords::ready(true).await;
        }
        let cookies = read.take_cookies();
        let mut result = on_main(&app, &label, move |_| {
            importers::finish(read.clone(), skip_duplicates)
        })
        .await?;
        // Cookies go into WebKit's jar; the dialog says how many it took.
        if let Some(cookies) = cookies {
            let (tx, rx) = tokio::sync::oneshot::channel::<usize>();
            let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
            on_main(&app, &label, move |_| {
                let tx = tx.clone();
                importers::set_cookies(cookies.clone(), move |taken| {
                    if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                        let _ = tx.send(taken);
                    }
                });
            })
            .await?;
            result["cookies"] = json!(rx.await.unwrap_or(0));
        }
        return Ok(result);
    }
    // The task manager: which tabs live where comes from the main thread, the processes'
    // memory and processor use from /proc, off it.
    if method == "tasks" {
        let tabs = on_main(&app, &label, |_| crate::tasks::tab_processes()).await?;
        let rows = tauri::async_runtime::spawn_blocking(move || crate::tasks::rows(tabs))
            .await
            .map_err(|e| e.to_string())?;
        return to_json(rows);
    }
    // Translation's languages: Firefox's model records may have to be fetched first.
    if method == "translationLanguages" {
        return tauri::async_runtime::spawn_blocking(crate::translation::languages)
            .await
            .map(Value::from)
            .map_err(|e| e.to_string());
    }
    // Read aloud's voices come from speech-dispatcher, which may first have to start.
    if method == "speechVoices" {
        return to_json(crate::speech::voices().await);
    }
    if method == "suggest" {
        // The search engine's suggestions are fetched here, off the main thread.
        let input = short(args.first(), 500);
        let local = on_main(&app, &label, move |win| omnibox::suggest_local(win, &input)).await?;
        return to_json(omnibox::finish(local).await);
    }
    // The desktop's tools (xdg-settings) take a moment: they run off the main thread too.
    if method == "setDefaultBrowser" {
        let result = tauri::async_runtime::spawn_blocking(crate::desktop::make_default).await;
        return Ok(json!(result.unwrap_or("failed")));
    }
    if method == "appInfo" {
        let default = tauri::async_runtime::spawn_blocking(crate::desktop::is_default);
        let mut info = on_main(&app, &label, |_| window::app_info()).await?;
        info.is_default_browser = default.await.unwrap_or(false);
        return to_json(info);
    }
    if method.starts_with("backup") {
        return backup_call(&app, &label, &method, &args).await;
    }
    on_main(&app, &label, move |win| invoke(win, &method, &args)).await?
}

/// Backups: the file dialogs and the data on the main thread; scrypt, which takes about half a
/// second, and the file off it.
async fn backup_call(
    app: &AppHandle,
    label: &str,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    let token = short(args.first(), 64);
    // Addresses and form entries are counted and copied with the key.
    let open_form_data = async || -> Result<(), String> {
        if on_main(app, label, |_| crate::formdata::needs_key()).await? {
            crate::passwords::ready(false).await;
        }
        Ok(())
    };
    match method {
        "backupCounts" => {
            open_form_data().await?;
            on_main(app, label, |_| crate::backup::local_counts()).await
        }
        "backupExport" => {
            let categories = ids(args.first());
            let password = text(args.get(1)).to_string();
            // The dialog asks for eight characters at least.
            if password.chars().count() < 8 {
                return Ok(json!("failed"));
            }
            if categories.iter().any(|c| c == "passwords") {
                crate::passwords::ready(false).await;
            }
            if categories.iter().any(|c| c == "autofill") {
                open_form_data().await?;
            }
            let chosen = on_main(app, label, |win| {
                (!win.incognito)
                    .then(|| crate::backup::choose_save_path(win))
                    .flatten()
            })
            .await?;
            let Some(path) = chosen else {
                return Ok(json!("cancelled"));
            };
            let payload = on_main(app, label, move |_| crate::backup::collect(&categories)).await?;
            let saved = tauri::async_runtime::spawn_blocking(move || {
                crate::backup::write(&path, &payload, &password)
            })
            .await
            .unwrap_or(false);
            Ok(json!(if saved { "saved" } else { "failed" }))
        }
        "backupOpen" => {
            let chosen = on_main(app, label, |win| {
                (!win.incognito)
                    .then(|| crate::backup::choose_backup_file(win))
                    .flatten()
            })
            .await?;
            let Some(path) = chosen else {
                return Ok(Value::Null);
            };
            tauri::async_runtime::spawn_blocking(move || crate::backup::open_path(&path))
                .await
                .map_err(|e| e.to_string())
        }
        "backupUnlock" => {
            let password = text(args.get(1)).to_string();
            Ok(tauri::async_runtime::spawn_blocking(move || {
                crate::backup::unlock(&token, &password)
            })
            .await
            .unwrap_or_else(|_| json!({ "error": "format" })))
        }
        "backupRestore" => {
            let categories = ids(args.get(1));
            if crate::backup::needs_key(&token, &categories) {
                crate::passwords::ready(true).await;
            }
            on_main(app, label, move |win| {
                crate::backup::restore(win, &token, &categories)
            })
            .await
        }
        _ => {
            eprintln!("[ipc] not ported yet: invoke {method}");
            Ok(Value::Null)
        }
    }
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
        "stats" => stats::stats(),
        // The harness's way in; the interface's calls wait for the key first (pw_invoke).
        m if crate::passwords::invoke_needs_key(m).is_some() => {
            crate::passwords::invoke(win, m, args)
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
        "certificate" => crate::security::certificate(win, &short(args.first(), 300)),
        "resetSettings" => {
            settings::reset();
            window::settings_changed();
            window::zoom_changed();
            Value::Null
        }
        "readingList" => {
            if win.incognito {
                json!([])
            } else {
                to_json(readinglist::list())?
            }
        }
        // The article of reader view or of an offline copy, as paragraphs to read aloud.
        "readAloud" => num(args.first())
            .and_then(|id| win.tab(id))
            .and_then(|tab| tab.article())
            .map(|article| {
                json!({
                    "title": article.title,
                    "lang": article.lang,
                    "paragraphs": reader::paragraphs(&article.content),
                })
            })
            .unwrap_or(Value::Null),
        "tasks" => to_json(crate::tasks::rows(crate::tasks::tab_processes()))?,
        "report" => crate::diagnostics::report(),
        "shieldReport" => to_json(
            win.active_tab()
                .map(|tab| tab.blocked_report(12))
                .unwrap_or_default(),
        )?,
        "translationModels" => crate::translation::models(),
        "extensionsList" => json!([]),
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
        "media" => {
            if text(arg(1)) == "toggle" {
                tab(0).iter().for_each(|tab| tab.toggle_media());
            }
        }
        "endTask" => {
            if let Some(pid) = arg(0)
                .and_then(Value::as_i64)
                .and_then(|p| i32::try_from(p).ok())
            {
                crate::tasks::end(pid);
            }
        }
        // Report a bug: the person copies the report, or opens the bug form with it.
        "reportCopy" => {
            let report = crate::diagnostics::report();
            menus::copy_text(report["text"].as_str().unwrap_or_default());
        }
        "reportIssue" => {
            let report = crate::diagnostics::report();
            menus::copy_text(report["text"].as_str().unwrap_or_default());
            win.open_tab(OpenOptions {
                url: Some(crate::diagnostics::issue_url(&report)),
                ..Default::default()
            });
        }
        "errorLogClear" => crate::diagnostics::clear(),
        // An error in the browser's own interface.
        "uiError" => crate::diagnostics::record("interface", &short(arg(0), 2000)),
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
        "toggleReader" => {
            if let Some(tab) = win.active_tab() {
                tab.toggle_reader();
            }
        }
        "speak" => {
            let Some(request) = arg(0).filter(|request| request.is_object()) else {
                return;
            };
            let field = |name: &str, max: usize| short(request.get(name), max);
            let voice = request.get("voice").filter(|voice| voice.is_object());
            crate::speech::speak(crate::speech::Utterance {
                owner: win.label.clone(),
                key: field("key", 64),
                text: field("text", crate::speech::MAX_TEXT),
                lang: field("lang", 35),
                voice: voice
                    .map(|voice| {
                        (
                            short(voice.get("module"), 100),
                            short(voice.get("name"), 200),
                        )
                    })
                    .filter(|(module, name)| !module.is_empty() && !name.is_empty()),
                rate: float(request.get("rate")),
            });
        }
        "speechStop" => crate::speech::stop(&win.label),
        "readingOpen" => {
            let id = short(arg(0), 64);
            let Some(url) = readinglist::url_for(&id).filter(|_| !win.incognito) else {
                return;
            };
            readinglist::mark_read(&id, true);
            if flag(arg(1)) {
                win.open_tab(OpenOptions {
                    url: Some(url),
                    ..OpenOptions::default()
                });
            } else if let Some(tab) = win.active_tab() {
                tab.load(&url);
            }
        }
        "readingMark" => readinglist::mark_read(&short(arg(0), 64), flag(arg(1))),
        "readingRemove" => readinglist::remove(&short(arg(0), 64)),
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
                "http" => tab.continue_over_http(),
                "proceed" => tab.proceed_insecure(),
                "open-anyway" => tab.open_anyway(),
                "dangerous" => tab.continue_to_dangerous(),
                other => eprintln!("[ipc] not ported yet: interstitial {other}"),
            }
        }
        "focusStart" => {
            let minutes = float(arg(0));
            focus::start(if minutes > 0.0 {
                minutes
            } else {
                f64::from(settings::get().focus_minutes)
            });
        }
        "focusStop" => focus::stop(),
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
        "restart" => crate::updater::restart(&win.app_handle()),
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
        "downloadsClear" => downloads::clear(0),
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
        "shieldToggleSite" => {
            let Some(tab) = win.active_tab().filter(|tab| tab.page().is_none()) else {
                return;
            };
            let Some(site) = crate::shield::site_of(&tab.url()) else {
                return;
            };
            let mut list = settings::get().shield_allowlist;
            if list.contains(&site) {
                list.retain(|s| *s != site);
            } else {
                list.push(site);
            }
            settings::update(&json!({ "shieldAllowlist": list }));
            window::settings_changed();
            tab.reload(false);
        }
        "certificateRevoke" => {
            if let Some(tab) = win.active_tab() {
                crate::security::revoke(&crate::pages::host_of(&tab.display_url()));
                tab.reload(false);
            }
        }
        "updateCheck" => crate::updater::check(&win.app_handle()),
        "updateInstall" => crate::updater::install_and_restart(&win.app_handle()),
        "forgetZoom" => {
            zoom::forget(arg(0).and_then(Value::as_str));
            window::zoom_changed();
        }
        "passwordCopy" => {
            let id = text(arg(0)).to_string();
            crate::passwords::when_ready(false, move || {
                crate::passwords::copy(&id);
            });
        }
        "passwordsRemove" => {
            let ids: Vec<String> = arg(0)
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            crate::passwords::remove(&ids);
        }
        "passwordsUndo" => crate::passwords::undo_remove(),
        "saveLoginAnswer" => {
            if let Some(id) = num(arg(0)) {
                crate::autofill::answer(id, text(arg(1)));
            }
        }
        "passwordNeverRemove" => crate::passwords::remove_never(text(arg(0))),
        "backupCancel" => crate::backup::forget(text(arg(0))),
        "translation" => {
            // Language codes as the models name them.
            let lang = |i: usize| {
                arg(i)
                    .and_then(Value::as_str)
                    .filter(|code| {
                        (2..=3).contains(&code.len())
                            && code.bytes().all(|b| b.is_ascii_lowercase())
                    })
                    .map(str::to_string)
            };
            if let Some(tab) = tab(0) {
                crate::translation::action(&tab, text(arg(1)), lang(2), lang(3));
            }
        }
        "translationRemove" => crate::translation::remove_model(text(arg(0)), text(arg(1))),
        "addressRemove" => {
            let id = short(arg(0), 64);
            crate::formdata::when_open(false, move || crate::formdata::remove_address(&id));
        }
        "formEntriesClear" => crate::formdata::clear_entries(0),
        _ => eprintln!("[ipc] not ported yet: send {method}"),
    }
}
