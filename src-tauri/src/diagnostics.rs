//! The error log behind "Report a bug". It holds errors from the browser itself: panics and
//! failures in its own code, errors in its interface, crashed page processes, and page load
//! failures as bare Chromium error names. Addresses, file paths, the user name and the like
//! are scrubbed out before anything is kept. It lives in memory; with the setting on, also in
//! a file on this computer (`logs/errors.jsonl`, as in 0.1.x) so a crash's errors survive a
//! restart. Nothing is sent anywhere: the person copies the report and decides where it goes.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

use gtk::gdk;
use gtk::gio::TlsCertificateFlags;
use gtk::glib::prelude::*;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::utils::config::BundleType;

use crate::tab::now_ms;
use crate::{paths, settings, window};

/// Errors kept, newest first; the same error again counts up instead.
const MAX: usize = 200;
/// The file is cut back to what's in memory past this size.
const FILE_LIMIT: u64 = 256 * 1024;
/// Errors the report lists.
const SHOWN: usize = 60;
const ISSUES: &str = "https://github.com/PEEPWICK/peepwick-browser/issues/new";

/// `eprintln!` for errors: the line also goes to the error log.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::diagnostics::error("main", &format!($($arg)*))
    };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorRecord {
    pub time: u64,
    /// main, interface, page, update.
    pub source: String,
    pub message: String,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

struct Log {
    records: Vec<ErrorRecord>,
    /// Page load failures by Chromium error name.
    loads: Vec<(&'static str, u32)>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    records: Vec::new(),
    loads: Vec::new(),
});

/// The setting, kept where a panicking thread can read it: that thread may hold the
/// settings' lock.
static KEEP_FILE: AtomicBool = AtomicBool::new(false);

fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn log_file() -> Option<PathBuf> {
    paths::profile_if_ready().map(|profile| profile.join("logs").join("errors.jsonl"))
}

/// Prints an error and keeps it for the report.
pub fn error(source: &str, message: &str) {
    eprintln!("{message}");
    record(source, message);
}

/// Keeps an error for the report, scrubbed.
pub fn record(source: &str, message: &str) {
    keep(source, message, true);
}

/// `wait`: false from the panic hook, which must not wait for a lock its own thread may hold.
fn keep(source: &str, message: &str, wait: bool) {
    let clean = scrub(message.trim(), &HOME, &USER);
    if clean.is_empty() {
        return;
    }
    let time = now_ms();
    let guard = if wait {
        Some(log())
    } else {
        LOG.try_lock().ok()
    };
    let mut kept = None;
    if let Some(mut log) = guard {
        add(&mut log.records, source, &clean, time);
        kept = Some(log.records.clone());
    }
    persist(
        &ErrorRecord {
            time,
            source: source.to_string(),
            message: clean,
            count: 1,
        },
        kept.as_deref(),
    );
}

fn add(records: &mut Vec<ErrorRecord>, source: &str, message: &str, time: u64) {
    match records
        .iter()
        .position(|r| r.source == source && r.message == message)
    {
        Some(at) => {
            let mut same = records.remove(at);
            same.count += 1;
            same.time = time;
            records.insert(0, same);
        }
        None => {
            records.insert(
                0,
                ErrorRecord {
                    time,
                    source: source.to_string(),
                    message: message.to_string(),
                    count: 1,
                },
            );
            records.truncate(MAX);
        }
    }
}

/// Adds one error to the file while the setting is on; cuts the file back to the errors in
/// memory (`kept`) when it grows too large.
fn persist(record: &ErrorRecord, kept: Option<&[ErrorRecord]>) {
    if !KEEP_FILE.load(Ordering::Relaxed) {
        return;
    }
    let Some(file) = log_file() else { return };
    // The log must never take the browser down with it.
    let _ = (|| -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut out = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)?;
        writeln!(out, "{}", serde_json::to_string(record)?)?;
        if out.metadata()?.len() > FILE_LIMIT
            && let Some(kept) = kept
        {
            let lines: Vec<String> = kept
                .iter()
                .filter_map(|r| serde_json::to_string(r).ok())
                .collect();
            crate::store::write_atomic(&file, format!("{}\n", lines.join("\n")).as_bytes())?;
        }
        Ok(())
    })();
}

