//! Development harness: with PEEPWICK_HARNESS=<steps.json>, plays steps against the first
//! window and saves screenshots to PEEPWICK_HARNESS_OUT. Only in debug builds, or release
//! builds with the `harness` feature; never in what we ship.

use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::WidgetExt;
use javascriptcore::ValueExt;
use serde_json::Value;
use tauri::AppHandle;
use webkit2gtk::{ContextMenu, HitTestResult, SnapshotOptions, SnapshotRegion, WebViewExt, glib};

use crate::settings;
use crate::window::{self, BrowserWindow, OpenOptions};

pub fn start(app: AppHandle) {
    let Ok(file) = std::env::var("PEEPWICK_HARNESS") else {
        return;
    };
    let out =
        std::env::var("PEEPWICK_HARNESS_OUT").unwrap_or_else(|_| "/tmp/peepwick-shots".into());
    let _ = fs::create_dir_all(&out);
    let steps: Vec<Value> = match fs::read_to_string(&file).map(|t| serde_json::from_str(&t)) {
        Ok(Ok(steps)) => steps,
        _ => {
            eprintln!("[harness] could not read {file}");
            return;
        }
    };
    glib::MainContext::default().spawn_local(async move {
        let win = loop {
            if let Some(win) = window::all().into_iter().next() {
                break win;
            }
            glib::timeout_future(Duration::from_millis(50)).await;
        };
        glib::timeout_future(Duration::from_millis(1500)).await;
        for step in steps {
            run(&app, &win, &step, &out).await;
        }
    });
}

