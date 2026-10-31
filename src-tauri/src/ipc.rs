//! The UI's three entry points (invoke, send and the pw:event channel), routed by method
//! name as the Electron main process did. GTK lives on the main thread, so every call is
//! carried out there.

use std::rc::Rc;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::settings;
use crate::state::PaneRect;
use crate::window::{self, BrowserWindow};

enum Reply {
    /// The window's UI asked before Rust finished setting the window up.
    NotReady,
    Done(Result<Value, String>),
}

#[tauri::command]
pub async fn pw_invoke(
    app: AppHandle,
    webview: tauri::Webview,
    method: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    let label = webview.label().to_string();
    for _ in 0..100 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (label, method, args) = (label.clone(), method.clone(), args.clone());
        app.run_on_main_thread(move || {
            let reply = match window::find(&label) {
                Some(win) => Reply::Done(invoke(&win, &method, &args)),
                None => Reply::NotReady,
            };
            let _ = tx.send(reply);
        })
        .map_err(|e| e.to_string())?;
        match rx.await.map_err(|e| e.to_string())? {
            Reply::Done(result) => return result,
            Reply::NotReady => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    Err(format!("{method}: the window isn't ready"))
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

fn text(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

fn invoke(win: &Rc<BrowserWindow>, method: &str, args: &[Value]) -> Result<Value, String> {
    let value = match method {
        "ready" => serde_json::to_value(win.initial_data()).map_err(|e| e.to_string())?,
        "appInfo" => {
            serde_json::to_value(window::app_info(&tauri_app(win))).map_err(|e| e.to_string())?
        }
        "stats" => {
            json!({ "blockedToday": 0, "blockedWeek": 0, "blockedTotal": 0, "focusMinutesToday": 0, "focusMinutesWeek": 0 })
        }
        "passwordsStatus" => json!({ "count": 0, "backend": "", "never": [] }),
        "clearDataCounts" => {
            json!({ "history": 0, "downloads": 0, "cookies": 0, "permissions": 0, "forms": 0, "passwords": 0 })
        }
        "formEntryCount" => json!(0),
        "suggest"
        | "palette"
        | "historyQuery"
        | "topSites"
        | "extensionsList"
        | "permissionsList"
        | "shieldReport"
        | "importSources"
        | "downloadsList"
        | "passwordsList"
        | "addressesList"
        | "siteZooms"
        | "tasks"
        | "readingList"
        | "translationLanguages"
        | "translationModels" => json!([]),
        _ => {
            eprintln!("[ipc] not ported yet: invoke {method}");
            Value::Null
        }
    };
    let _ = args;
    Ok(value)
}

fn send(win: &Rc<BrowserWindow>, method: &str, args: &[Value]) {
    let tab = |i: usize| num(args.get(i)).and_then(|id| win.tab(id));
    match method {
        "windowControl" => win.window_control(text(args.first())),
        "setPanes" => {
            let rects: Vec<PaneRect> = args
                .first()
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            win.set_panes(rects);
        }
        "setOverlay" => win.set_overlay(args.first().and_then(Value::as_bool).unwrap_or(false)),
        "setRaised" => win.set_raised(args.first().and_then(Value::as_bool).unwrap_or(false)),
        "newTab" => {
            let options = args.first();
            let url = options
                .and_then(|o| o.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let background = options
                .and_then(|o| o.get("background"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            win.new_tab(url, background);
        }
        "closeTab" => {
            if let Some(id) = num(args.first()) {
                win.close_tab(id);
            }
        }
        "activateTab" | "focusTab" => {
            if let Some(id) = num(args.first()) {
                win.activate_tab(id);
            }
        }
        "navigate" => {
            if let Some(id) = num(args.first()) {
                win.navigate(id, text(args.get(1)));
            }
        }
        "back" => tab(0).iter().for_each(|t| t.back()),
        "forward" => tab(0).iter().for_each(|t| t.forward()),
        "reload" => {
            let hard = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            tab(0).iter().for_each(|t| t.reload(hard));
        }
        "stop" => tab(0).iter().for_each(|t| t.stop()),
        "toggleMute" => tab(0).iter().for_each(|t| t.set_muted(!t.is_muted())),
        "togglePin" => tab(0).iter().for_each(|t| {
            t.pinned.set(!t.pinned.get());
            win.mark_dirty();
        }),
        "openURL" => win.open_url(
            text(args.first()),
            args.get(1).and_then(Value::as_bool).unwrap_or(false),
        ),
        "openPage" => win.open_page(text(args.first()), text(args.get(1))),
        "setSettings" => {
            if let Some(patch) = args.first() {
                settings::update(patch);
                window::settings_changed();
            }
        }
        "onboardingDone" => {
            settings::update(&json!({ "onboarded": true }));
            window::settings_changed();
        }
        _ => eprintln!("[ipc] not ported yet: send {method}"),
    }
}

fn tauri_app(win: &BrowserWindow) -> AppHandle {
    win.app_handle()
}
