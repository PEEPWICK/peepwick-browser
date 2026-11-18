//! Page translation with Bergamot, the open engine Firefox's translations are built on, on this
//! computer. The engine (WebAssembly) runs in a worker of the interface's translator.html, in a
//! view of its own that is never shown. The models are Firefox's (Mozilla's public model
//! records), downloaded once per language pair after the person agrees and checked against the
//! SHA-256 the records list; only the 1.x generation, which this engine runs. The page's text is
//! collected, translated and put back by scripts in a world of their own, and never leaves the
//! browser. Languages without a direct model go through English.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use gtk::prelude::*;
use gtk::{gio, glib};
use javascriptcore::ValueExt;
use serde::Serialize;
use serde_json::{Value, json};
use webkit2gtk::{
    SecurityManagerExt, URISchemeRequest, URISchemeRequestExt, UserContentManager,
    UserContentManagerExt, WebContext, WebContextExt, WebView, WebViewExt,
};

use crate::tab::Tab;
use crate::{i18n, paths, settings, store, window};

const RECORDS_URL: &str = "https://firefox.settings.services.mozilla.com/v1/buckets/main/collections/translations-models/records";
const SERVER_URL: &str = "https://firefox.settings.services.mozilla.com/v1/";
/// Where the records' files are, when the server doesn't say.
const ATTACHMENTS: &str = "https://firefox-settings-attachments.cdn.mozilla.net/";
const RECORDS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const PIVOT: &str = "en";
/// A model's files, in the order the engine takes them.
const KINDS: [&str; 5] = ["model", "lex", "vocab", "srcvocab", "trgvocab"];

/// What the 1.x models translate, known without asking the server (Firefox's records of
/// 2026-11): into English, and out of it. The address bar offers translation from these.
const INTO_ENGLISH: [&str; 38] = [
    "bg", "bn", "ca", "cs", "da", "de", "el", "es", "et", "fa", "fi", "fr", "gu", "he", "hi", "hr",
    "hu", "id", "it", "kn", "lt", "lv", "ml", "ms", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sq",
    "sr", "sv", "te", "tr", "uk", "vi",
];
const FROM_ENGLISH: [&str; 31] = [
    "az", "bg", "bn", "ca", "cs", "da", "de", "el", "es", "et", "fa", "fi", "fr", "gu", "he", "hi",
    "hr", "id", "it", "kn", "ml", "ms", "nl", "pl", "pt", "ro", "sl", "sv", "ta", "te", "tr",
];

// ── Model records ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct RemoteFile {
    kind: String,
    /// Under the server's attachment address.
    location: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Pair {
    from: String,
    to: String,
    /// The model, the lexical shortlist, then one shared vocabulary or the source's and the
    /// target's.
    files: Vec<RemoteFile>,
}

fn valid_code(code: &str) -> bool {
    (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase())
}

/// The pairs Firefox's records offer in the 1.x generation, each in its newest final 1.x
/// version. Records only some Firefox builds get (a filter expression) are left out.
fn pick_pairs(records: &Value) -> Vec<Pair> {
    let code = |value: &Value| value.as_str().filter(|c| valid_code(c)).map(str::to_string);
    let mut found: HashMap<(String, String), HashMap<u32, HashMap<String, RemoteFile>>> =
        HashMap::new();
    for record in records.as_array().into_iter().flatten() {
        if record["filter_expression"]
            .as_str()
            .is_some_and(|filter| !filter.trim().is_empty())
        {
            continue;
        }
        let Some(minor) = record["version"]
            .as_str()
            .and_then(|v| v.strip_prefix("1."))
            .and_then(|minor| minor.parse::<u32>().ok())
        else {
            continue;
        };
        let (Some(from), Some(to)) = (code(&record["fromLang"]), code(&record["toLang"])) else {
            continue;
        };
        let Some(kind) = record["fileType"].as_str().filter(|k| KINDS.contains(k)) else {
            continue;
        };
        let attachment = &record["attachment"];
        let (Some(location), Some(size), Some(hash)) = (
            attachment["location"].as_str(),
            attachment["size"].as_u64(),
            attachment["hash"].as_str(),
        ) else {
            continue;
        };
        if location.contains("..") || location.starts_with('/') || hash.len() != 64 {
            continue;
        }
        found
            .entry((from, to))
            .or_default()
            .entry(minor)
            .or_default()
            .insert(
                kind.to_string(),
                RemoteFile {
                    kind: kind.to_string(),
                    location: location.to_string(),
                    size,
                    sha256: hash.to_lowercase(),
                },
            );
    }
    let mut pairs = Vec::new();
    for ((from, to), versions) in found {
        let mut minors: Vec<u32> = versions.keys().copied().collect();
        minors.sort_unstable_by(|a, b| b.cmp(a));
        for minor in minors {
            let files = &versions[&minor];
            let take = |kind: &str| files.get(kind).cloned();
            let vocabs = match (take("vocab"), take("srcvocab"), take("trgvocab")) {
                (Some(shared), _, _) => vec![shared],
                (None, Some(source), Some(target)) => vec![source, target],
                _ => continue,
            };
            let (Some(model), Some(lex)) = (take("model"), take("lex")) else {
                continue;
            };
            let mut list = vec![model, lex];
            list.extend(vocabs);
            pairs.push(Pair {
                from: from.clone(),
                to: to.clone(),
                files: list,
            });
            break;
        }
    }
    pairs.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    pairs
}

fn root() -> PathBuf {
    paths::user_path("translation")
}

