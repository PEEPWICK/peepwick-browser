//! Site permissions. Requests for the location, the camera, the microphone or notifications
//! become a bar above the page; answers can be remembered per site, in permissions.json (the
//! file the Electron version kept) or, for private windows, in memory only.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use webkit2gtk::{
    DeviceInfoPermissionRequest, GeolocationPermissionRequest, NotificationExt,
    NotificationPermissionRequest, PermissionRequest, PermissionRequestExt,
    PointerLockPermissionRequest, SecurityOrigin, UserMediaPermissionRequest,
    UserMediaPermissionRequestExt, WebContext, WebContextExt, WebView, WebViewExt,
};

use crate::tab::Tab;
use crate::window::{self, BrowserWindow};
use crate::{notify, paths, store};

/// Site → kind → allowed.
type Decisions = BTreeMap<String, BTreeMap<String, bool>>;

/// The kinds a remembered answer can be about, as `PermissionKind` in the UI.
const KINDS: [&str; 9] = [
    "camera",
    "microphone",
    "geolocation",
    "notifications",
    "midi",
    "clipboard",
    "screen",
    "external",
    "other",
];

struct Pending {
    id: u32,
    tab_id: u32,
    window_id: u32,
    origin: String,
    kinds: Vec<&'static str>,
    incognito: bool,
    request: PermissionRequest,
}

thread_local! {
    static SAVED: RefCell<Decisions> = const { RefCell::new(BTreeMap::new()) };
    static PRIVATE: RefCell<Decisions> = const { RefCell::new(BTreeMap::new()) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> std::path::PathBuf {
    paths::user_path("permissions.json")
}

pub fn load() {
    if let Some(saved) = store::read_json::<Decisions>(&file()) {
        SAVED.with(|cell| cell.replace(saved));
    }
}

pub fn flush() {
    if SAVE_PENDING.with(|pending| pending.replace(false)) {
        let saved = SAVED.with(|cell| cell.borrow().clone());
        match serde_json::to_vec(&saved) {
            Ok(json) => {
                if let Err(error) = store::write_atomic(&file(), &json) {
                    eprintln!("[permissions] could not save: {error}");
                }
            }
            Err(error) => eprintln!("[permissions] could not serialize: {error}"),
        }
    }
}

fn changed() {
    if !SAVE_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_millis(500), flush);
    }
    refresh_notifications();
}

/// scheme://host[:port] of an address; empty for addresses without one.
pub fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.origin().ascii_serialization())
        .ok()
        .filter(|o| o != "null")
        .unwrap_or_default()
}

fn decisions<R>(incognito: bool, f: impl FnOnce(&mut Decisions) -> R) -> R {
    if incognito {
        PRIVATE.with(|cell| f(&mut cell.borrow_mut()))
    } else {
        SAVED.with(|cell| f(&mut cell.borrow_mut()))
    }
}

fn decision(incognito: bool, origin: &str, kind: &str) -> Option<bool> {
    decisions(incognito, |d| {
        d.get(origin).and_then(|kinds| kinds.get(kind)).copied()
    })
}

/// Private windows start with nothing remembered and forget when the last one closes.
pub fn forget_private() {
    PRIVATE.with(|cell| cell.borrow_mut().clear());
}

// ── Requests ──────────────────────────────────────────────────────────────────

/// Answers a tab's requests: from what was remembered, or with the bar above the page.
pub fn attach(tab: &Rc<Tab>, view: &WebView) {
    let weak = Rc::downgrade(tab);
    view.connect_permission_request(move |view, request| {
        match weak.upgrade() {
            Some(tab) => handle(&tab, view, request),
            None => request.deny(),
        }
        true
    });
    view.connect_show_notification(|_, notification| {
        let title = notification
            .title()
            .map(|t| t.to_string())
            .unwrap_or_default();
        let body = notification
            .body()
            .map(|b| b.to_string())
            .unwrap_or_default();
        notify::show(&title, &body);
        true
    });
}

fn handle(tab: &Rc<Tab>, view: &WebView, request: &PermissionRequest) {
    // Locking the pointer (games, maps) needs no question; leaving it takes Esc.
    if request
        .dynamic_cast_ref::<PointerLockPermissionRequest>()
        .is_some()
    {
        request.allow();
        return;
    }
    let Some(win) = tab.window() else {
        request.deny();
        return;
    };
    let origin = origin_of(&view.uri().map(|u| u.to_string()).unwrap_or_default());
    if origin.is_empty() {
        request.deny();
        return;
    }
    let incognito = win.incognito;
    // Device names, for sites already allowed the camera or the microphone.
    if request
        .dynamic_cast_ref::<DeviceInfoPermissionRequest>()
        .is_some()
    {
        let allowed = ["camera", "microphone"]
            .iter()
            .any(|kind| decision(incognito, &origin, kind) == Some(true));
        if allowed {
            request.allow()
        } else {
            request.deny()
        }
        return;
    }
    let kinds: Vec<&'static str> = if request
        .dynamic_cast_ref::<GeolocationPermissionRequest>()
        .is_some()
    {
        vec!["geolocation"]
    } else if let Some(media) = request.dynamic_cast_ref::<UserMediaPermissionRequest>() {
        let mut kinds = Vec::new();
        if media.is_for_video_device() {
            kinds.push("camera");
        }
        if media.is_for_audio_device() {
            kinds.push("microphone");
        }
        kinds
    } else if request
        .dynamic_cast_ref::<NotificationPermissionRequest>()
        .is_some()
    {
        vec!["notifications"]
    } else {
        Vec::new()
    };
    // Screen sharing, codec installs, DRM and anything unknown are turned down.
    if kinds.is_empty() {
        request.deny();
        return;
    }
    if kinds
        .iter()
        .any(|kind| decision(incognito, &origin, kind) == Some(false))
    {
        request.deny();
        return;
    }
    if kinds
        .iter()
        .all(|kind| decision(incognito, &origin, kind) == Some(true))
    {
        request.allow();
        return;
    }
    let id = NEXT_ID.with(|next| next.replace(next.get() + 1));
    PENDING.with(|pending| {
        pending.borrow_mut().push(Pending {
            id,
            tab_id: tab.id,
            window_id: win.id,
            origin,
            kinds,
            incognito,
            request: request.clone(),
        })
    });
    win.mark_dirty();
}

