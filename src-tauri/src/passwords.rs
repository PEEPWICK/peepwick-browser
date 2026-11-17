//! Saved passwords. Each one is encrypted with AES-256-GCM under a key the desktop's Secret
//! Service keeps (GNOME Keyring, KWallet), as Chrome and 0.1.x keep theirs; on a desktop with
//! no keyring a fixed key stands in, which protects nothing, and the passwords page says so.
//! Passwords leave this module only to be filled into a page of the same site, copied,
//! exported, or shown on the passwords page at the person's request.
//!
//! passwords.json keeps 0.1.x's layout (version 1, logins, the sites never to save for); only
//! the encryption differs, so 0.1.x's file can't be read: its passwords come over as CSV or
//! in a backup (backup.rs).
//!
//! The list lives on the main thread. The key is read from the keyring off it, the first time
//! something needs it (`ready`, `when_ready`), and kept in memory after.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use gtk::glib;
use gtk::prelude::*;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::passwordcsv::{self, CsvRow, origin_of};
use crate::secrets;
use crate::tab::now_ms;
use crate::window::{self, BrowserWindow};

const FILE: &str = "passwords.json";
/// The keyring item that holds the key.
const KEY_LABEL: &str = "PEEPWICK Browser Safe Storage";
const KEY_ATTRIBUTES: [(&str, &str); 2] = [
    ("application", "peepwick-browser"),
    ("xdg:schema", "com.peepwick.Browser.SafeStorage"),
];
/// What a value starts with: encrypted with the keyring's key, or with the fixed one. The
/// numbers follow Chromium's (v11 keyring, v10 none), the letter says the cipher (GCM).
const KEYRING_PREFIX: &[u8; 3] = b"g11";
const BASIC_PREFIX: &[u8; 3] = b"g10";
const NONCE: usize = 12;
const TAG: usize = 16;
const MAX_USERNAME: usize = 500;
const MAX_PASSWORD: usize = 1000;
const MAX_NOTE: usize = 2000;
/// A copied password leaves the clipboard after this long, unless something replaced it.
const CLIPBOARD_SECONDS: u32 = 60;

// ── The key ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum Keyring {
    /// Not asked yet.
    Unknown,
    Ready([u8; 32]),
    /// The keyring is there but holds no key yet.
    Missing,
    /// No Secret Service on this desktop: the fixed key stands in.
    Unavailable,
    /// The keyring stayed locked, or failed: nothing can be read or saved until it opens.
    Locked,
}

static KEYRING: Mutex<Keyring> = Mutex::new(Keyring::Unknown);
/// One look at the keyring at a time.
static ASKING: Mutex<()> = Mutex::new(());

fn keyring() -> Keyring {
    KEYRING.lock().map(|k| *k).unwrap_or(Keyring::Locked)
}

fn set_keyring(state: Keyring) {
    if let Ok(mut k) = KEYRING.lock() {
        *k = state;
    }
}

/// The key that protects nothing, for a desktop without a keyring.
fn basic_key() -> [u8; 32] {
    openssl::sha::sha256(b"PEEPWICK Browser: no keyring")
}

/// Whether what needs the key can go ahead without asking the keyring again.
fn settled(create: bool) -> bool {
    match keyring() {
        Keyring::Ready(_) | Keyring::Unavailable => true,
        Keyring::Missing => !create,
        Keyring::Unknown | Keyring::Locked => false,
    }
}

/// Reads the key from the keyring, or makes one with `create` when it has none. Blocking.
fn prepare(create: bool) {
    let Ok(_one) = ASKING.lock() else { return };
    if settled(create) {
        return;
    }
    let state = match secrets::lookup(&KEY_ATTRIBUTES) {
        Ok(Some(secret)) => match BASE64
            .decode(&secret)
            .ok()
            .and_then(|key| <[u8; 32]>::try_from(key).ok())
        {
            Some(key) => {
                eprintln!("[passwords] key read from the keyring");
                Keyring::Ready(key)
            }
            None => {
                // Not ours to overwrite: passwords saved with it would be lost for good.
                crate::log_error!("[passwords] the keyring holds a key in an unknown form");
                Keyring::Locked
            }
        },
        Ok(None) if create => {
            let mut key = [0u8; 32];
            if openssl::rand::rand_bytes(&mut key).is_err() {
                Keyring::Locked
            } else {
                match secrets::store(KEY_LABEL, &KEY_ATTRIBUTES, BASE64.encode(key).as_bytes()) {
                    Ok(()) => {
                        eprintln!("[passwords] key made and kept in the keyring");
                        Keyring::Ready(key)
                    }
                    Err(secrets::Error::Unavailable(_)) => Keyring::Unavailable,
                    Err(error) => {
                        crate::log_error!("[passwords] could not keep the key: {error}");
                        Keyring::Locked
                    }
                }
            }
        }
        Ok(None) => Keyring::Missing,
        Err(secrets::Error::Unavailable(why)) => {
            eprintln!("[passwords] no keyring, passwords are kept without protection: {why}");
            Keyring::Unavailable
        }
        Err(secrets::Error::Dismissed) => Keyring::Locked,
        Err(error) => {
            crate::log_error!("[passwords] keyring: {error}");
            Keyring::Locked
        }
    };
    set_keyring(state);
}