fn records_path() -> PathBuf {
    root().join("records.json")
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("PEEPWICK-Browser/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| format!("offline: {e}"))
}

/// The records and where their files are, from the server.
fn fetch_records() -> Result<Value, String> {
    let client = client()?;
    let get = |url: &str| -> Result<Value, String> {
        let text = client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
            .map_err(|e| format!("offline: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("offline: {e}"))
    };
    let records = get(RECORDS_URL)?;
    let base = get(SERVER_URL)
        .ok()
        .and_then(|server| {
            server["capabilities"]["attachments"]["base_url"]
                .as_str()
                .map(str::to_string)
        })
        .filter(|base| base.starts_with("https://") && base.ends_with('/'))
        .unwrap_or_else(|| ATTACHMENTS.to_string());
    Ok(json!({ "base": base, "data": records["data"] }))
}

static PAIRS: Mutex<Option<(String, Vec<Pair>)>> = Mutex::new(None);

/// The pairs on offer and the attachment address: from the copy in the profile while it's less
/// than a week old, else from the server (the old copy when offline). Blocking.
fn pairs() -> Result<(String, Vec<Pair>), String> {
    if let Some(found) = PAIRS.lock().ok().and_then(|pairs| pairs.clone()) {
        return Ok(found);
    }
    let cached = || -> Option<(String, Vec<Pair>)> {
        let saved: Value = serde_json::from_slice(&std::fs::read(records_path()).ok()?).ok()?;
        Some((
            saved["base"].as_str()?.to_string(),
            pick_pairs(&saved["data"]),
        ))
    };
    let fresh = std::fs::metadata(records_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .is_some_and(|age| age < RECORDS_MAX_AGE);
    let found = match cached().filter(|_| fresh) {
        Some(found) => found,
        None => match fetch_records() {
            Ok(saved) => {
                let _ = std::fs::create_dir_all(root());
                let _ = store::write_atomic(&records_path(), saved.to_string().as_bytes());
                (
                    saved["base"].as_str().unwrap_or(ATTACHMENTS).to_string(),
                    pick_pairs(&saved["data"]),
                )
            }
            Err(error) => cached().ok_or(error)?,
        },
    };
    if let Ok(mut pairs) = PAIRS.lock() {
        *pairs = Some(found.clone());
    }
    Ok(found)
}

/// The steps from one language to another: one model, or two through English.
fn route(from: &str, to: &str) -> Vec<(String, String)> {
    if from == to {
        return Vec::new();
    }
    if from == PIVOT || to == PIVOT {
        return vec![(from.to_string(), to.to_string())];
    }
    vec![
        (from.to_string(), PIVOT.to_string()),
        (PIVOT.to_string(), to.to_string()),
    ]
}

/// Language tags as pages write them, as the models name languages.
pub fn normalize_lang(tag: &str) -> Option<String> {
    let value = tag.trim().replace('_', "-").to_lowercase();
    let base = value.split('-').next().unwrap_or_default();
    let base = match base {
        "no" => "nb",
        "iw" => "he",
        "in" => "id",
        other => other,
    };
    valid_code(base).then(|| base.to_string())
}

fn step_known(from: &str, to: &str) -> bool {
    (to == PIVOT && INTO_ENGLISH.contains(&from))
        || (from == PIVOT && FROM_ENGLISH.contains(&to))
        || model_files(from, to).is_some()
        || PAIRS.lock().ok().is_some_and(|pairs| {
            pairs
                .as_ref()
                .is_some_and(|(_, list)| list.iter().any(|p| p.from == from && p.to == to))
        })
}

/// Whether there are models for a translation, downloaded or not.
pub fn can_translate(from: &str, to: &str) -> bool {
    let steps = route(from, to);
    !steps.is_empty() && steps.iter().all(|(f, t)| step_known(f, t))
}

/// The language to translate into: the interface's when there are models for it, else English.
pub fn default_target() -> String {
    let ui = i18n::code(settings::get().language);
    if FROM_ENGLISH.contains(&ui) {
        ui.to_string()
    } else {
        PIVOT.to_string()
    }
}

/// Languages that can be translated from or into. Blocking: the records may be fetched.
pub fn languages() -> Vec<String> {
    let mut found: HashSet<String> = HashSet::from([PIVOT.to_string()]);
    match pairs() {
        Ok((_, list)) => {
            for pair in list {
                found.insert(pair.from);
                found.insert(pair.to);
            }
        }
        Err(_) => found.extend(
            INTO_ENGLISH
                .iter()
                .chain(&FROM_ENGLISH)
                .map(|c| c.to_string()),
        ),
    }
    for model in downloaded() {
        found.insert(model.0);
        found.insert(model.1);
    }
    let mut list: Vec<String> = found.into_iter().collect();
    list.sort();
    list
}

// ── Downloaded models ───────────────────────────────────────────────────────────

fn pair_dir(from: &str, to: &str) -> PathBuf {
    root().join(format!("{from}-{to}"))
}

/// A downloaded model's files, in the order the engine takes them, once every one is there.
fn model_files(from: &str, to: &str) -> Option<Vec<String>> {
    // Before the profile is known (the unit tests) there are none.
    if !valid_code(from) || !valid_code(to) || paths::profile_if_ready().is_none() {
        return None;
    }
    let dir = pair_dir(from, to);
    let files: Vec<String> =
        serde_json::from_slice(&std::fs::read(dir.join("files.json")).ok()?).ok()?;
    let complete = !files.is_empty()
        && files
            .iter()
            .all(|kind| KINDS.contains(&kind.as_str()) && dir.join(kind).is_file());
    complete.then_some(files)
}

/// (from, to, bytes) of every downloaded model.
fn downloaded() -> Vec<(String, String, u64)> {
    let Ok(entries) = std::fs::read_dir(root()) else {
        return Vec::new();
    };
    let mut models: Vec<(String, String, u64)> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| {
            let (from, to) = name.split_once('-')?;
            let files = model_files(from, to)?;
            let size = files
                .iter()
                .filter_map(|kind| std::fs::metadata(pair_dir(from, to).join(kind)).ok())
                .map(|meta| meta.len())
                .sum();
            Some((from.to_string(), to.to_string(), size))
        })
        .collect();
    models.sort();
    models
}

/// For Settings: the models downloaded so far.
pub fn models() -> Value {
    downloaded()
        .into_iter()
        .map(|(from, to, size)| json!({ "from": from, "to": to, "size": size }))
        .collect()
}

pub fn remove_model(from: &str, to: &str) {
    if !valid_code(from) || !valid_code(to) {
        return;
    }
    unload();
    if let Err(error) = std::fs::remove_dir_all(pair_dir(from, to))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        crate::log_error!("[translation] could not remove a model: {error}");
    }
}

/// The models a translation still needs; "unsupported" when a step has none. Blocking.
fn missing(from: &str, to: &str) -> Result<(String, Vec<Pair>), String> {
    let steps: Vec<(String, String)> = route(from, to)
        .into_iter()
        .filter(|(f, t)| model_files(f, t).is_none())
        .collect();
    if steps.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let (base, list) = pairs()?;
    let needed = steps
        .iter()
        .map(|(f, t)| {
            list.iter()
                .find(|p| p.from == *f && p.to == *t)
                .cloned()
                .ok_or_else(|| "unsupported".to_string())
        })
        .collect::<Result<Vec<Pair>, String>>()?;
    Ok((base, needed))
}

/// How many bytes a translation would download first. Blocking.
fn download_size(from: &str, to: &str) -> Result<u64, String> {
    Ok(missing(from, to)?
        .1
        .iter()
        .flat_map(|pair| &pair.files)
        .map(|file| file.size)
        .sum())
}

fn download_file(
    client: &reqwest::blocking::Client,
    url: &str,
    file: &RemoteFile,
    target: &std::path::Path,
    on_bytes: &mut dyn FnMut(usize),
) -> Result<(), String> {
    let mut response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("offline: {e}"))?;
    let mut hasher = openssl::sha::Sha256::new();
    let mut data = Vec::with_capacity(usize::try_from(file.size).unwrap_or(0));
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("offline: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        data.extend_from_slice(&buffer[..read]);
        on_bytes(read);
    }
    let digest: String = hasher.finish().iter().map(|b| format!("{b:02x}")).collect();
    if digest != file.sha256 {
        return Err("offline: checksum".into());
    }
    let part = target.with_extension("part");
    std::fs::write(&part, &data).map_err(|e| format!("failed: {e}"))?;
    std::fs::rename(&part, target).map_err(|e| format!("failed: {e}"))
}

/// Downloads what a translation needs, telling `progress` how far it got (0 to 1). Blocking.
fn ensure_models(from: &str, to: &str, progress: &dyn Fn(f64)) -> Result<(), String> {
    let (base, needed) = missing(from, to)?;
    if needed.is_empty() {
        return Ok(());
    }
    let total: u64 = needed.iter().flat_map(|p| &p.files).map(|f| f.size).sum();
    let mut received: u64 = 0;
    let client = client()?;
    for pair in needed {
        let dir = pair_dir(&pair.from, &pair.to);
        std::fs::create_dir_all(&dir).map_err(|e| format!("failed: {e}"))?;
        for file in &pair.files {
            let url = format!("{base}{}", file.location);
            download_file(&client, &url, file, &dir.join(&file.kind), &mut |n| {
                received += n as u64;
                progress(if total > 0 {
                    (received as f64 / total as f64).min(1.0)
                } else {
                    1.0
                });
            })?;
        }
        // Written last: a model counts as downloaded once every file is in place.
        let kinds: Vec<&str> = pair.files.iter().map(|f| f.kind.as_str()).collect();
        store::write_atomic(&dir.join("files.json"), json!(kinds).to_string().as_bytes())
            .map_err(|e| format!("failed: {e}"))?;
    }
    Ok(())
}

// ── The engine ──────────────────────────────────────────────────────────────────

const HANDLER: &str = "peepwickTranslator";
const MODEL_SCHEME: &str = "peepwick-model";
/// The engine holds a few hundred megabytes: it goes after this long without use.
const IDLE: Duration = Duration::from_secs(5 * 60);

type Answer = tokio::sync::oneshot::Sender<Result<Value, String>>;

struct Engine {
    window: gtk::OffscreenWindow,
    view: WebView,
    ready: Cell<bool>,
    /// Requests made before its page said it was ready.
    queued: RefCell<Vec<String>>,
    pending: RefCell<HashMap<u64, Answer>>,
    next: Cell<u64>,
    loaded: RefCell<HashSet<String>>,
    idle: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
    /// WebKit contexts the model scheme is registered in.
    static MODEL_CONTEXTS: RefCell<Vec<WebContext>> = const { RefCell::new(Vec::new()) };
}

impl Engine {
    fn run(&self, script: String) {
        if self.ready.get() {
            self.view
                .evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
        } else {
            self.queued.borrow_mut().push(script);
        }
    }

    fn touch(&self) {
        if let Some(timer) = self.idle.take() {
            timer.remove();
        }
        let timer = glib::timeout_add_local_once(IDLE, || {
            let idle = ENGINE.with(|engine| {
                engine.borrow().as_ref().is_some_and(|e| {
                    e.idle.take();
                    e.pending.borrow().is_empty()
                })
            });
            if idle {
                unload();
            }
        });
        self.idle.replace(Some(timer));
    }
}

/// Lets the engine go: its memory, and the models it loaded.
fn unload() {
    if let Some(engine) = ENGINE.with(|engine| engine.borrow_mut().take()) {
        if let Some(timer) = engine.idle.take() {
            timer.remove();
        }
        // Waiting requests fail: their answers are dropped.
        engine.pending.borrow_mut().clear();
        engine.window.close();
    }
}

fn engine() -> Result<Rc<Engine>, String> {
    if let Some(engine) = ENGINE.with(|engine| engine.borrow().clone()) {
        return Ok(engine);
    }
    let win = window::all()
        .into_iter()
        .next()
        .ok_or_else(|| "failed: no window".to_string())?;
    let context = win
        .ui_context()
        .ok_or_else(|| "failed: no interface context".to_string())?;
    crate::autofill::register_scheme(win.app_handle(), &context);
    register_models(&context);
    let manager = UserContentManager::new();
    manager.register_script_message_handler(HANDLER);
    manager.connect_script_message_received(Some(HANDLER), |_, result| {
        let text = result
            .js_value()
            .map(|value| value.to_str().to_string())
            .unwrap_or_default();
        engine_message(&text);
    });
    let view = WebView::builder()
        .web_context(&context)
        .user_content_manager(&manager)
        .build();
    // Never on screen: an offscreen window gives the view a place to live.
    let window = gtk::OffscreenWindow::new();
    window.add(&view);
    window.show_all();
    view.load_uri(&crate::autofill::interface_url(&win, "translator.html"));
    // Its page gone, its answers never come: the requests waiting fail, and the next gets a
    // new engine.
    view.connect_web_process_terminated(|_, _| {
        glib::idle_add_local_once(unload);
    });
    let engine = Rc::new(Engine {
        window,
        view,
        ready: Cell::new(false),
        queued: RefCell::new(Vec::new()),
        pending: RefCell::new(HashMap::new()),
        next: Cell::new(1),
        loaded: RefCell::new(HashSet::new()),
        idle: RefCell::new(None),
    });
    ENGINE.with(|slot| slot.replace(Some(engine.clone())));
    Ok(engine)
}

/// From translator.html: ready, or an answer.
fn engine_message(text: &str) {
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(engine) = ENGINE.with(|engine| engine.borrow().clone()) else {
        return;
    };
    if message["ready"] == true {
        engine.ready.set(true);
        for script in engine.queued.take() {
            engine.run(script);
        }
        return;
    }
    let Some(answer) = message["id"]
        .as_u64()
        .and_then(|id| engine.pending.borrow_mut().remove(&id))
    else {
        return;
    };
    let result = match message["error"].as_str() {
        Some(error) => Err(format!("failed: {error}")),
        None => Ok(message["result"].clone()),
    };
    let _ = answer.send(result);
    engine.touch();
}

/// How long the engine gets for one request before it's thought stuck and let go.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

async fn call(mut request: Value) -> Result<Value, String> {
    let engine = engine()?;
    let id = engine.next.get();
    engine.next.set(id + 1);
    request["id"] = json!(id);
    let (answer, result) = tokio::sync::oneshot::channel();
    engine.pending.borrow_mut().insert(id, answer);
    engine.touch();
    engine.run(format!("window.__pwTranslator({request})"));
    drop(engine);
    let result = match glib::future_with_timeout(CALL_TIMEOUT, result).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("failed: the translation engine stopped".to_string()),
        Err(_) => Err("failed: the translation engine took too long".to_string()),
    };
    // An engine that failed may be in no state to go on (WebAssembly that aborted stays
    // aborted): the next request gets a new one.
    if result.is_err() {
        unload();
    }
    result
}

