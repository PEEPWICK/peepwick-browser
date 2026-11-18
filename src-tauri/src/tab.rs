//! One tab: at most one WebKitGTK view. Built-in pages (new tab, settings…), error screens
//! and warnings are drawn by the UI instead, and sleeping tabs have no view until they are
//! shown again.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use gtk::glib;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use webkit2gtk::{
    BackForwardListExt, FindControllerExt, LoadEvent, NavigationPolicyDecision,
    NavigationPolicyDecisionExt, NetworkError, PolicyDecision, PolicyDecisionExt,
    PolicyDecisionType, PolicyError, ResponsePolicyDecision, ResponsePolicyDecisionExt,
    ScriptDialogType, URIRequestExt, URIResponseExt, UserMessage, UserMessageExt,
    WebProcessTerminationReason, WebView, WebViewExt, WebViewSessionState,
};

use crate::i18n::t;
use crate::reader::{self, Article, Reader};
use crate::state::TabState;
use crate::window::{BrowserWindow, OpenOptions};
use crate::{
    focus, history, menus, pages, readinglist, safebrowsing, security, settings, shortcuts, zoom,
};

/// What a closed or restored tab needs to come back.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub muted: bool,
    /// WebKit's back/forward history, base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FindState {
    pub text: String,
    pub matches: u32,
    pub active: u32,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn page_title(page: &str) -> String {
    let key = match page {
        "newtab" => "newtab.title",
        "settings" => "settings.title",
        "history" => "history.title",
        "extensions" => "ext.title",
        "bookmarks" => "bookmarks.title",
        "downloads" => "downloads.title",
        "passwords" => "passwords.title",
        _ => "reading.title",
    };
    t(key)
}

pub struct Tab {
    pub id: u32,
    win: Weak<BrowserWindow>,
    view: RefCell<Option<WebView>>,
    url: RefCell<String>,
    title: RefCell<String>,
    favicon: RefCell<Option<String>>,
    /// Set while the UI draws a built-in page instead of the view.
    page: Cell<Option<&'static str>>,
    pub pinned: Cell<bool>,
    muted: Cell<bool>,
    /// An error, crash or warning screen the UI draws in place of the page.
    interstitial: RefCell<Option<Value>>,
    find: RefCell<Option<FindState>>,
    /// Requests the shield stopped on the current page, and from which hosts.
    pub blocked: Cell<u32>,
    blocked_hosts: RefCell<HashMap<String, u32>>,
    /// The page's language, when it could be translated (translation.rs), and the translate bar.
    pub(crate) page_lang: RefCell<Option<String>>,
    pub(crate) translation: RefCell<Option<crate::translation::TranslationState>>,
    /// Bumped whenever a translation should stop: a new one, a new page, the bar closing.
    pub(crate) translation_run: Cell<u32>,
    pub last_active: Cell<u64>,
    pub opener_id: Cell<Option<u32>>,
    /// History to restore when the view is made: a tab restored from the last session, or
    /// one that slept.
    saved_history: RefCell<Option<Vec<u8>>>,
    /// The certificate of the last load that failed with a certificate problem.
    tls_failure: RefCell<Option<(gtk::gio::TlsCertificate, gtk::gio::TlsCertificateFlags)>>,
    /// Reader view over the page, while it's open.
    reader: RefCell<Option<Reader>>,
    /// Whether the page has an article for reader view.
    reader_available: Cell<bool>,
    /// Moves on with every page and every reader view, so a late answer about an older one
    /// is dropped.
    reader_token: Cell<u32>,
    /// The address the page is loading, until it commits: WebKit commits a blank page in place
    /// of an address it won't load.
    loading: RefCell<Option<String>>,
    /// The web process the page lives in, for the task manager.
    pid: Cell<Option<i32>>,
    /// Video or audio in the page: playing, or paused after playing. The web process tells.
    media: Cell<Option<&'static str>>,
}

impl Tab {
    pub fn new(id: u32, win: Weak<BrowserWindow>, saved: SavedTab) -> Rc<Tab> {
        let history = saved
            .history
            .as_deref()
            .and_then(|h| base64::engine::general_purpose::STANDARD.decode(h).ok());
        let tab = Rc::new(Tab {
            id,
            win,
            view: RefCell::new(None),
            url: RefCell::new(String::new()),
            title: RefCell::new(saved.title),
            favicon: RefCell::new(None),
            page: Cell::new(None),
            pinned: Cell::new(saved.pinned),
            muted: Cell::new(saved.muted),
            interstitial: RefCell::new(None),
            find: RefCell::new(None),
            blocked: Cell::new(0),
            blocked_hosts: RefCell::new(HashMap::new()),
            page_lang: RefCell::new(None),
            translation: RefCell::new(None),
            translation_run: Cell::new(0),
            last_active: Cell::new(now_ms()),
            opener_id: Cell::new(None),
            saved_history: RefCell::new(history),
            tls_failure: RefCell::new(None),
            reader: RefCell::new(None),
            reader_available: Cell::new(false),
            reader_token: Cell::new(0),
            loading: RefCell::new(None),
            pid: Cell::new(None),
            media: Cell::new(None),
        });
        tab.set_address(if saved.url.is_empty() {
            pages::NEW_TAB_URL
        } else {
            &saved.url
        });
        tab
    }

    pub fn window(&self) -> Option<Rc<BrowserWindow>> {
        self.win.upgrade()
    }

    pub(crate) fn changed(&self) {
        if let Some(win) = self.window() {
            win.mark_dirty();
        }
    }

    fn relayout(&self) {
        if let Some(win) = self.window() {
            win.layout();
        }
    }

    pub fn view(&self) -> Option<WebView> {
        self.view.borrow().clone()
    }

    /// The view on screen: reader view while it's open, otherwise the page.
    pub fn shown_view(&self) -> Option<WebView> {
        self.reader
            .borrow()
            .as_ref()
            .map(|r| r.view.clone())
            .or_else(|| self.view())
    }

