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