async fn load_model(from: &str, to: &str) -> Result<(), String> {
    let engine = engine()?;
    let key = format!("{from}-{to}");
    if engine.loaded.borrow().contains(&key) {
        return Ok(());
    }
    let files = model_files(from, to).ok_or_else(|| "unsupported".to_string())?;
    let urls: Vec<String> = files
        .iter()
        .map(|kind| format!("{MODEL_SCHEME}://local/{key}/{kind}"))
        .collect();
    call(json!({ "op": "load", "from": from, "to": to, "urls": urls })).await?;
    engine.loaded.borrow_mut().insert(key);
    Ok(())
}

/// One request to the engine for the steps of a translation, loading their models first.
async fn engine_translate(
    steps: &[(String, String)],
    texts: &[String],
    html: bool,
) -> Result<Vec<String>, String> {
    for (from, to) in steps {
        load_model(from, to).await?;
    }
    let pairs: Vec<Value> = steps
        .iter()
        .map(|(from, to)| json!({ "from": from, "to": to }))
        .collect();
    let result =
        call(json!({ "op": "translate", "pairs": pairs, "texts": texts, "html": html })).await?;
    let translated: Vec<String> =
        serde_json::from_value(result).map_err(|e| format!("failed: {e}"))?;
    if translated.len() != texts.len() {
        return Err("failed: the engine answered for other texts".into());
    }
    Ok(translated)
}

