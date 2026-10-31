//! Bookmarks: a JSON tree in the profile (bookmarks.json, the file the Electron version kept),
//! edited only here and sent to every window's UI when it changes. Netscape HTML files, the
//! format every browser imports and exports, are read and written here too.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::{new_window, open_page};
use crate::i18n::{Var, fold, t, tv};
use crate::menu::{self, ClickEvent, MenuSpec, item, separator, submenu};
use crate::menus::{copy_text, on_window};
use crate::tab::now_ms;
use crate::window::{self, BrowserWindow, OpenOptions};
use crate::{paths, settings, store};

/// Asking first before opening more links than this at once.
const OPEN_ALL_CONFIRM: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "url")]
    Link,
    #[serde(rename = "folder")]
    Folder,
}

/// A bookmark or a folder, as `BookmarkNode` in `src/shared/bookmarks.ts`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    #[serde(rename = "type")]
    pub kind: Kind,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub added_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Node>>,
}

impl Node {
    fn folder(id: String, title: String, added_at: u64) -> Node {
        Node {
            kind: Kind::Folder,
            id,
            title,
            url: None,
            added_at,
            children: Some(Vec::new()),
        }
    }

    fn link(id: String, title: String, url: String, added_at: u64) -> Node {
        Node {
            kind: Kind::Link,
            id,
            title,
            url: Some(url),
            added_at,
            children: None,
        }
    }

    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }

    fn children(&self) -> &[Node] {
        self.children.as_deref().unwrap_or(&[])
    }

    fn children_mut(&mut self) -> &mut Vec<Node> {
        self.children.get_or_insert_with(Vec::new)
    }

    fn url(&self) -> &str {
        self.url.as_deref().unwrap_or("")
    }

    fn valid(&self) -> bool {
        match self.kind {
            Kind::Link => self.url.is_some(),
            Kind::Folder => self
                .children
                .as_ref()
                .is_some_and(|c| c.iter().all(Node::valid)),
        }
    }
}

/// The two roots, as in Chromium: the bookmarks bar and "other bookmarks".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tree {
    pub bar: Node,
    pub other: Node,
}

impl Tree {
    fn empty() -> Tree {
        Tree {
            bar: Node::folder("bar".into(), String::new(), 0),
            other: Node::folder("other".into(), String::new(), 0),
        }
    }

    fn roots(&self) -> [&Node; 2] {
        [&self.bar, &self.other]
    }

    fn root_mut(&mut self, index: usize) -> &mut Node {
        if index == 0 {
            &mut self.bar
        } else {
            &mut self.other
        }
    }

    fn valid(&self) -> bool {
        self.bar.id == "bar"
            && self.other.id == "other"
            && self.bar.is_folder()
            && self.other.is_folder()
            && self.bar.valid()
            && self.other.valid()
    }
}

/// Imported bookmarks before they get ids.
#[derive(Clone, Debug, PartialEq)]
pub enum Draft {
    Link {
        title: String,
        url: String,
        added_at: Option<u64>,
    },
    Folder {
        title: String,
        added_at: Option<u64>,
        children: Vec<Draft>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DraftTree {
    pub bar: Vec<Draft>,
    pub other: Vec<Draft>,
}

thread_local! {
    static TREE: RefCell<Tree> = RefCell::new(Tree::empty());
    /// The last deletion, for "Undo" in the toast: the node, its folder and its place.
    static LAST_REMOVED: RefCell<Vec<(Node, String, usize)>> = const { RefCell::new(Vec::new()) };
    /// The folder the last bookmark went into; Ctrl+D uses it again.
    static LAST_FOLDER: RefCell<String> = RefCell::new("bar".into());
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> std::path::PathBuf {
    paths::user_path("bookmarks.json")
}

pub fn load() {
    if let Some(tree) = store::read_json::<Tree>(&file()).filter(Tree::valid) {
        TREE.with(|cell| cell.replace(tree));
    }
}

pub fn tree() -> Tree {
    TREE.with(|cell| cell.borrow().clone())
}

fn save() {
    match serde_json::to_vec(&tree()) {
        Ok(json) => {
            if let Err(error) = store::write_atomic(&file(), &json) {
                eprintln!("[bookmarks] could not save: {error}");
            }
        }
        Err(error) => eprintln!("[bookmarks] could not serialize: {error}"),
    }
}

/// Writes a change that is still waiting, before the browser quits.
pub fn flush() {
    if SAVE_PENDING.with(|pending| pending.replace(false)) {
        save();
    }
}

/// Every window's UI gets the new tree now; the file is written a moment later.
fn changed() {
    let tree = tree();
    for win in window::all() {
        win.emit("bookmarks", &tree);
    }
    if !SAVE_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_millis(400), flush);
    }
}

/// A random id in the UUID format the Electron version used.
fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        bytes = (now_ms() as u128 ^ (u128::from(std::process::id()) << 64)).to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub fn bookmarkable(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http:", "https:", "ftp:", "file:"]
        .iter()
        .any(|s| lower.starts_with(s))
}

/// For duplicate checks: no fragment, no lone trailing slash.
pub fn url_key(url: &str) -> String {
    match url::Url::parse(url.trim()) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            let mut text = parsed.to_string();
            if text.ends_with('/') && parsed.path() == "/" && parsed.query().is_none() {
                text.pop();
            }
            text
        }
        Err(_) => url.trim().to_string(),
    }
}

