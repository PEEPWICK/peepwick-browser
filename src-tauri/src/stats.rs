//! Daily counts for the new tab page and the focus panel: minutes of focus, and requests the
//! shield blocked. WebKitGTK doesn't report blocked requests, so those stay as the Electron
//! version left them. Kept in stats.json in the Electron version's format, for 90 days.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{paths, store};

const KEEP_DAYS: i32 = 90;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
struct Day {
    blocked: u64,
    focus: f64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stats {
    days: BTreeMap<String, Day>,
    blocked_total: u64,
}

thread_local! {
    static DATA: RefCell<Stats> = RefCell::new(Stats::default());
}

fn file() -> PathBuf {
    paths::user_path("stats.json")
}

/// The local date `days_ago` days back, as stats.json names its days.
fn day_key(days_ago: i32) -> String {
    glib::DateTime::now_local()
        .and_then(|now| now.add_days(-days_ago))
        .and_then(|day| day.format("%Y-%m-%d"))
        .map(|key| key.to_string())
        .unwrap_or_default()
}

pub fn load() {
    let mut data: Stats = store::read_json(&file()).unwrap_or_default();
    let oldest = day_key(KEEP_DAYS);
    data.days.retain(|key, _| *key >= oldest);
    DATA.with(|d| *d.borrow_mut() = data);
}

fn save(data: &Stats) {
    let written = serde_json::to_vec(data)
        .map_err(std::io::Error::other)
        .and_then(|bytes| store::write_atomic(&file(), &bytes));
    if let Err(error) = written {
        crate::log_error!("[stats] could not save: {error}");
    }
}

pub fn add_focus_minutes(minutes: f64) {
    if !(minutes > 0.0) {
        return;
    }
    let today = day_key(0);
    DATA.with(|d| {
        let mut data = d.borrow_mut();
        data.days.entry(today).or_default().focus += minutes;
        save(&data);
    });
}

/// Days with focus minutes or blocked requests, for the backup dialog.
pub fn active_days() -> usize {
    DATA.with(|d| {
        d.borrow()
            .days
            .values()
            .filter(|day| day.focus > 0.0 || day.blocked > 0)
            .count()
    })
}

/// stats.json as it is, for a backup.
pub fn export() -> Value {
    DATA.with(|d| serde_json::to_value(&*d.borrow()).unwrap_or_default())
}

/// "2026-11-17", as stats.json names its days.
fn is_day_key(key: &str) -> bool {
    key.len() == 10
        && key.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// A backup's focus history merged in: each day keeps the larger of the two counts, as the
/// Electron version did. Returns how many of the last 90 days it raised.
pub fn merge(incoming: &Value) -> usize {
    let Some(days) = incoming.get("days").and_then(Value::as_object) else {
        return 0;
    };
    let oldest = day_key(KEEP_DAYS);
    let mut raised = 0;
    DATA.with(|d| {
        let mut data = d.borrow_mut();
        for (key, day) in days {
            let blocked = day.get("blocked").and_then(Value::as_f64);
            let focus = day.get("focus").and_then(Value::as_f64);
            let (Some(blocked), Some(focus)) = (blocked, focus) else {
                continue;
            };
            if !is_day_key(key)
                || *key < oldest
                || !(blocked.is_finite() && blocked >= 0.0 && focus.is_finite() && focus >= 0.0)
            {
                continue;
            }
            let current = data.days.entry(key.clone()).or_default();
            let blocked = blocked.round() as u64;
            if blocked > current.blocked || focus > current.focus {
                raised += 1;
            }
            current.blocked = current.blocked.max(blocked);
            current.focus = current.focus.max(focus);
        }
        if let Some(total) = incoming.get("blockedTotal").and_then(Value::as_u64) {
            data.blocked_total = data.blocked_total.max(total);
        }
        save(&data);
    });
    raised
}

/// What the new tab page and the focus panel show.
pub fn stats() -> Value {
    DATA.with(|d| {
        let data = d.borrow();
        let week: Vec<Day> = (0..7)
            .map(|i| data.days.get(&day_key(i)).copied().unwrap_or_default())
            .collect();
        json!({
            "blockedToday": week[0].blocked,
            "blockedWeek": week.iter().map(|d| d.blocked).sum::<u64>(),
            "blockedTotal": data.blocked_total,
            "focusMinutesToday": week[0].focus.round() as i64,
            "focusMinutesWeek": week.iter().map(|d| d.focus).sum::<f64>().round() as i64,
        })
    })
}
