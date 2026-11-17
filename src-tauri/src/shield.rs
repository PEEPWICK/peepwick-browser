//! The privacy shield: Brave's adblock-rust engine turns the filter lists into WebKit content
//! rules, which WebKit compiles once, keeps on disk and applies in every page's process. There
//! are two sets: the standard one (ads and trackers), and what the strict level adds (cookie
//! notices and other annoyances), whose lists are fetched only once someone chooses that level.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use adblock::lists::{FilterSet, ParseOptions};
use webkit2gtk::glib;
use webkit2gtk::{UserContentManager, UserContentManagerExt};

use crate::settings::{self, ShieldLevel};
use crate::{pages, paths};

const REFRESH_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);

/// A set of rules: the lists it is made from (each kept as shield-<name>.txt) and its name in
/// WebKit's store.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Set {
    /// Ads and trackers, at the standard and the strict level.
    Standard,
    /// What the strict level adds: cookie notices and other annoyances.
    Strict,
}

impl Set {
    fn lists(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Set::Standard => &[
                ("easylist", "https://easylist.to/easylist/easylist.txt"),
                (
                    "easyprivacy",
                    "https://easylist.to/easylist/easyprivacy.txt",
                ),
            ],
            Set::Strict => &[
                (
                    "cookies",
                    "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt",
                ),
                (
                    "annoyances",
                    "https://easylist.to/easylist/fanboy-annoyance.txt",
                ),
            ],
        }
    }

    fn id(self) -> &'static str {
        match self {
            Set::Standard => "peepwick-shield-standard",
            Set::Strict => "peepwick-shield-strict",
        }
    }

    fn slot(self) -> &'static std::thread::LocalKey<RefCell<Option<ffi::ContentFilter>>> {
        match self {
            Set::Standard => &STANDARD,
            Set::Strict => &STRICT,
        }
    }
}

thread_local! {
    static STANDARD: RefCell<Option<ffi::ContentFilter>> = const { RefCell::new(None) };
    static STRICT: RefCell<Option<ffi::ContentFilter>> = const { RefCell::new(None) };
    /// Sets being fetched and compiled now: each is built once at a time.
    static BUILDING: RefCell<Vec<Set>> = const { RefCell::new(Vec::new()) };
    /// The level the tabs' rules follow.
    static LEVEL: Cell<Option<ShieldLevel>> = const { Cell::new(None) };
    /// Whether the strict set was loaded, or asked for, this session.
    static STRICT_ASKED: Cell<bool> = const { Cell::new(false) };
    static MANAGERS: RefCell<Vec<glib::WeakRef<UserContentManager>>> = const { RefCell::new(Vec::new()) };
    /// Tabs showing a site with the shield off: their pages load without the rules.
    static PAUSED: RefCell<Vec<glib::WeakRef<UserContentManager>>> = const { RefCell::new(Vec::new()) };
    static STORE: ffi::FilterStore = ffi::FilterStore::new(&paths::user_path("shield-rules"));
}

fn level() -> ShieldLevel {
    settings::get().shield_level
}

/// Puts on a content manager the rules its pages should have: the standard set, and at the
/// strict level what that adds.
fn add_filters(manager: &UserContentManager) {
    let add = |set: Set| {
        set.slot().with(|filter| {
            if let Some(filter) = filter.borrow().as_ref() {
                filter.add_to(manager);
            }
        })
    };
    add(Set::Standard);
    if level() == ShieldLevel::Strict {
        add(Set::Strict);
    }
}

/// Every tab that isn't paused gets its rules again: a set was compiled, or the level changed.
/// Pages loaded already keep the rules they had until they load again.
fn reapply() {
    MANAGERS.with(|managers| {
        for manager in managers.borrow().iter().filter_map(|m| m.upgrade()) {
            if !is_paused(&manager) {
                manager.remove_all_filters();
                add_filters(&manager);
            }
        }
    });
}

