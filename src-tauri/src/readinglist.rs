//! The reading list: pages saved to read later. When a page has an article, its text is kept
//! on disk too, so it can be read offline at peepwick://saved/<id>, drawn like reader view
//! with no scripts, forms or frames. readinglist.json and the reading/ folder have 0.1.x's
//! format.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use webkit2gtk::{
    LoadEvent, PolicyDecisionExt, PolicyDecisionType, ResponsePolicyDecision,
    ResponsePolicyDecisionExt, URISchemeRequest, URISchemeRequestExt, WebViewExt, glib,
};

use crate::i18n::t;
use crate::reader::{self, Article};
use crate::tab::{Tab, now_ms};
use crate::window::{self, BrowserWindow};
use crate::{bookmarks, pages, paths, safebrowsing, store};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub site: String,
    /// Words in the saved article; 0 when only the link is kept.
    #[serde(default)]
    pub words: u64,
    /// The article is on disk and can be read offline.
    #[serde(default)]
    pub saved: bool,
    #[serde(default)]
    pub added_at: u64,
    #[serde(default)]
    pub read_at: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

thread_local! {
    static ITEMS: RefCell<Vec<Item>> = const { RefCell::new(Vec::new()) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

fn file() -> std::path::PathBuf {
    paths::user_path("readinglist.json")
}

fn article_path(id: &str) -> std::path::PathBuf {
    paths::user_path("reading").join(format!("{id}.json"))
}

/// An id that can name a file and an address: the UUIDs 0.1.x made, or ours.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn load() {
    let Some(data) = store::read_json::<File>(&file()) else {
        return;
    };
    let items = data
        .items
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Item>(value).ok())
        .filter(|item| valid_id(&item.id))
        .collect();
    ITEMS.with(|cell| cell.replace(items));
}

fn save() {
    let items = list();
    match serde_json::to_vec(&json!({ "items": items })) {
        Ok(data) => {
            if let Err(error) = store::write_atomic(&file(), &data) {
                eprintln!("[reading] could not save: {error}");
            }
        }
        Err(error) => eprintln!("[reading] could not serialize: {error}"),
    }
}

/// Writes a change that is still waiting, before the browser quits.
pub fn flush() {
    if SAVE_PENDING.with(|pending| pending.replace(false)) {
        save();
    }
}

fn changed() {
    for win in window::all() {
        win.command(json!({ "type": "reading-changed" }));
    }
    if !SAVE_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_millis(300), flush);
    }
}

pub fn list() -> Vec<Item> {
    ITEMS.with(|cell| cell.borrow().clone())
}

fn find(id: &str) -> Option<Item> {
    ITEMS.with(|cell| cell.borrow().iter().find(|i| i.id == id).cloned())
}

/// Puts a page at the top of the list, with its article when it has one; a page already on
/// the list moves up and keeps its offline copy.
fn store(url: &str, title: &str, article: Option<&Article>) -> Item {
    let existing = ITEMS.with(|cell| cell.borrow().iter().find(|i| i.url == url).cloned());
    let id = existing
        .as_ref()
        .map(|i| i.id.clone())
        .unwrap_or_else(bookmarks::new_id);
    let mut saved = existing.as_ref().is_some_and(|i| i.saved);
    if let Some(article) = article {
        let path = article_path(&id);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| serde_json::to_vec(article).map_err(std::io::Error::other))
            .and_then(|data| store::write_atomic(&path, &data));
        match written {
            Ok(()) => saved = true,
            Err(error) => eprintln!("[reading] could not keep the article: {error}"),
        }
    }
    let host = reader::display_host(url);
    let item = Item {
        id: id.clone(),
        url: url.to_string(),
        title: article
            .map(|a| a.title.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| Some(title.to_string()).filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| host.clone()),
        site: article
            .and_then(|a| a.site_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(host),
        words: article.map_or_else(|| existing.as_ref().map_or(0, |i| i.words), |a| a.length),
        saved,
        added_at: now_ms(),
        read_at: None,
    };
    ITEMS.with(|cell| {
        let mut items = cell.borrow_mut();
        items.retain(|i| i.id != id);
        items.insert(0, item.clone());
    });
    changed();
    item
}

