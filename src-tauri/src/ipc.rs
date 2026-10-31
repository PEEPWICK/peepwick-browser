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

use crate::actions::{self, COMMANDS};
use crate::i18n::t;
use crate::input::resolve_input;
use crate::menu::{self, ClickEvent};
use crate::settings::{self, Lang};
use crate::state::PaneRect;
use crate::window::{self, BrowserWindow, OpenOptions};
use crate::{menus, zoom};

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
        "palette" => palette(win, &short(args.first(), 200)),
        "stats" => {
            json!({ "blockedToday": 0, "blockedWeek": 0, "blockedTotal": 0, "focusMinutesToday": 0, "focusMinutesWeek": 0 })
        }
        "passwordsStatus" => json!({ "count": 0, "backend": "", "never": [] }),
        "clearDataCounts" => {
            json!({ "history": 0, "downloads": 0, "cookies": 0, "permissions": 0, "forms": 0, "passwords": 0 })
        }
        "formEntryCount" => json!(0),
        "suggest"
        | "historyQuery"
        | "topSites"
        | "extensionsList"
        | "permissionsList"
        | "shieldReport"
        | "importSources"
        | "downloadsList"
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

/// Lower case for matching, with the Turkish dotted and dotless i.
fn fold(text: &str, lang: Lang) -> String {
    match lang {
        Lang::Tr => text
            .chars()
            .map(|c| match c {
                'I' => 'ı',
                'İ' => 'i',
                c => c,
            })
            .collect::<String>()
            .to_lowercase(),
        Lang::En => text.to_lowercase(),
    }
}

/// The command bar: commands, then open tabs.
fn palette(win: &Rc<BrowserWindow>, input: &str) -> Value {
    let lang = settings::get().language;
    let needle = fold(input.trim(), lang);
    let matches = |text: &str| needle.is_empty() || fold(text, lang).contains(&needle);
    let commands = COMMANDS
        .iter()
        .filter(|id| actions::is_available(win, id))
        .map(|id| (id, t(&format!("cmd.{id}"))))
        .filter(|(_, title)| matches(title))
        .take(if needle.is_empty() { 8 } else { 6 })
        .map(
            |(id, title)| json!({ "kind": "command", "title": title, "value": id, "command": id }),
        );
    let tabs = win
        .all_tabs()
        .into_iter()
        .filter(|tab| matches(&tab.display_title()) || tab.url().to_lowercase().contains(&needle))
        .take(8)
        .map(|tab| {
            json!({ "kind": "tab", "title": tab.display_title(), "value": tab.display_url(), "favicon": tab.favicon(), "tabId": tab.id })
        });
    Value::Array(commands.chain(tabs).collect())
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
        "forgetZoom" => {
            zoom::forget(arg(0).and_then(Value::as_str));
            window::zoom_changed();
        }
        _ => eprintln!("[ipc] not ported yet: send {method}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_turkish_letters() {
        assert_eq!(fold("IŞIK İzmir", Lang::Tr), "ışık izmir");
        assert_eq!(fold("Issue", Lang::En), "issue");
    }
}
