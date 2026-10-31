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
        "evalTab" => {
            let Some(view) = win.active_tab().and_then(|t| t.view()) else {
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
                        "{} {} loading={} {:?}",
                        t.id, t.url, t.loading, t.interstitial
                    )
                })
                .collect();
            eprintln!("[harness] state {tabs:?}");
        }
        "capture" => match capture(win, &format!("{out}/{}.png", get("name"))).await {
            Ok(()) => eprintln!("[harness] captured {}", get("name")),
            Err(error) => eprintln!("[harness] capture failed: {error}"),
        },
        "measure" => {
            let (kb, count) = memory();
            eprintln!("[harness] memory {} MB in {count} processes", kb / 1024);
        }
        "mark" => eprintln!("[harness] mark {}", get("name")),
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
                    let view = tab.view()?;
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
            eprintln!(
                "[harness] layout panes {panes:?} views {views:?} ui_on_top={}",
                win.ui_on_top()
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
        "quit" => crate::session::quit(app),
        other => eprintln!("[harness] unknown step {other}"),
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
    if win.ui_on_top() {
        paint_ui(&cr)?;
    }
    drop(cr);
    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    surface.write_to_png(&mut file).map_err(|e| e.to_string())
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