/// Gives a tab's content manager the rules, now or once they are ready.
pub fn attach(manager: &UserContentManager) {
    use glib::prelude::*;
    MANAGERS.with(|managers| {
        let mut managers = managers.borrow_mut();
        managers.retain(|m| m.upgrade().is_some());
        managers.push(manager.downgrade());
    });
    add_filters(manager);
}

pub(crate) fn is_paused(manager: &UserContentManager) -> bool {
    PAUSED.with(|paused| {
        paused
            .borrow()
            .iter()
            .any(|m| m.upgrade().as_ref() == Some(manager))
    })
}

/// Takes the rules off a tab's pages, or puts them back; a tab follows the site it navigates to.
pub fn set_paused(manager: &UserContentManager, paused: bool) {
    use glib::prelude::*;
    if is_paused(manager) == paused {
        return;
    }
    PAUSED.with(|list| {
        let mut list = list.borrow_mut();
        list.retain(|m| m.upgrade().is_some_and(|m| &m != manager));
        if paused {
            list.push(manager.downgrade());
        }
    });
    manager.remove_all_filters();
    if !paused {
        add_filters(manager);
    }
}

/// Whether the shield is off for an address: everywhere, or for its site.
pub fn paused_for(url: &str) -> bool {
    let prefs = settings::get();
    if prefs.shield_level == settings::ShieldLevel::Off {
        return true;
    }
    let host = pages::host_of(url);
    !host.is_empty()
        && prefs
            .shield_allowlist
            .iter()
            .any(|site| host == *site || host.ends_with(&format!(".{site}")))
}

/// The site an address belongs to (example.com for www.example.com), for the shield's list.
pub fn site_of(url: &str) -> Option<String> {
    let host = pages::host_of(url);
    if host.is_empty() {
        return None;
    }
    // Addresses and names without a public suffix (localhost, a printer) are their own site.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() || !host.contains('.') {
        return Some(host);
    }
    Some(psl::domain_str(&host).unwrap_or(&host).to_string())
}

fn list_path(name: &str) -> PathBuf {
    paths::user_path(&format!("shield-{name}.txt"))
}

fn is_fresh(path: &PathBuf) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < REFRESH_AFTER)
}

/// Loads the compiled rules at once and refreshes the lists in the background when they are old
/// (or the first time, when there are none yet); the strict level's set only at that level.
pub fn start() {
    let level = level();
    LEVEL.set(Some(level));
    load(Set::Standard);
    if level == ShieldLevel::Strict {
        STRICT_ASKED.set(true);
        load(Set::Strict);
    }
}

/// Settings changed: when the level did, the strict set comes the first time it's needed, and
/// the tabs get the rules for the new level.
pub fn settings_changed() {
    let level = level();
    if LEVEL.replace(Some(level)) == Some(level) {
        return;
    }
    if level == ShieldLevel::Strict && !STRICT_ASKED.replace(true) {
        load(Set::Strict);
    }
    reapply();
}

/// A set's compiled rules from WebKit's store; built when they aren't there or their lists are old.
fn load(set: Set) {
    let stale = set
        .lists()
        .iter()
        .any(|(name, _)| !is_fresh(&list_path(name)));
    let started = Instant::now();
    STORE.with(|store| {
        store.load(set.id(), move |result| match result {
            Ok(filter) => {
                eprintln!("[shield] {set:?} rules loaded in {:?}", started.elapsed());
                set.slot().with(|slot| slot.replace(Some(filter)));
                reapply();
                if stale {
                    build(set);
                }
            }
            Err(_) => {
                eprintln!("[shield] no compiled {set:?} rules yet");
                build(set);
            }
        })
    });
}