// ── Lookup ────────────────────────────────────────────────────────────────────

/// Where a node is: the root (0 for the bar, 1 for other bookmarks), then child indices.
type Path = Vec<usize>;

fn find(tree: &Tree, id: &str) -> Option<Path> {
    fn search(folder: &Node, id: &str, path: &mut Path) -> bool {
        for (i, node) in folder.children().iter().enumerate() {
            path.push(i);
            if node.id == id || (node.is_folder() && search(node, id, path)) {
                return true;
            }
            path.pop();
        }
        false
    }
    for (r, root) in tree.roots().into_iter().enumerate() {
        let mut path = vec![r];
        if root.id == id || search(root, id, &mut path) {
            return Some(path);
        }
    }
    None
}

fn at<'a>(tree: &'a Tree, path: &[usize]) -> &'a Node {
    path[1..]
        .iter()
        .fold(tree.roots()[path[0]], |node, &i| &node.children()[i])
}

fn at_mut<'a>(tree: &'a mut Tree, path: &[usize]) -> &'a mut Node {
    let mut node = tree.root_mut(path[0]);
    for &i in &path[1..] {
        node = &mut node.children_mut()[i];
    }
    node
}

fn folder_path(tree: &Tree, id: &str) -> Option<Path> {
    find(tree, id).filter(|path| at(tree, path).is_folder())
}

/// True when `id` is `ancestor` or lies inside it.
fn is_inside(tree: &Tree, id: &str, ancestor: &str) -> bool {
    let Some(path) = find(tree, id) else {
        return false;
    };
    (1..=path.len()).any(|n| at(tree, &path[..n]).id == ancestor)
}

fn clamp(len: usize, index: Option<usize>) -> usize {
    index.map_or(len, |i| i.min(len))
}

fn walk_links(node: &Node, out: &mut Vec<Node>) {
    for child in node.children() {
        if child.is_folder() {
            walk_links(child, out);
        } else {
            out.push(child.clone());
        }
    }
}

pub fn links() -> Vec<Node> {
    let tree = tree();
    let mut out = Vec::new();
    walk_links(&tree.bar, &mut out);
    walk_links(&tree.other, &mut out);
    out
}

pub fn bookmarked_keys() -> HashSet<String> {
    links().iter().map(|link| url_key(link.url())).collect()
}

/// Folders as a flat list with their depth, for "Move to".
fn folder_list(tree: &Tree) -> Vec<(Node, usize)> {
    fn go(folder: &Node, depth: usize, out: &mut Vec<(Node, usize)>) {
        out.push((folder.clone(), depth));
        for node in folder.children().iter().filter(|n| n.is_folder()) {
            go(node, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    go(&tree.bar, 0, &mut out);
    go(&tree.other, 0, &mut out);
    out
}

/// Bookmarks matching what was typed, for the address bar and the command bar.
pub fn matching(text: &str, limit: usize) -> Vec<Node> {
    let strip = |s: &str| {
        let s = s
            .strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
            .unwrap_or(s);
        s.strip_prefix("www.").unwrap_or(s).to_string()
    };
    let needle = strip(&fold(text.trim(), settings::Lang::Tr));
    if needle.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u8, Node)> = links()
        .into_iter()
        .filter_map(|link| {
            let title = fold(&link.title, settings::Lang::Tr);
            let url = strip(&link.url().to_lowercase());
            let score = if url.starts_with(&needle) {
                3
            } else if title.starts_with(&needle) {
                2
            } else if title.contains(&needle) || url.contains(&needle) {
                1
            } else {
                0
            };
            (score > 0).then_some((score, link))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, link)| link)
        .collect()
}

// ── Editing ───────────────────────────────────────────────────────────────────

/// Adds a bookmark (with an address) or a folder; returns its id.
pub fn create(
    parent_id: &str,
    index: Option<usize>,
    title: &str,
    url: Option<&str>,
) -> Option<String> {
    let title: String = title.trim().chars().take(500).collect();
    let node = match url.map(str::trim) {
        Some(url) => {
            if !bookmarkable(url) || url.len() > 8192 {
                return None;
            }
            Node::link(
                new_id(),
                if title.is_empty() {
                    url.to_string()
                } else {
                    title
                },
                url.to_string(),
                now_ms(),
            )
        }
        None => Node::folder(
            new_id(),
            if title.is_empty() {
                t("bookmarks.newFolder")
            } else {
                title
            },
            now_ms(),
        ),
    };
    let id = node.id.clone();
    let is_link = !node.is_folder();
    let added = TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        let path = folder_path(&tree, parent_id)?;
        let children = at_mut(&mut tree, &path).children_mut();
        let position = clamp(children.len(), index);
        children.insert(position, node);
        Some(())
    });
    added?;
    if is_link {
        LAST_FOLDER.with(|last| last.replace(parent_id.to_string()));
    }
    changed();
    Some(id)
}

pub fn update(id: &str, title: Option<&str>, url: Option<&str>) {
    let updated = TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        let path = find(&tree, id).filter(|p| p.len() > 1)?;
        let node = at_mut(&mut tree, &path);
        if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
            node.title = title.chars().take(500).collect();
        }
        if !node.is_folder()
            && let Some(url) = url
                .map(str::trim)
                .filter(|u| bookmarkable(u) && u.len() <= 8192)
        {
            node.url = Some(url.to_string());
        }
        Some(())
    });
    if updated.is_some() {
        changed();
    }
}

