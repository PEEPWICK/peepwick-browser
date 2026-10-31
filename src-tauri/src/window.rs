//! A browser window: Tauri's window with the React UI, and the tabs' WebKitGTK views stacked
//! with it in a GTK overlay. The UI normally sits under the tab views, which are placed
//! where the UI asks (setPanes); while a menu or dialog is open it moves on top, transparent
//! over the pages, the way Electron raised its UI view.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU32, Ordering};

use gtk::prelude::*;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use webkit2gtk::{
    CookieAcceptPolicy, CookieManagerExt, CookiePersistentStorage, HardwareAccelerationPolicy,
    SettingsExt, UserContentManager, WebContext, WebContextExt, WebView, WebViewExt,
    WebsiteDataManager, WebsiteDataManagerExt,
};

use crate::settings::{self, Lang, Theme};
use crate::state::{
    AppInfo, FocusState, GlobalState, InitialData, PaneRect, SplitState, UpdateState, WindowState,
    WorkspaceState,
};
use crate::tab::Tab;
use crate::{pages, paths, shield};

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<BrowserWindow>>> = const { RefCell::new(Vec::new()) };
    /// Cookies, storage and cache shared by every normal window.
    static BROWSING: RefCell<Option<WebContext>> = const { RefCell::new(None) };
}

static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);
static NEXT_TAB: AtomicU32 = AtomicU32::new(1);

pub struct Workspace {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub tab_ids: Vec<u32>,
    pub active_tab_id: Option<u32>,
    pub split: Option<SplitState>,
}

impl Workspace {
    fn state(&self) -> WorkspaceState {
        WorkspaceState {
            id: self.id.clone(),
            name: self.name.clone(),
            icon: self.icon.clone(),
            tab_ids: self.tab_ids.clone(),
            active_tab_id: self.active_tab_id,
            split: self.split,
        }
    }
}

pub struct BrowserWindow {
    pub id: u32,
    pub label: String,
    pub incognito: bool,
    app: AppHandle,
    window: WebviewWindow,
    weak: Weak<BrowserWindow>,
    overlay: gtk::Overlay,
    /// Holds the tab views; passes input through wherever there is no view.
    fixed: gtk::Fixed,
    ui: WebView,
    context: WebContext,
    web_settings: webkit2gtk::Settings,
    tabs: RefCell<BTreeMap<u32, Rc<Tab>>>,
    workspaces: RefCell<Vec<Workspace>>,
    active_workspace: RefCell<String>,
    panes: RefCell<Vec<PaneRect>>,
    overlay_open: Cell<bool>,
    raised: Cell<bool>,
    dirty: Cell<bool>,
}

/// Opens a browser window. Its UI view is moved into the overlay once Tauri has made it.
pub fn open(app: &AppHandle, incognito: bool) -> tauri::Result<()> {
    let id = NEXT_WINDOW.fetch_add(1, Ordering::Relaxed);
    let label = format!("window-{id}");
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        // The interface's own storage stays in the profile with everything else.
        .data_directory(paths::user_path("ui"))
        .title("PEEPWICK Browser")
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 420.0)
        .decorations(false)
        .transparent(true)
        .visible(false)
        .build()?;
    let app = app.clone();
    let handle = window.clone();
    window.with_webview(move |platform| {
        match BrowserWindow::attach(app, handle, platform.inner(), id, label, incognito) {
            Ok(win) => WINDOWS.with(|windows| windows.borrow_mut().push(win)),
            Err(error) => eprintln!("[window] could not set up the window: {error}"),
        }
    })?;
    Ok(())
}

pub fn find(label: &str) -> Option<Rc<BrowserWindow>> {
    WINDOWS.with(|windows| windows.borrow().iter().find(|w| w.label == label).cloned())
}

pub fn all() -> Vec<Rc<BrowserWindow>> {
    WINDOWS.with(|windows| windows.borrow().clone())
}

fn browsing_context(app: &AppHandle) -> WebContext {
    BROWSING.with(|cell| {
        if let Some(context) = cell.borrow().as_ref() {
            return context.clone();
        }
        let data = paths::user_path("web");
        let cache = app
            .path()
            .app_cache_dir()
            .unwrap_or_else(|_| paths::user_path("cache"));
        let manager = WebsiteDataManager::builder()
            .base_data_directory(data.to_string_lossy().as_ref())
            .base_cache_directory(cache.to_string_lossy().as_ref())
            .build();
        if let Some(cookies) = manager.cookie_manager() {
            let file = data.join("cookies.sqlite");
            cookies
                .set_persistent_storage(&file.to_string_lossy(), CookiePersistentStorage::Sqlite);
        }
        let context = WebContext::with_website_data_manager(&manager);
        configure_context(&context, &manager);
        cell.replace(Some(context.clone()));
        context
    })
}

