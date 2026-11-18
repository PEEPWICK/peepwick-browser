//! Reader view: Mozilla's Readability finds the article in a page, in a script world of its
//! own, and the article is shown in a view laid over the tab. That view runs none of the
//! article's scripts, has a strict content security policy and an in-memory WebKit context of
//! its own, so the article's HTML never reaches the browser's interface or the tab's cookies.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gtk::prelude::*;
use javascriptcore::ValueExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use webkit2gtk::{
    SecurityManagerExt, SettingsExt, URISchemeRequest, URISchemeRequestExt, URISchemeResponse,
    URISchemeResponseExt, UserContentManager, WebContext, WebContextExt, WebView, WebViewExt,
    WebsiteDataManager, gio, glib,
};

use crate::i18n::{self, Var};
use crate::settings::{self, Theme};
use crate::{pages, shield};

const READABILITY: &str = include_str!("../../node_modules/@mozilla/readability/Readability.js");
const READERABLE: &str =
    include_str!("../../node_modules/@mozilla/readability/Readability-readerable.js");
/// The script world Readability runs in, apart from the page's own scripts.
const WORLD: &str = "peepwick-reader";

/// The typefaces of reader view and of the reading list's offline copies.
const FONTS: [(&str, &[u8]); 8] = [
    (
        "newsreader-latin-standard-normal.woff2",
        include_bytes!("../../resources/fonts/newsreader-latin-standard-normal.woff2"),
    ),
    (
        "newsreader-latin-ext-standard-normal.woff2",
        include_bytes!("../../resources/fonts/newsreader-latin-ext-standard-normal.woff2"),
    ),
    (
        "newsreader-latin-standard-italic.woff2",
        include_bytes!("../../resources/fonts/newsreader-latin-standard-italic.woff2"),
    ),
    (
        "newsreader-latin-ext-standard-italic.woff2",
        include_bytes!("../../resources/fonts/newsreader-latin-ext-standard-italic.woff2"),
    ),
    (
        "archivo-latin-standard-normal.woff2",
        include_bytes!("../../resources/fonts/archivo-latin-standard-normal.woff2"),
    ),
    (
        "archivo-latin-ext-standard-normal.woff2",
        include_bytes!("../../resources/fonts/archivo-latin-ext-standard-normal.woff2"),
    ),
    (
        "ibm-plex-mono-latin-400-normal.woff2",
        include_bytes!("../../resources/fonts/ibm-plex-mono-latin-400-normal.woff2"),
    ),
    (
        "ibm-plex-mono-latin-ext-400-normal.woff2",
        include_bytes!("../../resources/fonts/ibm-plex-mono-latin-ext-400-normal.woff2"),
    ),
];

/// What reader view and the reading list keep of a page. The reading list's files on disk
/// have this shape, as in 0.1.x.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Article {
    pub title: String,
    pub content: String,
    pub byline: Option<String>,
    pub site_name: Option<String>,
    /// Words in the article.
    pub length: u64,
    pub lang: Option<String>,
    pub dir: Option<String>,
}

thread_local! {
    static CONTEXT: RefCell<Option<WebContext>> = const { RefCell::new(None) };
    /// Each open reader view's page, by the token in its address.
    static PAGES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
}

// ── Finding the article ───────────────────────────────────────────────────────

/// Whether the page looks like an article, by Readability's quick test.
pub fn check(view: &WebView, done: impl FnOnce(bool) + 'static) {
    let script = format!(
        "{READERABLE}\n;isProbablyReaderable(document, {{ minContentLength: 180, minScore: 30 }})"
    );
    view.evaluate_javascript(
        &script,
        Some(WORLD),
        None,
        None::<&gio::Cancellable>,
        move |result| done(result.is_ok_and(|v| v.is_boolean() && v.to_boolean())),
    );
}

/// The page's article, or None when Readability finds none.
pub fn extract(view: &WebView, done: impl FnOnce(Option<Article>) + 'static) {
    let script = format!(
        r#"{READABILITY}
;(() => {{
  const article = new Readability(document.cloneNode(true), {{ charThreshold: 300 }}).parse()
  if (!article) return null
  return JSON.stringify({{
    title: article.title || document.title || '',
    content: article.content || '',
    byline: article.byline || null,
    siteName: article.siteName || null,
    length: (article.textContent || '').trim().split(/\s+/).length,
    lang: article.lang || document.documentElement.lang || null,
    dir: article.dir || null,
  }})
}})()"#
    );
    view.evaluate_javascript(
        &script,
        Some(WORLD),
        None,
        None::<&gio::Cancellable>,
        move |result| {
            let article = result
                .ok()
                .filter(|v| v.is_string())
                .and_then(|v| serde_json::from_str::<Article>(&v.to_str()).ok())
                .filter(|a| !a.content.is_empty());
            done(article);
        },
    );
}