/// Moves nodes into a folder, starting at `index`; a folder never goes inside itself.
pub fn move_nodes(ids: &[String], parent_id: &str, index: usize) {
    let moved = TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        let target = folder_path(&tree, parent_id)?;
        let mut position = index.min(at(&tree, &target).children().len());
        for id in ids {
            let Some(path) = find(&tree, id).filter(|p| p.len() > 1) else {
                continue;
            };
            if at(&tree, &path).is_folder() && is_inside(&tree, parent_id, id) {
                continue;
            }
            let (parent, index) = path.split_at(path.len() - 1);
            let node = at_mut(&mut tree, parent).children_mut().remove(index[0]);
            if at(&tree, parent).id == parent_id && index[0] < position {
                position -= 1;
            }
            let Some(target) = folder_path(&tree, parent_id) else {
                break;
            };
            let children = at_mut(&mut tree, &target).children_mut();
            position = position.min(children.len());
            children.insert(position, node);
            position += 1;
        }
        Some(())
    });
    if moved.is_some() {
        changed();
    }
}

/// Deletes nodes, remembering them for "Undo"; returns how many went.
pub fn remove(ids: &[String]) -> usize {
    let mut removed = Vec::new();
    TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        for id in ids {
            let Some(path) = find(&tree, id).filter(|p| p.len() > 1) else {
                continue;
            };
            let (parent, index) = path.split_at(path.len() - 1);
            let parent_id = at(&tree, parent).id.clone();
            let node = at_mut(&mut tree, parent).children_mut().remove(index[0]);
            removed.push((node, parent_id, index[0]));
        }
    });
    let count = removed.len();
    if count > 0 {
        LAST_REMOVED.with(|last| last.replace(removed));
        changed();
    }
    count
}

pub fn undo_remove() {
    let removed = LAST_REMOVED.with(|last| last.take());
    if removed.is_empty() {
        return;
    }
    TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        for (node, parent_id, index) in removed.into_iter().rev() {
            let path = folder_path(&tree, &parent_id).unwrap_or_else(|| vec![1]);
            let children = at_mut(&mut tree, &path).children_mut();
            let position = clamp(children.len(), Some(index));
            children.insert(position, node);
        }
    });
    changed();
}

/// Compares titles the way people expect: case and accents aside first, and numbers by value.
fn compare_titles(a: &str, b: &str, lang: settings::Lang) -> Ordering {
    const ALPHABET: &str = "aâbcçdefgğhıiîjklmnoöpqrsştuûüvwxyz";
    let key = |c: char| {
        ALPHABET
            .chars()
            .position(|l| l == c)
            .map_or(u32::from(c) + 100, |p| p as u32)
    };
    let (a, b) = (fold(a, lang), fold(b, lang));
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n: u128 = 0;
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n = n
                            .saturating_mul(10)
                            .saturating_add(u128::from(c as u8 - b'0'));
                        it.next();
                    }
                    n
                };
                match take(&mut x).cmp(&take(&mut y)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                match key(c).cmp(&key(d)) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
        }
    }
}

/// Folders first, then links, each in alphabetical order for the interface language.
pub fn sort_folder(id: &str) {
    let lang = settings::get().language;
    let sorted = TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        let path = folder_path(&tree, id)?;
        at_mut(&mut tree, &path).children_mut().sort_by(|a, b| {
            b.is_folder()
                .cmp(&a.is_folder())
                .then_with(|| compare_titles(&a.title, &b.title, lang))
        });
        Some(())
    });
    if sorted.is_some() {
        changed();
    }
}

/// Ctrl+D: bookmarks the page, or finds the bookmark it already has.
pub fn bookmark_page(url: &str, title: &str) -> Option<(String, bool)> {
    if !bookmarkable(url) {
        return None;
    }
    let key = url_key(url);
    if let Some(existing) = links().into_iter().find(|link| url_key(link.url()) == key) {
        return Some((existing.id, false));
    }
    let last = LAST_FOLDER.with(|last| last.borrow().clone());
    let parent = if TREE.with(|cell| folder_path(&cell.borrow(), &last).is_some()) {
        last
    } else {
        "bar".into()
    };
    create(&parent, None, title, Some(url)).map(|id| (id, true))
}

/// Ctrl+Shift+D: a new folder in the bar holding the open pages of the space.
pub fn bookmark_tabs(tabs: Vec<(String, String)>, folder_title: &str) -> Option<String> {
    let usable: Vec<(String, String)> = tabs
        .into_iter()
        .filter(|(url, _)| bookmarkable(url))
        .collect();
    if usable.is_empty() {
        return None;
    }
    let folder = create("bar", None, folder_title, None)?;
    TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        if let Some(path) = folder_path(&tree, &folder) {
            let now = now_ms();
            let children = at_mut(&mut tree, &path).children_mut();
            for (url, title) in usable {
                children.push(Node::link(
                    new_id(),
                    if title.is_empty() { url.clone() } else { title },
                    url,
                    now,
                ));
            }
        }
    });
    changed();
    Some(folder)
}