/// Texts translated, plain or as markup; None for a text the engine couldn't take. A text the
/// engine fails on doesn't stop the others: they go again one at a time, and only an engine that
/// fails on all of them is an error.
async fn translate_texts(
    from: &str,
    to: &str,
    texts: Vec<String>,
    html: bool,
) -> Result<Vec<Option<String>>, String> {
    let steps = route(from, to);
    if steps.is_empty() || texts.is_empty() {
        return Ok(texts.into_iter().map(Some).collect());
    }
    match engine_translate(&steps, &texts, html).await {
        Ok(list) => Ok(list.into_iter().map(Some).collect()),
        Err(error) if error.starts_with("unsupported") || texts.len() == 1 => Err(error),
        Err(error) => {
            let mut list = Vec::with_capacity(texts.len());
            for text in &texts {
                let one = engine_translate(&steps, std::slice::from_ref(text), html).await;
                list.push(one.ok().and_then(|mut one| one.pop()));
            }
            if list.iter().all(Option::is_none) {
                Err(error)
            } else {
                Ok(list)
            }
        }
    }
}

fn register_models(context: &WebContext) {
    if MODEL_CONTEXTS.with(|known| known.borrow().contains(context)) {
        return;
    }
    MODEL_CONTEXTS.with(|known| known.borrow_mut().push(context.clone()));
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(MODEL_SCHEME);
        security.register_uri_scheme_as_cors_enabled(MODEL_SCHEME);
    }
    context.register_uri_scheme(MODEL_SCHEME, serve_model);
}

/// peepwick-model://local/<from>-<to>/<file>: a downloaded model's file, in the interface's
/// WebKit context only, read off the main thread.
fn serve_model(request: &URISchemeRequest) {
    let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
    let path = request.path().map(|p| p.to_string()).unwrap_or_default();
    let mut parts = path.trim_start_matches('/').splitn(2, '/');
    let (pair, kind) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let allowed = crate::pages::host_of(&uri) == "local"
        && KINDS.contains(&kind)
        && pair
            .split_once('-')
            .is_some_and(|(f, t)| valid_code(f) && valid_code(t));
    if !allowed {
        return crate::reader::not_found(request);
    }
    let file = gio::File::for_path(root().join(pair).join(kind));
    let request = request.clone();
    glib::MainContext::default().spawn_local(async move {
        match file.load_contents_future().await {
            // The engine's page has an origin of its own (the interface's files, or the
            // development server); only the interface's context has this scheme at all.
            Ok((bytes, _)) => crate::reader::respond_with_headers(
                &request,
                glib::Bytes::from(&bytes[..]),
                "application/octet-stream",
                true,
            ),
            Err(_) => crate::reader::not_found(&request),
        }
    });
}

// ── The page ────────────────────────────────────────────────────────────────────

/// A script world of our own in the page, away from its scripts. The web extension makes it
/// (webext/peepwick-webext.c), so what COLLECT keeps there lasts from one call to the next.
const WORLD: &str = "peepwick-translate";
/// Runs go to the engine in blocks of about this many characters, or this many runs, so the
/// page changes while the person watches.
const BLOCK_CHARS: usize = 2000;
const BLOCK_RUNS: usize = 40;

const DETECT: &str = "document.documentElement.lang || document.querySelector('meta[http-equiv=\"content-language\" i]')?.content || ''";