/// Makes sure the key is known (and made, with `create`) before what needs it; from async code.
pub async fn ready(create: bool) {
    if !settled(create) {
        let _ = tauri::async_runtime::spawn_blocking(move || prepare(create)).await;
    }
}

/// The same for the main thread: `then` runs there, at once if the key is known.
pub fn when_ready(create: bool, then: impl FnOnce() + Send + 'static) {
    if settled(create) {
        then();
        return;
    }
    std::thread::spawn(move || {
        prepare(create);
        glib::MainContext::default().invoke(then);
    });
}

fn seal(prefix: &[u8; 3], key: &[u8; 32], plain: &str) -> Option<String> {
    let mut nonce = [0u8; NONCE];
    openssl::rand::rand_bytes(&mut nonce).ok()?;
    let mut tag = [0u8; TAG];
    let sealed = encrypt_aead(
        Cipher::aes_256_gcm(),
        key,
        Some(&nonce),
        &[],
        plain.as_bytes(),
        &mut tag,
    )
    .ok()?;
    let mut out = Vec::with_capacity(3 + NONCE + sealed.len() + TAG);
    out.extend_from_slice(prefix);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    out.extend_from_slice(&tag);
    Some(BASE64.encode(out))
}

fn unseal(secret: &str, keyring_key: Option<[u8; 32]>) -> Option<String> {
    let data = BASE64.decode(secret).ok()?;
    if data.len() < 3 + NONCE + TAG {
        return None;
    }
    let (prefix, rest) = data.split_at(3);
    let key = if prefix == KEYRING_PREFIX {
        keyring_key?
    } else if prefix == BASIC_PREFIX {
        basic_key()
    } else {
        return None;
    };
    let (nonce, rest) = rest.split_at(NONCE);
    let (sealed, tag) = rest.split_at(rest.len() - TAG);
    let plain = decrypt_aead(Cipher::aes_256_gcm(), &key, Some(nonce), &[], sealed, tag).ok()?;
    String::from_utf8(plain).ok()
}

fn encrypt(plain: &str) -> Option<String> {
    match keyring() {
        Keyring::Ready(key) => seal(KEYRING_PREFIX, &key, plain),
        Keyring::Unavailable => seal(BASIC_PREFIX, &basic_key(), plain),
        _ => None,
    }
}

fn decrypt(secret: &str) -> Option<String> {
    let key = match keyring() {
        Keyring::Ready(key) => Some(key),
        _ => None,
    };
    unseal(secret, key)
}

/// For autofill's other data (formdata.rs), sealed the same way with the same key.
pub(crate) fn seal_text(plain: &str) -> Option<String> {
    encrypt(plain)
}

pub(crate) fn open_text(secret: &str) -> Option<String> {
    decrypt(secret)
}

/// Whether sealing works now: the key is known, or there's no keyring and the fixed key stands in.
pub(crate) fn can_seal() -> bool {
    matches!(keyring(), Keyring::Ready(_) | Keyring::Unavailable)
}

/// Whether the keyring has been asked and answered: what the key can't open now, it never will.
pub(crate) fn key_settled() -> bool {
    matches!(
        keyring(),
        Keyring::Ready(_) | Keyring::Unavailable | Keyring::Missing
    )
}

// ── The list ────────────────────────────────────────────────────────────────────