fn materialize(drafts: Vec<Draft>) -> Vec<Node> {
    drafts
        .into_iter()
        .map(|draft| match draft {
            Draft::Link {
                title,
                url,
                added_at,
            } => Node::link(new_id(), title, url, added_at.unwrap_or_else(now_ms)),
            Draft::Folder {
                title,
                added_at,
                children,
            } => {
                let mut folder = Node::folder(new_id(), title, added_at.unwrap_or_else(now_ms));
                folder.children = Some(materialize(children));
                folder
            }
        })
        .collect()
}

/// Adds imported bookmarks as Chromium does: straight into empty roots, else into one folder
/// in the bar.
pub fn insert_draft(draft: DraftTree, folder_title: &str) {
    TREE.with(|cell| {
        let mut tree = cell.borrow_mut();
        if tree.bar.children().is_empty() && tree.other.children().is_empty() {
            tree.bar.children_mut().extend(materialize(draft.bar));
            tree.other.children_mut().extend(materialize(draft.other));
            return;
        }
        let mut children = materialize(draft.bar);
        if !draft.other.is_empty() {
            let other = materialize(draft.other);
            if children.is_empty() {
                children.extend(other);
            } else {
                let mut folder = Node::folder(new_id(), t("bookmarks.other"), now_ms());
                folder.children = Some(other);
                children.push(folder);
            }
        }
        let mut folder = Node::folder(new_id(), folder_title.to_string(), now_ms());
        folder.children = Some(children);
        tree.bar.children_mut().push(folder);
    });
    changed();
}

// ── Opening ───────────────────────────────────────────────────────────────────

fn links_under(ids: &[String]) -> Vec<String> {
    let tree = tree();
    let mut out = Vec::new();
    for id in ids {
        if let Some(path) = find(&tree, id) {
            let node = at(&tree, &path);
            if node.is_folder() {
                let mut found = Vec::new();
                walk_links(node, &mut found);
                out.extend(found.into_iter().map(|link| link.url().to_string()));
            } else {
                out.push(node.url().to_string());
            }
        }
    }
    out
}

/// Opens bookmarks: "current" (one link in the active tab), "tab", "background", "window" or
/// "incognito". Many links at once are asked about first.
pub fn open(win: &Rc<BrowserWindow>, ids: &[String], how: &str) {
    let urls = links_under(ids);
    if urls.is_empty() {
        return;
    }
    let how = how.to_string();
    if urls.len() <= OPEN_ALL_CONFIRM {
        open_urls(win, urls, &how);
        return;
    }
    let n = urls.len() as i64;
    let question = menu::Question {
        title: tv("bookmarks.openAllTitle", &[("n", Var::Number(n))]),
        body: None,
        confirm: tv("bookmarks.openAllConfirm", &[("n", Var::Number(n))]),
        cancel: t("common.cancel"),
        danger: false,
    };
    let target = win.clone();
    menu::confirm(win, question, move |ok| {
        if ok {
            open_urls(&target, urls, &how);
        }
    });
}

fn open_urls(win: &Rc<BrowserWindow>, urls: Vec<String>, how: &str) {
    match how {
        "window" | "incognito" => new_window(win, how == "incognito", urls),
        "current" if urls.len() == 1 && win.active_tab().is_some() => {
            if let Some(tab) = win.active_tab() {
                tab.load(&urls[0]);
                win.focus_active();
            }
        }
        _ => {
            for (i, url) in urls.into_iter().enumerate() {
                let background = how == "background" || i > 0;
                win.open_tab(OpenOptions {
                    url: Some(url),
                    background,
                    ..OpenOptions::default()
                });
            }
        }
    }
}

// ── Menus ─────────────────────────────────────────────────────────────────────

fn short_label(text: &str) -> String {
    if text.chars().count() > 60 {
        format!("{}…", text.chars().take(57).collect::<String>())
    } else {
        text.to_string()
    }
}

fn folder_items(win: &Rc<BrowserWindow>, folder: &Node, from: usize) -> Vec<MenuSpec> {
    let nodes: Vec<&Node> = folder.children().iter().skip(from).collect();
    let mut items: Vec<MenuSpec> = nodes
        .iter()
        .map(|node| {
            let id = node.id.clone();
            let context_win = win.clone();
            let context_id = node.id.clone();
            let context = move || bookmark_menu_items(&context_win, &context_id);
            if node.is_folder() {
                submenu(short_label(&node.title), folder_items(win, node, 0))
                    .folder()
                    .context(context)
            } else {
                let label = short_label(if node.title.is_empty() {
                    node.url()
                } else {
                    &node.title
                });
                let target = win.clone();
                item(label, move |event: ClickEvent| {
                    let how = if event.ctrl_key || event.meta_key {
                        "background"
                    } else if event.shift_key {
                        "window"
                    } else {
                        "current"
                    };
                    open(&target, std::slice::from_ref(&id), how);
                })
                .favicon(node.url())
                .context(context)
            }
        })
        .collect();
    if items.is_empty() {
        items.push(item(t("bookmarks.empty"), |_| {}).enabled(false));
    }
    let link_ids: Vec<String> = nodes
        .iter()
        .filter(|n| !n.is_folder())
        .map(|n| n.id.clone())
        .collect();
    if link_ids.len() > 1 {
        let n = link_ids.len() as i64;
        items.push(separator());
        items.push(item(
            tv("bookmarks.openAll", &[("n", Var::Number(n))]),
            on_window(win, move |win| open(win, &link_ids, "tab")),
        ));
    }
    items
}

