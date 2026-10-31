//! One tab: its WebKitGTK view, made the first time the tab shows a web page, and what the
//! UI shows about it.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use base64::Engine;
use gtk::prelude::*;
use serde_json::{Value, json};
use webkit2gtk::{LoadEvent, NetworkError, PolicyError, WebView, WebViewExt};

use crate::pages;
use crate::state::TabState;
use crate::window::BrowserWindow;

pub struct Tab {
    pub id: u32,
    win: Weak<BrowserWindow>,
    view: RefCell<Option<WebView>>,
    url: RefCell<String>,
    title: RefCell<String>,
    favicon: RefCell<Option<String>>,
    /// Set while the UI draws a built-in page (new tab, settings…) instead of the view.
    page: Cell<Option<&'static str>>,
    pub pinned: Cell<bool>,
    muted: Cell<bool>,
    /// An error, crash or warning screen the UI draws in place of the page.
    interstitial: RefCell<Option<Value>>,
    pub blocked: Cell<u32>,
}

impl Tab {
    pub fn new(id: u32, win: Weak<BrowserWindow>, url: &str) -> Rc<Tab> {
        let tab = Rc::new(Tab {
            id,
            win,
            view: RefCell::new(None),
            url: RefCell::new(String::new()),
            title: RefCell::new(String::new()),
            favicon: RefCell::new(None),
            page: Cell::new(None),
            pinned: Cell::new(false),
            muted: Cell::new(false),
            interstitial: RefCell::new(None),
            blocked: Cell::new(0),
        });
        tab.set_address(url);
        tab
    }

    fn window(&self) -> Option<Rc<BrowserWindow>> {
        self.win.upgrade()
    }

    fn changed(&self) {
        if let Some(win) = self.window() {
            win.mark_dirty();
        }
    }

    pub fn view(&self) -> Option<WebView> {
        self.view.borrow().clone()
    }

    /// Whether the page area shows this tab's view, not a built-in page or a warning.
    pub fn shows_web(&self) -> bool {
        self.page.get().is_none()
            && self.interstitial.borrow().is_none()
            && self.view.borrow().is_some()
    }

    fn set_address(&self, url: &str) {
        *self.url.borrow_mut() = url.to_string();
        self.page.set(pages::internal_page(url));
    }

    /// Loads a URL: built-in pages are drawn by the UI, everything else in the view.
    pub fn load(self: &Rc<Self>, url: &str) {
        self.interstitial.replace(None);
        self.set_address(url);
        if self.page.get().is_some() {
            self.title.replace(String::new());
            self.favicon.replace(None);
        } else {
            self.ensure_view().load_uri(url);
        }
        if let Some(win) = self.window() {
            win.layout();
        }
        self.changed();
    }

    pub fn ensure_view(self: &Rc<Self>) -> WebView {
        if let Some(view) = self.view() {
            return view;
        }
        let win = self.window().expect("a tab lives in a window");
        let view = win.create_view(None);
        self.adopt(&win, view.clone());
        view
    }

    /// Takes a new view (also one a page opened with window.open) and wires its signals.
    pub fn adopt(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: WebView) {
        win.place_view(&view);
        view.set_is_muted(self.muted.get());
        self.connect(win, &view);
        self.view.replace(Some(view));
    }