    pub fn page(&self) -> Option<&'static str> {
        self.page.get()
    }

    pub fn url(&self) -> String {
        self.url.borrow().clone()
    }

    pub fn favicon(&self) -> Option<String> {
        self.favicon.borrow().clone()
    }

    pub fn has_interstitial(&self) -> bool {
        self.interstitial.borrow().is_some()
    }

    pub fn sleeping(&self) -> bool {
        self.view.borrow().is_none()
            && self.page.get().is_none()
            && self.interstitial.borrow().is_none()
    }

    pub fn is_audible(&self) -> bool {
        self.view().is_some_and(|v| v.is_playing_audio())
    }

    /// Whether the page area shows this tab's view, not a built-in page or a warning.
    pub fn shows_web(&self) -> bool {
        self.page.get().is_none() && self.interstitial.borrow().is_none()
    }

    fn set_address(&self, url: &str) {
        *self.url.borrow_mut() = url.to_string();
        self.page.set(pages::internal_page(url));
    }

    // ── View lifecycle ────────────────────────────────────────────────────────

    /// Makes the view if needed, restoring saved history or loading the tab's address.
    pub fn ensure_view(self: &Rc<Self>) -> Option<WebView> {
        if let Some(view) = self.view() {
            return Some(view);
        }
        let win = self.window()?;
        let view = win.create_view(None);
        self.adopt(&win, view.clone());
        let history = self.saved_history.take();
        let restored = history.is_some_and(|bytes| {
            view.restore_session_state(&WebViewSessionState::new(&glib::Bytes::from_owned(bytes)));
            match view
                .back_forward_list()
                .and_then(|list| list.current_item())
            {
                Some(item) => {
                    view.go_to_back_forward_list_item(&item);
                    true
                }
                None => false,
            }
        });
        if !restored && self.shows_web() {
            let url = self.url();
            self.follow_shield(&url);
            view.load_uri(&url);
        }
        Some(view)
    }

    /// Takes a new view (also one a page opened with window.open) and wires its signals.
    pub fn adopt(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: WebView) {
        win.place_view(&view);
        view.set_is_muted(self.muted.get());
        self.connect(win, &view);
        self.view.replace(Some(view));
    }

    fn destroy_view(&self) {
        self.close_reader();
        crate::permissions::deny_pending_for(self.id);
        self.loading.replace(None);
        self.pid.set(None);
        self.media.set(None);
        if let Some(view) = self.view.take() {
            view.stop_loading();
            if let Some(parent) = view.parent() {
                parent
                    .downcast::<gtk::Container>()
                    .expect("views live in a container")
                    .remove(&view);
            }
        }
    }

    /// Frees the view of a background tab; it comes back from saved history when shown.
    pub fn sleep(&self) -> bool {
        let Some(view) = self.view() else {
            return false;
        };
        if view.is_playing_audio() || self.page.get().is_some() {
            return false;
        }
        self.saved_history.replace(capture_history(&view));
        self.destroy_view();
        self.find.replace(None);
        self.changed();
        true
    }

    pub fn destroy(&self) {
        self.destroy_view();
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    /// Loads an address; built-in pages are drawn by the UI.
    pub fn load(self: &Rc<Self>, url: &str) {
        self.close_reader();
        self.interstitial.replace(None);
        if let Some(page) = pages::internal_page(url) {
            self.show_page(page, &pages::internal_subpage(url));
            return;
        }
        self.set_address(url);
        if focus::should_gate(url) {
            // Nothing is asked of the site; whatever was loading stops too.
            self.stop();
            self.hold_for_focus(url);
            return;
        }
        if let Some(threat) = safebrowsing::threat_for(url) {
            self.stop();
            self.warn_dangerous(url, threat);
            return;
        }
        let had_view = self.view.borrow().is_some();
        if !had_view {
            // A sleeping tab's old history would be loaded instead of the new address.
            self.saved_history.replace(None);
        }
        // A new view loads the address itself; loading it twice would look like a redirect
        // loop to HTTPS-Only.
        if let Some(view) = self.ensure_view()
            && had_view
        {
            self.follow_shield(url);
            view.load_uri(url);
        }
        self.relayout();
        self.changed();
    }

    pub fn show_page(&self, page: &'static str, subpage: &str) {
        self.close_reader();
        // An HTTPS attempt still going would offer HTTP over this page.
        security::https_settled(self.id);
        self.set_address(&pages::internal_url(page, subpage));
        self.title.replace(String::new());
        self.favicon.replace(None);
        self.interstitial.replace(None);
        self.relayout();
        self.changed();
    }

    fn view_url(&self) -> Option<String> {
        self.view()
            .and_then(|v| v.uri())
            .map(|u| u.to_string())
            .filter(|u| !u.is_empty())
    }

    pub fn can_go_back(&self) -> bool {
        if self.page.get().is_some() && self.view_url().is_some() {
            return true;
        }
        if self.interstitial.borrow().is_some() {
            return true;
        }
        self.view().is_some_and(|v| v.can_go_back())
    }

    pub fn can_go_forward(&self) -> bool {
        self.page.get().is_none()
            && !self.holds_page()
            && self.view().is_some_and(|v| v.can_go_forward())
    }

    pub fn back(self: &Rc<Self>) {
        let view = self.view();
        // A built-in page opened over a web page: back shows the page again.
        if self.page.get().is_some()
            && let Some(url) = self.view_url()
        {
            self.set_address(&url);
            self.title.replace(
                view.as_ref()
                    .and_then(|v| v.title())
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
            );
            self.relayout();
            self.changed();
            return;
        }
        if let Some(warning) = self.interstitial.replace(None) {
            let held = warning["kind"] == "focus" || warning["kind"] == "dangerous";
            let under = self.view_url();
            match &view {
                // The held link was clicked on a page that is still there underneath.
                Some(view) if held && under.as_ref().is_some_and(|u| !is_held(u)) => {
                    if let Some(url) = &under {
                        self.set_address(url);
                        if self.favicon.borrow().is_none() {
                            self.favicon.replace(history::favicon_for_url(url));
                        }
                    }
                    self.title
                        .replace(view.title().map(|t| t.to_string()).unwrap_or_default());
                }
                Some(view) if view.can_go_back() => view.go_back(),
                Some(_) if !held && under.is_some() => {
                    if let Some(url) = &under {
                        self.set_address(url);
                    }
                }
                _ => {
                    self.show_page("newtab", "");
                    return;
                }
            }
            self.relayout();
            self.changed();
            return;
        }
        if let Some(view) = view
            && view.can_go_back()
        {
            view.go_back();
        }
    }

    pub fn forward(&self) {
        if let Some(view) = self.view()
            && view.can_go_forward()
        {
            view.go_forward();
        }
    }

    pub fn reload(self: &Rc<Self>, hard: bool) {
        if self.page.get().is_some() {
            return;
        }
        if self.interstitial.replace(None).is_some() {
            let url = self.url();
            self.load(&url);
            return;
        }
        let Some(view) = self.view() else {
            self.ensure_view();
            return;
        };
        if hard {
            view.reload_bypass_cache();
        } else {
            view.reload();
        }
    }

    pub fn stop(&self) {
        if let Some(view) = self.view() {
            view.stop_loading();
        }
    }

    // ── Page tools ────────────────────────────────────────────────────────────

    pub fn set_muted(&self, muted: bool) {
        self.muted.set(muted);
        if let Some(view) = self.view() {
            view.set_is_muted(muted);
        }
        self.changed();
    }

    pub fn is_muted(&self) -> bool {
        self.muted.get()
    }

    // ── Processes and media ───────────────────────────────────────────────────

    /// Asks which web process the page lives in, for the task manager. A page that hangs
    /// can't answer: it keeps the answer it gave before.
    fn ask_pid(self: &Rc<Self>, view: &WebView) {
        let weak = Rc::downgrade(self);
        let asked = view.clone();
        ask_pid(view, move |pid| {
            if let Some(tab) = weak.upgrade()
                && tab.view().as_ref() == Some(&asked)
            {
                tab.pid.set(Some(pid));
            }
        });
    }

    /// The processes the tab lives in: its page's and its reader view's. One it doesn't know
    /// yet is asked for again, for the next look.
    pub fn processes(self: &Rc<Self>) -> (Option<i32>, Option<i32>) {
        if self.pid.get().is_none()
            && let Some(view) = self.view()
        {
            self.ask_pid(&view);
        }
        let reader = self.reader.borrow().as_ref().and_then(|r| r.pid.get());
        (self.pid.get(), reader)
    }

    /// Ends the page's process if it is `pid`, or closes the reader view living there.
    /// Whether the tab had it.
    pub fn end_process(&self, pid: i32) -> bool {
        if self
            .reader
            .borrow()
            .as_ref()
            .is_some_and(|r| r.pid.get() == Some(pid))
        {
            self.close_reader();
            self.relayout();
            self.changed();
            return true;
        }
        match self.view() {
            // Its tabs show the "page stopped" screen (web_process_terminated).
            Some(view) if self.pid.get() == Some(pid) => {
                view.terminate_web_process();
                true
            }
            _ => false,
        }
    }

    /// Pauses what plays in the page, or plays again what this paused, in all its frames
    /// (webext/peepwick-webext.c).
    pub fn toggle_media(&self) {
        if let Some(view) = self.view() {
            view.send_message_to_page(
                &UserMessage::new("media-toggle", None),
                None::<&gtk::gio::Cancellable>,
                |_| {},
            );
        }
    }

    fn is_private(&self) -> bool {
        self.window().is_some_and(|w| w.incognito)
    }

    /// Loads the shield stopped, as (page, resource) addresses from the web process. Those of
    /// a page the tab has left since are dropped. Private windows count for the tab only:
    /// the day's numbers are written to disk.
    pub fn record_blocked(&self, loads: &[(String, String)]) {
        let counted = count_blocked(&self.url(), loads, &mut self.blocked_hosts.borrow_mut());
        if counted == 0 {
            return;
        }
        self.blocked.set(self.blocked.get().saturating_add(counted));
        if !self.is_private() {
            crate::stats::add_blocked(u64::from(counted));
        }
        self.changed();
    }

    /// The hosts the shield blocked most on this page, for its panel.
    pub fn blocked_report(&self, limit: usize) -> Vec<Value> {
        let mut hosts: Vec<(String, u32)> = self
            .blocked_hosts
            .borrow()
            .iter()
            .map(|(host, count)| (host.clone(), *count))
            .collect();
        hosts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hosts
            .into_iter()
            .take(limit)
            .map(|(host, count)| json!({ "host": host, "count": count }))
            .collect()
    }

    /// Applies the remembered or default zoom again, after either changed in Settings.
    pub fn refresh_zoom(&self) {
        if let Some(view) = self.view()
            && self.shows_web()
            && let Some(url) = view.uri()
        {
            view.set_zoom_level(zoom::zoom_for(&url, self.is_private()));
        }
    }

    pub fn zoom(&self, direction: &str) {
        let Some(view) = self.shown_view() else {
            return;
        };
        let next = match direction {
            "reset" => settings::get().default_zoom,
            _ => zoom::step(view.zoom_level(), direction),
        };
        view.set_zoom_level(next);
        // Reader view's size isn't the site's.
        if self.reader.borrow().is_none()
            && let Some(url) = view.uri()
        {
            zoom::remember(&url, next, self.is_private());
        }
        self.changed();
    }

    pub fn find_in_page(&self, text: &str, forward: bool, find_next: bool) {
        let Some(view) = self.shown_view() else {
            return;
        };
        let Some(finder) = view.find_controller() else {
            return;
        };
        if self.page.get().is_some() {
            return;
        }
        if text.is_empty() {
            finder.search_finish();
            self.find.replace(Some(FindState::default()));
            self.changed();
            return;
        }
        let fresh = self.find.borrow().as_ref().is_none_or(|f| f.text != text);
        let mut state = self.find.borrow().clone().unwrap_or_default();
        if fresh {
            state = FindState {
                text: text.to_string(),
                matches: 0,
                active: 0,
            };
            // Case-insensitive, wrapping around, like other browsers.
            let options =
                webkit2gtk::FindOptions::CASE_INSENSITIVE | webkit2gtk::FindOptions::WRAP_AROUND;
            finder.search(text, options.bits(), 1000);
        } else if find_next {
            if forward {
                finder.search_next();
                state.active = if state.active >= state.matches {
                    1
                } else {
                    state.active + 1
                };
            } else {
                finder.search_previous();
                state.active = if state.active <= 1 {
                    state.matches
                } else {
                    state.active - 1
                };
            }
        }
        self.find.replace(Some(state));
        self.changed();
    }

    pub fn stop_find(&self) {
        if let Some(finder) = self.shown_view().and_then(|v| v.find_controller()) {
            finder.search_finish();
        }
        self.find.replace(None);
        self.changed();
    }

    pub fn find_text(&self) -> Option<String> {
        self.find
            .borrow()
            .as_ref()
            .map(|f| f.text.clone())
            .filter(|t| !t.is_empty())
    }

    pub fn is_finding(&self) -> bool {
        self.find.borrow().is_some()
    }

    pub fn is_loading(&self) -> bool {
        self.shows_web() && self.view().is_some_and(|v| v.is_loading())
    }

    // ── State ─────────────────────────────────────────────────────────────────

    pub fn serialize(&self) -> SavedTab {
        let history = if self.page.get().is_some() {
            None
        } else {
            self.view()
                .map(|v| capture_history(&v))
                .unwrap_or_else(|| self.saved_history.borrow().clone())
        };
        SavedTab {
            url: self.display_url(),
            title: self.title.borrow().clone(),
            pinned: self.pinned.get(),
            muted: self.muted.get(),
            history: history.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
        }
    }

    pub fn display_url(&self) -> String {
        if let Some(interstitial) = self.interstitial.borrow().as_ref()
            && let Some(url) = interstitial.get("url").and_then(Value::as_str)
        {
            return url.to_string();
        }
        self.url()
    }

    pub fn display_title(&self) -> String {
        if let Some(page) = self.page.get() {
            return page_title(page);
        }
        let title = self.title.borrow().clone();
        if !title.is_empty() && self.interstitial.borrow().is_none() {
            return title;
        }
        let url = self.display_url();
        let host = pages::host_of(&url);
        let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
        if host.is_empty() { url } else { host }
    }

    pub fn state(&self) -> TabState {
        let view = self.view();
        let url = self.display_url();
        let web = self.page.get().is_none();
        TabState {
            id: self.id,
            contents_id: None,
            security: if web { self.security(&url) } else { "internal" },
            title: self.display_title(),
            url,
            favicon: if web {
                self.favicon.borrow().clone()
            } else {
                None
            },
            loading: self.is_loading(),
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            audible: view.as_ref().is_some_and(|v| v.is_playing_audio()),
            muted: self.muted.get(),
            pinned: self.pinned.get(),
            sleeping: self.sleeping(),
            blocked: self.blocked.get(),
            zoom: self.shown_view().map(|v| v.zoom_level()).unwrap_or(1.0),
            page: self.page.get(),
            interstitial: self.interstitial.borrow().clone(),
            reader_available: self.reader_available.get(),
            reader: self.reader.borrow().is_some(),
            find: self.find.borrow().as_ref().map(|f| json!(f)),
            media: self.media.get(),
            page_lang: self.page_lang.borrow().clone(),
            translation: self
                .translation
                .borrow()
                .as_ref()
                .and_then(|state| serde_json::to_value(state).ok()),
        }
    }

    // ── Signals ───────────────────────────────────────────────────────────────

    fn connect(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: &WebView) {
        let weak = Rc::downgrade(self);
        let tab_of = move || weak.upgrade();

        let tab = tab_of.clone();
        view.connect_load_changed(move |view, event| {
            let Some(tab) = tab() else { return };
            let wanted = match event {
                LoadEvent::Started | LoadEvent::Redirected => {
                    tab.loading.replace(view.uri().map(|uri| uri.to_string()));
                    None
                }
                LoadEvent::Committed => tab.loading.take(),
                _ => None,
            };
            if event == LoadEvent::Committed {
                // A new page: what played in the last one stopped with it, and the page may
                // live in another process now.
                tab.media.set(None);
                tab.ask_pid(view);
            }
            match event {
                LoadEvent::Started => {
                    crate::permissions::deny_pending_for(tab.id);
                    if let Some(win) = tab.window() {
                        crate::autofill::hide_tab(&win, tab.id);
                    }
                    // A new load ends the last one's HTTPS attempt; the web process reports this
                    // one's after this.
                    security::https_settled(tab.id);
                    // The page under the focus gate or a dangerous site warning moving on
                    // doesn't lift it.
                    if !tab.holds_page() {
                        tab.interstitial.replace(None);
                    }
                    if tab.find.borrow().is_some() {
                        tab.stop_find();
                    }
                    tab.close_reader();
                    tab.reader_available.set(false);
                }
                LoadEvent::Finished if tab.shows_web() => {
                    tab.check_reader(view);
                    crate::translation::detect(&tab);
                }
                LoadEvent::Committed if tab.holds_page() => {
                    // The gate keeps the address it holds.
                    security::https_settled(tab.id);
                }
                LoadEvent::Committed => {
                    security::https_settled(tab.id);
                    tab.tls_failure.replace(None);
                    // An address on a restricted port: WebKit commits a blank page instead,
                    // with no load failure. The tab keeps the address and says why.
                    if view.uri().as_deref() == Some("about:blank")
                        && let Some(wanted) = wanted.filter(|w| pages::restricted_port(w))
                    {
                        crate::diagnostics::load_error(UNSAFE_PORT);
                        tab.set_address(&wanted);
                        tab.show_error(
                            &wanted,
                            UNSAFE_PORT,
                            "Not allowed to use restricted network port",
                            false,
                        );
                        return;
                    }
                    if let Some(uri) = view.uri()
                        && uri.starts_with("https:")
                        && let Some((certificate, flags)) =
                            view.tls_info().filter(|(_, f)| !f.is_empty())
                        && !security::has_exception(&pages::host_of(&uri))
                    {
                        crate::diagnostics::certificate_error(flags);
                        tab.certificate_warning(&uri, &certificate, flags);
                        return;
                    }
                    if let Some(uri) = view.uri() {
                        // Pages back from WebKit's back-forward cache come without a reply.
                        tab.follow_shield(&uri);
                        let host_changed = pages::host_of(&uri) != pages::host_of(&tab.url());
                        tab.set_address(&uri);
                        if host_changed {
                            // The stored icon until the page's own arrives; an offline copy
                            // has its page's.
                            let page = readinglist::saved_source(&uri);
                            tab.favicon
                                .replace(history::favicon_for_url(page.as_deref().unwrap_or(&uri)));
                        }
                        view.set_zoom_level(zoom::zoom_for(&uri, tab.is_private()));
                        if !tab.is_private() {
                            let title = view.title().map(|t| t.to_string()).unwrap_or_default();
                            history::add_visit(&uri, &title);
                        }
                    }
                    tab.blocked.set(0);
                    tab.blocked_hosts.borrow_mut().clear();
                    crate::translation::new_page(&tab);
                }
                _ => {}
            }
            tab.relayout();
            tab.changed();
        });

        let tab = tab_of.clone();
        view.connect_is_loading_notify(move |_| {
            if let Some(tab) = tab() {
                tab.changed();
            }
        });

        let tab = tab_of.clone();
        view.connect_title_notify(move |view| {
            let Some(tab) = tab() else { return };
            let title = view.title().map(|t| t.to_string()).unwrap_or_default();
            if !tab.is_private()
                && let Some(uri) = view.uri()
            {
                history::update_title(&uri, &title);
            }
            tab.title.replace(title);
            tab.changed();
            if let Some(win) = tab.window() {
                win.update_title();
            }
        });

        let tab = tab_of.clone();
        view.connect_uri_notify(move |view| {
            let Some(tab) = tab() else { return };
            if tab.page.get().is_none()
                && tab.interstitial.borrow().is_none()
                && let Some(uri) = view.uri()
            {
                let moved = *tab.url.borrow() != uri.as_str();
                tab.url.replace(uri.to_string());
                // A page that changed its address itself (history.pushState, #anchors).
                if moved && !view.is_loading() && !tab.is_private() {
                    let title = view.title().map(|t| t.to_string()).unwrap_or_default();
                    history::add_visit(&uri, &title);
                }
            }
            tab.changed();
        });

        let tab = tab_of.clone();
        view.connect_favicon_notify(move |view| {
            let Some(tab) = tab() else { return };
            let Some(data) = view.favicon().and_then(favicon_data_url) else {
                return;
            };
            if !tab.is_private()
                && let Some(uri) = view.uri()
                && history::set_favicon(&pages::host_of(&uri), &data)
            {
                crate::window::favicons_changed();
            }
            tab.favicon.replace(Some(data));
            tab.changed();
        });

        let tab = tab_of.clone();
        view.connect_is_playing_audio_notify(move |_| {
            if let Some(tab) = tab() {
                tab.changed();
            }
        });

        let tab = tab_of.clone();
        view.connect_load_failed(move |_, _, uri, error| {
            // Stopped loads and downloads aren't failures.
            if error.matches(NetworkError::Cancelled)
                || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange)
            {
                return false;
            }
            let Some(tab) = tab() else { return false };
            // The page under a gate isn't what the tab shows.
            if tab.holds_page() {
                return true;
            }
            let code = error_code(error);
            match security::https_fallback(tab.id, uri, code) {
                Some(http) => tab.show_https_only(&http),
                None => {
                    crate::diagnostics::load_error(code);
                    tab.show_error(uri, code, &error.message(), false);
                }
            }
            true
        });

        let tab = tab_of.clone();
        view.connect_load_failed_with_tls_errors(move |_, uri, certificate, flags| {
            let Some(tab) = tab() else { return false };
            if tab.holds_page() {
                return true;
            }
            match security::https_fallback(tab.id, uri, -202) {
                Some(http) => tab.show_https_only(&http),
                None => {
                    crate::diagnostics::certificate_error(flags);
                    tab.certificate_warning(uri, certificate, flags);
                }
            }
            true
        });

        let tab = tab_of.clone();
        view.connect_web_process_terminated(move |_, reason| {
            let Some(tab) = tab() else { return };
            // Ended from the task manager: not an error.
            match reason {
                WebProcessTerminationReason::TerminatedByApi => {}
                WebProcessTerminationReason::ExceededMemoryLimit => crate::diagnostics::record(
                    "page",
                    "Web process gone: it went over the memory limit",
                ),
                _ => crate::diagnostics::record("page", "Web process gone: crashed"),
            }
            tab.pid.set(None);
            tab.media.set(None);
            tab.interstitial.replace(Some(json!({ "kind": "crashed" })));
            tab.relayout();
            tab.changed();
        });

        // HTTPS-Only, from the web process (webext/peepwick-webext.c): the page was moved to
        // HTTPS, or its secure version sent it back to HTTP and the load stopped.
        let tab = tab_of.clone();
        view.connect_user_message_received(move |page, message| {
            let Some(tab) = tab() else { return false };
            let parameters = message.parameters();
            match message.name().as_deref() {
                // A sign-in field got focus or lost it, a key moved through the list of saved
                // logins, or a sign-in was sent, with WebKit's address for the page.
                Some("forms") => {
                    if let Some((kind, payload, uri)) =
                        parameters.and_then(|p| p.get::<(String, String, String)>())
                    {
                        crate::autofill::page_message(&tab, page, &kind, &payload, &uri);
                    }
                    true
                }
                // Loads the shield stopped, as the page's console heard of them.
                Some("blocked") => {
                    if let Some(loads) = parameters.and_then(|p| p.get::<Vec<(String, String)>>()) {
                        tab.record_blocked(&loads);
                    }
                    true
                }
                // A frame's video or audio started or stopped playing.
                Some("media") => {
                    let playing = parameters.and_then(|p| p.get::<bool>()) == Some(true);
                    let media = if playing {
                        Some("playing")
                    } else {
                        tab.media.get().map(|_| "paused")
                    };
                    if media != tab.media.get() {
                        tab.media.set(media);
                        tab.changed();
                    }
                    true
                }
                Some("https-upgraded") => {
                    if let Some((http, secure)) =
                        parameters.and_then(|p| p.get::<(String, String)>())
                    {
                        security::upgraded(&tab, &http, &secure);
                    }
                    true
                }
                Some("https-loop") => {
                    if let Some(http) = parameters.and_then(|p| p.get::<String>()) {
                        security::https_settled(tab.id);
                        tab.show_https_only(&http);
                    }
                    true
                }
                _ => false,
            }
        });

        // A page closed itself (window.close() after a sign-in popup, for example).
        let tab = tab_of.clone();
        view.connect_close(move |_| {
            let Some(tab) = tab() else { return };
            if let Some(win) = tab.window() {
                win.close_tab(tab.id, false);
            }
        });

        let tab = tab_of.clone();
        let opener = Rc::downgrade(win);
        view.connect_create(move |view, _| {
            let (win, tab) = (opener.upgrade()?, tab()?);
            Some(win.open_from(view, &tab))
        });

        let tab = tab_of.clone();
        view.connect_decide_policy(move |_, decision, kind| {
            let Some(tab) = tab() else { return false };
            tab.decide(decision, kind)
        });

        let tab = tab_of.clone();
        view.connect_enter_fullscreen(move |_| {
            if let Some(tab) = tab()
                && let Some(win) = tab.window()
            {
                win.set_html_fullscreen(tab.id, true);
            }
            false
        });
        let tab = tab_of.clone();
        view.connect_leave_fullscreen(move |_| {
            if let Some(tab) = tab()
                && let Some(win) = tab.window()
            {
                win.set_html_fullscreen(tab.id, false);
            }
            false
        });

        let tab = tab_of.clone();
        view.connect_context_menu(move |view, proposed, event, hit| {
            if let Some(tab) = tab() {
                let (x, y) = event.coords().unwrap_or((0.0, 0.0));
                menus::page_menu(&tab, view, proposed, hit, x, y);
            }
            true
        });

        let window = Rc::downgrade(win);
        view.connect_key_press_event(move |_, event| match window.upgrade() {
            Some(win) if shortcuts::handle(&win, event) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });

        // Ctrl+wheel zooms, as in other browsers.
        let tab = tab_of.clone();
        view.connect_scroll_event(move |_, event| {
            let Some(tab) = tab() else {
                return glib::Propagation::Proceed;
            };
            if !event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            let up = match event.direction() {
                gtk::gdk::ScrollDirection::Up => true,
                gtk::gdk::ScrollDirection::Down => false,
                _ => match event.delta() {
                    (_, dy) if dy < 0.0 => true,
                    (_, dy) if dy > 0.0 => false,
                    _ => return glib::Propagation::Stop,
                },
            };
            tab.zoom(if up { "in" } else { "out" });
            glib::Propagation::Stop
        });

        let tab = tab_of.clone();
        view.connect_focus_in_event(move |_, _| {
            if let Some(tab) = tab()
                && let Some(win) = tab.window()
            {
                win.pane_focused(tab.id);
            }
            glib::Propagation::Proceed
        });

        // "Leave this page?" is asked in the browser's own UI.
        let tab = tab_of.clone();
        view.connect_script_dialog(move |_, dialog| {
            if dialog.dialog_type() != ScriptDialogType::BeforeUnloadConfirm {
                return false;
            }
            let (Some(tab), dialog) = (tab(), dialog.clone()) else {
                return false;
            };
            let Some(win) = tab.window() else {
                return false;
            };
            let question = crate::menu::Question {
                title: tab.display_title(),
                body: Some(t("dialog.leaveBody")),
                confirm: t("dialog.leave"),
                cancel: t("common.cancel"),
                danger: true,
            };
            crate::menu::confirm(&win, question, move |leave| {
                dialog.confirm_set_confirmed(leave);
                dialog.close();
            });
            true
        });

        crate::permissions::attach(self, view);
        self.connect_finder(view);
    }

    /// Find in page's counts, from the page or from reader view.
    fn connect_finder(self: &Rc<Self>, view: &WebView) {
        let Some(finder) = view.find_controller() else {
            return;
        };
        let weak = Rc::downgrade(self);
        finder.connect_found_text(move |_, count| {
            let Some(tab) = weak.upgrade() else { return };
            if let Some(find) = tab.find.borrow_mut().as_mut() {
                find.matches = count;
                find.active = find.active.clamp(1, count.max(1));
            }
            tab.changed();
        });
        let weak = Rc::downgrade(self);
        finder.connect_failed_to_find_text(move |_| {
            let Some(tab) = weak.upgrade() else { return };
            if let Some(find) = tab.find.borrow_mut().as_mut() {
                find.matches = 0;
                find.active = 0;
            }
            tab.changed();
        });
    }

    // ── Reader view ───────────────────────────────────────────────────────────

    pub fn has_reader(&self) -> bool {
        self.reader.borrow().is_some()
    }

    pub fn reader_available(&self) -> bool {
        self.reader_available.get()
    }

    /// The article reader view shows, or the offline copy's, for reading it aloud.
    pub fn article(&self) -> Option<Article> {
        if let Some(reader) = self.reader.borrow().as_ref() {
            return Some(reader.article.clone());
        }
        readinglist::article_at(&self.url())
    }

    /// Looks whether the page that just loaded has an article.
    fn check_reader(self: &Rc<Self>, view: &WebView) {
        let Some(url) = view.uri().map(|u| u.to_string()) else {
            return;
        };
        if !(url.starts_with("http:") || url.starts_with("https:")) {
            return;
        }
        let token = self.reader_token.get();
        let weak = Rc::downgrade(self);
        reader::check(view, move |available| {
            let Some(tab) = weak.upgrade() else { return };
            if tab.reader_token.get() != token || tab.view_url().as_deref() != Some(url.as_str()) {
                return;
            }
            if tab.reader_available.replace(available) != available {
                tab.changed();
            }
        });
    }

    /// Opens reader view over the page, or closes it.
    pub fn toggle_reader(self: &Rc<Self>) {
        if self.reader.borrow().is_some() {
            self.close_reader();
            self.relayout();
            self.changed();
            if let Some(win) = self.window() {
                win.focus_active();
            }
            return;
        }
        let Some(view) = self.view() else { return };
        let url = self.url();
        if !self.shows_web() || !(url.starts_with("http:") || url.starts_with("https:")) {
            return;
        }
        let token = self.reader_token.get().wrapping_add(1);
        self.reader_token.set(token);
        let weak = Rc::downgrade(self);
        reader::extract(&view, move |article| {
            let Some(tab) = weak.upgrade() else { return };
            if tab.reader_token.get() != token || !tab.shows_web() {
                return;
            }
            match article {
                Some(article) => tab.open_reader(article, &url),
                None => {
                    tab.reader_available.set(false);
                    tab.changed();
                }
            }
        });
    }

    fn open_reader(self: &Rc<Self>, article: Article, source: &str) {
        let Some(win) = self.window() else { return };
        if self.find.borrow().is_some() {
            self.stop_find();
        }
        let zoom = self.view().map(|v| v.zoom_level()).unwrap_or(1.0);
        let reader = Reader::new(article, source, reader::colors(win.incognito), zoom);
        win.place_view(&reader.view);
        self.connect_reader(&win, &reader.view);
        self.reader.replace(Some(reader));
        self.relayout();
        self.changed();
        if win.active_tab().is_some_and(|t| t.id == self.id) {
            win.focus_active();
        }
    }

    pub fn close_reader(&self) {
        self.reader_token
            .set(self.reader_token.get().wrapping_add(1));
        if self.reader.borrow().is_none() {
            return;
        }
        if self.find.borrow().is_some() {
            self.stop_find();
        }
        if let Some(reader) = self.reader.take() {
            reader.close();
        }
    }

    /// Reader view's own signals: links open in the tab, and the menus, keys, zoom and find
    /// work as on the page.
    fn connect_reader(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: &WebView) {
        let weak = Rc::downgrade(self);
        let tab_of = move || weak.upgrade();

        // Its own process, for the task manager.
        let tab = tab_of.clone();
        view.connect_load_changed(move |view, event| {
            if event != LoadEvent::Committed {
                return;
            }
            let pid = tab().and_then(|tab| {
                let reader = tab.reader.borrow();
                reader
                    .as_ref()
                    .filter(|r| &r.view == view)
                    .map(|r| r.pid.clone())
            });
            if let Some(pid) = pid {
                ask_pid(view, move |id| pid.set(Some(id)));
            }
        });

        let tab = tab_of.clone();
        view.connect_decide_policy(move |_, decision, kind| {
            let Some(tab) = tab() else { return false };
            let new_window = kind == PolicyDecisionType::NewWindowAction;
            if kind != PolicyDecisionType::NavigationAction && !new_window {
                return false;
            }
            let Some(action) = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
            else {
                return false;
            };
            let Some(url) = action
                .request()
                .and_then(|r| r.uri())
                .map(|u| u.to_string())
            else {
                return false;
            };
            if !new_window && tab.reader.borrow().as_ref().is_some_and(|r| r.is_own(&url)) {
                return false;
            }
            decision.ignore();
            if !(url.starts_with("http:") || url.starts_with("https:")) {
                return true;
            }
            let ctrl = action.modifiers() & gtk::gdk::ModifierType::CONTROL_MASK.bits() != 0;
            if new_window || action.mouse_button() == 2 || ctrl {
                if let Some(win) = tab.window() {
                    win.open_tab(OpenOptions {
                        url: Some(url),
                        background: !new_window,
                        opener: Some(tab.id),
                        ..OpenOptions::default()
                    });
                }
            } else {
                // After the decision returns: loading closes this view.
                glib::idle_add_local_once(move || tab.load(&url));
            }
            true
        });

        let tab = tab_of.clone();
        view.connect_context_menu(move |view, proposed, event, hit| {
            if let Some(tab) = tab() {
                let (x, y) = event.coords().unwrap_or((0.0, 0.0));
                menus::page_menu(&tab, view, proposed, hit, x, y);
            }
            true
        });

        let window = Rc::downgrade(win);
        view.connect_key_press_event(move |_, event| match window.upgrade() {
            Some(win) if shortcuts::handle(&win, event) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });

        let tab = tab_of.clone();
        view.connect_scroll_event(move |_, event| {
            let Some(tab) = tab() else {
                return glib::Propagation::Proceed;
            };
            if !event.state().contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            match event.direction() {
                gtk::gdk::ScrollDirection::Up => tab.zoom("in"),
                gtk::gdk::ScrollDirection::Down => tab.zoom("out"),
                _ => match event.delta() {
                    (_, dy) if dy < 0.0 => tab.zoom("in"),
                    (_, dy) if dy > 0.0 => tab.zoom("out"),
                    _ => {}
                },
            }
            glib::Propagation::Stop
        });

        let tab = tab_of.clone();
        view.connect_focus_in_event(move |_, _| {
            if let Some(tab) = tab()
                && let Some(win) = tab.window()
            {
                win.pane_focused(tab.id);
            }
            glib::Propagation::Proceed
        });

        self.connect_finder(view);
    }

    fn security(&self, url: &str) -> &'static str {
        let base = pages::security_of(url);
        let problem = self.tls_failure.borrow().is_some()
            || security::has_exception(&pages::host_of(url))
            || self
                .view()
                .and_then(|v| v.tls_info())
                .is_some_and(|(_, flags)| !flags.is_empty());
        if base == "secure" && problem {
            "unverified"
        } else {
            base
        }
    }

    pub fn tls_failure(&self) -> Option<(gtk::gio::TlsCertificate, gtk::gio::TlsCertificateFlags)> {
        self.tls_failure.borrow().clone()
    }

    /// Focus mode: the site waits behind a calm gate instead of loading.
    fn hold_for_focus(&self, url: &str) {
        let host = pages::host_of(url);
        let host = host.strip_prefix("www.").unwrap_or(&host);
        security::https_settled(self.id);
        self.set_address(url);
        self.interstitial
            .replace(Some(json!({ "kind": "focus", "url": url, "host": host })));
        self.relayout();
        self.changed();
    }

    fn focus_gate_url(&self) -> Option<String> {
        self.interstitial
            .borrow()
            .as_ref()
            .filter(|i| i["kind"] == "focus")
            .and_then(|i| i["url"].as_str().map(str::to_string))
    }

    /// The focus gate or a dangerous site warning stands in for a page that wasn't let through.
    fn holds_page(&self) -> bool {
        self.interstitial
            .borrow()
            .as_ref()
            .is_some_and(|i| i["kind"] == "focus" || i["kind"] == "dangerous")
    }

    /// "Open anyway" on the focus gate: the site may load for the next five minutes.
    pub fn open_anyway(self: &Rc<Self>) {
        if let Some(url) = self.focus_gate_url() {
            focus::grant_pass(&url);
            self.load(&url);
        }
    }

    /// The dangerous site warning instead of the page: the address is on a malware or
    /// phishing list.
    fn warn_dangerous(&self, url: &str, threat: safebrowsing::Threat) {
        security::https_settled(self.id);
        self.set_address(url);
        self.title.replace(String::new());
        self.favicon.replace(None);
        self.interstitial.replace(Some(
            json!({ "kind": "dangerous", "url": url, "threat": threat.name() }),
        ));
        self.relayout();
        self.changed();
    }

    /// "Continue anyway" on the dangerous site warning: the site opens until the browser
    /// closes.
    pub fn continue_to_dangerous(self: &Rc<Self>) {
        let url = self
            .interstitial
            .borrow()
            .as_ref()
            .filter(|i| i["kind"] == "dangerous")
            .and_then(|i| i["url"].as_str().map(str::to_string));
        if let Some(url) = url {
            safebrowsing::pass(&url);
            self.load(&url);
        }
    }

    /// HTTPS-Only: the secure version didn't work; ask before going on unencrypted.
    pub fn show_https_only(&self, url: &str) {
        self.stop();
        self.interstitial
            .replace(Some(json!({ "kind": "https-only", "url": url })));
        self.relayout();
        self.changed();
    }

    fn certificate_warning(
        &self,
        uri: &str,
        certificate: &gtk::gio::TlsCertificate,
        flags: gtk::gio::TlsCertificateFlags,
    ) {
        let (name, code) = security::verification(flags);
        self.tls_failure.replace(Some((certificate.clone(), flags)));
        self.show_error(uri, code, name.trim_start_matches("net::"), true);
    }

    /// "Continue over HTTP" on the HTTPS-Only warning: the site stays on HTTP this session.
    pub fn continue_over_http(self: &Rc<Self>) {
        let url = self
            .interstitial
            .borrow()
            .as_ref()
            .filter(|i| i["kind"] == "https-only")
            .and_then(|i| i["url"].as_str().map(str::to_string));
        if let Some(url) = url {
            security::allow_http(&url);
            self.load(&url);
        }
    }

    /// "Continue anyway" on a certificate warning: this host's certificate is accepted this
    /// session.
    pub fn proceed_insecure(self: &Rc<Self>) {
        let url = self
            .interstitial
            .borrow()
            .as_ref()
            .filter(|i| i["certificate"] == true)
            .and_then(|i| i["url"].as_str().map(str::to_string));
        let failure = self.tls_failure.borrow().clone();
        if let (Some(url), Some((certificate, _)), Some(win)) = (url, failure, self.window()) {
            security::allow_certificate(&win, &certificate, &pages::host_of(&url));
            self.load(&url);
        }
    }

    fn show_error(&self, uri: &str, code: i32, description: &str, certificate: bool) {
        self.interstitial.replace(Some(json!({
            "kind": "error",
            "code": code,
            "description": description,
            "url": uri,
            "certificate": certificate,
        })));
        self.relayout();
        self.changed();
    }

    /// Links opened with the middle button or Ctrl go to a new tab behind this one; replies
    /// WebKit can't show become downloads. What depends on where the tab's own page goes (the
    /// focus gate, the shield) waits for the reply: before it, WebKit can't tell the page from
    /// a frame in it. HTTPS-Only is done in the web process (webext/peepwick-webext.c).
    fn decide(self: &Rc<Self>, decision: &PolicyDecision, kind: PolicyDecisionType) -> bool {
        match kind {
            PolicyDecisionType::NavigationAction => {
                let Some(action) = decision
                    .downcast_ref::<NavigationPolicyDecision>()
                    .and_then(|d| d.navigation_action())
                else {
                    return false;
                };
                let ctrl = action.modifiers() & gtk::gdk::ModifierType::CONTROL_MASK.bits() != 0;
                if action.mouse_button() != 2 && !ctrl {
                    return false;
                }
                let Some(url) = action.request().and_then(|r| r.uri()) else {
                    return false;
                };
                if let Some(win) = self.window() {
                    win.open_tab(OpenOptions {
                        url: Some(url.to_string()),
                        background: true,
                        opener: Some(self.id),
                        ..OpenOptions::default()
                    });
                }
                decision.ignore();
                true
            }
            PolicyDecisionType::Response => {
                let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>() else {
                    return false;
                };
                let page = response
                    .is_main_frame_main_resource()
                    .then(|| response.response().and_then(|r| r.uri()))
                    .flatten();
                // An embedded video in another page still plays; frames aren't warned about
                // either, as in 0.1.x.
                if let Some(url) = &page
                    && focus::should_gate(url)
                {
                    decision.ignore();
                    self.hold_for_focus(url);
                    return true;
                }
                // Before a download too: a malware site's file isn't saved.
                if let Some(url) = &page
                    && let Some(threat) = safebrowsing::threat_for(url)
                {
                    decision.ignore();
                    self.warn_dangerous(url, threat);
                    return true;
                }
                if !response.is_mime_type_supported() {
                    // A download leaves the page as it is; an upgrade to HTTPS that got this far
                    // worked.
                    if page.is_some() {
                        security::https_settled(self.id);
                    }
                    decision.download();
                    return true;
                }
                if let Some(url) = &page {
                    self.follow_shield(url);
                }
                false
            }
            _ => false,
        }
    }

    /// The shield's rules follow the site the tab's page is on, set before anything in the
    /// page loads.
    fn follow_shield(&self, url: &str) {
        if let Some(manager) = self.view().and_then(|v| v.user_content_manager()) {
            crate::shield::set_paused(&manager, crate::shield::paused_for(url));
        }
    }

    pub fn print(&self) {
        let (Some(view), Some(win)) = (self.shown_view(), self.window()) else {
            return;
        };
        if self.page.get().is_some() {
            return;
        }
        let operation = webkit2gtk::PrintOperation::new(&view);
        let parent = win.gtk_window();
        webkit2gtk::PrintOperationExt::run_dialog(&operation, parent.as_ref());
    }

    pub fn devtools(&self) {
        if let Some(inspector) = self.view().and_then(|v| v.inspector())
            && self.page.get().is_none()
        {
            webkit2gtk::WebInspectorExt::show(&inspector);
        }
    }
}

