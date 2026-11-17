//! Autofill: what a page's form script reports (webext/forms.js, in the "forms" world of
//! webext/peepwick-webext.c), the list under a field (saved logins, a strong password, saved
//! addresses, values typed into the same kind of field before), filling what the person picks,
//! "Save password?" after a sign-in, and remembering what a sent form held (formdata.rs).
//!
//! The list is the interface's popup.html in a small view of its own over the page, so the page
//! keeps the keyboard: the page script passes on the keys that move through the list, and what
//! is typed, which narrows it. The page's address always comes from WebKit through the
//! extension, never from what the page says, and only a tab's main frame takes part.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib::{self, ToVariant};
use gtk::prelude::*;
use serde_json::{Value, json};
use tauri::AppHandle;
use webkit2gtk::{
    LoadEvent, SecurityManagerExt, URISchemeRequest, URISchemeRequestExt, UserContentManager,
    UserContentManagerExt, UserMessage, WebContext, WebContextExt, WebView, WebViewExt,
};

use crate::actions::open_page;
use crate::formdata::{self, Address};
use crate::i18n::{fold, t};
use crate::passwords::{self, Offer, PlainLogin};
use crate::tab::Tab;
use crate::window::{self, BrowserWindow};
use crate::{passwordcsv, settings};

/// The list's rows, as popup.css draws them.
const ROW: i32 = 40;
const MANAGE_ROW: i32 = 37;
const PADDING: i32 = 10;
/// A click in the list can blur the page first; the pick gets this long.
const BLUR_GRACE: Duration = Duration::from_millis(250);
/// An unanswered "Save password?" goes away after this long.
const PROMPT_LIFE: Duration = Duration::from_secs(180);
/// popup.html's way to the browser, and its address in a build.
const HANDLER: &str = "peepwickPopup";
const SCHEME: &str = "peepwick-ui";

#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Username,
    Password,
    NewPassword,
    /// Any other text field: addresses and values typed before.
    Text,
}

#[derive(Clone)]
enum Pick {
    Login(String),
    Generated(String),
    Value(String),
    Address(Box<Address>),
    Manage,
    ManageAddresses,
}

impl Pick {
    fn manages(&self) -> bool {
        matches!(self, Pick::Manage | Pick::ManageAddresses)
    }
}

struct Suggestion {
    /// An AutofillItem, as popup.ts draws it.
    item: Value,
    pick: Pick,
}

/// The field with focus in a window's tab, as its page reported it.
#[derive(Clone)]
struct Focus {
    tab_id: u32,
    origin: String,
    kind: FieldKind,
    /// The field, in the page's CSS pixels: x, y, width, height.
    rect: (f64, f64, f64, f64),
    /// For other text fields: what the field is called (to remember values by), what's in it,
    /// and which part of an address it asks for.
    key: String,
    value: String,
    address: Option<&'static str>,
}

/// The list a window shows under a field.
struct Active {
    focus: Focus,
    suggestions: Vec<Suggestion>,
    selected: i32,
}

/// A window's list view, and what to draw once its page says it's ready.
struct Popup {
    view: WebView,
    loaded: Cell<bool>,
    waiting: RefCell<Option<String>>,
}

impl Popup {
    fn run(&self, script: String) {
        if self.loaded.get() {
            self.view
                .evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
        } else {
            self.waiting.replace(Some(script));
        }
    }
}

/// A "Save password?" waiting for an answer.
struct Pending {
    win_id: u32,
    origin: String,
    username: String,
    password: String,
    timer: Option<glib::SourceId>,
}

thread_local! {
    /// Kept while the field has focus, list or not: typing can bring the list back.
    static FOCUS: RefCell<HashMap<u32, Focus>> = RefCell::new(HashMap::new());
    static ACTIVE: RefCell<HashMap<u32, Active>> = RefCell::new(HashMap::new());
    static POPUPS: RefCell<HashMap<u32, Rc<Popup>>> = RefCell::new(HashMap::new());
    static HIDE_TIMERS: RefCell<HashMap<u32, glib::SourceId>> = RefCell::new(HashMap::new());
    static PENDING: RefCell<HashMap<u32, Pending>> = RefCell::new(HashMap::new());
    static NEXT_PROMPT: Cell<u32> = const { Cell::new(1) };
    /// WebKit contexts the list's address works in.
    static SCHEMES: RefCell<Vec<WebContext>> = const { RefCell::new(Vec::new()) };
}