/// The page's text in runs: the text and inline elements between two block-level ones, as
/// markup the engine translates whole sentences of, as Firefox does. Inline elements go in
/// numbered, so the translation says where each one goes; ones it must not touch (code, images,
/// what the page marks translate="no") go in as numbered images. Putting a translation in place
/// keeps the page's own nodes: text nodes get the new words, elements stay with their parents
/// and only change places, so the page's scripts still find what they made. Undoing puts every
/// word and place back. A run the page changed meanwhile is left as it is.
const COLLECT: &str = r#"(() => {
  window.__pwTranslation?.restore()
  // Elements whose words belong to the sentence around them while the page shows them inline.
  const INLINE = new Set(['A', 'ABBR', 'B', 'BDI', 'BDO', 'BIG', 'CITE', 'DATA', 'DEL', 'DFN', 'EM', 'FONT', 'I', 'INS', 'LABEL', 'MARK', 'NOBR', 'Q', 'S', 'SMALL', 'SPAN', 'STRIKE', 'STRONG', 'SUB', 'SUP', 'TIME', 'TT', 'U'])
  // Never translated.
  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'TEXTAREA', 'INPUT', 'SELECT', 'OPTION', 'CODE', 'PRE', 'KBD', 'SAMP', 'VAR', 'IMG', 'PICTURE', 'CANVAS'])
  // Never moved, so nothing loads or starts again: a sentence ends at them.
  const FIXED = 'iframe, video, audio, object, embed'
  const memo = (compute) => {
    const known = new WeakMap()
    return (el) => {
      if (!known.has(el)) known.set(el, compute(el))
      return known.get(el)
    }
  }
  const skipped = memo((el) => el.namespaceURI !== 'http://www.w3.org/1999/xhtml' || SKIP.has(el.tagName) || el.isContentEditable || el.getAttribute('translate') === 'no' || el.classList.contains('notranslate') || (!!el.parentElement && skipped(el.parentElement)))
  // How an element takes part in the text around it: 'text', its words are part of the
  // sentence; 'keep', it stays as it is, where the translation puts it; 'apart', the sentence
  // ends there and its own text is translated by itself.
  const kind = memo((el) => {
    const display = getComputedStyle(el).display
    if (el.tagName === 'BR' || !(display.startsWith('inline') || display === 'contents' || display === 'none')) return 'apart'
    if (el.matches(FIXED) || el.querySelector(FIXED)) return 'apart'
    if (skipped(el) || display === 'none' || !/\S/.test(el.textContent)) return 'keep'
    // Footnote marks and the like, with no words: moved whole, as the engine would take them apart.
    if (!/\p{L}/u.test(el.textContent) && (el.firstElementChild || el.tagName === 'SUP' || el.tagName === 'SUB')) return 'keep'
    if ((display === 'inline' || display === 'contents') && INLINE.has(el.tagName) && [...el.children].every((child) => kind(child) !== 'apart')) return 'text'
    return 'apart'
  })
  const esc = (text) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  // What a kept element shows, when short: the engine gets it so it can tell where the element
  // goes in the translation, and its answer for it is not used.
  const shows = (el) => {
    const text = el.textContent.replace(/[ \t\n\r\f]+/g, ' ').trim()
    return text.length <= 120 ? text : ''
  }
  // A run's markup, its elements numbered in the order they come.
  const markup = (nodes, elements) => {
    let html = ''
    let words = false
    const add = (list) => {
      for (const node of list) {
        if (node.nodeType === 3) {
          html += esc(node.data)
          words ||= /\p{L}/u.test(node.data)
        } else if (node.nodeType === 1) {
          const i = elements.push(node) - 1
          if (kind(node) === 'keep') html += shows(node) ? `<span data-i="${i}">${esc(shows(node))}</span>` : `<img data-i="${i}">`
          else {
            html += `<span data-i="${i}">`
            add(node.childNodes)
            html += '</span>'
          }
        }
      }
    }
    add(nodes)
    return { html: html.replace(/[ \t\n\r\f]+/g, ' '), words }
  }
  const units = []
  const visit = (el) => {
    if (skipped(el)) return
    let run = []
    const end = (next) => {
      const { html, words } = markup(run, [])
      if (words) units.push({ parent: el, nodes: run, next, html: html.trim(), lead: html.startsWith(' '), trail: html.endsWith(' ') })
      run = []
    }
    for (const child of [...el.childNodes]) {
      if (child.nodeType === 3 || (child.nodeType === 1 && kind(child) !== 'apart')) run.push(child)
      else if (child.nodeType === 1) {
        end(child)
        visit(child)
      }
    }
    end(null)
  }
  if (document.body) visit(document.body)
  // Children in this order before `next`, moving only what isn't in its place.
  const order = (parent, children, next) => {
    let ref = next?.parentNode === parent ? next : null
    for (let i = children.length - 1; i >= 0; i--) {
      const node = children[i]
      if (node.parentNode && node.parentNode !== parent) continue
      if (node.parentNode !== parent || node.nextSibling !== ref) parent.insertBefore(node, ref)
      ref = node
    }
  }
  const place = (unit, html) => {
    if (!unit.nodes.every((node) => node.parentNode === unit.parent)) return
    const elements = []
    if (markup(unit.nodes, elements).html.trim() !== unit.html) return
    const template = document.createElement('template')
    template.innerHTML = (unit.lead ? ' ' : '') + html + (unit.trail ? ' ' : '')
    // Where each element goes: where the translation first has it, or for a kept element, where
    // it has what the element shows (the engine sometimes has an element twice).
    const chosen = new Map()
    for (const node of template.content.querySelectorAll('[data-i]')) {
      const el = elements[Number(node.getAttribute('data-i'))]
      if (!el) continue
      const best = chosen.get(el)
      const want = kind(el) === 'keep' && shows(el)
      if (!best || (want && !best.textContent.includes(want) && node.textContent.includes(want))) chosen.set(el, node)
    }
    // Text the engine put in a kept element, but for what the element shows (it keeps its own)
    // and stray pieces of that.
    const tidy = (extra, el) => {
      const want = shows(el)
      const rest = want ? extra.split(want).join('') : extra
      return /\p{L}/u.test(rest) ? rest : rest.replace(/[^\s.,;:!?…]/gu, '')
    }
    const saved = { texts: [], created: [], order: [[unit.parent, unit.nodes, unit.next]] }
    unit.saved = saved
    const used = new Set()
    // A live parent's own nodes, given what the translation has under it: its elements where
    // the translation puts them, everything else as text in its own text nodes.
    const merge = (parent, own, translated, next) => {
      const owned = new Set(own.filter((node) => node.nodeType === 1))
      const sequence = []
      let text = ''
      const walk = (node) => {
        for (const child of node.childNodes) {
          if (child.nodeType === 3) text += child.data
          else if (child.nodeType === 1) {
            const index = child.getAttribute('data-i')
            const el = index === null ? undefined : elements[Number(index)]
            const here = !!el && chosen.get(el) === child
            if (here && owned.has(el)) {
              used.add(el)
              if (text) sequence.push(text)
              text = ''
              sequence.push([el, child])
            } else if (el && kind(el) === 'keep') {
              if (!here) text += tidy(child.textContent, el)
            } else walk(child)
          }
        }
      }
      walk(translated)
      if (text) sequence.push(text)
      // Kept elements: the engine's answer for what they show is not used, but it may have
      // taken words of the sentence in with it, or left part of it outside.
      const list = []
      for (let k = 0; k < sequence.length; k++) {
        const item = sequence[k]
        if (typeof item === 'string' || kind(item[0]) !== 'keep') {
          list.push(item)
          continue
        }
        const [el, translatedEl] = item
        const want = shows(el)
        const last = () => (typeof list[list.length - 1] === 'string' ? list[list.length - 1] : '')
        const setLast = (text) => {
          if (typeof list[list.length - 1] === 'string') list[list.length - 1] = text
          else if (text) list.push(text)
        }
        const next = () => (typeof sequence[k + 1] === 'string' ? sequence[k + 1] : '')
        let got = translatedEl.textContent
        // What it shows split across its edges ("[<span>4]</span>"): those parts are its own.
        for (let n = 1; want && !got.includes(want) && n <= Math.min(last().length, want.length); n++) {
          if ((last().slice(-n) + got).includes(want)) {
            got = last().slice(-n) + got
            setLast(last().slice(0, -n))
          }
        }
        for (let n = 1; want && !got.includes(want) && n <= Math.min(next().length, want.length); n++) {
          if ((got + next().slice(0, n)).includes(want)) {
            got += next().slice(0, n)
            sequence[k + 1] = next().slice(n)
          }
        }
        // Anything else in it goes around it. Words in place of what it shows are the
        // sentence's too, unless what it shows has words: then they're its translation.
        const at = want ? got.indexOf(want) : -1
        setLast(last() + (at > 0 ? tidy(got.slice(0, at), el) : ''))
        const after = at >= 0 ? tidy(got.slice(at + want.length), el) : /\p{L}/u.test(want) ? '' : tidy(got, el)
        // Put right against a word, it gets back the space it had.
        const { previousSibling: left, nextSibling: right } = el
        if (left?.nodeType === 3 && /\s$/.test(left.data) && /[\p{L}\p{N}]$/u.test(last())) setLast(last() + ' ')
        list.push(item)
        if (after && typeof sequence[k + 1] === 'string') sequence[k + 1] = after + sequence[k + 1]
        else if (after) sequence.splice(k + 1, 0, after)
        if (right?.nodeType === 3 && /^\s/.test(right.data) && /^[\p{L}\p{N}]/u.test(next())) sequence[k + 1] = ' ' + next()
      }
      const items = list.filter((item) => item !== '')
      const texts = own.filter((node) => node.nodeType === 3)
      const placed = items.map((item) => {
        if (typeof item !== 'string') return item[0]
        const node = texts.shift()
        if (!node) {
          const created = document.createTextNode(item)
          saved.created.push(created)
          return created
        }
        saved.texts.push([node, node.data])
        node.data = item
        return node
      })
      for (const node of texts) {
        saved.texts.push([node, node.data])
        node.data = ''
      }
      order(parent, placed, next)
      const inner = (el, translatedEl) => {
        if (kind(el) !== 'text') return
        const children = [...el.childNodes]
        saved.order.push([el, children, null])
        merge(el, children, translatedEl, null)
      }
      for (const item of items) if (typeof item !== 'string') inner(item[0], item[1])
      // Elements the translation has elsewhere, or not at all: their words are elsewhere.
      for (const el of owned) if (!used.has(el)) inner(el, document.createDocumentFragment())
    }
    merge(unit.parent, unit.nodes, template.content, unit.next)
  }
  const undo = (unit) => {
    const saved = unit.saved
    if (!saved) return
    unit.saved = null
    for (const node of saved.created) node.remove()
    for (const [node, data] of saved.texts) node.data = data
    for (const [parent, children, next] of saved.order) order(parent, children, next)
  }
  const state = {
    title: document.title,
    setTitle(text) {
      state.translatedTitle = document.title = text
    },
    apply(start, translations) {
      translations.forEach((html, i) => {
        const unit = units[start + i]
        if (!unit || typeof html !== 'string') return
        try {
          place(unit, html)
        } catch {}
      })
    },
    restore() {
      for (const unit of [...units].reverse()) {
        try {
          undo(unit)
        } catch {}
      }
      if (state.translatedTitle !== undefined && document.title === state.translatedTitle) document.title = state.title
    },
  }
  window.__pwTranslation = state
  return JSON.stringify({ texts: units.map((unit) => unit.html), title: document.title })
})()"#;