    pub fn destroy(&self) {
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

    fn connect(self: &Rc<Self>, win: &Rc<BrowserWindow>, view: &WebView) {
        let weak = Rc::downgrade(self);

        let w = weak.clone();
        view.connect_load_changed(move |view, event| {
            let Some(tab) = w.upgrade() else { return };
            match event {
                LoadEvent::Started => {
                    tab.interstitial.replace(None);
                }
                LoadEvent::Committed => {
                    if let Some(uri) = view.uri() {
                        tab.set_address(&uri);
                    }
                    tab.blocked.set(0);
                }
                _ => {}
            }
            if let Some(win) = tab.window() {
                win.layout();
            }
            tab.changed();
        });

        let w = weak.clone();
        view.connect_is_loading_notify(move |_| {
            if let Some(tab) = w.upgrade() {
                tab.changed();
            }
        });

        let w = weak.clone();
        view.connect_title_notify(move |view| {
            let Some(tab) = w.upgrade() else { return };
            tab.title
                .replace(view.title().map(|t| t.to_string()).unwrap_or_default());
            tab.changed();
        });

        let w = weak.clone();
        view.connect_uri_notify(move |view| {
            let Some(tab) = w.upgrade() else { return };
            if tab.page.get().is_none()
                && let Some(uri) = view.uri()
            {
                tab.url.replace(uri.to_string());
            }
            tab.changed();
        });

        let w = weak.clone();
        view.connect_favicon_notify(move |view| {
            let Some(tab) = w.upgrade() else { return };
            tab.favicon
                .replace(view.favicon().and_then(favicon_data_url));
            tab.changed();
        });

        let w = weak.clone();
        view.connect_is_playing_audio_notify(move |_| {
            if let Some(tab) = w.upgrade() {
                tab.changed();
            }
        });

        let w = weak.clone();
        view.connect_load_failed(move |_, _, uri, error| {
            // Stopped loads and downloads aren't failures.
            if error.matches(NetworkError::Cancelled)
                || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange)
            {
                return false;
            }
            let Some(tab) = w.upgrade() else { return false };
            tab.interstitial.replace(Some(json!({
                "kind": "error",
                "code": error_code(error),
                "description": error.message(),
                "url": uri,
                "certificate": false,
            })));
            if let Some(win) = tab.window() {
                win.layout();
            }
            tab.changed();
            true
        });

        let w = weak.clone();
        view.connect_web_process_terminated(move |_, _| {
            let Some(tab) = w.upgrade() else { return };
            tab.interstitial.replace(Some(json!({ "kind": "crashed" })));
            if let Some(win) = tab.window() {
                win.layout();
            }
            tab.changed();
        });

        let w = weak.clone();
        view.connect_close(move |_| {
            let Some(tab) = w.upgrade() else { return };
            if let Some(win) = tab.window() {
                win.close_tab(tab.id);
            }
        });

        let opener = Rc::downgrade(win);
        view.connect_create(move |view, _action| opener.upgrade().map(|win| win.open_from(view)));
    }

    pub fn back(&self) {
        if let Some(view) = self.view() {
            view.go_back();
        }
    }

    pub fn forward(&self) {
        if let Some(view) = self.view() {
            view.go_forward();
        }
    }

    pub fn reload(self: &Rc<Self>, hard: bool) {
        if self.page.get().is_some() {
            return;
        }
        let failed = self.interstitial.replace(None).is_some();
        let Some(view) = self.view() else { return };
        if failed {
            // The warning or error page replaced the page: load its address again.
            let url = self.url.borrow().clone();
            view.load_uri(&url);
        } else if hard {
            view.reload_bypass_cache();
        } else {
            view.reload();
        }
        if let Some(win) = self.window() {
            win.layout();
        }
        self.changed();
    }

    pub fn stop(&self) {
        if let Some(view) = self.view() {
            view.stop_loading();
        }
    }

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

    pub fn state(&self) -> TabState {
        let view = self.view();
        let url = self.url.borrow().clone();
        let web = self.page.get().is_none();
        TabState {
            id: self.id,
            contents_id: None,
            security: if web {
                pages::security_of(&url)
            } else {
                "internal"
            },
            url,
            title: self.title.borrow().clone(),
            favicon: self.favicon.borrow().clone(),
            loading: web && view.as_ref().is_some_and(|v| v.is_loading()),
            can_go_back: view.as_ref().is_some_and(|v| v.can_go_back()),
            can_go_forward: web && view.as_ref().is_some_and(|v| v.can_go_forward()),
            audible: view.as_ref().is_some_and(|v| v.is_playing_audio()),
            muted: self.muted.get(),
            pinned: self.pinned.get(),
            sleeping: false,
            blocked: self.blocked.get(),
            zoom: view.as_ref().map(|v| v.zoom_level()).unwrap_or(1.0),
            page: self.page.get(),
            interstitial: self.interstitial.borrow().clone(),
            reader_available: false,
            reader: false,
            find: None,
            media: None,
            page_lang: None,
            translation: None,
        }
    }
}

/// The UI knows Chromium's error codes (-105 for "name not resolved"…); map WebKit's onto them.
fn error_code(error: &webkit2gtk::glib::Error) -> i32 {
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