async fn run(app: &AppHandle, win: &Rc<BrowserWindow>, step: &Value, out: &str) {
    let get = |key: &str| step.get(key).and_then(Value::as_str).unwrap_or("");
    match get("do") {
        "wait" => {
            glib::timeout_future(Duration::from_millis(step["ms"].as_u64().unwrap_or(500))).await
        }
        "navigate" => {
            if let Some(tab) = win.active_tab() {
                tab.load(get("url"));
            }
        }
        "newTab" => {
            win.open_tab(OpenOptions {
                url: Some(get("url").to_string()),
                ..OpenOptions::default()
            });
        }
        "settings" => {
            settings::update(&step["patch"]);
            window::settings_changed();
        }
        "overlay" => win.set_overlay(step["open"].as_bool().unwrap_or(false)),
        "resize" => {
            let size = tauri::LogicalSize::new(
                step["width"].as_f64().unwrap_or(1400.0),
                step["height"].as_f64().unwrap_or(860.0),
            );
            let _ = win.tauri_window().set_size(size);
        }
        "eval" => {
            let result = glib::future_with_timeout(
                Duration::from_secs(5),
                win.ui_view()
                    .evaluate_javascript_future(get("js"), None, None),
            )
            .await;
            eprintln!(
                "[harness] eval {}",
                result.map(describe).unwrap_or_else(|_| "timeout".into())
            );
        }
        // In the view on screen: reader view while it's open.
        "evalTab" => {
            let Some(view) = win.active_tab().and_then(|t| t.shown_view()) else {
                eprintln!("[harness] evalTab: no page");
                return;
            };
            let result = glib::future_with_timeout(
                Duration::from_secs(5),
                view.evaluate_javascript_future(get("js"), None, None),
            )
            .await;
            eprintln!(
                "[harness] evalTab {}",
                result.map(describe).unwrap_or_else(|_| "timeout".into())
            );
        }
        "state" => {
            let tabs: Vec<String> = win
                .state()
                .tabs
                .values()
                .map(|t| {
                    format!(
                        "{} {} loading={} {:?} reader={}/{} zoom={:.2} find={} media={:?} audible={}",
                        t.id,
                        t.url,
                        t.loading,
                        t.interstitial,
                        t.reader_available,
                        t.reader,
                        t.zoom,
                        t.find.as_ref().map(|f| f.to_string()).unwrap_or_default(),
                        t.media,
                        t.audible
                    )
                })
                .collect();
            eprintln!("[harness] state {tabs:?}");
        }
        "capture" => match capture(win, &format!("{out}/{}.png", get("name"))).await {
            Ok(()) => eprintln!("[harness] captured {}", get("name")),
            Err(error) => eprintln!("[harness] capture failed: {error}"),
        },
        "snapshot" => match snapshot(win, &format!("{out}/{}.png", get("name"))).await {
            Ok(()) => eprintln!("[harness] snapshot {}", get("name")),
            Err(error) => eprintln!("[harness] snapshot failed: {error}"),
        },
        "measure" => {
            let (kb, count) = memory();
            eprintln!("[harness] memory {} MB in {count} processes", kb / 1024);
        }
        "mark" => eprintln!("[harness] mark {}", get("name")),
        // Which view has the keyboard inside the window, also while the window itself isn't the
        // active one on the desktop (document.hasFocus() is false then).
        "focus" => {
            let page = win
                .active_tab()
                .and_then(|t| t.view())
                .is_some_and(|v| v.is_focus());
            eprintln!(
                "[harness] focus ui={} page={page}",
                win.ui_view().is_focus()
            );
        }
        // Whether the shield's rules are off for the active tab's page.
        "shield" => {
            let paused = win
                .active_tab()
                .and_then(|t| t.view())
                .and_then(|v| v.user_content_manager())
                .map(|m| crate::shield::is_paused(&m));
            eprintln!("[harness] shield paused={paused:?}");
        }
        // Where each tab's view is, and where the UI asked for it.
        "layout" => {
            let panes: Vec<String> = win
                .visible_views()
                .iter()
                .map(|(_, r)| format!("{} {}x{}+{}+{}", r.tab_id, r.width, r.height, r.x, r.y))
                .collect();
            let views: Vec<String> = win
                .all_tabs()
                .iter()
                .filter_map(|tab| {
                    let view = tab.shown_view()?;
                    let a = view.allocation();
                    Some(format!(
                        "{} {}x{}+{}+{} visible={}",
                        tab.id,
                        a.width(),
                        a.height(),
                        a.x(),
                        a.y(),
                        view.is_visible()
                    ))
                })
                .collect();
            let gtk = win.gtk_window();
            let mapped = gtk.as_ref().is_some_and(|w| w.is_mapped());
            let ui = win.ui_view().allocation();
            eprintln!(
                "[harness] layout panes {panes:?} views {views:?} ui_on_top={} mapped={mapped} ui={}x{}",
                win.ui_on_top(),
                ui.width(),
                ui.height()
            );
        }
        "send" => {
            let args = step["args"].as_array().cloned().unwrap_or_default();
            crate::ipc::send(win, get("method"), &args);
        }
        "invoke" => {
            let args = step["args"].as_array().cloned().unwrap_or_default();
            let result = crate::ipc::invoke(win, get("method"), &args);
            eprintln!(
                "[harness] invoke {} {}",
                get("method"),
                result.map(|v| v.to_string()).unwrap_or_else(|e| e)
            );
        }
        "suggest" => {
            let local = crate::omnibox::suggest_local(win, get("text"));
            let out = crate::omnibox::finish(local).await;
            eprintln!(
                "[harness] suggest {}",
                serde_json::to_string(&out).unwrap_or_default()
            );
        }
        // A right-click without a mouse: `context` takes WebKit's hit test flags (2 page, 4 link…).
        "pageMenu" => {
            let Some(tab) = win.active_tab() else { return };
            let Some(view) = tab.view() else { return };
            let mut hit =
                HitTestResult::builder().context(step["context"].as_u64().unwrap_or(2) as u32);
            if !get("link").is_empty() {
                hit = hit.link_uri(get("link"));
            }
            let (x, y) = (
                step["x"].as_f64().unwrap_or(200.0),
                step["y"].as_f64().unwrap_or(200.0),
            );
            crate::menus::page_menu(&tab, &view, &ContextMenu::new(), &hit.build(), x, y);
        }
        // Imports a bookmark file without the file dialog.
        "importHtml" => {
            let html = fs::read_to_string(get("path")).unwrap_or_default();
            let result = crate::bookmarks::import_parsed(
                crate::bookmarks::parse_netscape(&html),
                true,
                "Imported",
            );
            eprintln!("[harness] importHtml {result}");
        }
        "bookmarks" => {
            let tree = serde_json::to_value(crate::bookmarks::tree()).unwrap_or_default();
            let count = |root: &str| tree[root]["children"].as_array().map_or(0, Vec::len);
            eprintln!(
                "[harness] bookmarks bar={} other={} {}",
                count("bar"),
                count("other"),
                tree["bar"]
            );
        }
        // An action on the newest download, or its menu.
        "downloadFirst" => {
            let Some(first) = crate::downloads::all("").into_iter().next() else {
                return;
            };
            match get("action") {
                "menu" => crate::downloads::menu(win, &first.id, 400.0, 200.0),
                action => crate::downloads::action(&first.id, action),
            }
        }
        // The lock panel and its "delete site data", which answer later.
        "siteInfo" => crate::security::site_info(win, |info| {
            let mut info = info;
            if let Some(cert) = info.get_mut("certificate").and_then(|c| c.as_object_mut()) {
                cert.remove("chain");
            }
            eprintln!("[harness] siteInfo {info}");
        }),
        "siteClear" => {
            crate::security::clear_site_data(win, |ok| eprintln!("[harness] siteClear {ok}"))
        }
        // Links from outside, as a second launch hands them over.
        "openUrls" => {
            let urls = step["urls"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|u| u.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            crate::window::open_urls(app, urls);
        }
        "quit" => crate::session::quit(app),
        // The list of saved logins under a field: what it offers, where it is, what it shows.
        "autofill" => {
            let shown = match crate::autofill::popup_view(win) {
                Some(view) if view.is_visible() => glib::future_with_timeout(
                    Duration::from_secs(5),
                    view.evaluate_javascript_future(
                        "[...document.querySelectorAll('.autofill-item')].map(li => li.className + ': ' + li.innerText.replace(/\\n/g, ' / ')).join(' | ')",
                        None,
                        None,
                    ),
                )
                .await
                .map(describe)
                .unwrap_or_else(|_| "timeout".into()),
                _ => "hidden".into(),
            };
            eprintln!(
                "[harness] autofill {} shows {shown}",
                crate::autofill::describe(win)
            );
        }
        // A press on one of the list's rows, as the pointer makes it.
        "autofillClick" => {
            if let Some(view) = crate::autofill::popup_view(win) {
                let js = format!(
                    "document.querySelectorAll('.autofill-item')[{}]?.dispatchEvent(new MouseEvent('mousedown', {{ bubbles: true, cancelable: true }}))",
                    step.get("index").and_then(Value::as_u64).unwrap_or(0)
                );
                let _ = view.evaluate_javascript_future(&js, None, None).await;
            }
        }
        // A secret in the (test) keyring, as another browser keeps one.
        "storeSecret" => {
            let (label, app_name, secret) = (
                get("label").to_string(),
                get("application").to_string(),
                get("secret").to_string(),
            );
            let stored = tauri::async_runtime::spawn_blocking(move || {
                crate::secrets::store(&label, &[("application", &app_name)], secret.as_bytes())
                    .map_err(|e| e.to_string())
            })
            .await;
            eprintln!("[harness] stored secret {stored:?}");
        }
        // The passwords page's CSV import and export, without their file dialogs.
        "passwordsCsv" => {
            if !get("export").is_empty() {
                crate::passwords::ready(false).await;
                let done = crate::passwords::export_to(std::path::Path::new(get("export")));
                eprintln!("[harness] passwords exported {done}");
            }
            if !get("import").is_empty() {
                crate::passwords::ready(true).await;
                let result = crate::passwords::import_csv_from(std::path::Path::new(get("import")));
                eprintln!("[harness] passwords imported {result}");
            }
        }
        // Keys pressed as the keyboard presses them, so a page sees trusted input: "text" types
        // characters, then "keys" presses named keys (Down, Return, BackSpace). To the active
        // page, or with "window" to the focused field of the window with that title (a file
        // dialog, which runs while the browser waits for it).
        "type" => {
            let target: Option<gtk::Widget> = if get("window").is_empty() {
                win.active_tab()
                    .and_then(|t| t.shown_view())
                    .map(gtk::prelude::Cast::upcast)
            } else {
                gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| gtk::prelude::Cast::downcast::<gtk::Window>(w).ok())
                    .find(|w| {
                        gtk::prelude::GtkWindowExt::title(w).as_deref() == Some(get("window"))
                    })
                    .map(gtk::prelude::Cast::upcast)
            };
            let Some(view) = target else {
                eprintln!("[harness] type: nothing to type into");
                return;
            };
            // Named keys may hold Ctrl: "ctrl+l".
            let mut keys: Vec<(u32, u32)> = Vec::new();
            for name in step["keys"].as_array().into_iter().flatten() {
                let Some(name) = name.as_str() else { continue };
                let (state, name) = match name.strip_prefix("ctrl+") {
                    Some(rest) => (gtk::gdk::ffi::GDK_CONTROL_MASK, rest),
                    None => (0, name),
                };
                if let Ok(name) = std::ffi::CString::new(name) {
                    keys.push((
                        unsafe { gtk::gdk::ffi::gdk_keyval_from_name(name.as_ptr()) },
                        state,
                    ));
                }
            }
            let typed = get("text")
                .chars()
                .map(|c| (unsafe { gtk::gdk::ffi::gdk_unicode_to_keyval(c as u32) }, 0));
            for (keyval, state) in typed.chain(keys) {
                press_key(&view, keyval, state);
                glib::timeout_future(Duration::from_millis(40)).await;
            }
        }
        // Backups without their file dialogs: what the export dialog offers, a backup written
        // to a path, and a file opened into the restore dialog (unlocked through the UI).
        "backupCounts" => eprintln!("[harness] backup counts {}", crate::backup::local_counts()),
        "backupExport" => {
            let categories: Vec<String> = step["categories"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if categories.iter().any(|c| c == "passwords") {
                crate::passwords::ready(false).await;
            }
            let payload = crate::backup::collect(&categories);
            let (path, password) = (get("path").to_string(), get("password").to_string());
            let saved = tauri::async_runtime::spawn_blocking(move || {
                crate::backup::write(std::path::Path::new(&path), &payload, &password)
            })
            .await;
            eprintln!("[harness] backup saved {saved:?}");
        }
        "backupOpen" => {
            let opened = crate::backup::open_path(std::path::Path::new(get("path")));
            eprintln!("[harness] backup opened {opened}");
            if let (Some(token), Some(name)) = (opened["token"].as_str(), opened["name"].as_str()) {
                win.command(
                    serde_json::json!({ "type": "backup-restore", "token": token, "name": name }),
                );
            }
        }
        other => eprintln!("[harness] unknown step {other}"),
    }
}

/// One key pressed and let go, handed to the widget as GTK hands it the keyboard's: a page view
/// gets it directly, a window passes it to its focused widget.
fn press_key(widget: &gtk::Widget, keyval: u32, state: u32) {
    use gtk::gdk::ffi as gdk;
    use gtk::glib::translate::ToGlibPtr;
    let Some(window) = widget.window() else {
        return;
    };
    let toplevel = gtk::glib::object::ObjectExt::is::<gtk::Window>(widget);
    unsafe {
        let display = gdk::gdk_window_get_display(window.to_glib_none().0);
        let seat = gdk::gdk_display_get_default_seat(display);
        let keyboard = if seat.is_null() {
            std::ptr::null_mut()
        } else {
            gdk::gdk_seat_get_keyboard(seat)
        };
        let mut keys: *mut gdk::GdkKeymapKey = std::ptr::null_mut();
        let mut count = 0;
        let mut keycode = 0;
        let keymap = gdk::gdk_keymap_get_for_display(display);
        if gdk::gdk_keymap_get_entries_for_keyval(keymap, keyval, &mut keys, &mut count) != 0 {
            if count > 0 {
                keycode = (*keys).keycode as u16;
            }
            glib::ffi::g_free(keys.cast());
        }
        for kind in [gdk::GDK_KEY_PRESS, gdk::GDK_KEY_RELEASE] {
            let event = gdk::gdk_event_new(kind);
            let key = event.cast::<gdk::GdkEventKey>();
            // Freeing the event lets go of this reference.
            (*key).window = window.to_glib_full();
            (*key).keyval = keyval;
            (*key).state = state;
            (*key).hardware_keycode = keycode;
            if !keyboard.is_null() {
                gdk::gdk_event_set_device(event, keyboard);
            }
            if toplevel {
                gtk::ffi::gtk_main_do_event(event);
            } else {
                gtk::ffi::gtk_widget_event(widget.to_glib_none().0, event);
            }
            gdk::gdk_event_free(event);
        }
    }
}

fn describe(result: Result<javascriptcore::Value, glib::Error>) -> String {
    match result {
        Ok(value) if value.is_undefined() => "undefined".into(),
        Ok(value) => value
            .to_json(0)
            .map(|s| s.to_string())
            .unwrap_or_else(|| value.to_str().to_string()),
        Err(error) => format!("error: {}", error.message()),
    }
}

/// The window as people see it: the UI, and the tab views where they sit (under the UI
/// while it is raised for a menu).
async fn capture(win: &Rc<BrowserWindow>, path: &str) -> Result<(), String> {
    let ui = win.ui_view();
    let (width, height) = (ui.allocated_width(), ui.allocated_height());
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .map_err(|e| e.to_string())?;
    let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    cr.set_source_rgb(11.0 / 255.0, 11.0 / 255.0, 12.0 / 255.0);
    cr.paint().map_err(|e| e.to_string())?;
    let ui_shot = glib::future_with_timeout(
        Duration::from_secs(5),
        ui.snapshot_future(
            SnapshotRegion::Visible,
            SnapshotOptions::TRANSPARENT_BACKGROUND,
        ),
    )
    .await
    .map_err(|_| "UI snapshot timed out".to_string())?
    .map_err(|e| e.message().to_string())?;
    let mut pages = Vec::new();
    for (view, rect) in win.visible_views() {
        if let Ok(Ok(shot)) = glib::future_with_timeout(
            Duration::from_secs(5),
            view.snapshot_future(SnapshotRegion::Visible, SnapshotOptions::NONE),
        )
        .await
        {
            pages.push((shot, rect));
        }
    }
    let paint_ui = |cr: &cairo::Context| -> Result<(), String> {
        cr.set_source_surface(&ui_shot, 0.0, 0.0)
            .map_err(|e| e.to_string())?;
        cr.paint().map_err(|e| e.to_string())
    };
    if !win.ui_on_top() {
        paint_ui(&cr)?;
    }
    for (shot, rect) in &pages {
        cr.set_source_surface(shot, rect.x, rect.y)
            .map_err(|e| e.to_string())?;
        cr.paint().map_err(|e| e.to_string())?;
    }
    // The list of saved logins over the page, where it sits.
    if let Some(popup) = crate::autofill::popup_view(win).filter(|v| v.is_visible())
        && let Some((x, y)) = popup.translate_coordinates(&ui, 0, 0)
        && let Ok(Ok(shot)) = glib::future_with_timeout(
            Duration::from_secs(5),
            popup.snapshot_future(
                SnapshotRegion::Visible,
                SnapshotOptions::TRANSPARENT_BACKGROUND,
            ),
        )
        .await
    {
        cr.set_source_surface(&shot, f64::from(x), f64::from(y))
            .map_err(|e| e.to_string())?;
        cr.paint().map_err(|e| e.to_string())?;
    }
    if win.ui_on_top() {
        paint_ui(&cr)?;
    }
    drop(cr);
    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    surface.write_to_png(&mut file).map_err(|e| e.to_string())
}

/// The whole page in the active tab, wider or taller than the window too: for images made
/// from HTML, such as the README banners in docs/brand.
async fn snapshot(win: &Rc<BrowserWindow>, path: &str) -> Result<(), String> {
    let view = win.active_tab().and_then(|t| t.view()).ok_or("no page")?;
    let shot = glib::future_with_timeout(
        Duration::from_secs(10),
        view.snapshot_future(SnapshotRegion::FullDocument, SnapshotOptions::NONE),
    )
    .await
    .map_err(|_| "page snapshot timed out".to_string())?
    .map_err(|e| e.message().to_string())?;
    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    shot.write_to_png(&mut file).map_err(|e| e.to_string())
}

/// Memory (proportional set size) of this process and everything it started: WebKit's web,
/// network and GPU processes included.
fn memory() -> (u64, usize) {
    let mut parent_of: HashMap<u32, u32> = HashMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // The command name may contain spaces; the fields after ")" are fixed.
            if let Some(rest) = stat.rsplit_once(')').map(|(_, rest)| rest)
                && let Some(ppid) = rest.split_whitespace().nth(1).and_then(|p| p.parse().ok())
            {
                parent_of.insert(pid, ppid);
            }
        }
    }
    let me = std::process::id();
    let mut family = vec![me];
    let mut i = 0;
    while i < family.len() {
        let parent = family[i];
        family.extend(
            parent_of
                .iter()
                .filter(|(_, p)| **p == parent)
                .map(|(pid, _)| *pid),
        );
        i += 1;
    }
    let total = family
        .iter()
        .filter_map(|pid| fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok())
        .filter_map(|text| {
            text.lines()
                .find(|l| l.starts_with("Pss:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        })
        .sum();
    (total, family.len())
}