// ── Drawing it ────────────────────────────────────────────────────────────────

/// Light, dark, or the system's choice through prefers-color-scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colors {
    Light,
    Dark,
    System,
}

/// The colors the browser's interface has: private windows are always dark.
pub fn colors(incognito: bool) -> Colors {
    match settings::get().theme {
        _ if incognito => Colors::Dark,
        Theme::Light => Colors::Light,
        Theme::Dark => Colors::Dark,
        Theme::System => Colors::System,
    }
}

const LIGHT: &str =
    "--bg: #FFFFFF; --text: #0B0B0C; --muted: #3A3B3F; --line: #E7E6E2; --surface: #E7E6E2;";
const DARK: &str =
    "--bg: #0B0B0C; --text: #F4F3F0; --muted: #8E8981; --line: #1D1E20; --surface: #1D1E20;";

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' | '<' | '>' | '"' | '\'' => out.push_str(&format!("&#{};", u32::from(c))),
            _ => out.push(c),
        }
    }
    out
}

/// The site's address without "www.", as the interface shows it.
pub fn display_host(url: &str) -> String {
    let host = pages::host_of(url);
    match host.strip_prefix("www.") {
        Some(rest) => rest.to_string(),
        None => host,
    }
}

static LANG_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z]{2,3}(-[a-z0-9]+)*$").expect("valid pattern"));