/// Milliseconds, read leniently: a number written as a float still counts.
fn millis<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Value::deserialize(d)?
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map_or(0, |n| n as u64))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    id: String,
    origin: String,
    username: String,
    /// The password, encrypted (`seal`).
    secret: String,
    #[serde(deserialize_with = "millis")]
    created: u64,
    #[serde(deserialize_with = "millis")]
    updated: u64,
    #[serde(deserialize_with = "millis")]
    last_used: u64,
    #[serde(deserialize_with = "millis")]
    times_used: u64,
    note: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct PasswordFile {
    version: u32,
    logins: Vec<Stored>,
    /// Sites where the person chose "never save".
    never: Vec<String>,
}

impl Default for PasswordFile {
    fn default() -> Self {
        PasswordFile {
            version: 1,
            logins: Vec::new(),
            never: Vec::new(),
        }
    }
}

/// A login in the clear, from another browser, a CSV file, or the add dialog.
#[derive(Clone, Debug, Default)]
pub struct PlainLogin {
    pub origin: String,
    pub username: String,
    pub password: String,
    pub note: Option<String>,
    pub created: Option<u64>,
    pub updated: Option<u64>,
}

thread_local! {
    static DATA: RefCell<PasswordFile> = RefCell::new(PasswordFile::default());
    static LAST_REMOVED: RefCell<Vec<Stored>> = const { RefCell::new(Vec::new()) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Counts copies, so an older copy's timer leaves a newer one alone.
static COPIES: AtomicU64 = AtomicU64::new(0);

fn file() -> PathBuf {
    crate::paths::user_path(FILE)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if openssl::rand::rand_bytes(&mut bytes).is_err() {
        bytes = (u128::from(now_ms()) ^ u128::from(COPIES.fetch_add(1, Ordering::Relaxed)))
            .to_le_bytes();
    }
    // A version 4 UUID, as 0.1.x's randomUUID made.
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

pub fn load() {
    let Ok(text) = std::fs::read_to_string(file()) else {
        return;
    };
    match serde_json::from_str::<PasswordFile>(&text) {
        Ok(saved) if saved.version == 1 => DATA.with(|data| data.replace(saved)),
        Ok(saved) => {
            crate::log_error!("[passwords] unknown file version {}", saved.version);
            return;
        }
        Err(error) => {
            crate::log_error!("[passwords] could not read the file: {error}");
            return;
        }
    };
}

/// Writes the file readable by its owner only, through a temporary file.
pub(crate) fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    out.write_all(data)?;
    out.sync_all()?;
    std::fs::rename(tmp, path)
}

fn save() {
    let data = DATA.with(|data| serde_json::to_vec(&*data.borrow()));
    match data {
        Ok(data) => {
            if let Err(error) = write_private(&file(), &data) {
                crate::log_error!("[passwords] could not save: {error}");
            }
        }
        Err(error) => crate::log_error!("[passwords] could not serialize: {error}"),
    }
}

/// Writes a change that is still waiting, before the browser quits.
pub fn flush() {
    if SAVE_PENDING.with(|pending| pending.replace(false)) {
        save();
    }
}

fn schedule_save() {
    // Unit tests have no main loop and no profile to write to.
    if cfg!(test) {
        return;
    }
    if !SAVE_PENDING.with(|pending| pending.replace(true)) {
        glib::timeout_add_local_once(Duration::from_millis(300), flush);
    }
}

/// Saves, and passwords pages in every window load the list again.
fn changed() {
    schedule_save();
    for win in window::all() {
        win.command(json!({ "type": "passwords-changed" }));
    }
}

fn with_data<T>(f: impl FnOnce(&mut PasswordFile) -> T) -> T {
    DATA.with(|data| f(&mut data.borrow_mut()))
}

pub fn count() -> usize {
    with_data(|data| data.logins.len())
}

/// The sites never to offer saving for, for a backup.
pub(crate) fn never_list() -> Vec<String> {
    with_data(|data| data.never.clone())
}

/// What the passwords page says about where the passwords are kept.
pub fn status() -> Value {
    let (count, never, weak) = with_data(|data| {
        let weak = data.logins.iter().any(|l| {
            BASE64
                .decode(&l.secret)
                .is_ok_and(|bytes| bytes.starts_with(BASIC_PREFIX))
        });
        (data.logins.len(), data.never.clone(), weak)
    });
    let backend = if weak || keyring() == Keyring::Unavailable {
        "basic_text"
    } else {
        "secret_service"
    };
    json!({ "count": count, "backend": backend, "never": never })
}

/// The passwords page's list, with the checkup's findings, sorted by site.
pub fn list() -> Value {
    let logins = with_data(|data| data.logins.clone());
    let plain: Vec<Option<String>> = logins.iter().map(|l| decrypt(&l.secret)).collect();
    // Reuse: the same password on more than one site.
    let mut sites: std::collections::HashMap<&str, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for (login, password) in logins.iter().zip(&plain) {
        if let Some(password) = password {
            let site = crate::shield::site_of(&login.origin).unwrap_or(login.origin.clone());
            sites.entry(password.as_str()).or_default().insert(site);
        }
    }
    let mut rows: Vec<(String, String, Value)> = logins
        .iter()
        .zip(&plain)
        .map(|(l, password)| {
            let weak = password
                .as_deref()
                .is_some_and(passwordcsv::is_weak_password);
            let reused = password
                .as_deref()
                .is_some_and(|p| sites.get(p).is_some_and(|s| s.len() > 1));
            (
                crate::pages::host_of(&l.origin),
                l.username.clone(),
                json!({
                    "id": l.id,
                    "origin": l.origin,
                    "username": l.username,
                    "created": l.created,
                    "updated": l.updated,
                    "lastUsed": l.last_used,
                    "timesUsed": l.times_used,
                    "note": l.note,
                    "weak": weak,
                    "reused": reused,
                }),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Value::Array(rows.into_iter().map(|(_, _, row)| row).collect())
}

pub fn reveal(id: &str) -> Option<String> {
    let secret = with_data(|data| {
        data.logins
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.secret.clone())
    })?;
    decrypt(&secret)
}

/// Copies a password; it leaves the clipboard after a minute unless something replaced it.
pub fn copy(id: &str) -> bool {
    let Some(password) = reveal(id) else {
        return false;
    };
    crate::menus::copy_text(&password);
    let copy = COPIES.fetch_add(1, Ordering::Relaxed) + 1;
    glib::timeout_add_seconds_local_once(CLIPBOARD_SECONDS, move || {
        if COPIES.load(Ordering::Relaxed) != copy {
            return;
        }
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.request_text(move |clipboard, text| {
            if text.is_some_and(|text| text == password) {
                clipboard.set_text("");
                clipboard.clear();
            }
        });
    });
    true
}

fn find_index(data: &PasswordFile, origin: &str, username: &str) -> Option<usize> {
    data.logins
        .iter()
        .position(|l| l.origin == origin && l.username == username)
}

/// Saves a login, or a new password for one already saved (same site and username). The id,
/// or None when the address isn't a web page's or the password can't be encrypted.
pub fn add(input: &PlainLogin) -> Option<String> {
    let origin = origin_of(&input.origin)?;
    if input.password.is_empty() {
        return None;
    }
    let secret = encrypt(&clip(&input.password, MAX_PASSWORD))?;
    let now = now_ms();
    let id = with_data(|data| {
        if let Some(at) = find_index(data, &origin, &input.username) {
            let login = &mut data.logins[at];
            login.secret = secret;
            login.updated = now;
            if let Some(note) = &input.note {
                login.note = clip(note, MAX_NOTE);
            }
            return login.id.clone();
        }
        let id = new_id();
        data.logins.push(Stored {
            id: id.clone(),
            origin: origin.clone(),
            username: clip(&input.username, MAX_USERNAME),
            secret,
            created: input.created.unwrap_or(now),
            updated: input.updated.unwrap_or(now),
            last_used: 0,
            times_used: 0,
            note: clip(input.note.as_deref().unwrap_or(""), MAX_NOTE),
        });
        data.never.retain(|o| *o != origin);
        id
    });
    changed();
    Some(id)
}

/// The edit dialog's changes. False when the login is gone or the new address isn't valid.
pub fn update(id: &str, patch: &Value) -> bool {
    let text = |key: &str| patch.get(key).and_then(Value::as_str);
    let origin = match text("origin") {
        Some(origin) => match origin_of(origin) {
            Some(origin) => Some(origin),
            None => return false,
        },
        None => None,
    };
    let secret = match text("password").filter(|p| !p.is_empty()) {
        Some(password) => match encrypt(&clip(password, MAX_PASSWORD)) {
            Some(secret) => Some(secret),
            None => return false,
        },
        None => None,
    };
    let done = with_data(|data| {
        let Some(login) = data.logins.iter_mut().find(|l| l.id == id) else {
            return false;
        };
        if let Some(origin) = origin {
            login.origin = origin;
        }
        if let Some(username) = text("username") {
            login.username = clip(username, MAX_USERNAME);
        }
        if let Some(secret) = secret {
            login.secret = secret;
        }
        if let Some(note) = text("note") {
            login.note = clip(note, MAX_NOTE);
        }
        login.updated = now_ms();
        true
    });
    if done {
        changed();
    }
    done
}

pub fn remove(ids: &[String]) -> usize {
    let removed = with_data(|data| {
        let (gone, kept): (Vec<Stored>, Vec<Stored>) = std::mem::take(&mut data.logins)
            .into_iter()
            .partition(|l| ids.contains(&l.id));
        data.logins = kept;
        gone
    });
    let n = removed.len();
    LAST_REMOVED.with(|last| last.replace(removed));
    if n > 0 {
        changed();
    }
    n
}

pub fn undo_remove() {
    let removed = LAST_REMOVED.with(|last| last.take());
    if removed.is_empty() {
        return;
    }
    with_data(|data| {
        for login in removed {
            if !data.logins.iter().any(|l| l.id == login.id) {
                data.logins.push(login);
            }
        }
    });
    changed();
}

/// Clearing browsing data: every saved password goes; the sites never to save for stay.
pub fn clear() {
    let had = with_data(|data| !std::mem::take(&mut data.logins).is_empty());
    if had {
        changed();
    }
}

pub fn never_save(origin: &str) {
    with_data(|data| {
        if !data.never.iter().any(|o| o == origin) {
            data.never.push(origin.to_string());
        }
    });
    changed();
}

pub fn remove_never(origin: &str) {
    with_data(|data| data.never.retain(|o| o != origin));
    changed();
}

// ── Filling and saving ─────────────────────────────────────────────────────────

fn site(origin: &str) -> Option<String> {
    crate::shield::site_of(origin)
}

/// A saved login a page can be offered.
pub struct Credential {
    pub id: String,
    pub username: String,
    pub origin: String,
    /// Saved for this very origin, not another on the same site.
    pub exact: bool,
}

/// Logins for a page: this exact origin first, then others of the same site over the same
/// kind of connection.
pub fn credentials_for(origin: &str) -> Vec<Credential> {
    let page_site = site(origin);
    let secure = origin.starts_with("https:");
    let mut found: Vec<Credential> = with_data(|data| {
        data.logins
            .iter()
            .filter(|l| {
                l.origin == origin
                    || (page_site.is_some()
                        && site(&l.origin) == page_site
                        && l.origin.starts_with("https:") == secure)
            })
            .map(|l| Credential {
                id: l.id.clone(),
                username: l.username.clone(),
                origin: l.origin.clone(),
                exact: l.origin == origin,
            })
            .collect()
    });
    found.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then_with(|| a.username.cmp(&b.username))
    });
    found
}

/// The login to fill into a page of `origin`, if it belongs to the same site.
pub fn login_for(id: &str, origin: &str) -> Option<(String, String)> {
    let (username, secret) = with_data(|data| {
        let login = data.logins.iter_mut().find(|l| l.id == id)?;
        if login.origin != origin && site(&login.origin) != site(origin) {
            return None;
        }
        Some((login.username.clone(), login.secret.clone()))
    })?;
    let password = decrypt(&secret)?;
    with_data(|data| {
        if let Some(login) = data.logins.iter_mut().find(|l| l.id == id) {
            login.last_used = now_ms();
            login.times_used += 1;
        }
    });
    schedule_save();
    Some((username, password))
}

/// After a sign-in: is there anything to offer?
#[derive(Debug, PartialEq)]
pub enum Offer {
    None,
    Save,
    Update,
}

pub fn after_submit(origin: &str, username: &str, password: &str) -> Offer {
    let same = with_data(|data| {
        if data.never.iter().any(|o| o == origin) {
            return Err(());
        }
        Ok(find_index(data, origin, username).map(|at| data.logins[at].secret.clone()))
    });
    match same {
        Err(()) => Offer::None,
        Ok(None) => Offer::Save,
        Ok(Some(secret)) if decrypt(&secret).as_deref() == Some(password) => {
            with_data(|data| {
                if let Some(at) = find_index(data, origin, username) {
                    data.logins[at].last_used = now_ms();
                    data.logins[at].times_used += 1;
                }
            });
            schedule_save();
            Offer::None
        }
        Ok(Some(_)) => Offer::Update,
    }
}

// ── Import and export ──────────────────────────────────────────────────────────

/// Adds logins from another browser or a CSV file. A login already here (same site and
/// username) keeps the newer password. (added, duplicates)
pub fn import_logins(logins: &[PlainLogin], never: &[String]) -> (usize, usize) {
    let mut added = 0;
    let mut duplicates = 0;
    let now = now_ms();
    for input in logins {
        let Some(origin) = origin_of(&input.origin) else {
            continue;
        };
        if input.password.is_empty() {
            continue;
        }
        let existing = with_data(|data| {
            find_index(data, &origin, &input.username)
                .map(|at| (at, data.logins[at].secret.clone(), data.logins[at].updated))
        });
        if let Some((at, secret, updated)) = existing {
            if decrypt(&secret).as_deref() == Some(input.password.as_str())
                || input.updated.unwrap_or(0) <= updated
            {
                duplicates += 1;
                continue;
            }
            let Some(secret) = encrypt(&clip(&input.password, MAX_PASSWORD)) else {
                continue;
            };
            with_data(|data| {
                data.logins[at].secret = secret;
                data.logins[at].updated = input.updated.unwrap_or(now);
            });
            added += 1;
            continue;
        }
        let Some(secret) = encrypt(&clip(&input.password, MAX_PASSWORD)) else {
            continue;
        };
        with_data(|data| {
            data.logins.push(Stored {
                id: new_id(),
                origin,
                username: clip(&input.username, MAX_USERNAME),
                secret,
                created: input.created.unwrap_or(now),
                updated: input.updated.or(input.created).unwrap_or(now),
                last_used: 0,
                times_used: 0,
                note: clip(input.note.as_deref().unwrap_or(""), MAX_NOTE),
            })
        });
        added += 1;
    }
    let mut never_added = false;
    with_data(|data| {
        for origin in never.iter().filter_map(|o| origin_of(o)) {
            if !data.never.contains(&origin) && !data.logins.iter().any(|l| l.origin == origin) {
                data.never.push(origin);
                never_added = true;
            }
        }
    });
    if added > 0 || never_added {
        changed();
    }
    (added, duplicates)
}

/// Every login whose password can be read, in the clear: for exporting and backups.
pub(crate) fn plain_logins() -> Vec<PlainLogin> {
    with_data(|data| data.logins.clone())
        .into_iter()
        .filter_map(|l| {
            Some(PlainLogin {
                password: decrypt(&l.secret)?,
                origin: l.origin,
                username: l.username,
                note: Some(l.note),
                created: Some(l.created),
                updated: Some(l.updated),
            })
        })
        .collect()
}

fn csv_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("CSV"));
    filter.add_pattern("*.csv");
    filter.add_pattern("*.CSV");
    filter
}