fn capture_history(view: &WebView) -> Option<Vec<u8>> {
    view.session_state()
        .and_then(|state| state.serialize())
        .map(|bytes| bytes.to_vec())
}

/// The UI knows Chromium's error codes (-105 for "name not resolved"…); map WebKit's onto them.
/// Whether an address would be held back instead of loading: by focus mode or a dangerous site
/// warning.
fn is_held(url: &str) -> bool {
    focus::should_gate(url) || safebrowsing::threat_for(url).is_some()
}

/// Asks a view's web process for its id (webext/peepwick-webext.c).
fn ask_pid(view: &WebView, keep: impl FnOnce(i32) + 'static) {
    view.send_message_to_page(
        &UserMessage::new("pid", None),
        None::<&gtk::gio::Cancellable>,
        move |reply| {
            if let Some(pid) = reply
                .ok()
                .and_then(|m| m.parameters())
                .and_then(|p| p.get::<i32>())
            {
                keep(pid);
            }
        },
    );
}

/// Chromium's ERR_UNSAFE_PORT, for an address on a restricted port (`pages::restricted_port`).
const UNSAFE_PORT: i32 = -312;

fn error_code(error: &glib::Error) -> i32 {
    // A redirect to a restricted port fails the load (an address typed in commits a blank page).
    if error.matches(PolicyError::CannotUseRestrictedPort) {
        return UNSAFE_PORT;
    }
    let message = error.message().to_lowercase();
    if message.contains("resolve") || message.contains("name or service") {
        -105
    } else if message.contains("refused") {
        -102
    } else if message.contains("timed out") || message.contains("timeout") {
        -118
    } else if message.contains("network is unreachable") || message.contains("offline") {
        -106
    } else {
        -2
    }
}

