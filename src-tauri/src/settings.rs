//! Settings: the same fields, defaults and checks as `src/shared/settings.ts`, which the UI
//! uses to draw the settings pages. Kept in settings.json in the profile.

use std::sync::RwLock;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{paths, store};

pub const ZOOM_LEVELS: [f64; 11] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
pub const FONT_SIZES: [u64; 6] = [12, 14, 16, 18, 20, 24];
pub const DEFAULT_FOCUS_SITES: [&str; 9] = [
    "youtube.com",
    "x.com",
    "twitter.com",
    "instagram.com",
    "tiktok.com",
    "facebook.com",
    "reddit.com",
    "twitch.tv",
    "netflix.com",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Tr,
    En,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    Duckduckgo,
    Brave,
    Startpage,
    Google,
    Ecosia,
    Bing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShieldLevel {
    Off,
    Standard,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BookmarksBar {
    Always,
    Newtab,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecureDns {
    Off,
    Automatic,
    Cloudflare,
    Quad9,
    Mullvad,
    Google,
}

/// What can be cleared, now or when the browser closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClearKind {
    History,
    Downloads,
    Cookies,
    Cache,
    Permissions,
    Forms,
    Passwords,
}

pub const CLEAR_KINDS: [ClearKind; 7] = [
    ClearKind::History,
    ClearKind::Downloads,
    ClearKind::Cookies,
    ClearKind::Cache,
    ClearKind::Permissions,
    ClearKind::Forms,
    ClearKind::Passwords,
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: Lang,
    pub theme: Theme,
    pub search_engine: SearchEngine,
    pub search_suggestions: bool,
    pub shield_level: ShieldLevel,
    pub block_third_party_cookies: bool,
    pub send_global_privacy_control: bool,
    /// Registrable domains (eTLD+1) where the shield is switched off.
    pub shield_allowlist: Vec<String>,
    /// Domains that wait behind the focus gate while a focus session runs.
    pub focus_sites: Vec<String>,
    pub focus_silence_notifications: bool,
    pub focus_minutes: u32,
    /// Background tabs sleep after this many minutes; 0 keeps them awake.
    pub sleep_after_minutes: u32,
    pub restore_session: bool,
    pub ask_where_to_save: bool,
    pub sidebar_collapsed: bool,
    pub bookmarks_bar: BookmarksBar,
    pub onboarded: bool,
    /// Keep the error log in a file on this computer between sessions. Nothing is sent.
    pub error_log: bool,
    /// Cleared every time the browser closes.
    pub clear_on_exit: Vec<ClearKind>,
    pub secure_dns: SecureDns,
    /// Where downloads go; empty for the system's Downloads folder.
    pub download_dir: String,
    /// Takes effect after a restart.
    pub hardware_acceleration: bool,
    pub password_save: bool,
    pub password_fill: bool,
    pub form_fill: bool,
    /// Page zoom for sites without their own (1 = 100 %).
    pub default_zoom: f64,
    /// Default font size in CSS pixels, for pages that don't set their own.
    pub font_size: u32,
    pub https_only: bool,
    pub safe_browsing: bool,
    pub fingerprint_protection: bool,
    pub reject_cookie_banners: bool,
}

impl Settings {
    pub fn defaults(language: Lang) -> Settings {
        Settings {
            language,
            theme: Theme::Dark,
            search_engine: SearchEngine::Duckduckgo,
            search_suggestions: false,
            shield_level: ShieldLevel::Standard,
            block_third_party_cookies: true,
            send_global_privacy_control: true,
            shield_allowlist: Vec::new(),
            focus_sites: DEFAULT_FOCUS_SITES.iter().map(|s| s.to_string()).collect(),
            focus_silence_notifications: true,
            focus_minutes: 50,
            sleep_after_minutes: 60,
            restore_session: true,
            ask_where_to_save: false,
            sidebar_collapsed: false,
            bookmarks_bar: BookmarksBar::Always,
            onboarded: false,
            error_log: false,
            clear_on_exit: Vec::new(),
            secure_dns: SecureDns::Automatic,
            download_dir: String::new(),
            hardware_acceleration: true,
            password_save: true,
            password_fill: true,
            form_fill: true,
            default_zoom: 1.0,
            font_size: 16,
            https_only: true,
            safe_browsing: true,
            fingerprint_protection: true,
            reject_cookie_banners: true,
        }
    }

    /// Takes the well-formed values from an untrusted patch and ignores the rest.
    pub fn apply_patch(&mut self, patch: &Value) {
        let Some(input) = patch.as_object() else {
            return;
        };
        fn parse<T: DeserializeOwned>(value: Option<&Value>) -> Option<T> {
            value.and_then(|v| serde_json::from_value(v.clone()).ok())
        }
        let flag = |key: &str| input.get(key).and_then(Value::as_bool);

        if let Some(v) = parse(input.get("language")) {
            self.language = v;
        }
        if let Some(v) = parse(input.get("theme")) {
            self.theme = v;
        }
        if let Some(v) = parse(input.get("searchEngine")) {
            self.search_engine = v;
        }
        if let Some(v) = parse(input.get("shieldLevel")) {
            self.shield_level = v;
        }
        if let Some(v) = parse(input.get("bookmarksBar")) {
            self.bookmarks_bar = v;
        }
        if let Some(v) = parse(input.get("secureDns")) {
            self.secure_dns = v;
        }
        for (key, field) in [
            ("searchSuggestions", &mut self.search_suggestions),
            (
                "blockThirdPartyCookies",
                &mut self.block_third_party_cookies,
            ),
            (
                "sendGlobalPrivacyControl",
                &mut self.send_global_privacy_control,
            ),
            (
                "focusSilenceNotifications",
                &mut self.focus_silence_notifications,
            ),
            ("restoreSession", &mut self.restore_session),
            ("askWhereToSave", &mut self.ask_where_to_save),
            ("sidebarCollapsed", &mut self.sidebar_collapsed),
            ("onboarded", &mut self.onboarded),
            ("errorLog", &mut self.error_log),
            ("hardwareAcceleration", &mut self.hardware_acceleration),
            ("passwordSave", &mut self.password_save),
            ("passwordFill", &mut self.password_fill),
            ("formFill", &mut self.form_fill),
            ("httpsOnly", &mut self.https_only),
            ("safeBrowsing", &mut self.safe_browsing),
            ("fingerprintProtection", &mut self.fingerprint_protection),
            ("rejectCookieBanners", &mut self.reject_cookie_banners),
        ] {
            if let Some(value) = flag(key) {
                *field = value;
            }
        }
        if let Some(zoom) = input.get("defaultZoom").and_then(Value::as_f64)
            && ZOOM_LEVELS.contains(&zoom)
        {
            self.default_zoom = zoom;
        }
        if let Some(size) = input.get("fontSize").and_then(Value::as_u64)
            && FONT_SIZES.contains(&size)
        {
            self.font_size = size as u32;
        }
        // Saved passwords are never wiped on exit.
        if let Some(kinds) = input.get("clearOnExit").and_then(Value::as_array) {
            let wanted: Vec<ClearKind> = kinds
                .iter()
                .filter_map(|k| serde_json::from_value(k.clone()).ok())
                .collect();
            self.clear_on_exit = CLEAR_KINDS
                .into_iter()
                .filter(|kind| *kind != ClearKind::Passwords && wanted.contains(kind))
                .collect();
        }
        // An absolute folder path, or empty for the system default.
        if let Some(dir) = input.get("downloadDir").and_then(Value::as_str)
            && (dir.is_empty() || (dir.starts_with('/') && dir.len() < 4096))
        {
            self.download_dir = dir.to_string();
        }
        if let Some(list) = input.get("shieldAllowlist").and_then(Value::as_array) {
            self.shield_allowlist = clean_domains(list);
        }
        if let Some(list) = input.get("focusSites").and_then(Value::as_array) {
            self.focus_sites = clean_domains(list);
        }
        if let Some(minutes) = input.get("focusMinutes").and_then(Value::as_f64)
            && minutes.is_finite()
        {
            self.focus_minutes = minutes.round().clamp(5.0, 240.0) as u32;
        }
        if let Some(minutes) = input.get("sleepAfterMinutes").and_then(Value::as_f64)
            && minutes.is_finite()
        {
            self.sleep_after_minutes = minutes.round().clamp(0.0, 24.0 * 60.0) as u32;
        }
    }
}

/// Normalises a site someone typed ("https://www.YouTube.com/watch" → "youtube.com").
pub fn normalize_domain(value: &str) -> Option<String> {
    let mut text = value.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    if !has_scheme(&text) {
        text = format!("https://{text}");
    }
    let url = url::Url::parse(&text).ok()?;
    let host = url.host_str()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    let host = host.strip_suffix('.').unwrap_or(host);
    let valid = host == "localhost"
        || (host.contains('.')
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            }));
    valid.then(|| host.to_string())
}