/// Fetches a set's lists (an old copy when offline), turns them into WebKit rules off the main
/// thread, has WebKit compile them and gives them to the tabs.
fn build(set: Set) {
    if BUILDING.with(|building| building.borrow().contains(&set)) {
        return;
    }
    BUILDING.with(|building| building.borrow_mut().push(set));
    std::thread::spawn(move || {
        let converted = download_lists(set.lists()).and_then(convert);
        glib::MainContext::default().invoke(move || {
            BUILDING.with(|building| building.borrow_mut().retain(|s| *s != set));
            let (json, count) = match converted {
                Ok(converted) => converted,
                Err(error) => {
                    crate::log_error!("[shield] {set:?} rules: {error}");
                    return;
                }
            };
            let compiling = Instant::now();
            STORE.with(|store| {
                store.save(set.id(), json, move |result| match result {
                    Ok(filter) => {
                        eprintln!(
                            "[shield] WebKit compiled {count} {set:?} rules in {:?}",
                            compiling.elapsed()
                        );
                        set.slot().with(|slot| slot.replace(Some(filter)));
                        reapply();
                    }
                    Err(error) => {
                        crate::log_error!(
                            "[shield] WebKit could not compile the {set:?} rules: {error}"
                        )
                    }
                })
            });
        });
    });
}

/// Filter lists as WebKit content rules (JSON), and how many rules.
fn convert(lists: Vec<String>) -> Result<(String, usize), String> {
    let converting = Instant::now();
    let mut set = FilterSet::new(true);
    for text in lists {
        set.add_filter_list(text, ParseOptions::default());
    }
    let (rules, _) = set
        .into_content_blocking()
        .map_err(|_| "could not convert the lists".to_string())?;
    let count = rules.len();
    let json = serde_json::to_string(&rules).map_err(|e| e.to_string())?;
    eprintln!(
        "[shield] {count} rules ({} KB) converted in {:?}",
        json.len() / 1024,
        converting.elapsed()
    );
    Ok((json, count))
}

fn download_lists(sources: &[(&str, &str)]) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("PEEPWICK-Browser/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut lists = Vec::new();
    for &(name, url) in sources {
        let path = list_path(name);
        let text = match client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
        {
            Ok(text) if text.starts_with("[Adblock") || text.contains("! Title:") => {
                let _ = crate::store::write_atomic(&path, text.as_bytes());
                text
            }
            // Offline or a broken download: an old list is better than none.
            _ => std::fs::read_to_string(&path).map_err(|_| format!("{name} unavailable"))?,
        };
        lists.push(text);
    }
    Ok(lists)
}

/// WebKitUserContentFilterStore and WebKitUserContentFilter, which the Rust bindings leave out.
mod ffi {
    use std::ffi::CString;
    use std::path::Path;
    use std::ptr::{self, NonNull};

    use webkit2gtk::UserContentManager;
    use webkit2gtk::ffi as wk;
    use webkit2gtk::gio::ffi as gio;
    use webkit2gtk::glib::translate::{FromGlibPtrFull, ToGlibPtr};
    use webkit2gtk::glib::{self, ffi as glib_ffi, gobject_ffi};

    pub struct ContentFilter(NonNull<wk::WebKitUserContentFilter>);

    impl ContentFilter {
        pub fn add_to(&self, manager: &UserContentManager) {
            unsafe {
                wk::webkit_user_content_manager_add_filter(
                    manager.to_glib_none().0,
                    self.0.as_ptr(),
                )
            }
        }
    }

    impl Drop for ContentFilter {
        fn drop(&mut self) {
            unsafe { wk::webkit_user_content_filter_unref(self.0.as_ptr()) }
        }
    }

    pub struct FilterStore(NonNull<wk::WebKitUserContentFilterStore>);

    type Done = Box<dyn FnOnce(Result<ContentFilter, String>)>;

    impl FilterStore {
        pub fn new(path: &Path) -> FilterStore {
            let path =
                CString::new(path.to_string_lossy().as_bytes()).expect("profile path without NUL");
            let store = unsafe { wk::webkit_user_content_filter_store_new(path.as_ptr()) };
            FilterStore(NonNull::new(store).expect("WebKit made a filter store"))
        }