fn favicon_data_url(surface: cairo::Surface) -> Option<String> {
    let image = cairo::ImageSurface::try_from(surface).ok()?;
    let mut png = Vec::new();
    image.write_to_png(&mut png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

/// Counts blocked loads that belong to the page at `current` (fragments aside) into `hosts`;
/// returns how many did.
fn count_blocked(
    current: &str,
    loads: &[(String, String)],
    hosts: &mut HashMap<String, u32>,
) -> u32 {
    let page = |url: &str| url.split('#').next().unwrap_or_default().to_string();
    let current = page(current);
    let mut counted = 0;
    for (document, resource) in loads {
        if page(document) != current {
            continue;
        }
        let host = pages::host_of(resource);
        *hosts
            .entry(if host.is_empty() {
                resource.clone()
            } else {
                host
            })
            .or_insert(0) += 1;
        counted += 1;
    }
    counted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_blocked_loads_of_the_current_page_only() {
        let mut hosts = HashMap::new();
        let loads = vec![
            (
                "https://a.example/page#top".to_string(),
                "https://ads.example/x.js".to_string(),
            ),
            (
                "https://a.example/page".to_string(),
                "https://ads.example/y.gif".to_string(),
            ),
            (
                "https://a.example/page".to_string(),
                "https://t.example/p".to_string(),
            ),
            (
                "https://a.example/before".to_string(),
                "https://ads.example/z.js".to_string(),
            ),
        ];
        assert_eq!(
            count_blocked("https://a.example/page", &loads, &mut hosts),
            3
        );
        assert_eq!(hosts.get("ads.example"), Some(&2));
        assert_eq!(hosts.get("t.example"), Some(&1));
        assert_eq!(hosts.len(), 2);
    }
}
