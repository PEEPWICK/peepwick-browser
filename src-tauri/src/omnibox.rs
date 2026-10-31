//! What the address bar and the command bar (Ctrl+K) understand: addresses, searches,
//! open tabs, history and commands.

use std::rc::Rc;
use std::time::Duration;

use serde::Serialize;

use crate::actions::{self, COMMANDS};
use crate::i18n::{fold, t};
use crate::input::resolve_input;
use crate::search::{search_url, suggest_url};
use crate::settings::{self, Lang, SearchEngine};
use crate::tab::now_ms;
use crate::window::{BrowserWindow, USER_AGENT};
use crate::{bookmarks, history};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: &'static str,
    pub title: String,
    /// The address to open, or the search text for "search".
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<&'static str>,
}

impl Suggestion {
    fn new(kind: &'static str, title: impl Into<String>, value: impl Into<String>) -> Self {
        Suggestion {
            kind,
            title: title.into(),
            value: value.into(),
            favicon: None,
            tab_id: None,
            command: None,
        }
    }
}

/// Suggestions found here, and the search to ask the engine about.
pub struct Local {
    out: Vec<Suggestion>,
    remote: Option<(SearchEngine, String)>,
}

/// Lower case without the scheme and www., for matching what people type.
fn normalize(text: &str) -> String {
    let folded = fold(text, Lang::Tr);
    let rest = folded
        .strip_prefix("https://")
        .or_else(|| folded.strip_prefix("http://"))
        .unwrap_or(&folded);
    rest.strip_prefix("www.").unwrap_or(rest).to_string()
}

/// Everything but the engine's own suggestions; runs on the main thread.
pub fn suggest_local(win: &Rc<BrowserWindow>, input: &str) -> Local {
    let text = input.trim();
    if text.is_empty() {
        return Local {
            out: Vec::new(),
            remote: None,
        };
    }
    let prefs = settings::get();
    let needle = normalize(text);
    let resolved = resolve_input(text, prefs.search_engine);
    let is_search = resolved == search_url(prefs.search_engine, text);
    let pages = if win.incognito {
        Vec::new()
    } else {
        history::match_pages(text, 40)
    };
    let mut out: Vec<Suggestion> = Vec::new();

    // Inline-style best match: one word that isn't an address yet, and a visited page whose
    // address starts with it ("wiki" → wikipedia.org).
    let host_match = if is_search && !text.contains(' ') {
        pages
            .iter()
            .position(|page| normalize(&page.url).starts_with(&needle))
    } else {
        None
    };
    if let Some(page) = host_match.map(|i| &pages[i]) {
        let mut suggestion = Suggestion::new(
            "history",
            if page.title.is_empty() {
                &page.url
            } else {
                &page.title
            },
            &page.url,
        );
        suggestion.favicon = history::favicon_for_url(&page.url);
        out.push(suggestion);
    }
    if is_search {
        out.push(Suggestion::new("search", text, text));
    } else if host_match.is_none_or(|i| pages[i].url != resolved) {
        out.push(Suggestion::new("url", &resolved, &resolved));
    }

    if !win.incognito {
        for link in bookmarks::matching(text, 4) {
            if out.len() >= 6 {
                break;
            }
            let url = link.url.clone().unwrap_or_default();
            if out.iter().any(|s| s.value == url) {
                continue;
            }
            out.push(Suggestion {
                favicon: history::favicon_for_url(&url),
                ..Suggestion::new("bookmark", &link.title, &url)
            });
        }
    }

    if !win.incognito {
        let active = win.active_tab().map(|tab| tab.id);
        for tab in win
            .all_tabs()
            .into_iter()
            .filter(|tab| Some(tab.id) != active && tab.page().is_none())
        {
            if out.len() >= 9 {
                break;
            }
            let title = tab.display_title();
            if normalize(&title).contains(&needle) || normalize(&tab.url()).contains(&needle) {
                let mut suggestion = Suggestion::new("tab", title, tab.url());
                suggestion.favicon = tab.favicon();
                suggestion.tab_id = Some(tab.id);
                out.push(suggestion);
            }
        }
    }

    let now = now_ms() as f64;
    let mut ranked: Vec<(f64, &history::PageMatch)> = pages
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != host_match)
        .map(|(_, page)| {
            let prefix = if normalize(&page.url).starts_with(&needle) {
                4.0
            } else {
                0.0
            };
            let recency = 1.0 / (1.0 + (now - page.last_visit as f64) / 86_400_000.0 / 3.0);
            (prefix + (1.0 + page.visits as f64).log2() + recency, page)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, page) in ranked {
        if out.len() >= 8 {
            break;
        }
        if out.iter().any(|s| s.value == page.url) {
            continue;
        }
        let mut suggestion = Suggestion::new(
            "history",
            if page.title.is_empty() {
                &page.url
            } else {
                &page.title
            },
            &page.url,
        );
        suggestion.favicon = history::favicon_for_url(&page.url);
        out.push(suggestion);
    }

    let remote = (prefs.search_suggestions && !win.incognito && is_search)
        .then(|| (prefs.search_engine, text.to_string()));
    Local { out, remote }
}