fn configure_context(context: &WebContext, manager: &WebsiteDataManager) {
    // Intelligent Tracking Prevention, and no third-party cookies unless switched off.
    manager.set_itp_enabled(true);
    if let Some(cookies) = manager.cookie_manager() {
        let policy = if settings::get().block_third_party_cookies {
            CookieAcceptPolicy::NoThirdParty
        } else {
            CookieAcceptPolicy::Always
        };
        cookies.set_accept_policy(policy);
    }
    context.set_spell_checking_enabled(true);
    context.set_spell_checking_languages(&["tr_TR", "en_US"]);
}

fn web_settings() -> webkit2gtk::Settings {
    let prefs = settings::get();
    let web = webkit2gtk::Settings::new();
    web.set_enable_developer_extras(true);
    web.set_enable_back_forward_navigation_gestures(true);
    web.set_javascript_can_open_windows_automatically(false);
    web.set_enable_media_stream(true);
    web.set_enable_webrtc(true);
    // Widevine and other DRM aren't available in WebKitGTK.
    web.set_enable_encrypted_media(false);
    web.set_default_font_size(prefs.font_size);
    web.set_hardware_acceleration_policy(if prefs.hardware_acceleration {
        HardwareAccelerationPolicy::Always
    } else {
        HardwareAccelerationPolicy::Never
    });
    web
}