pub fn folder_menu(win: &Rc<BrowserWindow>, folder_id: &str, x: f64, y: f64, from: usize) {
    let tree = tree();
    let Some(path) = folder_path(&tree, folder_id) else {
        return;
    };
    menu::popup(
        win,
        folder_items(win, at(&tree, &path), from),
        x,
        y,
        false,
        None,
    );
}

fn root_title(id: &str, title: &str) -> String {
    match id {
        "bar" => t("bookmarks.bar"),
        "other" => t("bookmarks.other"),
        _ => short_label(title),
    }
}

/// "Move to": every folder the node may go into, indented by depth.
fn move_targets(node_id: &str, is_folder: bool, parent_id: &str) -> Vec<MenuSpec> {
    let tree = tree();
    folder_list(&tree)
        .into_iter()
        .filter(|(folder, _)| !is_folder || !is_inside(&tree, &folder.id, node_id))
        .map(|(folder, depth)| {
            let label = format!(
                "{}{}",
                "\u{a0}\u{a0}\u{a0}".repeat(depth),
                root_title(&folder.id, &folder.title)
            );
            let (target, node) = (folder.id.clone(), node_id.to_string());
            let here = folder.id == parent_id;
            let spec = item(label, move |_| {
                let length = TREE.with(|cell| {
                    let tree = cell.borrow();
                    folder_path(&tree, &target).map(|p| at(&tree, &p).children().len())
                });
                move_nodes(std::slice::from_ref(&node), &target, length.unwrap_or(0));
                LAST_FOLDER.with(|last| last.replace(target.clone()));
            });
            if here {
                spec.checkbox(true).enabled(false)
            } else {
                spec.folder()
            }
        })
        .collect()
}

/// The right-click menu of a bookmark or folder, wherever it shows.
fn bookmark_menu_items(win: &Rc<BrowserWindow>, id: &str) -> Vec<MenuSpec> {
    let tree = tree();
    let Some(path) = find(&tree, id) else {
        return Vec::new();
    };
    let node = at(&tree, &path).clone();
    let parent_id = if path.len() > 1 {
        at(&tree, &path[..path.len() - 1]).id.clone()
    } else {
        "bar".into()
    };
    let is_root = path.len() == 1;
    let add_to = if node.is_folder() {
        node.id.clone()
    } else {
        parent_id.clone()
    };
    let ids = vec![id.to_string()];
    let (page_target, folder_target) = (add_to.clone(), add_to);
    let add = vec![
        item(
            t(if node.is_folder() {
                "bookmarks.addPageHere"
            } else {
                "bookmarks.addPage"
            }),
            on_window(win, move |win| {
                win.command(json!({ "type": "bookmark-edit", "parentId": page_target }))
            }),
        ),
        item(
            t(if node.is_folder() {
                "bookmarks.addSubfolder"
            } else {
                "bookmarks.addFolder"
            }),
            on_window(win, move |win| {
                win.command(
                    json!({ "type": "bookmark-edit", "parentId": folder_target, "folder": true }),
                )
            }),
        ),
    ];
    let mut items = Vec::new();
    if !node.is_folder() {
        for (key, how) in [
            ("bookmarks.open", "current"),
            ("bookmarks.openNewTab", "background"),
            ("bookmarks.openNewWindow", "window"),
            ("bookmarks.openIncognito", "incognito"),
        ] {
            let ids = ids.clone();
            items.push(item(
                t(key),
                on_window(win, move |win| open(win, &ids, how)),
            ));
        }
        items.push(separator());
        let edit = id.to_string();
        items.push(item(
            t("bookmarks.edit"),
            on_window(win, move |win| {
                win.command(json!({ "type": "bookmark-edit", "id": edit }))
            }),
        ));
        let rename = id.to_string();
        items.push(item(
            t("bookmarks.rename"),
            on_window(win, move |win| {
                win.command(json!({ "type": "bookmark-rename", "id": rename }))
            }),
        ));
        let url = node.url().to_string();
        items.push(item(t("ctx.copyLink"), move |_| copy_text(&url)));
        items.push(submenu(
            t("bookmarks.moveTo"),
            move_targets(id, false, &parent_id),
        ));
        let remove_ids = ids.clone();
        items.push(item(
            t("bookmarks.delete"),
            on_window(win, move |win| remove_and_tell(win, &remove_ids)),
        ));
        items.push(separator());
        items.extend(add);
    } else {
        let count = links_under(&ids).len();
        let n = count as i64;
        for (label, how) in [
            (tv("bookmarks.openAll", &[("n", Var::Number(n))]), "tab"),
            (t("bookmarks.openAllWindow"), "window"),
            (t("bookmarks.openAllIncognito"), "incognito"),
        ] {
            let ids = ids.clone();
            items.push(
                item(label, on_window(win, move |win| open(win, &ids, how))).enabled(count > 0),
            );
        }
        items.push(separator());
        items.extend(add);
        items.push(separator());
        if !is_root {
            let rename = id.to_string();
            items.push(item(
                t("bookmarks.rename"),
                on_window(win, move |win| {
                    win.command(json!({ "type": "bookmark-rename", "id": rename }))
                }),
            ));
            items.push(submenu(
                t("bookmarks.moveTo"),
                move_targets(id, true, &parent_id),
            ));
        }
        let sort = id.to_string();
        items.push(
            item(t("bookmarks.sort"), move |_| sort_folder(&sort))
                .enabled(node.children().len() > 1),
        );
        if !is_root {
            let remove_ids = ids.clone();
            items.push(item(
                t("bookmarks.deleteFolder"),
                on_window(win, move |win| remove_and_tell(win, &remove_ids)),
            ));
        }
        if id == "bar" {
            items.push(separator());
            items.push(
                item(
                    t("bookmarks.showBar"),
                    on_window(win, |win| {
                        crate::actions::run_command(win, "toggle-bookmarks-bar")
                    }),
                )
                .checkbox(settings::get().bookmarks_bar != settings::BookmarksBar::Never)
                .accelerator("Ctrl+Shift+B"),
            );
        }
    }
    items.push(separator());
    items.push(
        item(
            t("bookmarks.manager"),
            on_window(win, |win| open_page(win, "bookmarks", "")),
        )
        .accelerator("Ctrl+Shift+O"),
    );
    items
}