/// Adds the search engine's suggestions, if asked for; offline or slow, they're skipped.
pub async fn finish(local: Local) -> Vec<Suggestion> {
    let Local { mut out, remote } = local;
    let Some((engine, text)) = remote else {
        return out;
    };
    let lowered = fold(&text, Lang::Tr);
    for phrase in fetch_suggestions(engine, &text).await {
        if out.len() >= 10 {
            break;
        }
        if fold(&phrase, Lang::Tr) != lowered {
            out.push(Suggestion::new("search", &phrase, &phrase));
        }
    }
    out
}

async fn fetch_suggestions(engine: SearchEngine, query: &str) -> Vec<String> {
    let Some(url) = suggest_url(engine, query) else {
        return Vec::new();
    };
    let Ok(client) = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_millis(1200))
        .build()
    else {
        return Vec::new();
    };
    let Ok(response) = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    else {
        return Vec::new();
    };
    let Ok(body) = response.bytes().await else {
        return Vec::new();
    };
    let Ok(data) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Vec::new();
    };
    data.get(1)
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .take(5)
                .collect()
        })
        .unwrap_or_default()
}

/// The command bar: commands, then open tabs.
pub fn palette(win: &Rc<BrowserWindow>, input: &str) -> Vec<Suggestion> {
    let lang = settings::get().language;
    let needle = fold(input.trim(), lang);
    let matches = |text: &str| needle.is_empty() || fold(text, lang).contains(&needle);
    let commands = COMMANDS
        .iter()
        .filter(|id| actions::is_available(win, id))
        .map(|id| (*id, t(&format!("cmd.{id}"))))
        .filter(|(_, title)| matches(title))
        .take(if needle.is_empty() { 8 } else { 6 })
        .map(|(id, title)| Suggestion {
            command: Some(id),
            ..Suggestion::new("command", title, id)
        });
    let tabs = win
        .all_tabs()
        .into_iter()
        .filter(|tab| matches(&tab.display_title()) || tab.url().to_lowercase().contains(&needle))
        .take(8)
        .map(|tab| Suggestion {
            favicon: tab.favicon(),
            tab_id: Some(tab.id),
            ..Suggestion::new("tab", tab.display_title(), tab.display_url())
        });
    // Bookmarks and visited pages, once something is typed.
    let (marks, pages): (Vec<Suggestion>, Vec<Suggestion>) = if needle.is_empty() || win.incognito {
        (Vec::new(), Vec::new())
    } else {
        let marks: Vec<Suggestion> = bookmarks::matching(input, 6)
            .into_iter()
            .map(|link| {
                let url = link.url.clone().unwrap_or_default();
                Suggestion {
                    favicon: history::favicon_for_url(&url),
                    ..Suggestion::new("bookmark", &link.title, &url)
                }
            })
            .collect();
        let pages = history::match_pages(input, 6)
            .into_iter()
            .filter(|page| !marks.iter().any(|m| m.value == page.url))
            .map(|page| Suggestion {
                favicon: history::favicon_for_url(&page.url),
                ..Suggestion::new(
                    "history",
                    if page.title.is_empty() {
                        &page.url
                    } else {
                        &page.title
                    },
                    &page.url,
                )
            })
            .collect();
        (marks, pages)
    };
    commands.chain(tabs).chain(marks).chain(pages).collect()
}
