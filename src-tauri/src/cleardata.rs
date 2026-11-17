//! Clearing browsing data: from the dialog for a time range, and every time the browser closes
//! when Settings asks for it. Cookies, site data and the cache go for all time whatever the
//! range, as in the Electron version: WebKit can't remove cookies by age.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::glib::translate::{IntoGlib, ToGlibPtr};
use serde::Serialize;
use serde_json::{Value, json};
use webkit2gtk::{WebsiteDataManager, WebsiteDataManagerExt, WebsiteDataTypes};

use crate::settings::{self, CLEAR_KINDS, ClearKind};
use crate::tab::now_ms;
use crate::{bookmarks, downloads, history, menus, permissions, window};

/// The dialog's time ranges in hours; 0 is all time.
const RANGES: [u64; 5] = [1, 24, 168, 672, 0];

/// How long closing the browser waits for WebKit at most.
const EXIT_LIMIT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Running,
    Done,
}

thread_local! {
    /// Clearing as the browser closes: not started, under way, or done.
    static EXIT: Cell<Stage> = const { Cell::new(Stage::Idle) };
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// Hours back; 0 for all time.
    pub range: u64,
    pub kinds: Vec<ClearKind>,
}

/// What the dialog shows next to each kind.
#[derive(Default, Serialize)]
struct Counts {
    history: i64,
    downloads: usize,
    /// Sites keeping cookies or other data.
    cookies: usize,
    permissions: usize,
    forms: usize,
    passwords: usize,
}

/// A range from the UI: one of the dialog's, or all time.
pub fn range(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .filter(|range| RANGES.contains(range))
        .unwrap_or(0)
}