fn window_by_id(id: u32) -> Option<Rc<BrowserWindow>> {
    window::all().into_iter().find(|w| w.id == id)
}

/// https://example.com for a page's address; None for anything but a web page.
fn web_origin(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The page view of a window's tab.
fn page_of(win: &BrowserWindow, tab_id: u32) -> Option<WebView> {
    win.tab(tab_id).and_then(|tab| tab.view())
}

/// To the page's form script: what to fill, or the state of the list.
fn tell_page(page: &WebView, message: Value) {
    let message = UserMessage::new("forms", Some(&message.to_string().to_variant()));
    page.send_message_to_page(&message, None::<&gio::Cancellable>, |_| {});
}

// ── Messages from pages ─────────────────────────────────────────────────────────

/// A message from a tab's form script. `uri` is WebKit's address for the page.
pub fn page_message(tab: &Rc<Tab>, page: &WebView, kind: &str, payload: &str, uri: &str) {
    let Some(origin) = web_origin(uri) else {
        return;
    };
    let Some(win) = tab.window() else { return };
    let payload: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    if kind == "submit" {
        return submitted(&win, &origin, &payload);
    }
    // Only the tab on screen has a list, and only for the page it shows now: a message from
    // the last page can arrive after the next one started loading.
    let on_screen = win.active_tab().is_some_and(|active| active.id == tab.id)
        && tab.shown_view().as_ref() == Some(page)
        && page.uri().and_then(|u| web_origin(&u)).as_deref() == Some(origin.as_str());
    if !on_screen {
        return;
    }
    match kind {
        "focus" => focused(&win, tab, page, origin, &payload),
        "input" => typed(&win, tab, page, &payload),
        "blur" => blurred(&win),
        "key" => key(&win, payload.as_str().unwrap_or_default()),
        _ => {}
    }
}

/// The field's place in the page, from a page message.
fn rect_of(payload: &Value) -> Option<(f64, f64, f64, f64)> {
    let number = |key: &str| {
        payload
            .get("rect")
            .and_then(|r| r.get(key))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    Some((
        number("x")?,
        number("y")?,
        number("width")?,
        number("height")?,
    ))
}

fn focused(
    win: &Rc<BrowserWindow>,
    tab: &Rc<Tab>,
    page: &WebView,
    origin: String,
    payload: &Value,
) {
    cancel_hide(win.id);
    let kind = match payload.get("kind").and_then(Value::as_str) {
        Some("username") => FieldKind::Username,
        Some("password") => FieldKind::Password,
        Some("new-password") => FieldKind::NewPassword,
        Some("text") => FieldKind::Text,
        _ => {
            FOCUS.with(|focus| focus.borrow_mut().remove(&win.id));
            return hide(win);
        }
    };
    let Some(rect) = rect_of(payload) else {
        FOCUS.with(|focus| focus.borrow_mut().remove(&win.id));
        return hide(win);
    };
    let text = |key: &str, max: usize| {
        clip(
            payload.get(key).and_then(Value::as_str).unwrap_or_default(),
            max,
        )
    };
    let address = payload
        .get("address")
        .and_then(Value::as_str)
        .and_then(|part| formdata::ADDRESS_FIELDS.into_iter().find(|f| *f == part));
    let focus = Focus {
        tab_id: tab.id,
        origin,
        kind,
        rect,
        key: text("key", 100),
        value: text("value", 200),
        address,
    };
    FOCUS.with(|f| f.borrow_mut().insert(win.id, focus));
    refresh(win, page);
}

/// What's typed into a text field narrows its list, or brings it back.
fn typed(win: &Rc<BrowserWindow>, tab: &Rc<Tab>, page: &WebView, payload: &Value) {
    let updated = FOCUS.with(|focus| {
        let mut focus = focus.borrow_mut();
        let field = focus.get_mut(&win.id)?;
        // Logins and strong passwords don't narrow as you type.
        if field.tab_id != tab.id || field.kind != FieldKind::Text {
            return None;
        }
        field.value = clip(
            payload
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            200,
        );
        if let Some(rect) = rect_of(payload) {
            field.rect = rect;
        }
        Some(())
    });
    if updated.is_some() {
        cancel_hide(win.id);
        refresh(win, page);
    }
}

/// Shows the list for the window's field with what fits it now, or hides it.
fn refresh(win: &Rc<BrowserWindow>, page: &WebView) {
    let Some(focus) = FOCUS.with(|f| f.borrow().get(&win.id).cloned()) else {
        return hide(win);
    };
    if focus.kind == FieldKind::Text && settings::get().form_fill && !formdata::ensure_open() {
        // The saved data waits for the key: the list comes once it has been read.
        let (win_id, tab_id) = (win.id, focus.tab_id);
        formdata::when_open(false, move || {
            let still = FOCUS.with(|f| f.borrow().get(&win_id).is_some_and(|f| f.tab_id == tab_id));
            if let (true, true, Some(win)) = (formdata::ensure_open(), still, window_by_id(win_id))
                && let Some(page) = page_of(&win, tab_id)
            {
                refresh(&win, &page);
            }
        });
        return hide(win);
    }
    let suggestions = suggestions_for(win, &focus);
    if suggestions.is_empty() {
        return hide(win);
    }
    ACTIVE.with(|active| {
        active.borrow_mut().insert(
            win.id,
            Active {
                focus,
                suggestions,
                selected: -1,
            },
        )
    });
    show(win, page);
}

fn suggestions_for(win: &BrowserWindow, focus: &Focus) -> Vec<Suggestion> {
    let prefs = settings::get();
    let mut out = Vec::new();
    match focus.kind {
        FieldKind::Username | FieldKind::Password => {
            if !prefs.password_fill {
                return out;
            }
            for login in passwords::credentials_for(&focus.origin) {
                let detail = if login.exact {
                    "••••••••".to_string()
                } else {
                    format!("{} · ••••••••", crate::pages::host_of(&login.origin))
                };
                let label = if login.username.is_empty() {
                    t("autofill.noUsername")
                } else {
                    login.username
                };
                out.push(Suggestion {
                    item: json!({ "kind": "login", "label": label, "detail": detail }),
                    pick: Pick::Login(login.id),
                });
            }
        }
        FieldKind::NewPassword => {
            if !prefs.password_fill || win.incognito {
                return out;
            }
            let Some(password) = passwordcsv::generate_password() else {
                return out;
            };
            out.push(Suggestion {
                item: json!({ "kind": "generate", "label": t("autofill.generate"), "detail": password }),
                pick: Pick::Generated(password),
            });
        }
        FieldKind::Text => return text_suggestions(focus, prefs.form_fill, prefs.language),
    }
    if !out.is_empty() {
        out.push(Suggestion {
            item: json!({ "kind": "manage", "label": t("autofill.managePasswords") }),
            pick: Pick::Manage,
        });
    }
    out
}

/// For other text fields: saved addresses whose part for this field goes on from what's typed,
/// then values typed into fields of the same name before.
fn text_suggestions(focus: &Focus, form_fill: bool, lang: settings::Lang) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    if !form_fill {
        return out;
    }
    let typed = fold(focus.value.trim(), lang);
    if let Some(part) = focus.address {
        for address in formdata::addresses() {
            let value = address.field(part).to_string();
            if value.is_empty() || !fold(&value, lang).starts_with(&typed) {
                continue;
            }
            let detail: Vec<&str> = [&address.name, &address.street, &address.city]
                .into_iter()
                .map(String::as_str)
                .filter(|v| !v.is_empty() && *v != value)
                .collect();
            out.push(Suggestion {
                item: json!({ "kind": "address", "label": value, "detail": detail.join(", ") }),
                pick: Pick::Address(Box::new(address)),
            });
        }
    }
    if !focus.key.is_empty() {
        for value in formdata::suggest(&focus.key, &focus.value) {
            if out.iter().any(|s| s.item["label"] == value.as_str()) {
                continue;
            }
            out.push(Suggestion {
                item: json!({ "kind": "value", "label": value }),
                pick: Pick::Value(value),
            });
        }
    }
    if out.iter().any(|s| matches!(s.pick, Pick::Address(_))) {
        out.push(Suggestion {
            item: json!({ "kind": "manage", "label": t("autofill.manageAddresses") }),
            pick: Pick::ManageAddresses,
        });
    }
    out.truncate(8);
    out
}

fn blurred(win: &BrowserWindow) {
    cancel_hide(win.id);
    let win_id = win.id;
    let timer = glib::timeout_add_local_once(BLUR_GRACE, move || {
        HIDE_TIMERS.with(|timers| timers.borrow_mut().remove(&win_id));
        FOCUS.with(|focus| focus.borrow_mut().remove(&win_id));
        if let Some(win) = window_by_id(win_id) {
            hide(&win);
        }
    });
    HIDE_TIMERS.with(|timers| timers.borrow_mut().insert(win_id, timer));
}

fn cancel_hide(win_id: u32) {
    if let Some(timer) = HIDE_TIMERS.with(|timers| timers.borrow_mut().remove(&win_id)) {
        timer.remove();
    }
}

enum KeyAction {
    Render,
    Pick(usize),
    Hide,
}

fn key(win: &Rc<BrowserWindow>, key: &str) {
    let action = ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let state = active.get_mut(&win.id)?;
        let count = state.suggestions.len() as i32;
        Some(match key {
            "ArrowDown" => {
                state.selected = (state.selected + 1) % count;
                KeyAction::Render
            }
            "ArrowUp" => {
                state.selected = if state.selected <= 0 {
                    count - 1
                } else {
                    state.selected - 1
                };
                KeyAction::Render
            }
            "Enter" if state.selected >= 0 => KeyAction::Pick(state.selected as usize),
            "Escape" | "Tab" => KeyAction::Hide,
            _ => KeyAction::Render,
        })
    });
    match action {
        Some(KeyAction::Render) => render(win),
        Some(KeyAction::Pick(index)) => pick(win, index),
        Some(KeyAction::Hide) => hide(win),
        None => {}
    }
}