/// Writes every password to a CSV file the person picks: Chrome's columns, readable by its
/// owner only.
pub fn export(win: &BrowserWindow) -> bool {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("passwords.export")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Save,
        None,
        None,
    );
    dialog.set_do_overwrite_confirmation(true);
    dialog.add_filter(csv_filter());
    let day = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|d| d.to_string())
        .unwrap_or_default();
    dialog.set_current_name(&format!("{}-{day}.csv", t("passwords.fileName")));
    if let Some(documents) = glib::user_special_dir(glib::UserDirectory::Documents) {
        let _ = dialog.set_current_folder(documents);
    }
    if dialog.run() != gtk::ResponseType::Accept {
        return false;
    }
    let Some(path) = dialog.filename() else {
        return false;
    };
    export_to(&path)
}

pub fn export_to(path: &Path) -> bool {
    let logins = plain_logins();
    let names: Vec<String> = logins
        .iter()
        .map(|l| crate::pages::host_of(&l.origin))
        .collect();
    let rows: Vec<CsvRow> = logins
        .iter()
        .zip(&names)
        .map(|(l, name)| CsvRow {
            name,
            url: &l.origin,
            username: &l.username,
            password: &l.password,
            note: l.note.as_deref().unwrap_or(""),
        })
        .collect();
    match write_private(path, passwordcsv::logins_to_csv(&rows).as_bytes()) {
        Ok(()) => true,
        Err(error) => {
            crate::log_error!("[passwords] could not export: {error}");
            false
        }
    }
}