/// A request from the UI, with the kinds it names in their usual order.
pub fn request(value: Option<&Value>) -> Request {
    let named: Vec<ClearKind> = value
        .and_then(|v| v.get("kinds"))
        .and_then(Value::as_array)
        .map(|kinds| {
            kinds
                .iter()
                .filter_map(|kind| serde_json::from_value(kind.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    Request {
        range: range(value.and_then(|v| v.get("range"))),
        kinds: CLEAR_KINDS
            .into_iter()
            .filter(|kind| named.contains(kind))
            .collect(),
    }
}

/// Where a range starts, in ms since 1970; 0 for all time.
fn since(range: u64, now: u64) -> u64 {
    if range == 0 {
        0
    } else {
        now.saturating_sub(range * 3_600_000)
    }
}

/// What sites store for themselves: cookies, storage, databases, service workers and their
/// caches. The dialog counts the sites keeping any of it.
fn stored() -> WebsiteDataTypes {
    WebsiteDataTypes::COOKIES
        | WebsiteDataTypes::LOCAL_STORAGE
        | WebsiteDataTypes::SESSION_STORAGE
        | WebsiteDataTypes::INDEXEDDB_DATABASES
        | WebsiteDataTypes::WEBSQL_DATABASES
        | WebsiteDataTypes::OFFLINE_APPLICATION_CACHE
        | WebsiteDataTypes::SERVICE_WORKER_REGISTRATIONS
        | WebsiteDataTypes::DOM_CACHE
}

fn cache() -> WebsiteDataTypes {
    WebsiteDataTypes::MEMORY_CACHE | WebsiteDataTypes::DISK_CACHE
}

/// "Cookies and site data": all WebKit keeps for sites but the cache, and the list of sites
/// that asked to be reached over HTTPS only (HSTS), which keeps them safe.
fn site_data() -> WebsiteDataTypes {
    WebsiteDataTypes::all().difference(cache() | WebsiteDataTypes::HSTS_CACHE)
}

/// The dialog's counts for a range. WebKit tells the number of sites with data a moment later.
pub fn counts(range: u64, done: impl FnOnce(Value) + 'static) {
    let since = since(range, now_ms());
    let mut counts = Counts {
        history: history::count_visits(since as i64),
        downloads: downloads::count_since(since),
        permissions: permissions::count(),
        passwords: crate::passwords::count(),
        ..Counts::default()
    };
    let Some(manager) = window::browsing_data() else {
        return done(serde_json::to_value(&counts).unwrap_or_default());
    };
    manager.fetch(stored(), None::<&gio::Cancellable>, move |result| {
        counts.cookies = result.map(|sites| sites.len()).unwrap_or(0);
        done(serde_json::to_value(&counts).unwrap_or_default());
    });
}

/// "Clear data" in the dialog; `done` follows once WebKit is done too.
pub fn clear_request(request: Request, done: impl FnOnce(Value) + 'static) {
    let since = since(request.range, now_ms());
    clear(since, &request.kinds, move || {
        menus::history_changed();
        done(json!(true));
    });
}

/// Clears what `kinds` name since `since` (ms; 0 for all time), then `done` once WebKit is done.
fn clear(since: u64, kinds: &[ClearKind], done: impl FnOnce() + 'static) {
    let has = |kind| kinds.contains(&kind);
    if has(ClearKind::History) {
        // Bookmarked sites keep their icons.
        history::delete_since(since as i64, &bookmarks::bookmarked_hosts());
        if since == 0 {
            window::clear_page_icons();
            window::favicons_changed();
        }
    }
    if has(ClearKind::Downloads) {
        downloads::clear(since);
    }
    if has(ClearKind::Permissions) {
        permissions::clear();
    }
    // Saved passwords go whatever the range, as in 0.1.x; form entries aren't kept yet.
    if has(ClearKind::Passwords) {
        crate::passwords::clear();
    }
    let mut types = WebsiteDataTypes::empty();
    if has(ClearKind::Cookies) {
        types |= site_data();
    }
    if has(ClearKind::Cache) {
        types |= cache();
    }
    match window::browsing_data() {
        Some(manager) if !types.is_empty() => clear_website_data(&manager, types, move |ok| {
            if !ok {
                crate::log_error!("[cleardata] WebKit could not clear the site data");
            }
            done();
        }),
        _ => done(),
    }
}

/// Clears all data of the given types through WebKit's C API, which the Rust bindings leave
/// out; `done` gets whether it worked.
fn clear_website_data(
    manager: &WebsiteDataManager,
    types: WebsiteDataTypes,
    done: impl FnOnce(bool) + 'static,
) {
    type Done = Box<dyn FnOnce(bool)>;
    unsafe extern "C" fn finished(
        source: *mut glib::gobject_ffi::GObject,
        result: *mut gio::ffi::GAsyncResult,
        user_data: glib::ffi::gpointer,
    ) {
        unsafe {
            let done: Box<Done> = Box::from_raw(user_data as *mut Done);
            let mut error = std::ptr::null_mut();
            let ok = webkit2gtk::ffi::webkit_website_data_manager_clear_finish(
                source as *mut _,
                result,
                &mut error,
            ) != 0;
            if !error.is_null() {
                glib::ffi::g_error_free(error);
            }
            done(ok);
        }
    }
    let done: Box<Done> = Box::new(Box::new(done));
    unsafe {
        // A timespan of 0: whenever it was stored.
        webkit2gtk::ffi::webkit_website_data_manager_clear(
            manager.to_glib_none().0,
            types.into_glib(),
            0,
            std::ptr::null_mut(),
            Some(finished),
            Box::into_raw(done) as glib::ffi::gpointer,
        );
    }
}

// ── When the browser closes ───────────────────────────────────────────────────

/// What Settings asks to clear when the browser closes. Saved passwords never are, whatever a
/// hand-edited settings file says.
fn exit_kinds() -> Vec<ClearKind> {
    settings::get()
        .clear_on_exit
        .into_iter()
        .filter(|kind| *kind != ClearKind::Passwords)
        .collect()
}

/// Whether quitting has to wait for the "clear when the browser closes" choices.
pub fn exit_waits() -> bool {
    match EXIT.get() {
        Stage::Idle => !exit_kinds().is_empty(),
        Stage::Running => true,
        Stage::Done => false,
    }
}

/// Runs the "clear when the browser closes" choices once, with the windows hidden, then
/// `then`: right away when there's nothing to clear, otherwise once WebKit is done or four
/// seconds have passed. While a run is under way `then` is dropped, as that run's own follows.
pub fn before_exit(then: impl FnOnce() + 'static) {
    match EXIT.get() {
        Stage::Running => return,
        Stage::Done => return then(),
        Stage::Idle => {}
    }
    let kinds = exit_kinds();
    if kinds.is_empty() {
        EXIT.set(Stage::Done);
        return then();
    }
    EXIT.set(Stage::Running);
    window::hide_all();
    let then = Rc::new(RefCell::new(Some(then)));
    let finish = move || {
        let next = then.borrow_mut().take();
        if let Some(next) = next {
            EXIT.set(Stage::Done);
            // A restart skips the usual saving on exit.
            downloads::flush();
            permissions::flush();
            next();
        }
    };
    glib::timeout_add_local_once(EXIT_LIMIT, finish.clone());
    clear(0, &kinds, finish);
}

/// At startup, before any page opens: a last run that didn't close properly (it was killed,
/// or the computer shut down) left what Settings asks to clear when the browser closes. After
/// a normal close nothing is left, and this costs little.
pub fn at_startup() {
    let kinds = exit_kinds();
    if !kinds.is_empty() {
        clear(0, &kinds, || {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_requests_from_the_ui() {
        let input = json!({ "range": 24, "kinds": ["cache", "history", "bogus", 3, "history"] });
        assert_eq!(
            request(Some(&input)),
            Request {
                range: 24,
                kinds: vec![ClearKind::History, ClearKind::Cache],
            }
        );
        let odd = json!({ "range": 5, "kinds": "cookies" });
        let nothing = Request {
            range: 0,
            kinds: Vec::new(),
        };
        assert_eq!(request(Some(&odd)), nothing);
        assert_eq!(request(None), nothing);
        assert_eq!(range(Some(&json!(-1))), 0);
        assert_eq!(range(Some(&json!(672))), 672);
    }

    #[test]
    fn ranges_count_back_from_now() {
        assert_eq!(since(0, 10_000_000), 0);
        assert_eq!(since(1, 10_000_000), 6_400_000);
        assert_eq!(since(672, 1_000), 0);
    }

    #[test]
    fn site_data_leaves_the_cache_and_hsts_alone() {
        assert!(site_data().contains(stored()));
        assert!(!site_data().intersects(cache() | WebsiteDataTypes::HSTS_CACHE));
        assert!(site_data().contains(WebsiteDataTypes::ITP));
    }
}
