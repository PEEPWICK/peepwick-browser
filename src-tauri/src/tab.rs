//! One tab: at most one WebKitGTK view. Built-in pages (new tab, settings…), error screens
//! and warnings are drawn by the UI instead, and sleeping tabs have no view until they are
//! shown again.

use std::cell::{Cell, RefCell};
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
    ScriptDialogType, URIRequestExt, WebView, WebViewExt, WebViewSessionState,
};

use crate::i18n::t;
use crate::state::TabState;
use crate::window::{BrowserWindow, OpenOptions};
use crate::{history, menus, pages, settings, shortcuts, zoom};

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
    pub blocked: Cell<u32>,
    pub last_active: Cell<u64>,
    pub opener_id: Cell<Option<u32>>,
    /// History to restore when the view is made: a tab restored from the last session, or
    /// one that slept.
    saved_history: RefCell<Option<Vec<u8>>>,
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
            last_active: Cell::new(now_ms()),
            opener_id: Cell::new(None),
            saved_history: RefCell::new(history),
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

    fn changed(&self) {
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
            view.load_uri(&self.url());
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
        crate::permissions::deny_pending_for(self.id);
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
        self.interstitial.replace(None);
        if let Some(page) = pages::internal_page(url) {
            self.show_page(page, "");
            return;
        }
        self.set_address(url);
        if self.view.borrow().is_none() {
            // A sleeping tab's old history would be loaded instead of the new address.
            self.saved_history.replace(None);
        }
        if let Some(view) = self.ensure_view() {
            view.load_uri(url);
        }
        self.relayout();
        self.changed();
    }

    pub fn show_page(&self, page: &'static str, subpage: &str) {
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
        self.page.get().is_none() && self.view().is_some_and(|v| v.can_go_forward())
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
        if self.interstitial.replace(None).is_some() {
            match &view {
                Some(view) if view.can_go_back() => view.go_back(),
                Some(_) if self.view_url().is_some() => {
                    if let Some(url) = self.view_url() {
                        self.set_address(&url);
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

    fn is_private(&self) -> bool {
        self.window().is_some_and(|w| w.incognito)
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
        let Some(view) = self.view() else { return };
        let next = match direction {
            "reset" => settings::get().default_zoom,
            _ => zoom::step(view.zoom_level(), direction),
        };
        view.set_zoom_level(next);
        if let Some(url) = view.uri() {
            zoom::remember(&url, next, self.is_private());
        }
        self.changed();
    }

    pub fn find_in_page(&self, text: &str, forward: bool, find_next: bool) {
        let Some(view) = self.view() else { return };
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
        if let Some(finder) = self.view().and_then(|v| v.find_controller()) {
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
            security: if web {
                pages::security_of(&url)
            } else {
                "internal"
            },
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
            zoom: view.as_ref().map(|v| v.zoom_level()).unwrap_or(1.0),
            page: self.page.get(),
            interstitial: self.interstitial.borrow().clone(),
            reader_available: false,
            reader: false,
            find: self.find.borrow().as_ref().map(|f| json!(f)),
            media: if view.as_ref().is_some_and(|v| v.is_playing_audio()) {
                Some("playing")
            } else {
                None
            },
            page_lang: None,
            translation: None,
        }
    }

    // ── Signals ───────────────────────────────────────────────────────────────

    fn connect(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: &WebView) {
        let weak = Rc::downgrade(self);
        let tab_of = move || weak.upgrade();

        let tab = tab_of.clone();
        view.connect_load_changed(move |view, event| {
            let Some(tab) = tab() else { return };
            match event {
                LoadEvent::Started => {
                    crate::permissions::deny_pending_for(tab.id);
                    tab.interstitial.replace(None);
                    if tab.find.borrow().is_some() {
                        tab.stop_find();
                    }
                }
                LoadEvent::Committed => {
                    if let Some(uri) = view.uri() {
                        let host_changed = pages::host_of(&uri) != pages::host_of(&tab.url());
                        tab.set_address(&uri);
                        if host_changed {
                            // The stored icon until the page's own arrives.
                            tab.favicon.replace(history::favicon_for_url(&uri));
                        }
                        view.set_zoom_level(zoom::zoom_for(&uri, tab.is_private()));
                        if !tab.is_private() {
                            let title = view.title().map(|t| t.to_string()).unwrap_or_default();
                            history::add_visit(&uri, &title);
                        }
                    }
                    tab.blocked.set(0);
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
            tab.show_error(uri, error_code(error), &error.message(), false);
            true
        });

        let tab = tab_of.clone();
        view.connect_load_failed_with_tls_errors(move |_, uri, _, _| {
            let Some(tab) = tab() else { return false };
            tab.show_error(uri, -202, "ERR_CERT_INVALID", true);
            true
        });

        let tab = tab_of.clone();
        view.connect_web_process_terminated(move |_, _| {
            let Some(tab) = tab() else { return };
            tab.interstitial.replace(Some(json!({ "kind": "crashed" })));
            tab.relayout();
            tab.changed();
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

        if let Some(finder) = view.find_controller() {
            let tab = tab_of.clone();
            finder.connect_found_text(move |_, count| {
                let Some(tab) = tab() else { return };
                if let Some(find) = tab.find.borrow_mut().as_mut() {
                    find.matches = count;
                    find.active = find.active.clamp(1, count.max(1));
                }
                tab.changed();
            });
            let tab = tab_of.clone();
            finder.connect_failed_to_find_text(move |_| {
                let Some(tab) = tab() else { return };
                if let Some(find) = tab.find.borrow_mut().as_mut() {
                    find.matches = 0;
                    find.active = 0;
                }
                tab.changed();
            });
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
    /// WebKit can't show become downloads.
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
                let Some(url) = action.request().and_then(|r| r.uri()) else {
                    return false;
                };
                if action.mouse_button() != 2 && !ctrl {
                    // The shield follows the site this tab is going to.
                    if let Some(manager) = self.view().and_then(|v| v.user_content_manager()) {
                        crate::shield::set_paused(&manager, crate::shield::paused_for(&url));
                    }
                    return false;
                }
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
                if response.is_mime_type_supported() {
                    return false;
                }
                decision.download();
                true
            }
            _ => false,
        }
    }

    pub fn print(&self) {
        let (Some(view), Some(win)) = (self.view(), self.window()) else {
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
fn error_code(error: &glib::Error) -> i32 {
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