/// The page reader view shows: the article in the browser's typefaces, its site, reading
/// time and byline, and a link back to the page.
pub fn render(article: &Article, source: &str, colors: Colors) -> String {
    let ui = settings::get().language;
    let minutes = ((article.length as f64 / 200.0).round() as i64).max(1);
    let site = article
        .site_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| display_host(source));
    let lang = article
        .lang
        .clone()
        .filter(|l| LANG_TAG.is_match(l))
        .unwrap_or_else(|| i18n::code(ui).to_string());
    let dir = if article.dir.as_deref() == Some("rtl") {
        "rtl"
    } else {
        "ltr"
    };
    let meta = format!(
        "{} · {}",
        escape(&site),
        escape(&i18n::translate(
            ui,
            "reader.minutes",
            &[("n", Var::Number(minutes))]
        ))
    );
    let palette = match colors {
        Colors::Light => format!(":root {{ {LIGHT} color-scheme: light; }}"),
        Colors::Dark => format!(":root {{ {DARK} color-scheme: dark; }}"),
        Colors::System => format!(
            ":root {{ {LIGHT} color-scheme: light dark; }}\n@media (prefers-color-scheme: dark) {{ :root {{ {DARK} }} }}"
        ),
    };
    let byline = article
        .byline
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .map(|b| format!("<p class=\"byline\">{}</p>\n", escape(b)))
        .unwrap_or_default();
    format!(
        r#"<!doctype html>
<html lang="{lang}" dir="{dir}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="{CSP}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<style>
@font-face {{ font-family: Newsreader; src: url(/fonts/newsreader-latin-ext-standard-normal.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0100-02BA, U+1E00-1EFF; }}
@font-face {{ font-family: Newsreader; src: url(/fonts/newsreader-latin-standard-normal.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }}
@font-face {{ font-family: Newsreader; font-style: italic; src: url(/fonts/newsreader-latin-ext-standard-italic.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0100-02BA, U+1E00-1EFF; }}
@font-face {{ font-family: Newsreader; font-style: italic; src: url(/fonts/newsreader-latin-standard-italic.woff2) format('woff2'); font-weight: 200 800; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }}
@font-face {{ font-family: Archivo; src: url(/fonts/archivo-latin-ext-standard-normal.woff2) format('woff2'); font-weight: 100 900; font-stretch: 62% 125%; unicode-range: U+0100-02BA, U+1E00-1EFF; }}
@font-face {{ font-family: Archivo; src: url(/fonts/archivo-latin-standard-normal.woff2) format('woff2'); font-weight: 100 900; font-stretch: 62% 125%; unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }}
@font-face {{ font-family: 'Plex Mono'; src: url(/fonts/ibm-plex-mono-latin-ext-400-normal.woff2) format('woff2'); unicode-range: U+0100-02BA, U+1E00-1EFF; }}
@font-face {{ font-family: 'Plex Mono'; src: url(/fonts/ibm-plex-mono-latin-400-normal.woff2) format('woff2'); unicode-range: U+0000-00FF, U+2000-206F, U+20AC, U+2122; }}
{palette}
:root {{ --fitil: #F2A93B; }}
* {{ box-sizing: border-box; }}
html {{ background: var(--bg); color: var(--text); -webkit-font-smoothing: antialiased; }}
body {{ margin: 0; padding: 72px 24px 128px; font: 400 20px/1.65 Newsreader, Georgia, serif; }}
main {{ max-width: 680px; margin: 0 auto; }}
.meta {{ font: 400 12px/1.3 'Plex Mono', monospace; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); margin: 0 0 24px; display: flex; align-items: center; gap: 12px; }}
.meta::before {{ content: ''; width: 14px; height: 6px; flex: none; background: radial-gradient(circle at 3px 3px, var(--fitil) 2.5px, transparent 3px), radial-gradient(circle at 11px 3px, var(--fitil) 2.5px, transparent 3px); }}
h1.title {{ font-family: Archivo, sans-serif; font-weight: 700; font-stretch: 112%; font-size: 44px; line-height: 1.05; letter-spacing: -0.015em; margin: 0 0 16px; }}
.byline {{ font-style: italic; color: var(--muted); margin: 0 0 48px; font-size: 18px; }}
article h1, article h2, article h3, article h4 {{ font-family: Archivo, sans-serif; line-height: 1.2; letter-spacing: -0.01em; margin: 48px 0 16px; }}
article h1 {{ font-size: 32px; font-weight: 700; font-stretch: 108%; }}
article h2 {{ font-size: 28px; font-weight: 650; font-stretch: 108%; }}
article h3 {{ font-size: 22px; font-weight: 600; }}
article h4 {{ font-size: 19px; font-weight: 600; }}
article p {{ margin: 0 0 24px; }}
article a {{ color: inherit; text-decoration: underline; text-decoration-color: var(--muted); text-underline-offset: 3px; }}
article a:hover {{ text-decoration-color: var(--fitil); }}
article img, article video, article svg {{ max-width: 100%; height: auto; display: block; margin: 32px auto; }}
article figure {{ margin: 32px 0; }}
article figcaption {{ font: 400 14px/1.5 Archivo, sans-serif; color: var(--muted); margin-top: -16px; }}
article blockquote {{ margin: 32px 0; padding: 0 0 0 24px; border-left: 2px solid var(--text); font-style: italic; font-size: 22px; line-height: 1.5; }}
article pre, article code {{ font-family: 'Plex Mono', monospace; font-size: 15px; }}
article pre {{ background: var(--surface); padding: 16px; overflow-x: auto; line-height: 1.5; border-radius: 4px; }}
article :not(pre) > code {{ background: var(--surface); padding: 1px 4px; border-radius: 4px; }}
article hr {{ border: 0; border-top: 1px solid var(--line); margin: 48px 0; }}
article table {{ border-collapse: collapse; width: 100%; font: 400 15px/1.5 Archivo, sans-serif; margin: 32px 0; }}
article th, article td {{ border-bottom: 1px solid var(--line); padding: 8px; text-align: start; }}
article ul, article ol {{ padding-inline-start: 24px; margin: 0 0 24px; }}
article li {{ margin: 0 0 8px; }}
footer {{ margin-top: 64px; padding-top: 24px; border-top: 1px solid var(--line); font: 400 12px/1.3 'Plex Mono', monospace; letter-spacing: .08em; text-transform: uppercase; }}
footer a {{ color: var(--muted); text-decoration: none; }}
footer a:hover {{ color: var(--text); }}
::selection {{ background: var(--fitil); color: #0B0B0C; }}
</style>
</head>
<body>
<main>
<p class="meta">{meta}</p>
<h1 class="title">{title}</h1>
{byline}<article>{content}</article>
<footer><a href="{source}">{host} →</a></footer>
</main>
</body>
</html>"#,
        CSP = CSP,
        title = escape(&article.title),
        content = article.content,
        source = escape(source),
        host = escape(&display_host(source)),
    )
}

/// No scripts at all, images and media from the web, our typefaces, and nothing sent anywhere
/// by forms. Also on the reading list's offline copies, which open in ordinary tabs.
const CSP: &str = "default-src 'none'; img-src https: http: data:; media-src https: http:; style-src 'unsafe-inline'; font-src peepwick:; form-action 'none'; base-uri 'none'";

// ── Serving it ────────────────────────────────────────────────────────────────

/// The typeface at `/fonts/<file>`, if it is one of ours.
pub fn font(path: &str) -> Option<&'static [u8]> {
    let name = path.strip_prefix("/fonts/")?;
    FONTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, data)| *data)
}

pub fn respond(request: &URISchemeRequest, body: glib::Bytes, content_type: &str) {
    let length = body.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&body);
    request.finish(&stream, length, Some(content_type));
}

/// A response whose type a page's fetch() sees as well: without headers, a custom scheme's
/// response has none to show. With `any_origin`, pages of other origins may read it.
pub fn respond_with_headers(
    request: &URISchemeRequest,
    body: glib::Bytes,
    content_type: &str,
    any_origin: bool,
) {
    let stream = gio::MemoryInputStream::from_bytes(&body);
    let response = URISchemeResponse::new(&stream, body.len() as i64);
    response.set_content_type(content_type);
    let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
    headers.append("Content-Type", content_type);
    if any_origin {
        headers.append("Access-Control-Allow-Origin", "*");
    }
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}

pub fn not_found(request: &URISchemeRequest) {
    let mut error = glib::Error::new(gio::IOErrorEnum::NotFound, "Not found");
    request.finish_error(&mut error);
}

/// Reader views' own WebKit context: in memory, and its peepwick: addresses serve only the
/// reader pages and their typefaces.
fn context() -> WebContext {
    CONTEXT.with(|cell| {
        if let Some(context) = cell.borrow().as_ref() {
            return context.clone();
        }
        let manager = WebsiteDataManager::new_ephemeral();
        let context = WebContext::with_website_data_manager(&manager);
        // The browser's extension too, given no settings: here it only tells the task manager
        // which process a reader view lives in.
        if let Some(dir) = crate::window::extension_directory() {
            context.set_web_extensions_directory(&dir);
        }
        if let Some(security) = context.security_manager() {
            security.register_uri_scheme_as_display_isolated("peepwick");
        }
        context.register_uri_scheme("peepwick", |request| {
            let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
            let path = request.path().map(|p| p.to_string()).unwrap_or_default();
            if pages::host_of(&uri) != "reader" {
                return not_found(request);
            }
            if let Some(data) = font(&path) {
                return respond(request, glib::Bytes::from_static(data), "font/woff2");
            }
            let token = path.trim_start_matches('/');
            match PAGES.with(|p| p.borrow().get(token).cloned()) {
                Some(html) => respond(request, glib::Bytes::from_owned(html), "text/html"),
                None => not_found(request),
            }
        });
        cell.replace(Some(context.clone()));
        context
    })
}

/// A reader view laid over one tab, and the article it shows.
pub struct Reader {
    pub view: WebView,
    pub article: Article,
    token: String,
    /// The view's web process, for the task manager.
    pub pid: Rc<Cell<Option<i32>>>,
}

impl Reader {
    /// The view for an article from `source`; the tab puts it in the window and wires it.
    pub fn new(article: Article, source: &str, colors: Colors, zoom: f64) -> Reader {
        let token = NEXT_TOKEN.with(|next| {
            let n = next.get();
            next.set(n + 1);
            format!("{n}-{}", crate::tab::now_ms())
        });
        PAGES.with(|p| {
            p.borrow_mut()
                .insert(token.clone(), render(&article, source, colors))
        });
        let manager = UserContentManager::new();
        // Trackers in the article's images stay blocked, unless the shield is off for the site.
        shield::attach(&manager);
        shield::set_paused(&manager, shield::paused_for(source));
        let settings = crate::window::web_settings();
        // The article's own scripts never run; the browser's (selection for the menu) still do.
        settings.set_enable_javascript_markup(false);
        settings.set_enable_developer_extras(false);
        settings.set_enable_media_stream(false);
        settings.set_enable_webrtc(false);
        settings.set_enable_html5_local_storage(false);
        settings.set_enable_html5_database(false);
        let view = WebView::builder()
            .user_content_manager(&manager)
            .settings(&settings)
            .web_context(&context())
            .build();
        view.set_zoom_level(zoom);
        view.load_uri(&format!("peepwick://reader/{token}"));
        Reader {
            view,
            article,
            token,
            pid: Rc::new(Cell::new(None)),
        }
    }

    /// Whether an address is this view's own page (or a place in it).
    pub fn is_own(&self, uri: &str) -> bool {
        uri.strip_prefix("peepwick://reader/")
            .is_some_and(|rest| rest.split('#').next() == Some(self.token.as_str()))
    }

    pub fn close(&self) {
        PAGES.with(|p| p.borrow_mut().remove(&self.token));
        self.view.stop_loading();
        if let Some(parent) = self.view.parent()
            && let Ok(container) = parent.downcast::<gtk::Container>()
        {
            container.remove(&self.view);
        }
    }
}

// ── Reading it aloud ──────────────────────────────────────────────────────────

static SKIPPED: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        "script", "style", "noscript", "svg", "math", "figure", "table", "pre",
    ]
    .iter()
    .map(|tag| Regex::new(&format!(r"(?is)<{tag}\b.*?</{tag}\s*>")).expect("valid pattern"))
    .collect()
});
static BREAK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid pattern"));
static BLOCK_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)</(p|div|h[1-6]|li|blockquote|figcaption|dd|dt|section|article|header|footer|tr)\s*>",
    )
    .expect("valid pattern")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").expect("valid pattern"));
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+);").expect("valid pattern"));
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid pattern"));
static WORDY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]").expect("valid pattern"));