pub fn bookmark_menu(win: &Rc<BrowserWindow>, id: &str, x: f64, y: f64) {
    menu::popup(win, bookmark_menu_items(win, id), x, y, false, None);
}

/// Deletes and lets the UI offer "Undo".
pub fn remove_and_tell(win: &BrowserWindow, ids: &[String]) {
    let count = remove(ids);
    if count > 0 {
        win.emit(
            "command",
            json!({ "type": "bookmarks-removed", "count": count }),
        );
    }
}

// ── Netscape bookmark files ───────────────────────────────────────────────────

pub fn decode_entities(text: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#[0-9]+|[a-z]+);").unwrap());
    ENTITY
        .replace_all(text, |caps: &regex::Captures| {
            let code = &caps[1];
            let decoded =
                if let Some(hex) = code.strip_prefix("#x").or_else(|| code.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = code.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    match code.to_ascii_lowercase().as_str() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        "nbsp" => Some(' '),
                        _ => None,
                    }
                };
            decoded
                .filter(|c| *c != '\0')
                .map_or_else(|| caps[0].to_string(), String::from)
        })
        .into_owned()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn attribute(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"(?i)\b{name}\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))"#);
    let caps = Regex::new(&pattern).ok()?.captures(attrs)?;
    let value = caps
        .get(2)
        .or_else(|| caps.get(3))
        .or_else(|| caps.get(4))
        .map_or("", |m| m.as_str());
    Some(decode_entities(value))
}

fn strip_tags(text: &str) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    decode_entities(&TAG.replace_all(text, ""))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn seconds_to_ms(value: Option<String>) -> Option<u64> {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(|n| n.saturating_mul(1000))
}

pub struct Parsed {
    pub tree: DraftTree,
    pub skipped: usize,
}

/// Reads a Netscape bookmark file. The folder marked PERSONAL_TOOLBAR_FOLDER becomes the
/// bookmarks bar, everything else goes to other bookmarks; bookmarklets and Firefox's smart
/// folders (place:) are skipped.
pub fn parse_netscape(html: &str) -> Parsed {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<DL\b[^>]*>|</DL\s*>|<H3\b([^>]*)>(.*?)</H3\s*>|<A\b([^>]*)>(.*?)</A\s*>")
            .unwrap()
    });
    static TOOLBAR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)PERSONAL_TOOLBAR_FOLDER\s*=\s*"?true"#).unwrap());
    // Folders in a flat list first (index 0 is the file's root), as children may still come.
    struct Item {
        draft: Draft,
        children: Vec<usize>,
    }
    let mut items = vec![Item {
        draft: Draft::Folder {
            title: String::new(),
            added_at: None,
            children: Vec::new(),
        },
        children: Vec::new(),
    }];
    let mut stack: Vec<usize> = Vec::new();
    let mut pending: Option<usize> = None;
    let mut toolbar: Option<usize> = None;
    let mut toolbar_pending = false;
    let mut skipped = 0;
    for caps in TOKEN.captures_iter(html) {
        let text = &caps[0];
        let lower = text.get(..3).unwrap_or("").to_ascii_lowercase();
        if lower.starts_with("<dl") {
            match pending.take() {
                Some(folder) => {
                    stack.push(folder);
                    if toolbar_pending {
                        toolbar = Some(folder);
                    }
                    toolbar_pending = false;
                }
                None => stack.push(stack.last().copied().unwrap_or(0)),
            }
        } else if lower.starts_with("</d") {
            stack.pop();
        } else if let Some(attrs) = caps.get(1) {
            let title = strip_tags(caps.get(2).map_or("", |m| m.as_str()));
            let added_at = seconds_to_ms(attribute(attrs.as_str(), "ADD_DATE"));
            let index = items.len();
            items.push(Item {
                draft: Draft::Folder {
                    title: if title.is_empty() {
                        t("bookmarks.folder")
                    } else {
                        title
                    },
                    added_at,
                    children: Vec::new(),
                },
                children: Vec::new(),
            });
            let parent = stack.last().copied().unwrap_or(0);
            items[parent].children.push(index);
            pending = Some(index);
            toolbar_pending = TOOLBAR.is_match(attrs.as_str());
        } else if let Some(attrs) = caps.get(3) {
            let url = attribute(attrs.as_str(), "HREF").unwrap_or_default();
            if !bookmarkable(&url) {
                skipped += 1;
                continue;
            }
            let title = strip_tags(caps.get(4).map_or("", |m| m.as_str()));
            let added_at = seconds_to_ms(attribute(attrs.as_str(), "ADD_DATE"));
            let index = items.len();
            items.push(Item {
                draft: Draft::Link {
                    title: if title.is_empty() { url.clone() } else { title },
                    url,
                    added_at,
                },
                children: Vec::new(),
            });
            let parent = stack.last().copied().unwrap_or(0);
            items[parent].children.push(index);
        }
    }
    fn build(items: &[Item], index: usize, skip: Option<usize>) -> Draft {
        match &items[index].draft {
            Draft::Folder {
                title, added_at, ..
            } => Draft::Folder {
                title: title.clone(),
                added_at: *added_at,
                children: items[index]
                    .children
                    .iter()
                    .filter(|child| Some(**child) != skip)
                    .map(|child| build(items, *child, skip))
                    .collect(),
            },
            link => link.clone(),
        }
    }
    let children_of = |draft: Draft| match draft {
        Draft::Folder { children, .. } => children,
        Draft::Link { .. } => Vec::new(),
    };
    let tree = match toolbar {
        Some(bar) => DraftTree {
            bar: children_of(build(&items, bar, None)),
            other: children_of(build(&items, 0, Some(bar))),
        },
        None => DraftTree {
            bar: Vec::new(),
            other: children_of(build(&items, 0, None)),
        },
    };
    Parsed { tree, skipped }
}