fn toast(win: &BrowserWindow, key: &str) {
    win.command(json!({ "type": "toast", "text": t(key) }));
}

fn added(win: &BrowserWindow, item: &Item) {
    toast(
        win,
        if item.saved {
            "reading.addedOffline"
        } else {
            "reading.added"
        },
    );
}

fn listable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Saves the page a tab shows, with its article when it has one. Sign-in forms and app pages
/// have no article worth keeping, only their address.
pub fn add_page(win: &Rc<BrowserWindow>, tab: &Rc<Tab>) {
    let Some(view) = tab.view() else { return };
    let url = tab.url();
    if !listable(&url) || win.incognito {
        return;
    }
    let title = tab.display_title();
    let win = Rc::downgrade(win);
    let page = view.clone();
    reader::check(&view, move |readerable| {
        let finish = move |article: Option<Article>| {
            let item = store(&url, &title, article.as_ref());
            if let Some(win) = win.upgrade() {
                added(&win, &item);
            }
        };
        if readerable {
            reader::extract(&page, finish);
        } else {
            finish(None);
        }
    });
}

/// How long a link's page may take to load before only its address is kept.
const LINK_TIMEOUT: Duration = Duration::from_secs(20);
/// Many pages fill in their text a moment after loading.
const SETTLE: Duration = Duration::from_millis(1200);

/// Saves a link: its page is loaded out of sight, muted, to keep its article.
pub fn add_link(win: &Rc<BrowserWindow>, url: &str, title: &str) {
    if !listable(url) || win.incognito {
        return;
    }
    toast(win, "reading.saving");
    let item = store(url, title, None);
    // A site known for malware isn't opened, not even out of sight.
    if safebrowsing::threat_for(url).is_some() {
        added(win, &item);
        return;
    }
    let view = win.create_view(None);
    win.place_view(&view);
    view.set_is_muted(true);
    // Nothing is downloaded and no window opens; only the page itself loads.
    view.connect_decide_policy(|_, decision, kind| {
        if kind == PolicyDecisionType::Response
            && let Some(response) = decision.downcast_ref::<ResponsePolicyDecision>()
            && !response.is_mime_type_supported()
        {
            decision.ignore();
            return true;
        }
        false
    });
    // An alert() would hold the page up out of sight.
    view.connect_script_dialog(|_, _| true);
    // Runs once: with the article, or without it when the page found none or took too long.
    let pending = Rc::new(RefCell::new(Some(item)));
    let end: Rc<dyn Fn(Option<Article>)> = {
        let (win, page) = (Rc::downgrade(win), view.downgrade());
        Rc::new(move |article| {
            let Some(item) = pending.borrow_mut().take() else {
                return;
            };
            let page = page.upgrade();
            let item = match article {
                // Removed again while it loaded: it stays removed.
                Some(article) if find(&item.id).is_some() => {
                    let title = page
                        .as_ref()
                        .and_then(|v| v.title())
                        .map(|t| t.to_string())
                        .unwrap_or_default();
                    store(&item.url, &title, Some(&article))
                }
                _ => item,
            };
            if let Some(page) = page {
                page.stop_loading();
                if let Some(parent) = page.parent()
                    && let Ok(container) = parent.downcast::<gtk::Container>()
                {
                    container.remove(&page);
                }
            }
            if let Some(win) = win.upgrade() {
                added(&win, &item);
            }
        })
    };
    let timeout = end.clone();
    glib::timeout_add_local_once(LINK_TIMEOUT, move || timeout(None));
    view.connect_load_changed(move |view, event| {
        if event != LoadEvent::Finished {
            return;
        }
        let (end, page) = (end.clone(), view.downgrade());
        glib::timeout_add_local_once(SETTLE, move || {
            let Some(page) = page.upgrade() else { return };
            let source = page.clone();
            reader::check(&page, move |readerable| {
                if readerable {
                    reader::extract(&source, move |article| end(article));
                } else {
                    end(None);
                }
            });
        });
    });
    view.load_uri(url);
}

