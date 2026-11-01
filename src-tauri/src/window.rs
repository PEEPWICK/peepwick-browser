//! A browser window: Tauri's window with the React UI, and the tabs' WebKitGTK views stacked
//! with it in a GTK overlay. The UI normally sits under the tab views, which are placed where
//! the UI asks (setPanes); while a menu or dialog is open it moves on top, transparent over
//! the pages, the way Electron raised its UI view.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use webkit2gtk::{
    CookieAcceptPolicy, CookieManagerExt, CookiePersistentStorage, HardwareAccelerationPolicy,
    SettingsExt, UserContentManager, WebContext, WebContextExt, WebView, WebViewExt,
    WebsiteDataManager, WebsiteDataManagerExt,
};

use crate::i18n::t;
use crate::settings::{self, Theme};
use crate::state::{
    AppInfo, FocusState, GlobalState, InitialData, PaneRect, SplitState, WindowState,
    WorkspaceState,
};
use crate::tab::{SavedTab, Tab, now_ms};
use crate::{bookmarks, downloads, pages, paths, permissions, session, shield, shortcuts, updater};

pub const WORKSPACE_ICONS: [&str; 12] = [
    "circle",
    "briefcase",
    "house",
    "book",
    "code",
    "palette",
    "music",
    "flask",
    "globe",
    "heart",
    "star",
    "flame",
];
const MAX_WORKSPACES: usize = 12;
const MAX_CLOSED: usize = 25;

thread_local! {
    static WINDOWS: RefCell<Vec<Rc<BrowserWindow>>> = const { RefCell::new(Vec::new()) };
    /// Cookies, storage and cache shared by every normal window.
    static BROWSING: RefCell<Option<WebContext>> = const { RefCell::new(None) };
    /// Shared by private windows while any is open; nothing is written to disk.
    static PRIVATE: RefCell<Option<WebContext>> = const { RefCell::new(None) };
    static LAST_FOCUSED: Cell<u32> = const { Cell::new(0) };
    /// Sent with "favicon": the UI's peepwick://favicon/ images carry it, to load again.
    static FAVICON_VERSION: Cell<u32> = const { Cell::new(0) };
    static FAVICON_PENDING: Cell<bool> = const { Cell::new(false) };
}