/// Passwords from a CSV export (Chrome, Firefox, Bitwarden, 1Password, LastPass, KeePassXC).
pub fn import_csv(win: &BrowserWindow) -> Value {
    let dialog = gtk::FileChooserNative::new(
        Some(&t("passwords.import")),
        win.gtk_window().as_ref(),
        gtk::FileChooserAction::Open,
        None,
        None,
    );
    dialog.add_filter(csv_filter());
    if dialog.run() != gtk::ResponseType::Accept {
        return Value::Null;
    }
    match dialog.filename() {
        Some(path) => import_csv_from(&path),
        None => Value::Null,
    }
}

pub fn import_csv_from(path: &Path) -> Value {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            crate::log_error!("[passwords] could not read {}: {error}", path.display());
            return Value::Null;
        }
    };
    let (rows, skipped) = passwordcsv::logins_from_csv(&text);
    let logins: Vec<PlainLogin> = rows
        .into_iter()
        .map(|l| PlainLogin {
            origin: l.url,
            username: l.username,
            password: l.password,
            note: Some(l.note),
            ..PlainLogin::default()
        })
        .collect();
    let (added, duplicates) = import_logins(&logins, &[]);
    json!({
        "links": 0,
        "folders": 0,
        "duplicates": 0,
        "skipped": skipped,
        "favicons": 0,
        "passwords": added,
        "passwordDuplicates": duplicates,
    })
}