/// A page didn't load: only the Chromium error name is kept, never the address.
pub fn load_error(code: i32) {
    let name = load_error_name(code);
    let mut log = log();
    match log.loads.iter_mut().find(|(n, _)| *n == name) {
        Some((_, count)) => *count += 1,
        None => log.loads.push((name, 1)),
    }
}

/// The names Chromium gives the failures the error page shows (`tab.rs` `error_code`).
fn load_error_name(code: i32) -> &'static str {
    match code {
        -102 => "ERR_CONNECTION_REFUSED",
        -105 => "ERR_NAME_NOT_RESOLVED",
        -106 => "ERR_INTERNET_DISCONNECTED",
        -118 => "ERR_CONNECTION_TIMED_OUT",
        -312 => "ERR_UNSAFE_PORT",
        -200 => "ERR_CERT_COMMON_NAME_INVALID",
        -201 => "ERR_CERT_DATE_INVALID",
        -202 => "ERR_CERT_AUTHORITY_INVALID",
        -206 => "ERR_CERT_REVOKED",
        -207 => "ERR_CERT_INVALID",
        -213 => "ERR_CERT_WEAK_KEY",
        _ => "ERR_FAILED",
    }
}

/// A certificate problem as the Chromium error it would have been.
pub fn certificate_error(flags: TlsCertificateFlags) {
    let code = if flags.contains(TlsCertificateFlags::UNKNOWN_CA) {
        -202
    } else if flags.contains(TlsCertificateFlags::BAD_IDENTITY) {
        -200
    } else if flags.intersects(TlsCertificateFlags::EXPIRED | TlsCertificateFlags::NOT_ACTIVATED) {
        -201
    } else if flags.contains(TlsCertificateFlags::REVOKED) {
        -206
    } else if flags.contains(TlsCertificateFlags::INSECURE) {
        -213
    } else {
        -207
    };
    load_error(code);
}

// ── Starting, the setting, clearing ─────────────────────────────────────────────

/// Picks up earlier sessions' errors when the file is kept, and catches panics from here on.
/// Call once, after the settings are loaded.
pub fn start() {
    let kept = settings::get().error_log;
    KEEP_FILE.store(kept, Ordering::Relaxed);
    if kept {
        let earlier = read_file();
        let mut log = log();
        for record in earlier {
            merge(&mut log.records, record);
        }
        log.records.sort_by(|a, b| b.time.cmp(&a.time));
        log.records.truncate(MAX);
    }
    let usual = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let what = info.payload_as_str().unwrap_or("a panic");
        keep("main", &format!("panicked{place}: {what}"), false);
        usual(info);
    }));
}

/// Folds an earlier session's line into the list: the same error adds up.
fn merge(records: &mut Vec<ErrorRecord>, record: ErrorRecord) {
    match records
        .iter_mut()
        .find(|r| r.source == record.source && r.message == record.message)
    {
        Some(same) => {
            same.count += record.count.max(1);
            same.time = same.time.max(record.time);
        }
        None => records.push(ErrorRecord {
            count: record.count.max(1),
            ..record
        }),
    }
}

fn read_file() -> Vec<ErrorRecord> {
    let Some(text) = log_file().and_then(|file| fs::read_to_string(file).ok()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<ErrorRecord>(line).ok())
        .collect()
}

fn delete_file() {
    if let Some(file) = log_file() {
        let _ = fs::remove_file(file);
    }
}

/// The setting changed: turned off, the file goes.
pub fn settings_changed() {
    let keep = settings::get().error_log;
    if KEEP_FILE.swap(keep, Ordering::Relaxed) && !keep {
        delete_file();
    }
}

pub fn clear() {
    let mut log = log();
    log.records.clear();
    log.loads.clear();
    drop(log);
    delete_file();
}

// ── The report ──────────────────────────────────────────────────────────────────

