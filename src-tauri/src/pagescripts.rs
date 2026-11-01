//! Scripts the browser puts into pages: fingerprinting protection (in the page's own world,
//! before its scripts) and rejecting cookie banners (in a world of its own, in every frame).
//! Both follow the shield: with the shield off, everywhere or for a site, pages get neither.

use std::cell::RefCell;

use gtk::glib;
use serde_json::json;
use webkit2gtk::{
    UserContentInjectedFrames, UserContentManager, UserContentManagerExt, UserScript,
    UserScriptInjectionTime,
};

use crate::settings::{self, ShieldLevel};
use crate::tab::now_ms;

const FINGERPRINT: &str = include_str!("../scripts/fingerprint.js");
const CONSENT: &str = include_str!("../scripts/consent.js");

thread_local! {
    /// A new key every launch, and another for private windows, so a site's seed never
    /// repeats across sessions or between normal and private browsing.
    static KEYS: [String; 2] = [random_key(), random_key()];
    static MANAGERS: RefCell<Vec<(glib::WeakRef<UserContentManager>, bool)>> = const { RefCell::new(Vec::new()) };
}

fn random_key() -> String {
    let mut bytes = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_err()
    {
        bytes[..8].copy_from_slice(&now_ms().to_le_bytes());
        bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The script with its settings in place of the last `__CONFIG__` (the first ones are in its
/// comments).
fn with_config(source: &str, config: &serde_json::Value) -> String {
    match source.rsplit_once("__CONFIG__") {
        Some((head, tail)) => format!("{head}{config}{tail}"),
        None => source.to_string(),
    }
}

fn add(manager: &UserContentManager, incognito: bool) {
    let prefs = settings::get();
    if prefs.shield_level == ShieldLevel::Off {
        return;
    }
    let paused = &prefs.shield_allowlist;
    if prefs.fingerprint_protection {
        let key = KEYS.with(|keys| keys[usize::from(incognito)].clone());
        let source = with_config(FINGERPRINT, &json!({ "key": key, "paused": paused }));
        manager.add_script(&UserScript::new(
            &source,
            UserContentInjectedFrames::AllFrames,
            UserScriptInjectionTime::Start,
            &[],
            &[],
        ));
    }
    if prefs.reject_cookie_banners {
        let source = with_config(CONSENT, &json!({ "paused": paused }));
        manager.add_script(&UserScript::for_world(
            &source,
            UserContentInjectedFrames::AllFrames,
            UserScriptInjectionTime::Start,
            "peepwick",
            &[],
            &[],
        ));
    }
}

/// Gives a tab's content manager the scripts the settings call for.
pub fn attach(manager: &UserContentManager, incognito: bool) {
    use glib::prelude::*;
    MANAGERS.with(|managers| {
        let mut managers = managers.borrow_mut();
        managers.retain(|(m, _)| m.upgrade().is_some());
        managers.push((manager.downgrade(), incognito));
    });
    add(manager, incognito);
}

/// The settings changed: pages get the new scripts when they next load.
pub fn refresh() {
    let managers: Vec<(UserContentManager, bool)> = MANAGERS.with(|managers| {
        managers
            .borrow()
            .iter()
            .filter_map(|(m, private)| Some((m.upgrade()?, *private)))
            .collect()
    });
    for (manager, incognito) in managers {
        manager.remove_all_scripts();
        add(&manager, incognito);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puts_settings_into_the_scripts() {
        let config = json!({ "key": "abc", "paused": ["example.com"] });
        for source in [FINGERPRINT, CONSENT] {
            let filled = with_config(source, &config);
            assert!(
                filled
                    .trim_end()
                    .ends_with(r#"})({"key":"abc","paused":["example.com"]})"#)
            );
        }
    }
}