// ── The interface's calls ───────────────────────────────────────────────────────

/// The passwords page's calls that need the key, on the main thread after `ready`.
pub fn invoke(win: &BrowserWindow, method: &str, args: &[Value]) -> Value {
    let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default();
    match method {
        "passwordsList" => list(),
        "passwordsStatus" => status(),
        "passwordReveal" => reveal(text(0)).map_or(Value::Null, Value::from),
        "passwordAdd" => {
            let login = args.first().cloned().unwrap_or_default();
            let field = |key: &str| {
                login
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            add(&PlainLogin {
                origin: field("origin"),
                username: field("username"),
                password: field("password"),
                note: login
                    .get("note")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                ..PlainLogin::default()
            })
            .map_or(Value::Null, Value::from)
        }
        "passwordUpdate" => Value::from(update(text(0), args.get(1).unwrap_or(&Value::Null))),
        "passwordsExport" => Value::from(export(win)),
        "passwordsImportCsv" => import_csv(win),
        _ => Value::Null,
    }
}

/// Which of the interface's calls `invoke` answers, and whether they may make the key.
pub fn invoke_needs_key(method: &str) -> Option<bool> {
    match method {
        "passwordsList" | "passwordsStatus" | "passwordReveal" | "passwordsExport" => Some(false),
        "passwordAdd" | "passwordUpdate" | "passwordsImportCsv" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn use_test_key() {
    set_keyring(Keyring::Ready(openssl::sha::sha256(b"test key")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_with_either_key() {
        let key = openssl::sha::sha256(b"keyring");
        let sealed = seal(KEYRING_PREFIX, &key, "şifre ğü 🔑").unwrap();
        assert_eq!(unseal(&sealed, Some(key)).as_deref(), Some("şifre ğü 🔑"));
        // The keyring's key is needed, and only it works.
        assert_eq!(unseal(&sealed, None), None);
        assert_eq!(unseal(&sealed, Some(basic_key())), None);
        let basic = seal(BASIC_PREFIX, &basic_key(), "hunter2").unwrap();
        assert_eq!(unseal(&basic, None).as_deref(), Some("hunter2"));
        // Two seals of the same password differ (a new nonce each time).
        assert_ne!(
            seal(KEYRING_PREFIX, &key, "x"),
            seal(KEYRING_PREFIX, &key, "x")
        );
        // A changed byte is caught.
        let mut bytes = BASE64.decode(&sealed).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(unseal(&BASE64.encode(bytes), Some(key)), None);
        assert_eq!(unseal("not base64!", Some(key)), None);
    }

    #[test]
    fn saves_updates_and_offers() {
        use_test_key();
        let login = |origin: &str, username: &str, password: &str| PlainLogin {
            origin: origin.into(),
            username: username.into(),
            password: password.into(),
            ..PlainLogin::default()
        };
        let id = add(&login("https://example.com/login", "ada", "first-pass")).unwrap();
        assert_eq!(reveal(&id).as_deref(), Some("first-pass"));
        // The same site and username: a new password, the same login.
        assert_eq!(
            add(&login("https://example.com", "ada", "second-pass")).as_deref(),
            Some(id.as_str())
        );
        assert_eq!(count(), 1);
        assert_eq!(
            after_submit("https://example.com", "ada", "second-pass"),
            Offer::None
        );
        assert_eq!(
            after_submit("https://example.com", "ada", "third"),
            Offer::Update
        );
        assert_eq!(after_submit("https://example.com", "bob", "x"), Offer::Save);
        never_save("https://other.example");
        assert_eq!(
            after_submit("https://other.example", "ada", "x"),
            Offer::None
        );
        // Filling: the same site only, any of its hosts.
        assert!(login_for(&id, "https://example.org").is_none());
        assert!(login_for(&id, "https://www.example.com").is_some());
        assert_eq!(
            login_for(&id, "https://example.com"),
            Some(("ada".into(), "second-pass".into()))
        );
        add(&login("https://accounts.example.com", "ada2", "pw-ada2")).unwrap();
        let offered = credentials_for("https://example.com");
        assert_eq!(offered.len(), 2);
        assert!(offered[0].exact && offered[0].username == "ada");
        assert!(add(&login("ftp://example.com", "x", "y")).is_none());
        assert!(add(&login("https://example.com", "x", "")).is_none());
        let ids: Vec<String> = vec![id.clone()];
        assert_eq!(remove(&ids), 1);
        assert_eq!(count(), 1);
        undo_remove();
        assert_eq!(count(), 2);
        clear();
        assert_eq!(count(), 0);
        assert_eq!(status()["never"], json!(["https://other.example"]));
    }

    #[test]
    fn checks_weak_and_reused_passwords() {
        use_test_key();
        for (origin, username, password) in [
            ("https://a.example.com", "x", "Same-Pass-2026!"),
            ("https://b.example.org", "x", "Same-Pass-2026!"),
            ("https://c.example.net", "y", "123456"),
        ] {
            add(&PlainLogin {
                origin: origin.into(),
                username: username.into(),
                password: password.into(),
                ..PlainLogin::default()
            });
        }
        let list = list();
        let by_origin = |origin: &str| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|l| l["origin"] == origin)
                .unwrap()
                .clone()
        };
        assert_eq!(by_origin("https://a.example.com")["reused"], true);
        assert_eq!(by_origin("https://a.example.com")["weak"], false);
        assert_eq!(by_origin("https://c.example.net")["weak"], true);
        assert_eq!(by_origin("https://c.example.net")["reused"], false);
        // Sorted by host.
        assert_eq!(list[0]["origin"], "https://a.example.com");
    }

    #[test]
    fn imports_keep_the_newer_password() {
        use_test_key();
        let at = |updated: u64, password: &str| PlainLogin {
            origin: "example.com".into(),
            username: "ada".into(),
            password: password.into(),
            updated: Some(updated),
            ..PlainLogin::default()
        };
        assert_eq!(
            import_logins(&[at(1_000, "old")], &["https://never.example".into()]),
            (1, 0)
        );
        // Older, or the same password: a duplicate.
        assert_eq!(import_logins(&[at(500, "older")], &[]), (0, 1));
        assert_eq!(import_logins(&[at(2_000, "old")], &[]), (0, 1));
        assert_eq!(import_logins(&[at(3_000, "newer")], &[]), (1, 0));
        let id = credentials_for("https://example.com")[0].id.clone();
        assert_eq!(reveal(&id).as_deref(), Some("newer"));
        assert_eq!(status()["never"], json!(["https://never.example"]));
    }

    #[test]
    fn reads_files_leniently() {
        let file: PasswordFile = serde_json::from_str(
            r#"{"version":1,"logins":[{"id":"a","origin":"https://x.example","username":"u","secret":"s","created":1.7e12,"updated":5,"lastUsed":0,"timesUsed":2,"note":""}],"never":["https://n.example"]}"#,
        )
        .unwrap();
        assert_eq!(file.logins[0].created, 1_700_000_000_000);
        assert_eq!(file.logins[0].times_used, 2);
        assert_eq!(file.never, vec!["https://n.example".to_string()]);
    }
}
