//! Menus and confirmations drawn by the browser UI (MenuModel and ConfirmModel in
//! `src/shared/types.ts`). The Rust side keeps the actions; the UI gets labels and ids and
//! says which one was chosen. Native GTK menus would take the desktop's theme instead of
//! the browser's.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::window::BrowserWindow;

/// Modifier keys held while choosing, for "open in a new tab" style actions.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClickEvent {
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub meta_key: bool,
}

type Action = Rc<dyn Fn(ClickEvent)>;
type ContextMenu = Rc<dyn Fn() -> Vec<MenuSpec>>;

/// One menu entry. Built with the helper functions: `item`, `submenu`, `separator`.
#[derive(Clone, Default)]
pub struct MenuSpec {
    label: String,
    separator: bool,
    check: bool,
    checked: bool,
    disabled: bool,
    accelerator: Option<&'static str>,
    favicon: Option<String>,
    folder: bool,
    submenu: Option<Vec<MenuSpec>>,
    click: Option<Action>,
    context: Option<ContextMenu>,
}

pub fn item(label: impl Into<String>, click: impl Fn(ClickEvent) + 'static) -> MenuSpec {
    MenuSpec {
        label: label.into(),
        click: Some(Rc::new(click)),
        ..MenuSpec::default()
    }
}

pub fn submenu(label: impl Into<String>, items: Vec<MenuSpec>) -> MenuSpec {
    MenuSpec {
        label: label.into(),
        submenu: Some(items),
        ..MenuSpec::default()
    }
}

pub fn separator() -> MenuSpec {
    MenuSpec {
        separator: true,
        ..MenuSpec::default()
    }
}

impl MenuSpec {
    pub fn accelerator(mut self, keys: &'static str) -> Self {
        self.accelerator = Some(keys);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.disabled = !enabled;
        self
    }

    pub fn checkbox(mut self, checked: bool) -> Self {
        self.check = true;
        self.checked = checked;
        self
    }

    /// Shows the site icon of this address.
    pub fn favicon(mut self, url: impl Into<String>) -> Self {
        self.favicon = Some(url.into());
        self
    }

    pub fn folder(mut self) -> Self {
        self.folder = true;
        self
    }

    /// The item's own right-click menu, opened on top of this one.
    pub fn context(mut self, build: impl Fn() -> Vec<MenuSpec> + 'static) -> Self {
        self.context = Some(Rc::new(build));
        self
    }
}

#[derive(Default)]
struct OpenMenu {
    actions: HashMap<u32, Action>,
    contexts: HashMap<u32, ContextMenu>,
}

thread_local! {
    /// Each window's open menus: one, or a folder menu with a right-click menu on top.
    static OPEN: RefCell<HashMap<u32, HashMap<u32, OpenMenu>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    static PENDING: RefCell<HashMap<u32, Box<dyn FnOnce(bool)>>> = RefCell::new(HashMap::new());
}

fn next_id() -> u32 {
    NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

fn build(specs: Vec<MenuSpec>, menu: &mut OpenMenu) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for spec in specs {
        if spec.separator {
            if out.last().is_some_and(|last| last["kind"] != "separator") {
                out.push(json!({ "kind": "separator" }));
            }
            continue;
        }
        let id = next_id();
        if let Some(click) = spec.click {
            menu.actions.insert(id, click);
        }
        let has_context = spec.context.is_some();
        if let Some(context) = spec.context {
            menu.contexts.insert(id, context);
        }
        let submenu = spec.submenu.map(|items| build(items, menu));
        let kind = if submenu.is_some() {
            "submenu"
        } else if spec.check {
            "check"
        } else {
            "item"
        };
        let enabled = !spec.disabled && submenu.as_ref().is_none_or(|items| !items.is_empty());
        let mut node = json!({
            "kind": kind,
            "id": id,
            "label": spec.label.chars().take(200).collect::<String>(),
            "enabled": enabled,
        });
        if let Some(keys) = spec.accelerator {
            node["accelerator"] = json!(keys);
        }
        if spec.check {
            node["checked"] = json!(spec.checked);
        }
        if let Some(url) = spec.favicon {
            node["favicon"] = json!(url);
        }
        if spec.folder {
            node["folder"] = json!(true);
        }
        if let Some(items) = submenu {
            node["submenu"] = json!(items);
        }
        if has_context {
            node["context"] = json!(true);
        }
        out.push(node);
    }
    while out.last().is_some_and(|last| last["kind"] == "separator") {
        out.pop();
    }
    out
}