static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);
static NEXT_TAB: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSplit {
    pub left: usize,
    pub right: usize,
    pub ratio: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWorkspace {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub active_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<SavedSplit>,
    pub tabs: Vec<SavedTab>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWindow {
    /// Logical pixels: x, y, width, height.
    #[serde(default)]
    pub bounds: Option<[f64; 4]>,
    #[serde(default)]
    pub maximized: bool,
    pub active_workspace_id: String,
    pub workspaces: Vec<SavedWorkspace>,
}

struct Workspace {
    id: String,
    name: String,
    icon: String,
    tab_ids: Vec<u32>,
    active_tab_id: Option<u32>,
    split: Option<SplitState>,
    /// Most recently used tabs first, to pick the next tab when one closes.
    recent: Vec<u32>,
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

    fn in_split(&self, id: u32) -> bool {
        self.split.is_some_and(|s| s.left == id || s.right == id)
    }
}

struct ClosedTab {
    workspace_id: String,
    index: usize,
    saved: SavedTab,
}

/// How to open a tab.
#[derive(Default)]
pub struct OpenOptions {
    pub url: Option<String>,
    pub saved: Option<SavedTab>,
    pub background: bool,
    pub workspace_id: Option<String>,
    /// The tab it was opened from: the new one goes next to it, after its other children.
    pub opener: Option<u32>,
    pub index: Option<usize>,
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
    closed_tabs: RefCell<Vec<ClosedTab>>,
    panes: RefCell<Vec<PaneRect>>,
    overlay_open: Cell<bool>,
    raised: Cell<bool>,
    fullscreen_tab: Cell<Option<u32>>,
    dirty: Cell<bool>,
    ui_ready: Cell<bool>,
}

// ── Opening ───────────────────────────────────────────────────────────────────

/// Opens a browser window, restoring a saved one or opening the given addresses. Its UI view
/// is moved into the overlay once Tauri has made it.
pub fn open(
    app: &AppHandle,
    incognito: bool,
    urls: Vec<String>,
    saved: Option<SavedWindow>,
) -> tauri::Result<()> {
    let id = NEXT_WINDOW.fetch_add(1, Ordering::Relaxed);
    let label = format!("window-{id}");
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        // The interface's own storage stays in the profile with everything else.
        .data_directory(paths::user_path("ui"))
        .title(t("app.name"))
        .min_inner_size(560.0, 400.0)
        .decorations(false)
        .transparent(true)
        .visible(false);
    builder = match saved.as_ref().and_then(|s| s.bounds) {
        Some([x, y, width, height]) => builder.position(x, y).inner_size(width, height),
        None => builder.inner_size(1360.0, 860.0).center(),
    };
    let window = builder.build()?;
    if saved.as_ref().is_some_and(|s| s.maximized) {
        let _ = window.maximize();
    }
    let app = app.clone();
    let handle = window.clone();
    window.with_webview(move |platform| {
        match BrowserWindow::attach(
            app,
            handle,
            platform.inner(),
            id,
            label,
            incognito,
            urls,
            saved,
        ) {
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

/// The normal window used last, for links that come from outside (another app, a second
/// launch). Private windows don't take them.
fn last_normal_window() -> Option<Rc<BrowserWindow>> {
    let last = LAST_FOCUSED.with(Cell::get);
    let windows: Vec<Rc<BrowserWindow>> = all().into_iter().filter(|w| !w.incognito).collect();
    windows
        .iter()
        .find(|w| w.id == last)
        .cloned()
        .or_else(|| windows.last().cloned())
}

/// Opens addresses from outside in new tabs and brings the window forward; with no address,
/// just brings it forward.
pub fn open_urls(app: &AppHandle, urls: Vec<String>) {
    let Some(win) = last_normal_window() else {
        if let Err(error) = open(app, false, urls, None) {
            eprintln!("[window] could not open a window: {error}");
        }
        return;
    };
    for url in urls {
        win.open_tab(OpenOptions {
            url: Some(url),
            ..OpenOptions::default()
        });
    }
    let _ = win.window.unminimize();
    let _ = win.window.set_focus();
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
        configure_context(&context, &manager, false);
        downloads::attach(&context, false);
        cell.replace(Some(context.clone()));
        context
    })
}

fn private_context() -> WebContext {
    PRIVATE.with(|cell| {
        if let Some(context) = cell.borrow().as_ref() {
            return context.clone();
        }
        let manager = WebsiteDataManager::new_ephemeral();
        let context = WebContext::with_website_data_manager(&manager);
        configure_context(&context, &manager, true);
        downloads::attach(&context, true);
        cell.replace(Some(context.clone()));
        context
    })
}

/// The browsing context of normal or private windows, if one is open.
pub fn web_context(incognito: bool) -> Option<WebContext> {
    if incognito {
        PRIVATE.with(|cell| cell.borrow().clone())
    } else {
        BROWSING.with(|cell| cell.borrow().clone())
    }
}

/// The window and tab a view belongs to.
pub fn tab_for_view(view: &WebView) -> Option<(Rc<BrowserWindow>, Rc<Tab>)> {
    all().into_iter().find_map(|win| {
        let tab = win
            .tabs
            .borrow()
            .values()
            .find(|tab| tab.view().as_ref() == Some(view))
            .cloned()?;
        Some((win, tab))
    })
}

fn configure_context(context: &WebContext, manager: &WebsiteDataManager, incognito: bool) {
    // Which sites may notify, before a page asks.
    context.connect_initialize_notification_permissions(move |context| {
        permissions::init_notifications(context, incognito)
    });
    // WebKit's own icon store, for the tabs' icons. Private windows only read it.
    let icons = paths::user_path("web").join("icons");
    context.set_favicon_database_directory(Some(icons.to_string_lossy().as_ref()));
    // Intelligent Tracking Prevention, and no third-party cookies unless switched off.
    manager.set_itp_enabled(true);
    apply_cookie_policy(manager);
    context.set_spell_checking_enabled(true);
    context.set_spell_checking_languages(&["tr_TR", "en_US"]);
}

fn apply_cookie_policy(manager: &WebsiteDataManager) {
    if let Some(cookies) = manager.cookie_manager() {
        let policy = if settings::get().block_third_party_cookies {
            CookieAcceptPolicy::NoThirdParty
        } else {
            CookieAcceptPolicy::Always
        };
        cookies.set_accept_policy(policy);
    }
}

/// Pages identify as Safari on Linux: WebKitGTK's own string names an old Safari, which some
/// sites turn away.
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

fn web_settings() -> webkit2gtk::Settings {
    let prefs = settings::get();
    let web = webkit2gtk::Settings::new();
    web.set_user_agent(Some(USER_AGENT));
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
    #[allow(clippy::too_many_arguments)]
    fn attach(
        app: AppHandle,
        window: WebviewWindow,
        ui: WebView,
        id: u32,
        label: String,
        incognito: bool,
        urls: Vec<String>,
        saved: Option<SavedWindow>,
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
            private_context()
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
            ui: ui.clone(),
            context,
            web_settings: web_settings(),
            tabs: RefCell::new(BTreeMap::new()),
            workspaces: RefCell::new(Vec::new()),
            active_workspace: RefCell::new(String::new()),
            closed_tabs: RefCell::new(Vec::new()),
            panes: RefCell::new(Vec::new()),
            overlay_open: Cell::new(false),
            raised: Cell::new(false),
            fullscreen_tab: Cell::new(None),
            dirty: Cell::new(false),
            ui_ready: Cell::new(false),
        });
        win.restore(saved, urls);
        LAST_FOCUSED.with(|last| last.set(id));

        let weak = Rc::downgrade(&win);
        ui.connect_key_press_event(move |_, event| match weak.upgrade() {
            Some(win) if shortcuts::handle(&win, event) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });

        // Window events arrive on the main thread; the window is found again by its label.
        let label = win.label.clone();
        window.on_window_event(move |event| {
            let Some(win) = find(&label) else { return };
            match event {
                tauri::WindowEvent::Resized(_) => {
                    win.layout();
                    win.mark_dirty();
                }
                tauri::WindowEvent::Focused(true) => LAST_FOCUSED.with(|last| last.set(win.id)),
                tauri::WindowEvent::CloseRequested { .. } => session::window_closing(&win),
                tauri::WindowEvent::Destroyed => win.destroyed(),
                _ => {}
            }
        });

        let weak = Rc::downgrade(&win);
        glib::timeout_add_local(Duration::from_secs(60), move || match weak.upgrade() {
            Some(win) => {
                win.sleep_idle_tabs();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        // Shown once the UI has drawn (see `ui_ready`), or after a moment regardless.
        let weak = Rc::downgrade(&win);
        glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(win) = weak.upgrade() {
                let _ = win.window.show();
            }
        });
        Ok(win)
    }

    fn restore(self: &Rc<Self>, saved: Option<SavedWindow>, urls: Vec<String>) {
        match saved {
            Some(saved) if !saved.workspaces.is_empty() => {
                for ws in saved.workspaces {
                    let id = self.add_workspace(&ws.name, &ws.icon, Some(ws.id));
                    let tabs: Vec<Rc<Tab>> = ws
                        .tabs
                        .into_iter()
                        .map(|saved| self.add_tab(&id, saved, None))
                        .collect();
                    if let Some(active) =
                        tabs.get(ws.active_index.min(tabs.len().saturating_sub(1)))
                    {
                        self.set_active_in_workspace(&id, active.id);
                    }
                    if let Some(split) = ws.split
                        && let (Some(left), Some(right)) =
                            (tabs.get(split.left), tabs.get(split.right))
                        && left.id != right.id
                    {
                        self.with_workspace(&id, |w| {
                            w.split = Some(SplitState {
                                left: left.id,
                                right: right.id,
                                ratio: split.ratio.clamp(0.2, 0.8),
                            })
                        });
                    }
                }
                let active = if self
                    .workspaces
                    .borrow()
                    .iter()
                    .any(|w| w.id == saved.active_workspace_id)
                {
                    saved.active_workspace_id
                } else {
                    self.workspaces.borrow()[0].id.clone()
                };
                self.active_workspace.replace(active);
            }
            _ => {
                let id = self.add_workspace(&t("ws.default"), "circle", None);
                self.active_workspace.replace(id);
            }
        }
        for url in urls {
            self.open_tab(OpenOptions {
                url: Some(url),
                ..OpenOptions::default()
            });
        }
        if self.active_workspace_state(|w| w.tab_ids.is_empty()) {
            self.open_tab(OpenOptions {
                url: Some(pages::NEW_TAB_URL.into()),
                ..OpenOptions::default()
            });
        } else if let Some(tab) = self.active_tab() {
            self.activate_tab(tab.id);
        }
    }

    fn destroyed(&self) {
        for tab in self.tabs.borrow().values() {
            tab.destroy();
        }
        crate::menu::window_closed(self.id);
        WINDOWS.with(|windows| windows.borrow_mut().retain(|w| w.id != self.id));
        // Private browsing data goes with the last private window.
        if self.incognito && !all().iter().any(|w| w.incognito) {
            PRIVATE.with(|cell| cell.replace(None));
            permissions::forget_private();
        }
    }

    /// The UI has drawn its first state: show the window without a blank frame.
    pub fn ui_ready(&self) {
        if !self.ui_ready.replace(true) {
            let _ = self.window.show();
            self.focus_active();
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    pub fn app_handle(&self) -> AppHandle {
        self.app.clone()
    }

    pub fn tauri_window(&self) -> WebviewWindow {
        self.window.clone()
    }

    pub fn gtk_window(&self) -> Option<gtk::ApplicationWindow> {
        self.window.gtk_window().ok()
    }

    pub fn ui_view(&self) -> WebView {
        self.ui.clone()
    }

    pub fn ui_on_top(&self) -> bool {
        self.overlay_open.get() || self.raised.get()
    }

    pub fn is_dark(&self) -> bool {
        self.incognito || settings::get().theme != Theme::Light
    }

    // ── Workspaces ────────────────────────────────────────────────────────────

    fn add_workspace(&self, name: &str, icon: &str, id: Option<String>) -> String {
        let id = id.unwrap_or_else(new_id);
        let name: String = name.trim().chars().take(40).collect();
        self.workspaces.borrow_mut().push(Workspace {
            id: id.clone(),
            name: if name.is_empty() {
                t("ws.default")
            } else {
                name
            },
            icon: if WORKSPACE_ICONS.contains(&icon) {
                icon.to_string()
            } else {
                "circle".into()
            },
            tab_ids: Vec::new(),
            active_tab_id: None,
            split: None,
            recent: Vec::new(),
        });
        id
    }

    fn with_workspace<R>(&self, id: &str, f: impl FnOnce(&mut Workspace) -> R) -> Option<R> {
        self.workspaces
            .borrow_mut()
            .iter_mut()
            .find(|w| w.id == id)
            .map(f)
    }

    pub fn active_workspace_id(&self) -> String {
        let id = self.active_workspace.borrow().clone();
        let workspaces = self.workspaces.borrow();
        if workspaces.iter().any(|w| w.id == id) {
            id
        } else {
            workspaces.first().map(|w| w.id.clone()).unwrap_or_default()
        }
    }

    fn active_workspace_state<R>(&self, f: impl FnOnce(&Workspace) -> R) -> R {
        let id = self.active_workspace_id();
        let workspaces = self.workspaces.borrow();
        f(workspaces
            .iter()
            .find(|w| w.id == id)
            .expect("a window has a workspace"))
    }

    pub fn workspace_of(&self, tab_id: u32) -> Option<String> {
        self.workspaces
            .borrow()
            .iter()
            .find(|w| w.tab_ids.contains(&tab_id))
            .map(|w| w.id.clone())
    }

    /// Every workspace's id, name and tab count, in order.
    pub fn workspace_list(&self) -> Vec<(String, String, usize)> {
        self.workspaces
            .borrow()
            .iter()
            .map(|w| (w.id.clone(), w.name.clone(), w.tab_ids.len()))
            .collect()
    }

    pub fn create_workspace(self: &Rc<Self>, name: &str, icon: &str) {
        if self.workspaces.borrow().len() >= MAX_WORKSPACES {
            return;
        }
        let id = self.add_workspace(name, icon, None);
        self.active_workspace.replace(id.clone());
        self.open_tab(OpenOptions {
            url: Some(pages::NEW_TAB_URL.into()),
            workspace_id: Some(id),
            ..OpenOptions::default()
        });
        self.layout();
        self.mark_dirty();
    }

    pub fn update_workspace(&self, id: &str, name: Option<&str>, icon: Option<&str>) {
        self.with_workspace(id, |w| {
            if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
                w.name = name.chars().take(40).collect();
            }
            if let Some(icon) = icon.filter(|i| WORKSPACE_ICONS.contains(i)) {
                w.icon = icon.to_string();
            }
        });
        self.mark_dirty();
    }

    pub fn delete_workspace(self: &Rc<Self>, id: &str) {
        let Some(tab_ids) = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.tab_ids.clone())
        else {
            return;
        };
        if self.workspaces.borrow().len() <= 1 {
            return;
        }
        for tab_id in tab_ids {
            self.close_tab_with(tab_id, false, true);
        }
        self.workspaces.borrow_mut().retain(|w| w.id != id);
        if *self.active_workspace.borrow() == id {
            let first = self.workspaces.borrow()[0].id.clone();
            self.activate_workspace(&first);
        }
        self.mark_dirty();
    }

    pub fn activate_workspace(self: &Rc<Self>, id: &str) {
        let Some(empty) = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.tab_ids.is_empty())
        else {
            return;
        };
        self.active_workspace.replace(id.to_string());
        if empty {
            self.open_tab(OpenOptions {
                url: Some(pages::NEW_TAB_URL.into()),
                workspace_id: Some(id.into()),
                ..OpenOptions::default()
            });
        }
        if let Some(tab) = self.active_tab() {
            self.activate_tab(tab.id);
        }
        self.layout();
        self.mark_dirty();
    }

    // ── Tabs ──────────────────────────────────────────────────────────────────

    pub fn tab(&self, id: u32) -> Option<Rc<Tab>> {
        self.tabs.borrow().get(&id).cloned()
    }

    pub fn active_tab(&self) -> Option<Rc<Tab>> {
        let id = self.active_workspace_state(|w| w.active_tab_id)?;
        self.tab(id)
    }

    /// Adds a tab to a workspace; pinned tabs stay in front of the others.
    fn add_tab(&self, workspace_id: &str, saved: SavedTab, index: Option<usize>) -> Rc<Tab> {
        let id = NEXT_TAB.fetch_add(1, Ordering::Relaxed);
        let tab = Tab::new(id, self.weak.clone(), saved);
        self.tabs.borrow_mut().insert(id, tab.clone());
        let pinned_count = self.pinned_count(workspace_id);
        self.with_workspace(workspace_id, |w| {
            let position = index.unwrap_or(w.tab_ids.len()).min(w.tab_ids.len());
            let position = if tab.pinned.get() {
                position.min(pinned_count)
            } else {
                position.max(pinned_count)
            };
            w.tab_ids.insert(position, id);
        });
        tab
    }

    fn pinned_count(&self, workspace_id: &str) -> usize {
        let tabs = self.tabs.borrow();
        self.workspaces
            .borrow()
            .iter()
            .find(|w| w.id == workspace_id)
            .map(|w| {
                w.tab_ids
                    .iter()
                    .filter(|id| tabs.get(id).is_some_and(|t| t.pinned.get()))
                    .count()
            })
            .unwrap_or(0)
    }

    pub fn open_tab(self: &Rc<Self>, options: OpenOptions) -> Rc<Tab> {
        let workspace_id = options
            .workspace_id
            .filter(|id| self.workspaces.borrow().iter().any(|w| &w.id == id))
            .or_else(|| options.opener.and_then(|opener| self.workspace_of(opener)))
            .unwrap_or_else(|| self.active_workspace_id());
        let mut index = options.index;
        if index.is_none()
            && let Some(opener) = options.opener
        {
            // Next to the tab that opened it, after its other children.
            let tab_ids = self
                .workspaces
                .borrow()
                .iter()
                .find(|w| w.id == workspace_id)
                .map(|w| w.tab_ids.clone())
                .unwrap_or_default();
            let mut position = tab_ids
                .iter()
                .position(|id| *id == opener)
                .map_or(tab_ids.len(), |i| i + 1);
            while position < tab_ids.len()
                && self
                    .tab(tab_ids[position])
                    .is_some_and(|t| t.opener_id.get() == Some(opener))
            {
                position += 1;
            }
            index = Some(position);
        }
        let mut saved = options.saved.unwrap_or_default();
        if let Some(url) = options.url {
            saved.url = url;
        }
        let tab = self.add_tab(&workspace_id, saved, index);
        tab.opener_id.set(options.opener);
        if !options.background {
            self.active_workspace.replace(workspace_id);
            self.activate_tab(tab.id);
        }
        self.mark_dirty();
        tab
    }

    /// A page asked for a new window (window.open, target=_blank): a tab with a view that shares
    /// the opener's process. WebKit loads the address into it once we return it.
    pub fn open_from(self: &Rc<Self>, opener_view: &WebView, opener: &Rc<Tab>) -> gtk::Widget {
        let tab = self.open_tab(OpenOptions {
            url: Some("about:blank".into()),
            opener: Some(opener.id),
            background: true,
            ..OpenOptions::default()
        });
        let view = self.create_view(Some(opener_view));
        tab.adopt(self, view.clone());
        let weak = self.weak.clone();
        let id = tab.id;
        view.connect_ready_to_show(move |_| {
            if let Some(win) = weak.upgrade() {
                win.activate_tab(id);
            }
        });
        view.upcast()
    }

    fn set_active_in_workspace(&self, workspace_id: &str, tab_id: u32) {
        self.with_workspace(workspace_id, |w| {
            w.active_tab_id = Some(tab_id);
            w.recent.retain(|id| *id != tab_id);
            w.recent.insert(0, tab_id);
            w.recent.truncate(30);
        });
    }

    pub fn activate_tab(self: &Rc<Self>, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        let Some(workspace_id) = self.workspace_of(id) else {
            return;
        };
        let previous = self.active_tab();
        self.active_workspace.replace(workspace_id.clone());
        self.set_active_in_workspace(&workspace_id, id);
        tab.last_active.set(now_ms());
        if let Some(previous) = previous.filter(|p| p.id != id) {
            previous.last_active.set(now_ms());
        }
        // Reuse the current pane until the UI reports new geometry, to avoid a blank frame.
        let in_split = self
            .workspaces
            .borrow()
            .iter()
            .any(|w| w.id == workspace_id && w.in_split(id));
        {
            let mut panes = self.panes.borrow_mut();
            if panes.len() == 1 && !in_split {
                panes[0].tab_id = id;
            }
        }
        if tab.shows_web() {
            tab.ensure_view();
        }
        // Both sides of a split view are on screen.
        let partner = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == workspace_id)
            .and_then(|w| w.split)
            .and_then(|s| {
                if s.left == id {
                    Some(s.right)
                } else if s.right == id {
                    Some(s.left)
                } else {
                    None
                }
            });
        if let Some(other) = partner.and_then(|p| self.tab(p)).filter(|t| t.shows_web()) {
            other.ensure_view();
        }
        self.layout();
        self.focus_active();
        self.update_title();
        self.mark_dirty();
    }

    pub fn close_tab(self: &Rc<Self>, id: u32, remember: bool) {
        self.close_tab_with(id, remember, false);
    }

    fn close_tab_with(self: &Rc<Self>, id: u32, remember: bool, keep_workspace: bool) {
        let Some(tab) = self.tab(id) else { return };
        let Some(workspace_id) = self.workspace_of(id) else {
            return;
        };
        let index = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == workspace_id)
            .and_then(|w| w.tab_ids.iter().position(|t| *t == id))
            .unwrap_or(0);
        if remember && tab.page() != Some("newtab") {
            let mut closed = self.closed_tabs.borrow_mut();
            closed.push(ClosedTab {
                workspace_id: workspace_id.clone(),
                index,
                saved: tab.serialize(),
            });
            if closed.len() > MAX_CLOSED {
                closed.remove(0);
            }
        }
        let was_active = self
            .with_workspace(&workspace_id, |w| {
                w.tab_ids.retain(|t| *t != id);
                w.recent.retain(|t| *t != id);
                if w.in_split(id) {
                    w.split = None;
                }
                let was_active = w.active_tab_id == Some(id);
                if was_active {
                    w.active_tab_id = None;
                }
                was_active
            })
            .unwrap_or(false);
        if self.fullscreen_tab.get() == Some(id) {
            self.fullscreen_tab.set(None);
        }
        self.tabs.borrow_mut().remove(&id);
        tab.destroy();

        if was_active {
            let opener = tab
                .opener_id
                .get()
                .filter(|o| self.workspace_of(*o).as_deref() == Some(workspace_id.as_str()));
            let next = opener.or_else(|| {
                let tabs = self.tabs.borrow();
                self.workspaces
                    .borrow()
                    .iter()
                    .find(|w| w.id == workspace_id)
                    .and_then(|w| {
                        w.recent
                            .iter()
                            .copied()
                            .find(|t| tabs.contains_key(t))
                            .or_else(|| {
                                w.tab_ids
                                    .get(index.min(w.tab_ids.len().saturating_sub(1)))
                                    .copied()
                            })
                    })
            });
            if let Some(next) = next {
                self.set_active_in_workspace(&workspace_id, next);
            }
        }
        let empty = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == workspace_id)
            .is_some_and(|w| w.tab_ids.is_empty());
        if empty && !keep_workspace {
            if self.incognito && self.tabs.borrow().is_empty() {
                let _ = self.window.close();
                return;
            }
            if workspace_id == self.active_workspace_id() {
                self.open_tab(OpenOptions {
                    url: Some(pages::NEW_TAB_URL.into()),
                    workspace_id: Some(workspace_id),
                    ..OpenOptions::default()
                });
                return;
            }
        }
        if was_active
            && workspace_id == self.active_workspace_id()
            && let Some(active) = self.active_tab()
        {
            self.activate_tab(active.id);
        }
        self.layout();
        self.mark_dirty();
    }

    pub fn reopen_closed_tab(self: &Rc<Self>) {
        let Some(closed) = self.closed_tabs.borrow_mut().pop() else {
            return;
        };
        let workspace_id = if self
            .workspaces
            .borrow()
            .iter()
            .any(|w| w.id == closed.workspace_id)
        {
            closed.workspace_id
        } else {
            self.active_workspace_id()
        };
        let tab = self.add_tab(&workspace_id, closed.saved, Some(closed.index));
        self.active_workspace.replace(workspace_id);
        self.activate_tab(tab.id);
    }

    pub fn duplicate_tab(self: &Rc<Self>, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        let workspace_id = self
            .workspace_of(id)
            .unwrap_or_else(|| self.active_workspace_id());
        let index = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == workspace_id)
            .and_then(|w| w.tab_ids.iter().position(|t| *t == id))
            .map(|i| i + 1);
        let mut saved = tab.serialize();
        saved.pinned = false;
        let copy = self.add_tab(&workspace_id, saved, index);
        self.activate_tab(copy.id);
    }

    pub fn move_tab(
        self: &Rc<Self>,
        id: u32,
        workspace_id: &str,
        index: usize,
        pinned: Option<bool>,
    ) {
        let Some(tab) = self.tab(id) else { return };
        let Some(from) = self.workspace_of(id) else {
            return;
        };
        if !self
            .workspaces
            .borrow()
            .iter()
            .any(|w| w.id == workspace_id)
        {
            return;
        }
        let was_active = self
            .with_workspace(&from, |w| {
                w.tab_ids.retain(|t| *t != id);
                w.recent.retain(|t| *t != id);
                if w.in_split(id) {
                    w.split = None;
                }
                let was_active = w.active_tab_id == Some(id);
                if was_active {
                    w.active_tab_id = None;
                }
                was_active
            })
            .unwrap_or(false);
        if let Some(pinned) = pinned {
            tab.pinned.set(pinned);
        }
        let pinned_count = self.pinned_count(workspace_id);
        self.with_workspace(workspace_id, |w| {
            let position = index.min(w.tab_ids.len());
            let position = if tab.pinned.get() {
                position.min(pinned_count)
            } else {
                position.max(pinned_count)
            };
            w.tab_ids.insert(position, id);
        });
        if was_active {
            let next = self
                .workspaces
                .borrow()
                .iter()
                .find(|w| w.id == from)
                .and_then(|w| w.recent.first().or(w.tab_ids.first()).copied());
            match next {
                Some(next) => self.set_active_in_workspace(&from, next),
                None if from == self.active_workspace_id() => {
                    self.active_workspace.replace(workspace_id.to_string());
                }
                None => {}
            }
        }
        let from_empty = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == from)
            .is_some_and(|w| w.tab_ids.is_empty());
        if from != workspace_id && from_empty && from == self.active_workspace_id() {
            self.active_workspace.replace(workspace_id.to_string());
            self.activate_tab(id);
        }
        self.layout();
        self.mark_dirty();
    }

    pub fn toggle_pin(&self, id: u32) {
        let Some(tab) = self.tab(id) else { return };
        let Some(workspace_id) = self.workspace_of(id) else {
            return;
        };
        tab.pinned.set(!tab.pinned.get());
        self.with_workspace(&workspace_id, |w| w.tab_ids.retain(|t| *t != id));
        let pinned_count = self.pinned_count(&workspace_id);
        self.with_workspace(&workspace_id, |w| {
            w.tab_ids.insert(pinned_count.min(w.tab_ids.len()), id)
        });
        self.mark_dirty();
    }

    /// Every tab of the window, in the order they were opened.
    pub fn all_tabs(&self) -> Vec<Rc<Tab>> {
        self.tabs.borrow().values().cloned().collect()
    }

    pub fn tabs_in_order(&self, workspace_id: Option<&str>) -> Vec<Rc<Tab>> {
        let id = workspace_id
            .map(str::to_string)
            .unwrap_or_else(|| self.active_workspace_id());
        let ids = self
            .workspaces
            .borrow()
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.tab_ids.clone())
            .unwrap_or_default();
        ids.into_iter().filter_map(|id| self.tab(id)).collect()
    }

    // ── Split view ────────────────────────────────────────────────────────────

    pub fn split(self: &Rc<Self>, left: u32, right: u32) {
        let (Some(a), Some(b)) = (self.workspace_of(left), self.workspace_of(right)) else {
            return;
        };
        if left == right || a != b {
            return;
        }
        self.with_workspace(&a, |w| {
            w.split = Some(SplitState {
                left,
                right,
                ratio: 0.5,
            })
        });
        self.activate_tab(left);
    }

    pub fn unsplit(&self) {
        let id = self.active_workspace_id();
        self.with_workspace(&id, |w| w.split = None);
        self.layout();
        self.mark_dirty();
    }

    pub fn set_split_ratio(&self, ratio: f64) {
        if !ratio.is_finite() {
            return;
        }
        let id = self.active_workspace_id();
        self.with_workspace(&id, |w| {
            if let Some(split) = w.split.as_mut() {
                split.ratio = ratio.clamp(0.2, 0.8);
            }
        });
        self.mark_dirty();
    }

    /// Clicking into a split pane makes that tab the active one.
    pub fn pane_focused(&self, id: u32) {
        let workspace_id = self.active_workspace_id();
        let should = self.active_workspace_state(|w| {
            w.split.is_some() && w.active_tab_id != Some(id) && w.in_split(id)
        });
        if should {
            self.set_active_in_workspace(&workspace_id, id);
            self.update_title();
            self.mark_dirty();
        }
    }

    /// The tab to split with when side-by-side is asked for without choosing.
    pub fn previous_tab(&self) -> Option<Rc<Tab>> {
        let id = self.active_workspace_state(|w| {
            w.recent
                .iter()
                .copied()
                .find(|t| Some(*t) != w.active_tab_id)
        })?;
        self.tab(id)
    }

    pub fn has_split(&self) -> bool {
        self.active_workspace_state(|w| w.split.is_some())
    }

    pub fn in_split(&self, id: u32) -> bool {
        self.workspaces.borrow().iter().any(|w| w.in_split(id))
    }

    // ── Layout ────────────────────────────────────────────────────────────────

    pub fn create_view(&self, related: Option<&WebView>) -> WebView {
        let manager = UserContentManager::new();
        shield::attach(&manager);
        crate::pagescripts::attach(&manager, self.incognito);
        let builder = WebView::builder()
            .user_content_manager(&manager)
            .settings(&self.web_settings);
        match related {
            // Views opened by a page share its process, as window.open needs.
            Some(opener) => builder.related_view(opener).build(),
            None => builder.web_context(&self.context).build(),
        }
    }

    /// Puts a new view in the window, hidden until a pane shows it.
    pub fn place_view(&self, view: &WebView) {
        self.fixed.put(view, 0, 0);
        view.hide();
    }

    pub fn set_panes(&self, rects: Vec<PaneRect>) {
        let tabs = self.tabs.borrow();
        let rects = rects
            .into_iter()
            .filter(|r| tabs.contains_key(&r.tab_id))
            .collect();
        drop(tabs);
        self.panes.replace(rects);
        self.layout();
    }

    /// Whether the tab is in the active workspace and on screen.
    pub fn is_shown(&self, id: u32) -> bool {
        self.active_workspace_state(|w| {
            if !w.tab_ids.contains(&id) {
                return false;
            }
            if w.active_tab_id == Some(id) {
                return true;
            }
            w.split.is_some_and(|s| {
                w.active_tab_id.is_some_and(|a| s.left == a || s.right == a)
                    && (s.left == id || s.right == id)
            })
        })
    }

    /// Places the views of the tabs on screen and hides the others.
    pub fn layout(&self) {
        let fullscreen = self.fullscreen_tab.get();
        let (width, height) = (
            self.overlay.allocated_width(),
            self.overlay.allocated_height(),
        );
        let panes = self.panes.borrow().clone();
        let tabs: Vec<Rc<Tab>> = self.tabs.borrow().values().cloned().collect();
        for tab in tabs {
            let Some(view) = tab.view() else { continue };
            let rect = if fullscreen == Some(tab.id) {
                Some((0, 0, width, height))
            } else if fullscreen.is_none() && tab.shows_web() && self.is_shown(tab.id) {
                panes.iter().find(|p| p.tab_id == tab.id).map(|p| {
                    (
                        p.x.round() as i32,
                        p.y.round() as i32,
                        p.width.round().max(1.0) as i32,
                        p.height.round().max(1.0) as i32,
                    )
                })
            } else {
                None
            };
            match rect {
                Some((x, y, w, h)) => {
                    // Shown first: GTK doesn't measure a hidden widget again when its size
                    // request changes, so it would come back at its old size.
                    view.show();
                    self.fixed.move_(&view, x, y);
                    view.set_size_request(w, h);
                }
                None => view.hide(),
            }
        }
    }

    /// The UI asks to be on top while a menu or dialog is open.
    pub fn set_overlay(&self, open: bool) {
        if self.overlay_open.replace(open) == open {
            return;
        }
        self.restack();
        if open {
            self.ui.grab_focus();
        } else {
            self.focus_active();
        }
    }

    /// Raises the UI over the pages without moving focus, for tooltips.
    pub fn set_raised(&self, raised: bool) {
        if self.raised.replace(raised) != raised {
            self.restack();
        }
    }

    fn restack(&self) {
        self.overlay
            .reorder_overlay(&self.ui, if self.ui_on_top() { -1 } else { 0 });
    }

    pub fn focus_active(&self) {
        match self
            .active_tab()
            .filter(|t| t.shows_web())
            .and_then(|t| t.view())
        {
            Some(view) if !self.overlay_open.get() => view.grab_focus(),
            _ => self.ui.grab_focus(),
        }
    }

    pub fn set_html_fullscreen(&self, id: u32, on: bool) {
        let current = self.fullscreen_tab.get();
        self.fullscreen_tab.set(if on {
            Some(id)
        } else if current == Some(id) {
            None
        } else {
            current
        });
        let _ = self.window.set_fullscreen(on);
        self.layout();
        self.mark_dirty();
    }

    pub fn toggle_fullscreen(&self) {
        let on = !self.window.is_fullscreen().unwrap_or(false);
        let _ = self.window.set_fullscreen(on);
        self.mark_dirty();
    }

    /// The tab views on screen, with where they are (for the harness).
    pub fn visible_views(&self) -> Vec<(WebView, PaneRect)> {
        let panes = self.panes.borrow();
        self.tabs
            .borrow()
            .values()
            .filter(|tab| tab.shows_web() && self.is_shown(tab.id))
            .filter_map(|tab| Some((tab.view()?, *panes.iter().find(|p| p.tab_id == tab.id)?)))
            .collect()
    }

    fn sleep_idle_tabs(&self) {
        let minutes = settings::get().sleep_after_minutes;
        if minutes == 0 {
            return;
        }
        let cutoff = now_ms().saturating_sub(u64::from(minutes) * 60_000);
        let tabs: Vec<Rc<Tab>> = self.tabs.borrow().values().cloned().collect();
        for tab in tabs {
            if tab.view().is_some() && !self.is_shown(tab.id) && tab.last_active.get() < cutoff {
                tab.sleep();
            }
        }
    }

    pub fn sleep_tab(&self, id: u32) {
        if !self.is_shown(id)
            && let Some(tab) = self.tab(id)
        {
            tab.sleep();
        }
    }

    // ── UI messaging ──────────────────────────────────────────────────────────

    pub fn emit(&self, name: &str, payload: impl Serialize) {
        let payload = json!({ "name": name, "payload": payload });
        if let Err(error) = self.app.emit_to(self.label.as_str(), "pw:event", payload) {
            eprintln!("[window] could not send {name}: {error}");
        }
    }

    /// Asks the UI to do something (open a menu, focus the address bar…).
    pub fn command(&self, command: Value) {
        self.ui.grab_focus();
        self.emit("command", command);
    }

    /// Collects changes into one state update for the UI.
    pub fn mark_dirty(&self) {
        if self.dirty.replace(true) {
            return;
        }
        let weak = self.weak.clone();
        glib::timeout_add_local_once(Duration::from_millis(16), move || {
            let Some(win) = weak.upgrade() else { return };
            win.dirty.set(false);
            win.emit("state", win.state());
            if !win.incognito {
                session::schedule_save();
            }
        });
    }

    pub fn update_title(&self) {
        let label = self
            .active_tab()
            .map(|tab| tab.display_title())
            .unwrap_or_default();
        let suffix = if self.incognito {
            format!("{} · {}", t("app.name"), t("incognito.label"))
        } else {
            t("app.name")
        };
        let title = if label.is_empty() {
            suffix
        } else {
            format!("{label} — {suffix}")
        };
        let _ = self.window.set_title(&title);
    }

    pub fn state(&self) -> WindowState {
        WindowState {
            window_id: self.id,
            incognito: self.incognito,
            maximized: self.window.is_maximized().unwrap_or(false),
            fullscreen: self.fullscreen_tab.get().is_some()
                || self.window.is_fullscreen().unwrap_or(false),
            workspaces: self
                .workspaces
                .borrow()
                .iter()
                .map(Workspace::state)
                .collect(),
            active_workspace_id: self.active_workspace_id(),
            tabs: self
                .tabs
                .borrow()
                .iter()
                .map(|(id, tab)| (*id, tab.state()))
                .collect(),
            permission: permissions::prompt_for(self),
            shield_paused: self
                .active_tab()
                .is_some_and(|tab| tab.page().is_none() && shield::paused_for(&tab.url())),
            extensions_partition: None,
        }
    }

    pub fn serialize(&self) -> SavedWindow {
        let maximized = self.window.is_maximized().unwrap_or(false);
        let bounds = (|| {
            let scale = self.window.scale_factor().ok()?;
            let position = self.window.outer_position().ok()?.to_logical::<f64>(scale);
            let size = self.window.inner_size().ok()?.to_logical::<f64>(scale);
            Some([position.x, position.y, size.width, size.height])
        })();
        let workspaces = self
            .workspaces
            .borrow()
            .iter()
            .map(|w| {
                let tabs: Vec<Rc<Tab>> = w.tab_ids.iter().filter_map(|id| self.tab(*id)).collect();
                let index_of = |id: u32| tabs.iter().position(|t| t.id == id);
                SavedWorkspace {
                    id: w.id.clone(),
                    name: w.name.clone(),
                    icon: w.icon.clone(),
                    active_index: w.active_tab_id.and_then(index_of).unwrap_or(0),
                    split: w.split.and_then(|s| {
                        Some(SavedSplit {
                            left: index_of(s.left)?,
                            right: index_of(s.right)?,
                            ratio: s.ratio,
                        })
                    }),
                    tabs: tabs.iter().map(|t| t.serialize()).collect(),
                }
            })
            .collect();
        SavedWindow {
            bounds: if maximized { None } else { bounds },
            maximized,
            active_workspace_id: self.active_workspace_id(),
            workspaces,
        }
    }

    pub fn initial_data(&self) -> InitialData {
        InitialData {
            state: self.state(),
            global: global_state(),
            settings: settings::get(),
            app: app_info(),
            bookmarks: serde_json::to_value(bookmarks::tree()).unwrap_or_default(),
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

/// A random id for a workspace, from the kernel's random source.
fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        bytes = (now_ms() as u128 ^ (NEXT_TAB.load(Ordering::Relaxed) as u128) << 64).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn global_state() -> GlobalState {
    GlobalState {
        focus: FocusState {
            active: false,
            started_at: 0,
            ends_at: 0,
            minutes: settings::get().focus_minutes,
        },
        update: updater::state(),
        downloads: downloads::recent(),
        version: display_version(),
    }
}

/// Something every window shows changed (an update's progress, downloads): send it to them.
pub fn global_changed() {
    let global = global_state();
    for win in all() {
        win.emit("global", &global);
    }
}

/// The version people see (0.2.0a): package.json's version with the release stage's suffix,
/// set by build.rs.
pub fn display_version() -> String {
    env!("PEEPWICK_VERSION").to_string()
}

pub fn app_info() -> AppInfo {
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
        stage: env!("PEEPWICK_STAGE"),
        webkit,
        tauri: tauri::VERSION.to_string(),
        platform: "linux",
        packaged: !cfg!(debug_assertions),
        is_default_browser: false,
        hardware_acceleration: settings::get().hardware_acceleration,
    }
}

/// Native parts (GTK dialogs, scrollbars) and pages' prefers-color-scheme follow the theme.
pub fn apply_theme() {
    let theme = settings::get().theme;
    if let Some(gtk_settings) = gtk::Settings::default()
        && theme != Theme::System
    {
        gtk_settings.set_gtk_application_prefer_dark_theme(theme == Theme::Dark);
    }
    let css = if theme == Theme::Light {
        ".peepwick-base { background-color: #E7E6E2; }"
    } else {
        ".peepwick-base { background-color: #0B0B0C; }"
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
    crate::pagescripts::refresh();
    for cell in [&BROWSING, &PRIVATE] {
        cell.with(|context| {
            if let Some(manager) = context
                .borrow()
                .as_ref()
                .and_then(|c| c.website_data_manager())
            {
                apply_cookie_policy(&manager);
            }
        });
    }
    for win in all() {
        win.web_settings.set_default_font_size(prefs.font_size);
        win.emit("settings", &prefs);
        win.mark_dirty();
    }
}

/// New site icons were stored: every UI loads its icon images again, once for a burst.
pub fn favicons_changed() {
    if FAVICON_PENDING.with(|pending| pending.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(Duration::from_millis(300), || {
        FAVICON_PENDING.with(|pending| pending.set(false));
        let version = FAVICON_VERSION.with(|v| {
            v.set(v.get() + 1);
            v.get()
        });
        for win in all() {
            win.emit("favicon", version);
        }
    });
}

/// Zoom levels changed in Settings: open pages take the new sizes.
pub fn zoom_changed() {
    for win in all() {
        let tabs: Vec<Rc<Tab>> = win.tabs.borrow().values().cloned().collect();
        for tab in tabs {
            tab.refresh_zoom();
        }
    }
}