/// The question to show for the window's active tab, if any.
pub fn prompt_for(win: &BrowserWindow) -> Option<Value> {
    let active = win.active_tab()?.id;
    PENDING.with(|pending| {
        pending
            .borrow()
            .iter()
            .find(|p| p.tab_id == active && p.window_id == win.id)
            .map(|p| json!({ "id": p.id, "tabId": p.tab_id, "origin": p.origin, "kinds": p.kinds }))
    })
}

pub fn respond(id: u32, allow: bool, remember: bool) {
    let Some(entry) = PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let index = pending.iter().position(|p| p.id == id)?;
        Some(pending.remove(index))
    }) else {
        return;
    };
    if remember {
        decisions(entry.incognito, |d| {
            let kinds = d.entry(entry.origin.clone()).or_default();
            for kind in &entry.kinds {
                kinds.insert((*kind).to_string(), allow);
            }
        });
        if !entry.incognito {
            changed();
        }
    }
    if allow {
        entry.request.allow()
    } else {
        entry.request.deny()
    }
    if let Some(win) = window::all().into_iter().find(|w| w.id == entry.window_id) {
        win.mark_dirty();
    }
}

/// A tab that leaves its page or closes takes its questions with it.
pub fn deny_pending_for(tab_id: u32) {
    let gone: Vec<Pending> = PENDING.with(|pending| {
        let mut pending = pending.borrow_mut();
        let (gone, kept) = pending.drain(..).partition(|p| p.tab_id == tab_id);
        *pending = kept;
        gone
    });
    for entry in &gone {
        entry.request.deny();
    }
    if let Some(entry) = gone.first()
        && let Some(win) = window::all().into_iter().find(|w| w.id == entry.window_id)
    {
        win.mark_dirty();
    }
}

// ── Remembered answers ────────────────────────────────────────────────────────

pub fn list() -> Value {
    let saved = SAVED.with(|cell| cell.borrow().clone());
    let rows: Vec<Value> = saved
        .iter()
        .flat_map(|(origin, kinds)| {
            kinds
                .iter()
                .map(move |(kind, allow)| json!({ "origin": origin, "kind": kind, "allow": allow }))
        })
        .collect();
    Value::Array(rows)
}

/// Sets one remembered answer for a site; `None` forgets it, so the site asks again.
pub fn set(origin: &str, kind: &str, allow: Option<bool>) {
    if !KINDS.contains(&kind) || origin_of(origin) != origin {
        return;
    }
    SAVED.with(|cell| {
        let mut saved = cell.borrow_mut();
        let kinds = saved.entry(origin.to_string()).or_default();
        match allow {
            Some(allow) => {
                kinds.insert(kind.to_string(), allow);
            }
            None => {
                kinds.remove(kind);
            }
        }
        if kinds.is_empty() {
            saved.remove(origin);
        }
    });
    changed();
}

/// The remembered answers for one site, for the site panel.
pub fn for_site(origin: &str, incognito: bool) -> Vec<Value> {
    decisions(incognito, |d| {
        d.get(origin)
            .map(|kinds| {
                kinds
                    .iter()
                    .map(|(kind, allow)| json!({ "origin": origin, "kind": kind, "allow": allow }))
                    .collect()
            })
            .unwrap_or_default()
    })
}

// ── Notifications ─────────────────────────────────────────────────────────────

/// Tells a browsing context which sites may show notifications, so `Notification.permission`
/// is right before a page asks.
pub fn init_notifications(context: &WebContext, incognito: bool) {
    let (mut allowed, mut denied) = (Vec::new(), Vec::new());
    decisions(incognito, |d| {
        for (origin, kinds) in d.iter() {
            match kinds.get("notifications") {
                Some(true) => allowed.push(SecurityOrigin::for_uri(origin)),
                Some(false) => denied.push(SecurityOrigin::for_uri(origin)),
                None => {}
            }
        }
    });
    let allowed: Vec<&SecurityOrigin> = allowed.iter().collect();
    let denied: Vec<&SecurityOrigin> = denied.iter().collect();
    context.initialize_notification_permissions(&allowed, &denied);
}

fn refresh_notifications() {
    for incognito in [false, true] {
        if let Some(context) = window::web_context(incognito) {
            init_notifications(&context, incognito);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    #[test]
    fn reads_origins() {
        assert_eq!(
            origin_of("https://maps.example.com/place?q=1"),
            "https://maps.example.com"
        );
        assert_eq!(origin_of("http://localhost:8080/"), "http://localhost:8080");
        assert_eq!(origin_of("about:blank"), "");
        assert_eq!(origin_of("not a url"), "");
    }
}