impl BrowserWindow {
    fn attach(
        app: AppHandle,
        window: WebviewWindow,
        ui: WebView,
        id: u32,
        label: String,
        incognito: bool,
    ) -> Result<Rc<BrowserWindow>, String> {
        let vbox = window.default_vbox().map_err(|e| e.to_string())?;
        // Take the UI view out of Tauri's box and stack it with the tab views.
        if let Some(parent) = ui.parent() {
            parent
                .downcast::<gtk::Container>()
                .map_err(|_| "unexpected UI parent".to_string())?
                .remove(&ui);
        }
        let overlay = gtk::Overlay::new();
        let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
        base.style_context().add_class("peepwick-base");
        overlay.add(&base);
        ui.set_halign(gtk::Align::Fill);
        ui.set_valign(gtk::Align::Fill);
        overlay.add_overlay(&ui);
        let fixed = gtk::Fixed::new();
        fixed.set_halign(gtk::Align::Fill);
        fixed.set_valign(gtk::Align::Fill);
        overlay.add_overlay(&fixed);
        overlay.set_overlay_pass_through(&fixed, true);
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        // Transparent, so pages show through while the UI is on top for a menu.
        ui.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));

        let context = if incognito {
            let manager = WebsiteDataManager::new_ephemeral();
            let context = WebContext::with_website_data_manager(&manager);
            configure_context(&context, &manager);
            context
        } else {
            browsing_context(&app)
        };
        apply_theme();

        let win = Rc::new_cyclic(|weak| BrowserWindow {
            id,
            label,
            incognito,
            app,
            window: window.clone(),
            weak: weak.clone(),
            overlay,
            fixed,
            ui,
            context,
            web_settings: web_settings(),
            tabs: RefCell::new(BTreeMap::new()),
            workspaces: RefCell::new(vec![Workspace {
                id: "personal".into(),
                name: match settings::get().language {
                    Lang::Tr => "Kişisel".into(),
                    Lang::En => "Personal".into(),
                },
                icon: "circle".into(),
                tab_ids: Vec::new(),
                active_tab_id: None,
                split: None,
            }]),
            active_workspace: RefCell::new("personal".into()),
            panes: RefCell::new(Vec::new()),
            overlay_open: Cell::new(false),
            raised: Cell::new(false),
            dirty: Cell::new(false),
        });
        win.new_tab(None, false);

        // Window events arrive on the main thread; the window is found again by its label.
        let label = win.label.clone();
        window.on_window_event(move |event| match event {
            tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_) => {
                if let Some(win) = find(&label) {
                    win.mark_dirty();
                }
            }
            tauri::WindowEvent::Destroyed => {
                if let Some(win) = find(&label) {
                    win.destroyed();
                }
            }
            _ => {}
        });
        let _ = window.show();
        Ok(win)
    }

    pub fn ui_view(&self) -> WebView {
        self.ui.clone()
    }

    pub fn ui_on_top(&self) -> bool {
        self.overlay_open.get() || self.raised.get()
    }

    pub fn tauri_window(&self) -> WebviewWindow {
        self.window.clone()
    }

    /// The tab views on screen, with where they are.
    pub fn visible_views(&self) -> Vec<(WebView, PaneRect)> {
        let panes = self.panes.borrow();
        self.tabs
            .borrow()
            .values()
            .filter(|tab| tab.shows_web())
            .filter_map(|tab| Some((tab.view()?, *panes.iter().find(|p| p.tab_id == tab.id)?)))
            .collect()
    }

    pub fn app_handle(&self) -> AppHandle {
        self.app.clone()
    }

    fn destroyed(&self) {
        for tab in self.tabs.borrow().values() {
            tab.destroy();
        }
        WINDOWS.with(|windows| windows.borrow_mut().retain(|w| w.id != self.id));
    }

    // ── State ─────────────────────────────────────────────────────────────────

    pub fn emit(&self, name: &str, payload: impl Serialize) {
        let payload = json!({ "name": name, "payload": payload });
        if let Err(error) = self.app.emit_to(self.label.as_str(), "pw:event", payload) {
            eprintln!("[window] could not send {name}: {error}");
        }
    }

    /// Collects changes into one state update for the UI.
    pub fn mark_dirty(&self) {
        if self.dirty.replace(true) {
            return;
        }
        let weak = self.weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(win) = weak.upgrade() {
                win.dirty.set(false);
                win.emit("state", win.state());
            }
        });
    }

    pub fn state(&self) -> WindowState {
        WindowState {
            window_id: self.id,
            incognito: self.incognito,
            maximized: self.window.is_maximized().unwrap_or(false),
            fullscreen: self.window.is_fullscreen().unwrap_or(false),
            workspaces: self
                .workspaces
                .borrow()
                .iter()
                .map(Workspace::state)
                .collect(),
            active_workspace_id: self.active_workspace.borrow().clone(),
            tabs: self
                .tabs
                .borrow()
                .iter()
                .map(|(id, tab)| (*id, tab.state()))
                .collect(),
            permission: None,
            shield_paused: false,
            extensions_partition: None,
        }
    }

    pub fn initial_data(&self) -> InitialData {
        let prefs = settings::get();
        let (bar, other) = match prefs.language {
            Lang::Tr => ("Yer imi çubuğu", "Diğer yer imleri"),
            Lang::En => ("Bookmarks bar", "Other bookmarks"),
        };
        InitialData {
            state: self.state(),
            global: global_state(),
            settings: prefs,
            app: app_info(&self.app),
            bookmarks: json!({
                "bar": { "type": "folder", "id": "bar", "title": bar, "addedAt": 0, "children": [] },
                "other": { "type": "folder", "id": "other", "title": other, "addedAt": 0, "children": [] },
            }),
        }
    }

    // ── Tabs ──────────────────────────────────────────────────────────────────

    pub fn tab(&self, id: u32) -> Option<Rc<Tab>> {
        self.tabs.borrow().get(&id).cloned()
    }

    pub fn active_tab(&self) -> Option<Rc<Tab>> {
        let active = self.active_workspace.borrow().clone();
        let id = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == active)?
            .active_tab_id?;
        self.tab(id)
    }

    pub fn create_view(&self, related: Option<&WebView>) -> WebView {
        let manager = UserContentManager::new();
        shield::attach(&manager);
        let builder = WebView::builder()
            .user_content_manager(&manager)
            .settings(&self.web_settings);
        let view = match related {
            // Views opened by a page share its process, as window.open needs.
            Some(opener) => builder.related_view(opener).build(),
            None => builder.web_context(&self.context).build(),
        };
        view
    }

    /// Puts a new view in the window, hidden until a pane shows it.
    pub fn place_view(&self, view: &WebView) {
        self.fixed.put(view, 0, 0);
        view.hide();
    }

    pub fn new_tab(&self, url: Option<String>, background: bool) -> Rc<Tab> {
        let id = NEXT_TAB.fetch_add(1, Ordering::Relaxed);
        let tab = Tab::new(id, self.weak.clone(), pages::NEW_TAB_URL);
        self.tabs.borrow_mut().insert(id, tab.clone());
        {
            let active = self.active_workspace.borrow().clone();
            let mut workspaces = self.workspaces.borrow_mut();
            if let Some(workspace) = workspaces.iter_mut().find(|w| w.id == active) {
                // Next to the current tab, like most browsers.
                let at = workspace
                    .active_tab_id
                    .and_then(|current| workspace.tab_ids.iter().position(|t| *t == current))
                    .map_or(workspace.tab_ids.len(), |i| i + 1);
                workspace.tab_ids.insert(at, id);
            }
        }
        if let Some(url) = url {
            tab.load(&url);
        }
        if !background {
            self.activate_tab(id);
        }
        self.mark_dirty();
        tab
    }

    /// A page asked for a new window (window.open, target=_blank): a new tab with a view that
    /// shares the opener's process. WebKit loads the address into it once we return it.
    pub fn open_from(&self, opener: &WebView) -> gtk::Widget {
        let id = NEXT_TAB.fetch_add(1, Ordering::Relaxed);
        let tab = Tab::new(id, self.weak.clone(), "about:blank");
        let view = self.create_view(Some(opener));
        let win = self
            .weak
            .upgrade()
            .expect("the window is alive while its pages are");
        tab.adopt(&win, view.clone());
        self.tabs.borrow_mut().insert(id, tab);
        {
            let active = self.active_workspace.borrow().clone();
            let mut workspaces = self.workspaces.borrow_mut();
            if let Some(workspace) = workspaces.iter_mut().find(|w| w.id == active) {
                let at = workspace
                    .active_tab_id
                    .and_then(|current| workspace.tab_ids.iter().position(|t| *t == current))
                    .map_or(workspace.tab_ids.len(), |i| i + 1);
                workspace.tab_ids.insert(at, id);
            }
        }
        let weak = self.weak.clone();
        view.connect_ready_to_show(move |_| {
            if let Some(win) = weak.upgrade() {
                win.activate_tab(id);
            }
        });
        self.mark_dirty();
        view.upcast()
    }

    pub fn activate_tab(&self, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        {
            let mut workspaces = self.workspaces.borrow_mut();
            if let Some(workspace) = workspaces.iter_mut().find(|w| w.tab_ids.contains(&id)) {
                workspace.active_tab_id = Some(id);
                self.active_workspace.replace(workspace.id.clone());
            }
        }
        if tab.shows_web()
            && let Some(view) = tab.view()
            && !self.raised.get()
        {
            view.grab_focus();
        }
        self.mark_dirty();
    }

    pub fn close_tab(&self, id: u32) {
        let Some(tab) = self.tabs.borrow_mut().remove(&id) else {
            return;
        };
        tab.destroy();
        let mut empty = false;
        {
            let active = self.active_workspace.borrow().clone();
            let mut workspaces = self.workspaces.borrow_mut();
            for workspace in workspaces.iter_mut() {
                if let Some(at) = workspace.tab_ids.iter().position(|t| *t == id) {
                    workspace.tab_ids.remove(at);
                    if workspace.active_tab_id == Some(id) {
                        let next = at.min(workspace.tab_ids.len().saturating_sub(1));
                        workspace.active_tab_id = workspace.tab_ids.get(next).copied();
                    }
                    if workspace.id == active && workspace.tab_ids.is_empty() {
                        empty = true;
                    }
                }
            }
        }
        if empty {
            self.new_tab(None, false);
        }
        self.layout();
        self.mark_dirty();
    }

    pub fn navigate(&self, id: u32, input: &str) {
        let Some(tab) = self.tab(id) else { return };
        let url = crate::input::resolve_input(input, settings::get().search_engine);
        if !url.is_empty() {
            tab.load(&url);
        }
    }

    pub fn open_url(&self, url: &str, new_tab: bool) {
        if new_tab {
            self.new_tab(Some(url.to_string()), false);
        } else if let Some(tab) = self.active_tab() {
            tab.load(url);
        }
    }

    /// Shows a built-in page: in the current tab when it is a new tab, otherwise in a new one.
    pub fn open_page(&self, page: &str, subpage: &str) {
        let url = pages::internal_url(page, subpage);
        match self.active_tab() {
            Some(tab) if tab.state().page == Some("newtab") => tab.load(&url),
            _ => {
                self.new_tab(Some(url), false);
            }
        }
    }

    // ── Layout ────────────────────────────────────────────────────────────────

    pub fn set_panes(&self, rects: Vec<PaneRect>) {
        self.panes.replace(rects);
        self.layout();
    }

    /// Places the views of the tabs the UI shows and hides the others.
    pub fn layout(&self) {
        let panes = self.panes.borrow();
        for tab in self.tabs.borrow().values() {
            let Some(view) = tab.view() else { continue };
            match panes
                .iter()
                .find(|p| p.tab_id == tab.id)
                .filter(|_| tab.shows_web())
            {
                Some(rect) => {
                    self.fixed
                        .move_(&view, rect.x.round() as i32, rect.y.round() as i32);
                    view.set_size_request(
                        rect.width.round().max(1.0) as i32,
                        rect.height.round().max(1.0) as i32,
                    );
                    view.show();
                }
                None => view.hide(),
            }
        }
    }

    /// The UI asks to be on top while a menu, dialog or tooltip is open.
    pub fn set_overlay(&self, open: bool) {
        self.overlay_open.set(open);
        self.restack();
    }

    pub fn set_raised(&self, raised: bool) {
        self.raised.set(raised);
        self.restack();
    }

    fn restack(&self) {
        let on_top = self.overlay_open.get() || self.raised.get();
        self.overlay
            .reorder_overlay(&self.ui, if on_top { -1 } else { 0 });
        if self.overlay_open.get() {
            self.ui.grab_focus();
        } else if let Some(tab) = self.active_tab()
            && tab.shows_web()
            && let Some(view) = tab.view()
        {
            view.grab_focus();
        }
    }

    pub fn window_control(&self, action: &str) {
        let _ = match action {
            "minimize" => self.window.minimize(),
            "maximize" => {
                if self.window.is_maximized().unwrap_or(false) {
                    self.window.unmaximize()
                } else {
                    self.window.maximize()
                }
            }
            "close" => self.window.close(),
            _ => Ok(()),
        };
    }
}

