//! Focus sessions: for a set time, distracting sites wait behind a calm gate instead of
//! loading. The user can still open one for five minutes after a short pause. The session
//! is kept in focus.json, as the Electron version kept it, so it outlives a restart.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gtk::glib;

use crate::i18n::{Var, t, tv};
use crate::state::FocusState;
use crate::tab::now_ms;
use crate::{notify, pages, paths, settings, stats, store, window};

const PASS_MINUTES: u64 = 5;
/// The end is checked against the clock this often, so a computer that slept through the
/// end of a session finishes it when it wakes up.
const TICK: Duration = Duration::from_secs(20);

thread_local! {
    static SESSION: RefCell<FocusState> = RefCell::new(FocusState::default());
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    /// Sites opened anyway, until when.
    static PASSES: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
}

fn file() -> PathBuf {
    paths::user_path("focus.json")
}

fn save(session: &FocusState) {
    let written = serde_json::to_vec(session)
        .map_err(std::io::Error::other)
        .and_then(|bytes| store::write_atomic(&file(), &bytes));
    if let Err(error) = written {
        eprintln!("[focus] could not save the session: {error}");
    }
}

/// Picks up what the last run left: a session still going, or one that ended while the
/// browser was closed (its minutes still count).
pub fn load() {
    let Some(saved) = store::read_json::<FocusState>(&file()).filter(|s| s.active) else {
        return;
    };
    if saved.ends_at > now_ms() {
        SESSION.with(|s| *s.borrow_mut() = saved);
        schedule();
    } else {
        stats::add_focus_minutes(saved.ends_at.saturating_sub(saved.started_at) as f64 / 60_000.0);
        save(&FocusState::default());
    }
}

pub fn state() -> FocusState {
    SESSION.with(|s| s.borrow().clone())
}

pub fn active() -> bool {
    SESSION.with(|s| s.borrow().active)
}

fn cancel_timer() {
    if let Some(id) = TIMER.with(|t| t.borrow_mut().take()) {
        id.remove();
    }
}

fn schedule() {
    cancel_timer();
    let left = SESSION
        .with(|s| s.borrow().ends_at)
        .saturating_sub(now_ms());
    let id = glib::timeout_add_local_once(Duration::from_millis(left).min(TICK), || {
        // This source has run; there is nothing left to remove.
        TIMER.with(|t| t.borrow_mut().take());
        if !active() {
            return;
        }
        if SESSION.with(|s| s.borrow().ends_at) <= now_ms() {
            finish(true);
        } else {
            schedule();
        }
    });
    TIMER.with(|t| *t.borrow_mut() = Some(id));
}

/// Starts a session of 5 to 240 minutes; one already going ends first.
pub fn start(minutes: f64) {
    let length = if minutes.is_finite() {
        minutes.round().clamp(5.0, 240.0)
    } else {
        50.0
    } as u32;
    if active() {
        finish(false);
    }
    let now = now_ms();
    let session = FocusState {
        active: true,
        started_at: now,
        ends_at: now + u64::from(length) * 60_000,
        minutes: length,
    };
    save(&session);
    SESSION.with(|s| *s.borrow_mut() = session);
    PASSES.with(|p| p.borrow_mut().clear());
    schedule();
    window::global_changed();
}

pub fn stop() {
    if active() {
        finish(false);
    }
}

fn finish(completed: bool) {
    cancel_timer();
    let session = SESSION.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let end = now_ms().min(session.ends_at);
    let minutes = (end.saturating_sub(session.started_at) as f64 / 60_000.0).round();
    stats::add_focus_minutes(minutes);
    PASSES.with(|p| p.borrow_mut().clear());
    save(&FocusState::default());
    window::global_changed();
    if completed {
        notify::show(
            &t("focus.done.title"),
            &tv("focus.done.body", &[("n", Var::Number(minutes as i64))]),
        );
    }
}

/// `m.youtube.com` belongs to `youtube.com`; `notyoutube.com` doesn't.
fn host_matches(host: &str, site: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == site
        || host
            .strip_suffix(site)
            .is_some_and(|rest| rest.ends_with('.'))
}

fn matching_site(url: &str) -> Option<String> {
    let host = pages::host_of(url);
    if host.is_empty() {
        return None;
    }
    settings::get()
        .focus_sites
        .into_iter()
        .find(|site| host_matches(&host, site))
}

/// Whether a page at `url` waits behind the focus gate.
pub fn should_gate(url: &str) -> bool {
    if !active() || !(url.starts_with("http:") || url.starts_with("https:")) {
        return false;
    }
    let Some(site) = matching_site(url) else {
        return false;
    };
    PASSES.with(|p| p.borrow().get(&site).is_none_or(|until| *until < now_ms()))
}

/// "Open anyway": the site may load for the next five minutes.
pub fn grant_pass(url: &str) {
    if let Some(site) = matching_site(url) {
        PASSES.with(|p| {
            p.borrow_mut()
                .insert(site, now_ms() + PASS_MINUTES * 60_000)
        });
    }
}

/// Sites' notifications wait while a session is on, if the settings say so.
pub fn silencing_notifications() -> bool {
    active() && settings::get().focus_silence_notifications
}

#[cfg(test)]
mod tests {
    use super::host_matches;

    #[test]
    fn matches_sites_and_their_subdomains() {
        assert!(host_matches("youtube.com", "youtube.com"));
        assert!(host_matches("m.youtube.com", "youtube.com"));
        assert!(host_matches("WWW.YouTube.com.", "youtube.com"));
        assert!(!host_matches("notyoutube.com", "youtube.com"));
        assert!(!host_matches("youtube.com.evil.example", "youtube.com"));
        assert!(!host_matches("x.co", "x.com"));
    }
}