/// How this copy was installed, as the bug form names it.
fn package_kind() -> &'static str {
    if cfg!(debug_assertions) {
        return "From source";
    }
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::AppImage) if std::env::var_os("APPIMAGE").is_some() => "AppImage",
        Some(BundleType::Deb) => ".deb",
        _ => "From source",
    }
}

/// The distribution, kernel, desktop and the display server the browser actually draws on.
fn system_line() -> String {
    let distribution = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|file| fs::read_to_string(file).ok())
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|name| name.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| "Linux".into());
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|k| k.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown desktop".into());
    let display = gdk::Display::default()
        .map(|d| d.type_().name().to_string())
        .unwrap_or_default();
    let session = if display.contains("Wayland") {
        "Wayland".to_string()
    } else if display.contains("X11") {
        "X11".to_string()
    } else {
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown session".into())
    };
    let mut line = format!(
        "{distribution} · Linux {kernel} · {} · {desktop} · {session}",
        std::env::consts::ARCH
    );
    if !settings::get().hardware_acceleration {
        line.push_str(" · acceleration off");
    }
    line
}

/// A time as the report shows it: UTC, to the minute.
fn stamp(ms: u64) -> String {
    let seconds = ms / 1000;
    let (year, month, day) = civil_date((seconds / 86_400) as i64);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// Days since 1970-01-01 as a calendar date (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What Settings › Report a bug shows, and the text the person copies.
pub fn report() -> Value {
    let prefs = serde_json::to_value(settings::get()).unwrap_or_default();
    let field = |key: &str| {
        prefs
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string()
    };
    let version = window::display_version();
    let system = system_line();
    let (records, mut loads) = {
        let log = log();
        (log.records.clone(), log.loads.clone())
    };
    loads.sort_by(|a, b| b.1.cmp(&a.1));
    let text = report_text(
        &[
            format!(
                "PEEPWICK Browser {version} ({}) · {}",
                env!("PEEPWICK_STAGE"),
                package_kind()
            ),
            format!(
                "Tauri {} · WebKitGTK {}",
                tauri::VERSION,
                window::app_info().webkit
            ),
            system.clone(),
            format!(
                "Language {} · Theme {} · Shield {}",
                field("language"),
                field("theme"),
                field("shieldLevel")
            ),
        ],
        &records,
        &loads,
    );
    json!({
        "text": text,
        "version": version,
        "system": system,
        "errors": records.iter().take(SHOWN).collect::<Vec<_>>(),
        "loadErrors": loads
            .iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect::<Vec<_>>(),
        "logFile": KEEP_FILE.load(Ordering::Relaxed),
    })
}

fn report_text(head: &[String], records: &[ErrorRecord], loads: &[(&str, u32)]) -> String {
    let mut lines = head.to_vec();
    lines.push(String::new());
    lines.push(format!("Errors ({})", records.len()));
    if records.is_empty() {
        lines.push("none".into());
    }
    for r in records.iter().take(SHOWN) {
        let message = r.message.lines().take(4).collect::<Vec<_>>().join(" ⏎ ");
        let times = if r.count > 1 {
            format!(" ×{}", r.count)
        } else {
            String::new()
        };
        lines.push(format!(
            "{} · {} · {message}{times}",
            stamp(r.time),
            r.source
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "Page load errors ({})",
        loads.iter().map(|(_, n)| n).sum::<u32>()
    ));
    if loads.is_empty() {
        lines.push("none".into());
    }
    for (name, count) in loads.iter().take(20) {
        lines.push(format!("{name} ×{count}"));
    }
    lines.join("\n")
}

/// The GitHub bug form with the version, system, package and report filled in. Long reports
/// are cut to keep the address short; the whole text is on the clipboard.
pub fn issue_url(report: &Value) -> String {
    let text = report["text"].as_str().unwrap_or_default();
    let text = if text.chars().count() > 3500 {
        format!("{}\n…", text.chars().take(3500).collect::<String>())
    } else {
        text.to_string()
    };
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("template", "bug_report.yml")
        .append_pair("version", report["version"].as_str().unwrap_or_default())
        .append_pair("system", report["system"].as_str().unwrap_or_default())
        .append_pair("package", package_kind())
        .append_pair("report", &text)
        .finish();
    format!("{ISSUES}?{query}")
}

// ── Scrubbing ───────────────────────────────────────────────────────────────────

static HOME: LazyLock<String> = LazyLock::new(|| {
    std::env::var("HOME")
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
});
static USER: LazyLock<String> = LazyLock::new(|| {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
});

struct Rules {
    url: Regex,
    crate_path: Regex,
    path: Regex,
    other_url: Regex,
    email: Regex,
    ip: Regex,
    token: Regex,
}

static RULES: LazyLock<Rules> = LazyLock::new(|| Rules {
    url: Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s'"<>()\[\]{}]+"#).expect("url rule"),
    // Panics in the crates the browser is built from keep the crate's own path.
    crate_path: Regex::new(r"(?:~|/[^\s'()<>,;]*)/\.cargo/registry/src/[^/\s]+/|/rustc/[0-9a-f]+/")
        .expect("crate rule"),
    path: Regex::new(r#"(^|[\s('"=:])(~/[^\s'"()<>,;]+|/[^\s'"()<>,;/][^\s'"()<>,;]*)"#)
        .expect("path rule"),
    other_url: Regex::new(r#"(?i)\b(?:data|blob|javascript|mailto|about):[^\s'"<>()]+"#)
        .expect("scheme rule"),
    email: Regex::new(r"[\w.+-]+@[\w-]+(\.[\w-]+)+").expect("email rule"),
    ip: Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("ip rule"),
    token: Regex::new(r"\b[A-Za-z0-9_-]{32,}\b").expect("token rule"),
});

/// Takes out of an error message anything that could identify the person or what they were
/// doing: addresses, their home folder and user name, file paths, e-mail and IP addresses,
/// and long tokens.
pub fn scrub(text: &str, home: &str, user: &str) -> String {
    let rules = &*RULES;
    let mut out = text.to_string();
    if home.len() > 1 {
        out = out.replace(home, "~");
    }
    out = rules.url.replace_all(&out, "<url>").into_owned();
    out = rules.crate_path.replace_all(&out, "").into_owned();
    out = rules
        .path
        .replace_all(&out, |c: &Captures| format!("{}<path>", &c[1]))
        .into_owned();
    out = rules.other_url.replace_all(&out, "<url>").into_owned();
    out = rules.email.replace_all(&out, "<email>").into_owned();
    out = rules.ip.replace_all(&out, "<ip>").into_owned();
    out = rules.token.replace_all(&out, "<token>").into_owned();
    if user.chars().count() > 2
        && let Ok(name) = Regex::new(&format!(r"\b{}\b", regex::escape(user)))
    {
        out = name.replace_all(&out, "<user>").into_owned();
    }
    out.chars().take(600).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_what_could_identify_the_person() {
        let text = "Failed to load https://bank.example/account?id=42 from /home/alex/Belgeler/secret.pdf for alex, \
                    mail alex@example.com, peer 192.168.1.20, key 4f3c2a1b9e8d7c6b5a4f3c2a1b9e8d7c6b, file:///home/alex/x";
        let out = scrub(text, "/home/alex", "alex");
        for leak in [
            "bank.example",
            "Belgeler",
            "secret.pdf",
            "alex",
            "192.168",
            "4f3c2a1b",
            "example.com",
        ] {
            assert!(!out.contains(leak), "{leak} in {out}");
        }
        assert!(out.contains("<url>") && out.contains("<path>"), "{out}");
    }

    #[test]
    fn keeps_error_names_and_code_places() {
        assert_eq!(
            scrub(
                "panicked at src/tab.rs:120:5: called `Option::unwrap()` on a `None` value",
                "/home/u",
                "someone"
            ),
            "panicked at src/tab.rs:120:5: called `Option::unwrap()` on a `None` value"
        );
        assert_eq!(
            scrub(
                "panicked at /home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/webkit2gtk-2.0.2/src/auto/web_view.rs:9:1: boom",
                "/home/u",
                "someone"
            ),
            "panicked at webkit2gtk-2.0.2/src/auto/web_view.rs:9:1: boom"
        );
        assert_eq!(
            scrub(
                "[settings] could not save: /home/u/.local/share/com.peepwick.browser/settings.json: Permission denied",
                "/home/u",
                "someone"
            ),
            "[settings] could not save: <path> Permission denied"
        );
        assert_eq!(
            scrub("downloaded 50 / 100 parts", "/home/u", "someone"),
            "downloaded 50 / 100 parts"
        );
    }

    #[test]
    fn counts_the_same_error_once() {
        let mut records = Vec::new();
        add(&mut records, "main", "a", 1);
        add(&mut records, "main", "b", 2);
        add(&mut records, "main", "a", 3);
        add(&mut records, "page", "a", 4);
        let seen: Vec<_> = records
            .iter()
            .map(|r| (r.source.as_str(), r.message.as_str(), r.count, r.time))
            .collect();
        assert_eq!(
            seen,
            [
                ("page", "a", 1, 4),
                ("main", "a", 2, 3),
                ("main", "b", 1, 2)
            ]
        );
        // An earlier session's lines add up the same way.
        let mut merged = Vec::new();
        for line in [
            ("main", "x", 2, 10),
            ("main", "x", 1, 30),
            ("main", "y", 0, 20),
        ] {
            merge(
                &mut merged,
                ErrorRecord {
                    source: line.0.into(),
                    message: line.1.into(),
                    count: line.2,
                    time: line.3,
                },
            );
        }
        assert_eq!(merged[0].count, 3);
        assert_eq!(merged[0].time, 30);
        assert_eq!(merged[1].count, 1);
        // 0.1.x's lines read back.
        let old: ErrorRecord =
            serde_json::from_str(r#"{"time":5,"source":"main","message":"m","count":1}"#).unwrap();
        assert_eq!(old.message, "m");
    }

    #[test]
    fn writes_the_report_text() {
        assert_eq!(stamp(0), "1970-01-01 00:00");
        assert_eq!(stamp(1_794_793_860_000), "2026-11-16 01:51");
        assert_eq!(stamp(951_782_400_000), "2000-02-29 00:00");
        let records = vec![
            ErrorRecord {
                time: 0,
                source: "page".into(),
                message: "Web process gone: crashed".into(),
                count: 2,
            },
            ErrorRecord {
                time: 60_000,
                source: "main".into(),
                message: "first\nsecond".into(),
                count: 1,
            },
        ];
        let text = report_text(
            &["head".into()],
            &records,
            &[("ERR_NAME_NOT_RESOLVED", 3), ("ERR_FAILED", 1)],
        );
        assert_eq!(
            text,
            "head\n\nErrors (2)\n1970-01-01 00:00 · page · Web process gone: crashed ×2\n\
             1970-01-01 00:01 · main · first ⏎ second\n\nPage load errors (4)\n\
             ERR_NAME_NOT_RESOLVED ×3\nERR_FAILED ×1"
        );
        assert_eq!(
            report_text(&[], &[], &[]),
            "\nErrors (0)\nnone\n\nPage load errors (0)\nnone"
        );
        assert_eq!(load_error_name(-105), "ERR_NAME_NOT_RESOLVED");
        assert_eq!(load_error_name(-999), "ERR_FAILED");
    }

    #[test]
    fn fills_in_the_bug_form() {
        let report = json!({ "text": "a b&c", "version": "0.2.5a", "system": "Zorin OS 18 · X11" });
        let url = issue_url(&report);
        assert!(url.starts_with(
            "https://github.com/PEEPWICK/peepwick-browser/issues/new?template=bug_report.yml&version=0.2.5a&system=Zorin+OS+18+%C2%B7+X11&package="
        ));
        assert!(url.ends_with("&report=a+b%26c"), "{url}");
        let long = json!({ "text": "x".repeat(5000), "version": "v", "system": "s" });
        assert!(issue_url(&long).ends_with(&format!("{}%0A%E2%80%A6", "x".repeat(10))));
        assert!(issue_url(&long).len() < 3700);
    }
}