pub fn global_state() -> GlobalState {
    GlobalState {
        focus: FocusState {
            active: false,
            started_at: 0,
            ends_at: 0,
            minutes: settings::get().focus_minutes,
        },
        update: UpdateState {
            status: "idle",
            version: None,
            progress: 0.0,
            error: None,
        },
        downloads: Vec::new(),
        version: display_version(),
    }
}

/// 0.2.0 → "0.2.0a" while in alpha.
pub fn display_version() -> String {
    format!("{}a", env!("CARGO_PKG_VERSION"))
}

pub fn app_info(_app: &AppHandle) -> AppInfo {
    let webkit = unsafe {
        format!(
            "{}.{}.{}",
            webkit2gtk::ffi::webkit_get_major_version(),
            webkit2gtk::ffi::webkit_get_minor_version(),
            webkit2gtk::ffi::webkit_get_micro_version()
        )
    };
    AppInfo {
        version: display_version(),
        stage: "alpha",
        electron: format!("Tauri {}", tauri::VERSION),
        chrome: format!("WebKitGTK {webkit}"),
        node: String::new(),
        platform: "linux",
        packaged: !cfg!(debug_assertions),
        is_default_browser: false,
        hardware_acceleration: settings::get().hardware_acceleration,
    }
}

/// Native parts (GTK dialogs, scrollbars) and pages' prefers-color-scheme follow the theme.
pub fn apply_theme() {
    let dark = match settings::get().theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => false,
    };
    if let Some(gtk_settings) = gtk::Settings::default() {
        gtk_settings.set_gtk_application_prefer_dark_theme(dark);
    }
    let css = if dark {
        ".peepwick-base { background-color: #0B0B0C; }"
    } else {
        ".peepwick-base { background-color: #E7E6E2; }"
    };
    let provider = gtk::CssProvider::new();
    if provider.load_from_data(css.as_bytes()).is_ok()
        && let Some(screen) = gtk::gdk::Screen::default()
    {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Sends the new settings to every window and applies what the native side owns.
pub fn settings_changed() {
    let prefs = settings::get();
    apply_theme();
    for win in all() {
        win.web_settings.set_default_font_size(prefs.font_size);
        win.emit("settings", &prefs);
    }
}