pub fn remove(id: &str) {
    let removed = ITEMS.with(|cell| {
        let mut items = cell.borrow_mut();
        let before = items.len();
        items.retain(|i| i.id != id);
        items.len() != before
    });
    if valid_id(id) {
        let _ = std::fs::remove_file(article_path(id));
    }
    if removed {
        changed();
    }
}

pub fn mark_read(id: &str, read: bool) {
    let changed_now = ITEMS.with(|cell| {
        let mut items = cell.borrow_mut();
        match items.iter_mut().find(|i| i.id == id) {
            Some(item) if item.read_at.is_some() != read => {
                item.read_at = read.then(now_ms);
                true
            }
            _ => false,
        }
    });
    if changed_now {
        changed();
    }
}

/// Where to open an item: its offline copy when there is one.
pub fn url_for(id: &str) -> Option<String> {
    let item = find(id)?;
    Some(if item.saved {
        format!("peepwick://saved/{}", item.id)
    } else {
        item.url
    })
}

/// The id in a peepwick://saved/<id> address.
fn saved_id(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("peepwick://saved/")?;
    let id = rest.split(['?', '#', '/']).next()?;
    valid_id(id).then_some(id)
}

/// The page an offline copy was saved from.
pub fn saved_source(url: &str) -> Option<String> {
    saved_id(url).and_then(find).map(|item| item.url)
}

pub fn saved_article(id: &str) -> Option<Article> {
    if !valid_id(id) {
        return None;
    }
    store::read_json::<Article>(&article_path(id))
}

/// The article of an offline copy a tab shows.
pub fn article_at(url: &str) -> Option<Article> {
    let id = saved_id(url)?;
    find(id)?;
    saved_article(id)
}

/// peepwick://saved/<id> in normal windows' tabs: the offline copy and the typefaces it
/// uses. Nothing else of the browser's is reachable from pages.
pub fn respond(request: &URISchemeRequest) {
    let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
    let path = request.path().map(|p| p.to_string()).unwrap_or_default();
    if pages::host_of(&uri) != "saved" {
        return reader::not_found(request);
    }
    if let Some(data) = reader::font(&path) {
        return reader::respond(request, glib::Bytes::from_static(data), "font/woff2");
    }
    let page = saved_id(&uri).and_then(|id| Some((find(id)?, saved_article(id)?)));
    match page {
        Some((item, article)) => {
            let html = reader::render(&article, &item.url, reader::colors(false));
            reader::respond(request, glib::Bytes::from_owned(html), "text/html");
        }
        None => reader::not_found(request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_list_of_0_1_x() {
        let text = r#"{"items":[
            {"id":"6f1c2b9e-1d2a-4c3b-9a8b-7c6d5e4f3a2b","url":"https://a.example/x","title":"A","site":"a.example","words":812,"saved":true,"addedAt":1700000000000,"readAt":null},
            {"id":"../../etc","url":"https://b.example/","title":"B","site":"b.example","words":0,"saved":false,"addedAt":1,"readAt":2},
            {"url":"https://c.example/"},
            {"id":"short-id","url":"https://d.example/","addedAt":5,"readAt":1700000000001}
        ]}"#;
        let file: File = serde_json::from_str(text).unwrap();
        let items: Vec<Item> = file
            .items
            .into_iter()
            .filter_map(|v| serde_json::from_value::<Item>(v).ok())
            .filter(|i| valid_id(&i.id))
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].words, 812);
        assert!(items[0].saved && items[0].read_at.is_none());
        assert_eq!(items[1].read_at, Some(1_700_000_000_001));
        let value = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(value["addedAt"], 1_700_000_000_000_u64);
        assert!(value["readAt"].is_null());
    }

    #[test]
    fn reads_saved_addresses() {
        assert_eq!(
            saved_id("peepwick://saved/6f1c2b9e-1d2a#top"),
            Some("6f1c2b9e-1d2a")
        );
        assert_eq!(saved_id("peepwick://saved/abc/fonts/x.woff2"), Some("abc"));
        assert_eq!(saved_id("peepwick://saved/../x"), None);
        assert_eq!(saved_id("peepwick://reading"), None);
        assert_eq!(saved_id("https://saved/abc"), None);
    }
}