/// Writes a Netscape bookmark file that Chrome, Firefox, Safari and Edge can import.
pub fn to_netscape(tree: &Tree) -> String {
    let mut lines = vec![
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>".to_string(),
        "<!-- This is an automatically generated file.".into(),
        "     It will be read and overwritten.".into(),
        "     DO NOT EDIT! -->".into(),
        r#"<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">"#.into(),
        "<TITLE>Bookmarks</TITLE>".into(),
        "<H1>Bookmarks</H1>".into(),
        "<DL><p>".into(),
    ];
    let date = |ms: u64| if ms > 0 { ms / 1000 } else { now_ms() / 1000 };
    fn write(nodes: &[Node], depth: usize, lines: &mut Vec<String>, date: &dyn Fn(u64) -> u64) {
        let pad = "    ".repeat(depth);
        for node in nodes {
            if node.is_folder() {
                lines.push(format!(
                    "{pad}<DT><H3 ADD_DATE=\"{}\">{}</H3>",
                    date(node.added_at),
                    escape_html(&node.title)
                ));
                lines.push(format!("{pad}<DL><p>"));
                write(node.children(), depth + 1, lines, date);
                lines.push(format!("{pad}</DL><p>"));
            } else {
                lines.push(format!(
                    "{pad}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>",
                    escape_html(node.url()),
                    date(node.added_at),
                    escape_html(&node.title)
                ));
            }
        }
    }
    lines.push(format!(
        "    <DT><H3 ADD_DATE=\"{}\" PERSONAL_TOOLBAR_FOLDER=\"true\">{}</H3>",
        date(tree.bar.added_at),
        escape_html(&tree.bar.title)
    ));
    lines.push("    <DL><p>".into());
    write(tree.bar.children(), 2, &mut lines, &date);
    lines.push("    </DL><p>".into());
    write(tree.other.children(), 1, &mut lines, &date);
    lines.push("</DL><p>".into());
    lines.join("\n") + "\n"
}

fn count(drafts: &[Draft]) -> (usize, usize) {
    drafts
        .iter()
        .fold((0, 0), |(links, folders), draft| match draft {
            Draft::Link { .. } => (links + 1, folders),
            Draft::Folder { children, .. } => {
                let (l, f) = count(children);
                (links + l, folders + f + 1)
            }
        })
}

/// Drops links already bookmarked (or twice in the file); returns how many went.
pub fn without_duplicates(tree: DraftTree, existing: &HashSet<String>) -> (DraftTree, usize) {
    fn filter(nodes: Vec<Draft>, seen: &mut HashSet<String>, duplicates: &mut usize) -> Vec<Draft> {
        let mut out = Vec::new();
        for node in nodes {
            match node {
                Draft::Link { ref url, .. } => {
                    if seen.insert(url_key(url)) {
                        out.push(node);
                    } else {
                        *duplicates += 1;
                    }
                }
                Draft::Folder {
                    title,
                    added_at,
                    children,
                } => {
                    let children = filter(children, seen, duplicates);
                    if !children.is_empty() {
                        out.push(Draft::Folder {
                            title,
                            added_at,
                            children,
                        });
                    }
                }
            }
        }
        out
    }
    let mut seen = existing.clone();
    let mut duplicates = 0;
    let bar = filter(tree.bar, &mut seen, &mut duplicates);
    let other = filter(tree.other, &mut seen, &mut duplicates);
    (DraftTree { bar, other }, duplicates)
}

/// Adds a parsed file's bookmarks; returns what the import dialog reports.
pub fn import_parsed(parsed: Parsed, skip_duplicates: bool, label: &str) -> Value {
    let (draft, duplicates) = if skip_duplicates {
        without_duplicates(parsed.tree, &bookmarked_keys())
    } else {
        (parsed.tree, 0)
    };
    let (bar_links, bar_folders) = count(&draft.bar);
    let (other_links, other_folders) = count(&draft.other);
    let (links, folders) = (bar_links + other_links, bar_folders + other_folders);
    if links + folders > 0 {
        insert_draft(draft, label);
    }
    json!({ "links": links, "folders": folders, "duplicates": duplicates, "skipped": parsed.skipped, "favicons": 0 })
}