const RESTORE: &str =
    "(() => { window.__pwTranslation?.restore(); delete window.__pwTranslation })()";

/// Puts the translations of runs `start` on in place; null leaves a run as it is.
fn apply_script(start: usize, texts: &[Value]) -> String {
    format!(
        "window.__pwTranslation?.apply({start}, {})",
        Value::Array(texts.to_vec())
    )
}

fn title_script(title: &str) -> String {
    format!("window.__pwTranslation?.setTitle({})", json!(title))
}

/// The runs in blocks for the engine, in the page's order.
fn blocks(texts: &[String]) -> Vec<std::ops::Range<usize>> {
    let mut list = Vec::new();
    let (mut start, mut chars) = (0, 0);
    for (i, text) in texts.iter().enumerate() {
        if i > start && (chars + text.len() > BLOCK_CHARS || i - start >= BLOCK_RUNS) {
            list.push(start..i);
            (start, chars) = (i, 0);
        }
        chars += text.len();
    }
    if start < texts.len() {
        list.push(start..texts.len());
    }
    list
}

async fn in_page(view: &WebView, script: &str) -> Result<String, String> {
    view.evaluate_javascript_future(script, Some(WORLD), None)
        .await
        .map(|value| value.to_str().to_string())
        .map_err(|e| format!("failed: {}", e.message()))
}