/// Whether text starts with a scheme followed by "//" (`https://`, `peepwick://`…).
pub fn has_scheme(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        && rest.starts_with("//")
}

fn clean_domains(values: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        if let Some(domain) = value.as_str().and_then(normalize_domain)
            && !out.contains(&domain)
        {
            out.push(domain);
        }
    }
    out
}

/// The interface language for a first run: Turkish on Turkish systems, English otherwise.
fn system_lang() -> Lang {
    for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            return if value.to_lowercase().starts_with("tr") {
                Lang::Tr
            } else {
                Lang::En
            };
        }
    }
    Lang::En
}

static SETTINGS: RwLock<Option<Settings>> = RwLock::new(None);

fn file() -> std::path::PathBuf {
    paths::user_path("settings.json")
}

pub fn load() {
    let mut settings = Settings::defaults(system_lang());
    if let Some(saved) = store::read_json::<Value>(&file()) {
        settings.apply_patch(&saved);
    }
    *SETTINGS.write().expect("settings lock") = Some(settings);
}

pub fn get() -> Settings {
    SETTINGS
        .read()
        .expect("settings lock")
        .clone()
        .expect("settings::load runs at startup")
}

/// Applies a patch from the UI, saves, and returns the new settings.
pub fn update(patch: &Value) -> Settings {
    let mut guard = SETTINGS.write().expect("settings lock");
    let settings = guard.as_mut().expect("settings::load runs at startup");
    settings.apply_patch(patch);
    if let Ok(json) = serde_json::to_vec_pretty(&*settings)
        && let Err(error) = store::write_atomic(&file(), &json)
    {
        eprintln!("[settings] could not save: {error}");
    }
    settings.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_only_well_formed_values() {
        let mut s = Settings::defaults(Lang::Tr);
        s.apply_patch(&json!({
            "theme": "light",
            "shieldLevel": "loud",
            "defaultZoom": 1.25,
            "fontSize": 13,
            "clearOnExit": ["cookies", "passwords", "history", "nonsense"],
            "focusMinutes": 1000,
            "httpsOnly": "yes",
            "downloadDir": "relative/path",
            "shieldAllowlist": ["https://www.Example.com/page", "not a domain", "example.com"]
        }));
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.shield_level, ShieldLevel::Standard);
        assert_eq!(s.default_zoom, 1.25);
        assert_eq!(s.font_size, 16);
        assert_eq!(
            s.clear_on_exit,
            vec![ClearKind::History, ClearKind::Cookies]
        );
        assert_eq!(s.focus_minutes, 240);
        assert!(s.https_only);
        assert_eq!(s.download_dir, "");
        assert_eq!(s.shield_allowlist, vec!["example.com".to_string()]);
    }

    #[test]
    fn serializes_like_the_ui_expects() {
        let value = serde_json::to_value(Settings::defaults(Lang::En)).unwrap();
        assert_eq!(value["searchEngine"], "duckduckgo");
        assert_eq!(value["secureDns"], "automatic");
        assert_eq!(value["bookmarksBar"], "always");
        assert_eq!(value["defaultZoom"], 1.0);
    }

    #[test]
    fn normalizes_domains() {
        assert_eq!(
            normalize_domain("https://www.YouTube.com/watch").as_deref(),
            Some("youtube.com")
        );
        assert_eq!(normalize_domain("localhost").as_deref(), Some("localhost"));
        assert_eq!(normalize_domain("nope"), None);
        assert_eq!(normalize_domain("  "), None);
    }
}