fn html_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("HTML"));
    filter.add_pattern("*.html");
    filter.add_pattern("*.htm");
    filter
}

/// Import from a bookmark file other browsers export; asks for the file.
pub fn import_html(win: &BrowserWindow, skip_duplicates: bool) -> Value {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("import.title")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Open,
        None,
        None,
    );
    dialog.add_filter(html_filter());
    if dialog.run() != gtk::ResponseType::Accept {
        return Value::Null;
    }
    let Some(path) = dialog.filename() else {
        return Value::Null;
    };
    match std::fs::read(&path) {
        Ok(bytes) => import_parsed(
            parse_netscape(&String::from_utf8_lossy(&bytes)),
            skip_duplicates,
            &t("import.fromFile"),
        ),
        Err(error) => {
            eprintln!("[bookmarks] could not read {}: {error}", path.display());
            Value::Null
        }
    }
}

/// Saves every bookmark to a file other browsers can import; asks where.
pub fn export(win: &BrowserWindow) -> bool {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("bookmarks.export")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Save,
        None,
        None,
    );
    dialog.set_do_overwrite_confirmation(true);
    dialog.add_filter(html_filter());
    let day = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|d| d.to_string())
        .unwrap_or_default();
    dialog.set_current_name(&format!("peepwick-bookmarks-{day}.html"));
    if let Some(documents) = glib::user_special_dir(glib::UserDirectory::Documents) {
        let _ = dialog.set_current_folder(documents);
    }
    if dialog.run() != gtk::ResponseType::Accept {
        return false;
    }
    let Some(path) = dialog.filename() else {
        return false;
    };
    let mut titled = tree();
    titled.bar.title = t("bookmarks.bar");
    titled.other.title = t("bookmarks.other");
    match std::fs::write(&path, to_netscape(&titled)) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[bookmarks] could not write {}: {error}", path.display());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<DL><p>
    <DT><H3 ADD_DATE="1700000000" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://example.com/" ADD_DATE="1700000001">Example &amp; Co</A>
        <DT><H3>Work</H3>
        <DL><p>
            <DT><A HREF="https://github.com/">GitHub</A>
        </DL><p>
        <DT><A HREF="javascript:alert(1)">Bookmarklet</A>
    </DL><p>
    <DT><A HREF='https://duckduckgo.com/'>DuckDuckGo</A>
    <DT><A HREF="place:sort=8">Recent</A>
</DL><p>"#;

    #[test]
    fn reads_netscape_files() {
        let parsed = parse_netscape(FILE);
        assert_eq!(parsed.skipped, 2);
        assert_eq!(parsed.tree.bar.len(), 2);
        assert_eq!(
            parsed.tree.bar[0],
            Draft::Link {
                title: "Example & Co".into(),
                url: "https://example.com/".into(),
                added_at: Some(1_700_000_001_000)
            }
        );
        let Draft::Folder {
            title, children, ..
        } = &parsed.tree.bar[1]
        else {
            panic!("a folder")
        };
        assert_eq!(title, "Work");
        assert_eq!(children.len(), 1);
        assert_eq!(parsed.tree.other.len(), 1);
    }

    #[test]
    fn writes_what_it_reads() {
        let mut tree = Tree::empty();
        tree.bar.title = "Bar".into();
        tree.bar.children_mut().push(Node::link(
            "1".into(),
            "A \"quoted\" <title>".into(),
            "https://a.example/?x=1&y=2".into(),
            1_000,
        ));
        let mut folder = Node::folder("2".into(), "Folder".into(), 2_000);
        folder.children_mut().push(Node::link(
            "3".into(),
            "B".into(),
            "https://b.example/".into(),
            3_000,
        ));
        tree.other.children_mut().push(folder);
        let parsed = parse_netscape(&to_netscape(&tree));
        assert_eq!(parsed.skipped, 0);
        assert_eq!(
            parsed.tree.bar,
            vec![Draft::Link {
                title: "A \"quoted\" <title>".into(),
                url: "https://a.example/?x=1&y=2".into(),
                added_at: Some(1_000)
            }]
        );
        assert_eq!(count(&parsed.tree.other), (1, 1));
    }

    #[test]
    fn finds_duplicates_by_address() {
        assert_eq!(url_key("https://Example.com/#top"), "https://example.com");
        assert_eq!(url_key("https://example.com/a/"), "https://example.com/a/");
        let existing: HashSet<String> = [url_key("https://example.com/")].into();
        let (tree, duplicates) = without_duplicates(parse_netscape(FILE).tree, &existing);
        assert_eq!(duplicates, 1);
        assert_eq!(count(&tree.bar), (1, 1));
    }

    #[test]
    fn sorts_titles_like_people_do() {
        let lang = settings::Lang::Tr;
        assert_eq!(compare_titles("çay", "dut", lang), Ordering::Less);
        assert_eq!(compare_titles("Item 9", "item 10", lang), Ordering::Less);
        assert_eq!(compare_titles("ılık", "iğne", lang), Ordering::Less);
    }
}