/// Translates the page in place, a block of runs at a time. Ok(false) when it stopped partway
/// because `cancelled` said so.
async fn translate_page(
    view: &WebView,
    from: &str,
    to: &str,
    progress: impl Fn(f64),
    cancelled: impl Fn() -> bool,
) -> Result<bool, String> {
    let collected: Value =
        serde_json::from_str(&in_page(view, COLLECT).await?).map_err(|e| format!("failed: {e}"))?;
    let texts: Vec<String> = serde_json::from_value(collected["texts"].clone()).unwrap_or_default();
    let title = collected["title"].as_str().unwrap_or("").trim().to_string();
    // What COLLECT left in the world has to be there for the rest (see WORLD): without the
    // extension's world, the page would stay as it is while the bar said it was translated.
    if in_page(view, "typeof window.__pwTranslation").await? != "object" {
        return Err("failed: the page's script world forgot the translation".into());
    }
    if !title.is_empty() {
        // A title the engine can't take leaves the title as it is.
        let translated = translate_texts(from, to, vec![title], false).await;
        if cancelled() {
            return Ok(false);
        }
        if let Some(Some(title)) = translated.ok().and_then(|list| list.into_iter().next()) {
            in_page(view, &title_script(&title)).await?;
        }
    }
    let mut done: HashMap<String, Option<String>> = HashMap::new();
    let total = texts.len().max(1);
    for range in blocks(&texts) {
        let block = &texts[range.clone()];
        let mut wanted: Vec<String> = Vec::new();
        for text in block {
            if !done.contains_key(text) && !wanted.contains(text) {
                wanted.push(text.clone());
            }
        }
        if !wanted.is_empty() {
            let translated = translate_texts(from, to, wanted.clone(), true).await?;
            done.extend(wanted.into_iter().zip(translated));
        }
        if cancelled() {
            return Ok(false);
        }
        let values: Vec<Value> = block
            .iter()
            .map(|text| {
                done.get(text)
                    .cloned()
                    .flatten()
                    .map_or(Value::Null, Value::String)
            })
            .collect();
        in_page(view, &apply_script(range.start, &values)).await?;
        progress(range.end as f64 / total as f64);
    }
    Ok(true)
}

// ── A tab's translation ─────────────────────────────────────────────────────────

/// What the translate bar shows, as `TranslationState` in the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationState {
    pub status: &'static str,
    pub from: String,
    pub to: String,
    pub progress: f64,
    /// Bytes still to download; -1 while unknown.
    pub download: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
}

/// The bar's word for what went wrong.
fn error_kind(error: &str) -> &'static str {
    if error.starts_with("unsupported") {
        "unsupported"
    } else if error.starts_with("offline") {
        "offline"
    } else {
        "failed"
    }
}

fn web(url: &str) -> bool {
    url.starts_with("http:") || url.starts_with("https:")
}

/// Stops what runs for the tab's translation; returns the new run's number.
fn bump(tab: &Tab) -> u32 {
    let run = tab.translation_run.get().wrapping_add(1);
    tab.translation_run.set(run);
    run
}

/// Changes the bar's state if the run is still the tab's.
fn update(tab: &Tab, run: u32, change: impl FnOnce(&mut TranslationState)) {
    if tab.translation_run.get() != run {
        return;
    }
    let changed = match tab.translation.borrow_mut().as_mut() {
        Some(state) => {
            change(state);
            true
        }
        None => false,
    };
    if changed {
        tab.changed();
    }
}

/// A new page: no language yet, no bar, and what ran for the last page stops.
pub fn new_page(tab: &Tab) {
    bump(tab);
    tab.page_lang.replace(None);
    tab.translation.replace(None);
}

/// Once a page has loaded: its language, when it could be translated into the default target.
pub fn detect(tab: &Rc<Tab>) {
    let Some(view) = tab.view() else { return };
    if !web(&tab.url()) || tab.page().is_some() {
        return;
    }
    let (weak, run) = (Rc::downgrade(tab), tab.translation_run.get());
    glib::MainContext::default().spawn_local(async move {
        let found = in_page(&view, DETECT).await.ok();
        let Some(tab) = weak.upgrade() else { return };
        if tab.translation_run.get() != run {
            return;
        }
        let target = default_target();
        let lang = found
            .as_deref()
            .and_then(normalize_lang)
            .filter(|lang| can_translate(lang, &target));
        if *tab.page_lang.borrow() != lang {
            tab.page_lang.replace(lang);
            tab.changed();
        }
    });
}

/// The bar's buttons and the address bar's: "offer", "choose", "start", "revert", "close".
pub fn action(tab: &Rc<Tab>, action: &str, from: Option<String>, to: Option<String>) {
    match (action, from, to) {
        ("offer", ..) => offer(tab),
        ("choose", Some(from), Some(to)) => choose(tab, from, to),
        ("start", Some(from), Some(to)) => start(tab, from, to),
        ("revert", ..) => revert(tab),
        ("close", ..) => close(tab),
        _ => {}
    }
}

/// Opens the translate bar, or closes it when it's open.
pub fn offer(tab: &Rc<Tab>) {
    if tab.translation.borrow().is_some() {
        return close(tab);
    }
    if tab.view().is_none() || tab.page().is_some() || tab.has_interstitial() {
        return;
    }
    let from = tab
        .page_lang
        .borrow()
        .clone()
        .unwrap_or_else(|| PIVOT.to_string());
    choose(tab, from, default_target());
}

/// The bar's languages changed: how much would be downloaded is looked up.
fn choose(tab: &Rc<Tab>, from: String, to: String) {
    let run = bump(tab);
    tab.translation.replace(Some(TranslationState {
        status: "offer",
        from: from.clone(),
        to: to.clone(),
        progress: 0.0,
        download: -1,
        error: None,
    }));
    tab.changed();
    let weak = Rc::downgrade(tab);
    glib::MainContext::default().spawn_local(async move {
        let size = gio::spawn_blocking(move || download_size(&from, &to))
            .await
            .unwrap_or_else(|_| Err("failed".into()));
        let Some(tab) = weak.upgrade() else { return };
        update(&tab, run, |state| match size {
            Ok(bytes) => state.download = i64::try_from(bytes).unwrap_or(i64::MAX),
            Err(error) => {
                state.download = 0;
                state.error = Some(error_kind(&error));
            }
        });
    });
}