/// Opens a menu at window coordinates (the top right corner with `align_end`). A new menu
/// replaces the open ones, unless it opens `over` one of them.
pub fn popup(
    win: &BrowserWindow,
    specs: Vec<MenuSpec>,
    x: f64,
    y: f64,
    align_end: bool,
    over: Option<u32>,
) {
    let mut menu = OpenMenu::default();
    let items = build(specs, &mut menu);
    if items.is_empty() {
        return;
    }
    let id = next_id();
    OPEN.with(|open| {
        let mut open = open.borrow_mut();
        let windows = open.entry(win.id).or_default();
        if over.is_none_or(|over| !windows.contains_key(&over)) {
            windows.clear();
        }
        windows.insert(id, menu);
    });
    let mut model = json!({ "id": id, "items": items, "x": x.round(), "y": y.round() });
    if align_end {
        model["align"] = json!("end");
    }
    if let Some(over) = over {
        model["over"] = json!(over);
    }
    win.command(json!({ "type": "menu", "menu": model }));
}

pub fn select(win: &Rc<BrowserWindow>, menu_id: u32, item_id: u32, event: ClickEvent) {
    let action = OPEN.with(|open| {
        let mut open = open.borrow_mut();
        let windows = open.get_mut(&win.id)?;
        let action = windows.get(&menu_id)?.actions.get(&item_id).cloned();
        // Choosing anything closes every open menu.
        windows.clear();
        action
    });
    if let Some(action) = action {
        // Lower the UI first, so focus the action moves (a new window, the page) stays put.
        win.set_overlay(false);
        action(event);
    }
}

pub fn context(win: &BrowserWindow, menu_id: u32, item_id: u32, x: f64, y: f64) {
    let build = OPEN.with(|open| {
        open.borrow()
            .get(&win.id)?
            .get(&menu_id)?
            .contexts
            .get(&item_id)
            .cloned()
    });
    if let Some(build) = build {
        popup(win, build(), x, y, false, Some(menu_id));
    }
}

pub fn closed(win: &BrowserWindow, menu_id: u32) {
    OPEN.with(|open| {
        if let Some(windows) = open.borrow_mut().get_mut(&win.id) {
            windows.remove(&menu_id);
        }
    });
}

/// Forgets a window's menus once it closes.
pub fn window_closed(window_id: u32) {
    OPEN.with(|open| open.borrow_mut().remove(&window_id));
}

/// A yes/no question in the window's own UI.
pub struct Question {
    pub title: String,
    pub body: Option<String>,
    pub confirm: String,
    pub cancel: String,
    pub danger: bool,
}

pub fn confirm(win: &BrowserWindow, question: Question, done: impl FnOnce(bool) + 'static) {
    let id = next_id();
    PENDING.with(|pending| pending.borrow_mut().insert(id, Box::new(done)));
    let mut dialog = json!({
        "id": id,
        "title": question.title,
        "confirm": question.confirm,
        "cancel": question.cancel,
        "danger": question.danger,
    });
    if let Some(body) = question.body {
        dialog["body"] = json!(body);
    }
    win.command(json!({ "type": "confirm", "dialog": dialog }));
}

pub fn confirm_reply(id: u32, ok: bool) {
    if let Some(done) = PENDING.with(|pending| pending.borrow_mut().remove(&id)) {
        done(ok);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_nodes_like_the_ui_expects() {
        let mut menu = OpenMenu::default();
        let nodes = build(
            vec![
                separator(),
                item("Yeni sekme", |_| {}).accelerator("Ctrl+T"),
                separator(),
                separator(),
                submenu("Boş", vec![]),
                item("Çubuk", |_| {}).checkbox(true).enabled(false),
                separator(),
            ],
            &mut menu,
        );
        let kinds: Vec<&str> = nodes.iter().map(|n| n["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["item", "separator", "submenu", "check"]);
        assert_eq!(nodes[0]["accelerator"], "Ctrl+T");
        assert_eq!(nodes[2]["enabled"], false);
        assert_eq!(nodes[3]["checked"], true);
        assert_eq!(menu.actions.len(), 2);
    }
}