        /// Compiles rules (Safari content blocker JSON) and keeps them under `id`.
        pub fn save(
            &self,
            id: &str,
            json: String,
            done: impl FnOnce(Result<ContentFilter, String>) + 'static,
        ) {
            let id = CString::new(id).expect("id without NUL");
            let bytes = glib::Bytes::from_owned(json.into_bytes());
            let done: Box<Done> = Box::new(Box::new(done));
            unsafe {
                wk::webkit_user_content_filter_store_save(
                    self.0.as_ptr(),
                    id.as_ptr(),
                    bytes.to_glib_none().0,
                    ptr::null_mut(),
                    Some(saved),
                    Box::into_raw(done).cast(),
                );
            }
        }

        /// Loads rules compiled earlier.
        pub fn load(&self, id: &str, done: impl FnOnce(Result<ContentFilter, String>) + 'static) {
            let id = CString::new(id).expect("id without NUL");
            let done: Box<Done> = Box::new(Box::new(done));
            unsafe {
                wk::webkit_user_content_filter_store_load(
                    self.0.as_ptr(),
                    id.as_ptr(),
                    ptr::null_mut(),
                    Some(loaded),
                    Box::into_raw(done).cast(),
                );
            }
        }
    }

    impl Drop for FilterStore {
        fn drop(&mut self) {
            unsafe { gobject_ffi::g_object_unref(self.0.as_ptr().cast()) }
        }
    }

    unsafe fn finish(
        result: Result<*mut wk::WebKitUserContentFilter, *mut glib_ffi::GError>,
        data: glib_ffi::gpointer,
    ) {
        let done: Box<Done> = unsafe { Box::from_raw(data.cast()) };
        match result {
            Ok(filter) => match NonNull::new(filter) {
                Some(filter) => done(Ok(ContentFilter(filter))),
                None => done(Err("WebKit returned no filter".into())),
            },
            Err(error) => {
                let error = unsafe { glib::Error::from_glib_full(error) };
                done(Err(error.message().to_string()))
            }
        }
    }

    unsafe extern "C" fn saved(
        source: *mut gobject_ffi::GObject,
        result: *mut gio::GAsyncResult,
        data: glib_ffi::gpointer,
    ) {
        let mut error = ptr::null_mut();
        let filter = unsafe {
            wk::webkit_user_content_filter_store_save_finish(source.cast(), result, &mut error)
        };
        unsafe {
            finish(
                if error.is_null() {
                    Ok(filter)
                } else {
                    Err(error)
                },
                data,
            )
        }
    }

    unsafe extern "C" fn loaded(
        source: *mut gobject_ffi::GObject,
        result: *mut gio::GAsyncResult,
        data: glib_ffi::gpointer,
    ) {
        let mut error = ptr::null_mut();
        let filter = unsafe {
            wk::webkit_user_content_filter_store_load_finish(source.cast(), result, &mut error)
        };
        unsafe {
            finish(
                if error.is_null() {
                    Ok(filter)
                } else {
                    Err(error)
                },
                data,
            )
        }
    }
}

#[cfg(test)]
mod site_tests {
    use super::site_of;

    #[test]
    fn finds_sites() {
        assert_eq!(
            site_of("https://www.example.co.uk/a").as_deref(),
            Some("example.co.uk")
        );
        assert_eq!(
            site_of("https://news.example.com/").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            site_of("http://127.0.0.1:8767/test.html").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            site_of("http://localhost:3000/").as_deref(),
            Some("localhost")
        );
        assert_eq!(site_of("about:blank"), None);
    }

    #[test]
    fn turns_lists_into_webkit_rules() {
        let list = "[Adblock Plus 2.0]\n! Title: Test\n||ads.example^\n##.cookie-banner\nnews.example##.newsletter-popup\n";
        let (json, count) = super::convert(vec![list.to_string()]).unwrap();
        let rules: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules.len(), count);
        let action = |kind: &str, needle: &str| {
            rules
                .iter()
                .any(|rule| rule["action"]["type"] == kind && rule.to_string().contains(needle))
        };
        assert!(action("block", "ads"));
        assert!(action("css-display-none", ".cookie-banner"));
        assert!(action("css-display-none", ".newsletter-popup"));
    }
}