fn decode_entities(text: &str) -> String {
    ENTITY
        .replace_all(text, |caps: &regex::Captures| {
            let code = &caps[1];
            let decoded = if let Some(number) = code.strip_prefix('#') {
                let value = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse::<u32>().ok(),
                };
                value.filter(|n| *n > 0).and_then(char::from_u32)
            } else {
                match code.to_ascii_lowercase().as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "mdash" => Some('—'),
                    "ndash" => Some('–'),
                    "hellip" => Some('…'),
                    "lsquo" => Some('‘'),
                    "rsquo" => Some('’'),
                    "ldquo" => Some('“'),
                    "rdquo" => Some('”'),
                    "laquo" => Some('«'),
                    "raquo" => Some('»'),
                    _ => None,
                }
            };
            decoded.map_or_else(|| caps[0].to_string(), String::from)
        })
        .into_owned()
}

/// The article's text read aloud: paragraphs in reading order, without code blocks, tables
/// or figures.
pub fn paragraphs(html: &str) -> Vec<String> {
    let mut text = html.to_string();
    for skipped in SKIPPED.iter() {
        text = skipped.replace_all(&text, "\n").into_owned();
    }
    let text = BREAK.replace_all(&text, "\n");
    let text = BLOCK_END.replace_all(&text, "\n");
    let text = TAG.replace_all(&text, "");
    decode_entities(&text)
        .split('\n')
        .map(|line| SPACE.replace_all(line, " ").trim().to_string())
        .filter(|line| WORDY.is_match(line))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_articles_saved_by_0_1_x() {
        let saved = r#"{"title":"T","content":"<p>x</p>","byline":null,"siteName":"Site","length":120,"lang":"tr","dir":null}"#;
        let article: Article = serde_json::from_str(saved).unwrap();
        assert_eq!(article.site_name.as_deref(), Some("Site"));
        assert_eq!(article.byline, None);
        assert_eq!(article.length, 120);
        let back = serde_json::to_value(&article).unwrap();
        assert_eq!(back["siteName"], "Site");
    }

    #[test]
    fn escapes_what_it_writes_around_the_article() {
        assert_eq!(
            escape(r#"<a href="x">'&'</a>"#),
            "&#60;a href=&#34;x&#34;&#62;&#39;&#38;&#39;&#60;/a&#62;"
        );
        assert_eq!(display_host("https://www.example.org/a"), "example.org");
    }

    #[test]
    fn serves_only_its_own_typefaces() {
        assert!(font("/fonts/archivo-latin-standard-normal.woff2").is_some_and(|f| f.len() > 1000));
        assert!(font("/fonts/../reader.rs").is_none());
        assert!(font("/archivo-latin-standard-normal.woff2").is_none());
    }

    #[test]
    fn turns_an_article_into_paragraphs() {
        let html = "<div><h2>Başlık &amp; alt</h2><p>Bir&nbsp;iki<br>üç</p>\
            <pre>code()</pre><figure><img src=x><figcaption>Not</figcaption></figure>\
            <ul><li>Madde &#8220;bir&#x201D;</li><li> </li></ul><p>&unknown; —</p>\
            <script>alert(1)</script><p>Son   söz</p></div>";
        assert_eq!(
            paragraphs(html),
            vec![
                "Başlık & alt",
                "Bir iki",
                "üç",
                "Madde “bir”",
                "&unknown; —",
                "Son söz"
            ]
        );
    }
}