// ── The list ────────────────────────────────────────────────────────────────────

/// popup.html: from the development server while developing, from the build's own files after.
fn popup_url(win: &BrowserWindow) -> String {
    if tauri::is_dev()
        && let Some(url) = win
            .app_handle()
            .config()
            .build
            .dev_url
            .as_ref()
            .and_then(|dev| dev.join("popup.html").ok())
    {
        return url.to_string();
    }
    format!("{SCHEME}://ui/popup.html")
}

/// Serves the interface's own files (popup.html and what it loads) at peepwick-ui://ui/.
fn register_scheme(app: AppHandle, context: &WebContext) {
    if SCHEMES.with(|known| known.borrow().contains(context)) {
        return;
    }
    SCHEMES.with(|known| known.borrow_mut().push(context.clone()));
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(SCHEME);
        security.register_uri_scheme_as_cors_enabled(SCHEME);
    }
    context.register_uri_scheme(SCHEME, move |request| serve(&app, request));
}

fn serve(app: &AppHandle, request: &URISchemeRequest) {
    let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
    let path = request
        .path()
        .map(|p| p.trim_start_matches('/').to_string())
        .unwrap_or_default();
    if crate::pages::host_of(&uri) != "ui" || path.is_empty() {
        return crate::reader::not_found(request);
    }
    match app.asset_resolver().get(path) {
        Some(asset) => {
            let mime = asset.mime_type().to_string();
            crate::reader::respond(request, glib::Bytes::from_owned(asset.bytes), &mime);
        }
        None => crate::reader::not_found(request),
    }
}