fn start(tab: &Rc<Tab>, from: String, to: String) {
    let Some(view) = tab.view() else { return };
    if tab.page().is_some() || tab.has_interstitial() || from == to {
        return;
    }
    let run = bump(tab);
    tab.translation.replace(Some(TranslationState {
        status: "downloading",
        from: from.clone(),
        to: to.clone(),
        progress: 0.0,
        download: -1,
        error: None,
    }));
    tab.changed();
    let weak = Rc::downgrade(tab);
    glib::MainContext::default().spawn_local(async move {
        let result = run_translation(&weak, run, &view, &from, &to).await;
        let Some(tab) = weak.upgrade() else { return };
        match result {
            Ok(true) => update(&tab, run, |state| {
                state.status = "done";
                state.progress = 1.0;
            }),
            Ok(false) => {}
            Err(error) => {
                eprintln!("[translation] {error}");
                update(&tab, run, |state| {
                    state.status = "error";
                    state.error = Some(error_kind(&error));
                });
            }
        }
    });
}

async fn run_translation(
    tab: &Weak<Tab>,
    run: u32,
    view: &WebView,
    from: &str,
    to: &str,
) -> Result<bool, String> {
    let current = |tab: &Weak<Tab>| tab.upgrade().filter(|t| t.translation_run.get() == run);
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let (f, t) = (from.to_string(), to.to_string());
    let download = gio::spawn_blocking(move || {
        ensure_models(&f, &t, &|fraction| {
            let _ = sender.send(fraction);
        })
    });
    // The channel closes when the download is over.
    while let Some(fraction) = receiver.recv().await {
        if let Some(tab) = current(tab) {
            update(&tab, run, |state| state.progress = fraction);
        }
    }
    download.await.unwrap_or_else(|_| Err("failed".into()))?;
    let Some(owner) = current(tab) else {
        return Ok(false);
    };
    update(&owner, run, |state| {
        state.status = "translating";
        state.progress = 0.0;
        state.download = 0;
    });
    drop(owner);
    translate_page(
        view,
        from,
        to,
        |fraction| {
            if let Some(tab) = current(tab) {
                update(&tab, run, |state| state.progress = fraction);
            }
        },
        || current(tab).is_none(),
    )
    .await
}

/// Back to the page as it was; the bar offers the same languages again.
fn revert(tab: &Rc<Tab>) {
    let previous = tab.translation.borrow().clone();
    bump(tab);
    if let Some(view) = tab.view() {
        view.evaluate_javascript(
            RESTORE,
            Some(WORLD),
            None,
            None::<&gio::Cancellable>,
            |_| {},
        );
    }
    if let Some(previous) = previous {
        choose(tab, previous.from, previous.to);
    }
}

/// Hides the bar; a translated page stays translated.
fn close(tab: &Tab) {
    bump(tab);
    tab.translation.replace(None);
    tab.changed();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(from: &str, to: &str, version: &str, kind: &str) -> Value {
        json!({
            "fromLang": from, "toLang": to, "version": version, "fileType": kind,
            "attachment": { "location": format!("main-workspace/translations-models/{from}{to}{version}{kind}.bin"), "size": 10, "hash": "a".repeat(64) }
        })
    }

    #[test]
    fn picks_the_newest_final_1x_model_of_each_pair() {
        let mut records = Vec::new();
        for kind in ["model", "lex", "vocab"] {
            records.push(record("tr", "en", "1.0", kind));
            records.push(record("tr", "en", "1.1", kind));
            records.push(record("tr", "en", "1.2a1", kind));
            records.push(record("tr", "en", "2.0", kind));
            records.push(record("en", "ja", "2.1", kind));
        }
        for kind in ["model", "lex", "srcvocab", "trgvocab"] {
            records.push(record("en", "tr", "1.0", kind));
        }
        // Only for some Firefox builds, or missing a file: left out.
        let mut nightly = record("de", "en", "1.0", "model");
        nightly["filter_expression"] = json!("env.channel == 'nightly'");
        records.push(nightly);
        records.push(record("fr", "en", "1.0", "model"));
        let pairs = pick_pairs(&Value::Array(records));
        let summary: Vec<(String, String, Vec<String>)> = pairs
            .iter()
            .map(|p| {
                (
                    p.from.clone(),
                    p.to.clone(),
                    p.files
                        .iter()
                        .map(|f| format!("{} {}", f.kind, f.location))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].0, "en");
        assert_eq!(
            summary[0]
                .2
                .iter()
                .map(|f| f.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            ["model", "lex", "srcvocab", "trgvocab"]
        );
        assert_eq!(summary[1].0, "tr");
        assert!(summary[1].2.iter().all(|f| f.contains("tren1.1")));
    }

    #[test]
    fn routes_through_english() {
        assert!(route("tr", "tr").is_empty());
        assert_eq!(route("tr", "en"), [("tr".to_string(), "en".to_string())]);
        assert_eq!(
            route("de", "tr"),
            [
                ("de".to_string(), "en".to_string()),
                ("en".to_string(), "tr".to_string())
            ]
        );
    }

    #[test]
    fn reads_language_tags() {
        assert_eq!(normalize_lang("tr-TR").as_deref(), Some("tr"));
        assert_eq!(normalize_lang(" en_US ").as_deref(), Some("en"));
        assert_eq!(normalize_lang("no").as_deref(), Some("nb"));
        assert_eq!(normalize_lang("iw").as_deref(), Some("he"));
        assert_eq!(normalize_lang(""), None);
        assert_eq!(normalize_lang("x-klingon"), None);
    }

    #[test]
    fn knows_the_1x_languages_without_the_server() {
        assert!(can_translate("tr", "en"));
        assert!(can_translate("de", "tr"));
        assert!(can_translate("en", "az"));
        // Azerbaijani has no model into English, Japanese none at all in 1.x.
        assert!(!can_translate("az", "en"));
        assert!(!can_translate("ja", "en"));
        assert!(!can_translate("en", "en"));
    }

    #[test]
    fn sends_runs_in_blocks_in_page_order() {
        let short: Vec<String> = (0..100).map(|i| format!("run {i}")).collect();
        let ranges = blocks(&short);
        assert_eq!(ranges, [0..40, 40..80, 80..100]);
        let mut mixed = vec!["x".repeat(1500), "y".repeat(400), "z".repeat(300)];
        mixed.push("w".repeat(5000));
        mixed.push("v".to_string());
        // A run longer than a block goes alone; none is ever left out or split.
        assert_eq!(blocks(&mixed), [0..2, 2..3, 3..4, 4..5]);
        assert!(blocks(&[]).is_empty());
    }
}