fn popup_for(win: &Rc<BrowserWindow>) -> Option<Rc<Popup>> {
    if let Some(popup) = POPUPS.with(|popups| popups.borrow().get(&win.id).cloned()) {
        return Some(popup);
    }
    let context = win.ui_context()?;
    register_scheme(win.app_handle(), &context);
    let manager = UserContentManager::new();
    manager.register_script_message_handler(HANDLER);
    let win_id = win.id;
    manager.connect_script_message_received(Some(HANDLER), move |_, result| {
        let text = result
            .js_value()
            .map(|value| javascriptcore::ValueExt::to_str(&value).to_string())
            .unwrap_or_default();
        popup_message(win_id, &text);
    });
    let view = WebView::builder()
        .web_context(&context)
        .user_content_manager(&manager)
        .build();
    view.set_background_color(&gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    // The page being filled keeps the keyboard.
    view.set_can_focus(false);
    let popup = Rc::new(Popup {
        view: view.clone(),
        loaded: Cell::new(false),
        waiting: RefCell::new(None),
    });
    // Loading again (its process went away): what comes waits for "ready" again.
    let weak = Rc::downgrade(&popup);
    view.connect_load_changed(move |_, event| {
        if event == LoadEvent::Started
            && let Some(popup) = weak.upgrade()
        {
            popup.loaded.set(false);
        }
    });
    view.load_uri(&popup_url(win));
    POPUPS.with(|popups| popups.borrow_mut().insert(win.id, popup.clone()));
    Some(popup)
}

/// Places the list under its field, or over it when there's no room below.
fn show(win: &Rc<BrowserWindow>, page: &WebView) {
    let Some((rect, rows, manage)) = ACTIVE.with(|active| {
        active.borrow().get(&win.id).map(|state| {
            let rows = state
                .suggestions
                .iter()
                .filter(|s| !s.pick.manages())
                .count() as i32;
            (
                state.focus.rect,
                rows,
                rows < state.suggestions.len() as i32,
            )
        })
    }) else {
        return;
    };
    let Some(popup) = popup_for(win) else { return };
    let Some((page_x, page_y)) = page.translate_coordinates(win.popup_layer(), 0, 0) else {
        return;
    };
    let zoom = page.zoom_level();
    let (fx, fy, fw, fh) = rect;
    let height = rows * ROW + if manage { MANAGE_ROW } else { 0 } + PADDING;
    let width = ((fw * zoom).round() as i32).clamp(300, 440);
    let (area_width, area_height) = win.content_size();
    let mut y = page_y + ((fy + fh) * zoom).round() as i32 + 2;
    if y + height > area_height - 8 {
        y = page_y + (fy * zoom).round() as i32 - height - 2;
    }
    let x = (page_x + (fx * zoom).round() as i32)
        .min(area_width - width - 8)
        .max(8);
    win.show_popup(&popup.view, x, y.max(8), width, height);
    render(win);
}

fn render(win: &BrowserWindow) {
    let Some((state, selected, tab_id)) = ACTIVE.with(|active| {
        active.borrow().get(&win.id).map(|s| {
            let items: Vec<Value> = s.suggestions.iter().map(|x| x.item.clone()).collect();
            (
                json!({
                    "items": items,
                    "selected": s.selected,
                    "dark": win.is_dark(),
                    "lang": settings::get().language,
                }),
                s.selected,
                s.focus.tab_id,
            )
        })
    }) else {
        return;
    };
    if let Some(popup) = POPUPS.with(|popups| popups.borrow().get(&win.id).cloned()) {
        popup.run(format!(
            "window.__peepwickAutofill && window.__peepwickAutofill({state})"
        ));
    }
    if let Some(page) = page_of(win, tab_id) {
        tell_page(
            &page,
            json!({ "type": "popup", "open": true, "selected": selected >= 0 }),
        );
    }
}

/// Closes the window's list, if it's open.
pub fn hide(win: &BrowserWindow) {
    cancel_hide(win.id);
    let state = ACTIVE.with(|active| active.borrow_mut().remove(&win.id));
    if let Some(popup) = POPUPS.with(|popups| popups.borrow().get(&win.id).cloned()) {
        popup.view.hide();
    }
    if let Some(page) = state.and_then(|state| page_of(win, state.focus.tab_id)) {
        tell_page(&page, json!({ "type": "popup", "open": false }));
    }
}

/// Closes the list if it belongs to this tab: its page is going.
pub fn hide_tab(win: &BrowserWindow, tab_id: u32) {
    FOCUS.with(|focus| {
        let mut focus = focus.borrow_mut();
        if focus.get(&win.id).is_some_and(|f| f.tab_id == tab_id) {
            focus.remove(&win.id);
        }
    });
    if ACTIVE.with(|active| {
        active
            .borrow()
            .get(&win.id)
            .is_some_and(|a| a.focus.tab_id == tab_id)
    }) {
        hide(win);
    }
}

/// A window closed: its list and its questions go with it.
pub fn forget(win_id: u32) {
    cancel_hide(win_id);
    FOCUS.with(|focus| focus.borrow_mut().remove(&win_id));
    ACTIVE.with(|active| active.borrow_mut().remove(&win_id));
    POPUPS.with(|popups| popups.borrow_mut().remove(&win_id));
    let ids: Vec<u32> = PENDING.with(|pending| {
        pending
            .borrow()
            .iter()
            .filter(|(_, p)| p.win_id == win_id)
            .map(|(id, _)| *id)
            .collect()
    });
    for id in ids {
        if let Some(timer) =
            PENDING.with(|pending| pending.borrow_mut().remove(&id).and_then(|p| p.timer))
        {
            timer.remove();
        }
    }
}

/// From popup.html: ready to draw, a row picked, or the pointer over one.
fn popup_message(win_id: u32, text: &str) {
    let Some(win) = window_by_id(win_id) else {
        return;
    };
    let message: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    if message.get("method").and_then(Value::as_str) == Some("ready") {
        if let Some(popup) = POPUPS.with(|popups| popups.borrow().get(&win_id).cloned()) {
            popup.loaded.set(true);
            if let Some(script) = popup.waiting.take() {
                popup.run(script);
            }
        }
        return;
    }
    let Some(index) = message.get("index").and_then(Value::as_u64) else {
        return;
    };
    let index = index as usize;
    match message.get("method").and_then(Value::as_str) {
        Some("autofillPick") => pick(&win, index),
        Some("autofillHover") => {
            let changed = ACTIVE.with(|active| {
                let mut active = active.borrow_mut();
                let state = active.get_mut(&win.id)?;
                (index < state.suggestions.len()).then(|| state.selected = index as i32)
            });
            if changed.is_some() {
                render(&win);
            }
        }
        _ => {}
    }
}

fn pick(win: &Rc<BrowserWindow>, index: usize) {
    let Some((tab_id, origin, pick)) = ACTIVE.with(|active| {
        active.borrow().get(&win.id).and_then(|state| {
            state.suggestions.get(index).map(|s| {
                (
                    state.focus.tab_id,
                    state.focus.origin.clone(),
                    s.pick.clone(),
                )
            })
        })
    }) else {
        return;
    };
    match pick {
        Pick::Login(id) => {
            let win_id = win.id;
            passwords::when_ready(false, move || {
                let Some(win) = window_by_id(win_id) else {
                    return;
                };
                match passwords::login_for(&id, &origin) {
                    Some((username, password)) => fill(
                        &win,
                        tab_id,
                        &origin,
                        json!({ "type": "login", "username": username, "password": password }),
                    ),
                    None => hide(&win),
                }
            });
        }
        Pick::Generated(password) => fill(
            win,
            tab_id,
            &origin,
            json!({ "type": "generated", "password": password }),
        ),
        Pick::Value(value) => fill(
            win,
            tab_id,
            &origin,
            json!({ "type": "value", "value": value }),
        ),
        Pick::Address(address) => fill(
            win,
            tab_id,
            &origin,
            json!({ "type": "address", "address": address }),
        ),
        Pick::Manage => {
            hide(win);
            open_page(win, "passwords", "");
        }
        Pick::ManageAddresses => {
            hide(win);
            open_page(win, "settings", "autofill");
        }
    }
}

/// Fills the page, if it's still at the address the list was for.
fn fill(win: &BrowserWindow, tab_id: u32, origin: &str, message: Value) {
    let page = page_of(win, tab_id)
        .filter(|page| page.uri().and_then(|u| web_origin(&u)).as_deref() == Some(origin));
    if let Some(page) = &page {
        tell_page(page, message);
    }
    hide(win);
    if let Some(page) = page {
        page.grab_focus();
    }
}

/// For the test harness: the window's list view.
#[cfg(any(debug_assertions, feature = "harness"))]
pub fn popup_view(win: &BrowserWindow) -> Option<WebView> {
    POPUPS.with(|popups| popups.borrow().get(&win.id).map(|p| p.view.clone()))
}

/// For the test harness: what the list offers and where it sits.
#[cfg(any(debug_assertions, feature = "harness"))]
pub fn describe(win: &BrowserWindow) -> String {
    let active = ACTIVE.with(|active| {
        active.borrow().get(&win.id).map(|a| {
            let labels: Vec<String> = a
                .suggestions
                .iter()
                .map(|s| format!("{}:{}", s.item["kind"], s.item["label"]))
                .collect();
            format!(
                "tab={} origin={} selected={} items={labels:?}",
                a.focus.tab_id, a.focus.origin, a.selected
            )
        })
    });
    let place = popup_view(win).filter(|v| v.is_visible()).map(|v| {
        let parent = v.parent();
        let (x, y) = parent
            .and_then(|p| p.downcast::<gtk::Fixed>().ok())
            .map(|f| (f.child_x(&v), f.child_y(&v)))
            .unwrap_or_default();
        format!("at {x},{y} size {:?}", v.size_request())
    });
    format!(
        "{} {}",
        active.unwrap_or_else(|| "none".into()),
        place.unwrap_or_default()
    )
}

// ── Saving after a sign-in ─────────────────────────────────────────────────────

fn submitted(win: &BrowserWindow, origin: &str, payload: &Value) {
    if win.incognito {
        return;
    }
    let prefs = settings::get();
    // What the form's text fields held, to suggest again.
    if prefs.form_fill {
        let sent: Vec<(String, String)> = payload
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .take(50)
                    .filter_map(|e| {
                        let key = e.get("key")?.as_str()?;
                        Some((clip(key, 100), clip(e.get("value")?.as_str()?, 500)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !sent.is_empty() {
            formdata::when_open(true, move || formdata::remember(&sent));
        }
    }
    if !prefs.password_save {
        return;
    }
    let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or_default();
    let password = clip(text("password"), 1000);
    if password.is_empty() {
        return;
    }
    let username = clip(text("username"), 500);
    let (win_id, origin) = (win.id, origin.to_string());
    passwords::when_ready(false, move || {
        let offer = passwords::after_submit(&origin, &username, &password);
        if offer == Offer::None {
            return;
        }
        if let Some(win) = window_by_id(win_id) {
            offer_save(&win, origin, username, password, offer == Offer::Update);
        }
    });
}

fn offer_save(
    win: &BrowserWindow,
    origin: String,
    username: String,
    password: String,
    update: bool,
) {
    // One question per window: a new sign-in replaces the last one.
    let old: Vec<u32> = PENDING.with(|pending| {
        pending
            .borrow()
            .iter()
            .filter(|(_, p)| p.win_id == win.id)
            .map(|(id, _)| *id)
            .collect()
    });
    for id in old {
        answer(id, "dismiss");
    }
    let id = NEXT_PROMPT.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    let timer = glib::timeout_add_local_once(PROMPT_LIFE, move || {
        PENDING.with(|pending| {
            if let Some(p) = pending.borrow_mut().get_mut(&id) {
                p.timer = None;
            }
        });
        answer(id, "dismiss");
    });
    let host = crate::pages::host_of(&origin);
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    win.command(json!({
        "type": "save-login",
        "prompt": { "id": id, "host": host, "username": username, "update": update },
    }));
    PENDING.with(|pending| {
        pending.borrow_mut().insert(
            id,
            Pending {
                win_id: win.id,
                origin,
                username,
                password,
                timer: Some(timer),
            },
        )
    });
}

/// The answer to "Save password?": save, never for this site, or not now.
pub fn answer(id: u32, answer: &str) {
    let Some(entry) = PENDING.with(|pending| pending.borrow_mut().remove(&id)) else {
        return;
    };
    if let Some(timer) = entry.timer {
        timer.remove();
    }
    let Pending {
        win_id,
        origin,
        username,
        password,
        ..
    } = entry;
    if let Some(win) = window_by_id(win_id) {
        win.command(json!({ "type": "save-login", "prompt": null }));
    }
    match answer {
        "save" => passwords::when_ready(true, move || {
            let saved = passwords::add(&PlainLogin {
                origin,
                username,
                password,
                ..PlainLogin::default()
            });
            if saved.is_none()
                && let Some(win) = window_by_id(win_id)
            {
                win.command(json!({ "type": "toast", "text": t("save.failed") }));
            }
        }),
        "never" => passwords::never_save(&origin),
        _ => {}
    }
}
