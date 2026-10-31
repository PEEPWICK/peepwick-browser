//! Page zoom remembered per site: zoom in on a site and it opens at that size next time.
//! Sites without their own size use the default from Settings › Appearance. Private windows
//! remember only until the browser closes.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::Serialize;

use crate::pages::host_of;
use crate::{paths, settings, store};

/// Zoom steps for Ctrl+plus and Ctrl+minus, as in Chromium.
pub const STEPS: [f64; 17] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

thread_local! {
    static SAVED: RefCell<BTreeMap<String, f64>> = RefCell::new(BTreeMap::new());
    static PRIVATE: RefCell<BTreeMap<String, f64>> = RefCell::new(BTreeMap::new());
}

fn file() -> std::path::PathBuf {
    paths::user_path("zoom.json")
}

pub fn load() {
    if let Some(saved) = store::read_json::<BTreeMap<String, f64>>(&file()) {
        SAVED.with(|s| {
            *s.borrow_mut() = saved
                .into_iter()
                .filter(|(_, z)| z.is_finite() && *z > 0.0)
                .collect()
        });
    }
}

fn save() {
    SAVED.with(|s| {
        if let Ok(json) = serde_json::to_vec(&*s.borrow())
            && let Err(error) = store::write_atomic(&file(), &json)
        {
            eprintln!("[zoom] could not save: {error}");
        }
    });
}

pub fn zoom_for(url: &str, private: bool) -> f64 {
    let host = host_of(url);
    let default = settings::get().default_zoom;
    if host.is_empty() {
        return default;
    }
    let own = |map: &RefCell<BTreeMap<String, f64>>| map.borrow().get(&host).copied();
    let private_zoom = if private { PRIVATE.with(own) } else { None };
    private_zoom.or_else(|| SAVED.with(own)).unwrap_or(default)
}

/// Remembers a site's zoom; the default size isn't stored.
pub fn remember(url: &str, factor: f64, private: bool) {
    let host = host_of(url);
    if host.is_empty()
        || !(url.starts_with("http:") || url.starts_with("https:") || url.starts_with("file:"))
    {
        return;
    }
    let at_default = (factor - settings::get().default_zoom).abs() < 0.001;
    let update = |map: &RefCell<BTreeMap<String, f64>>| {
        let mut map = map.borrow_mut();
        if at_default {
            map.remove(&host);
        } else {
            map.insert(host.clone(), (factor * 100.0).round() / 100.0);
        }
    };
    if private {
        PRIVATE.with(update);
    } else {
        SAVED.with(update);
        save();
    }
}

#[derive(Serialize)]
pub struct SiteZoom {
    host: String,
    zoom: f64,
}

pub fn site_zooms() -> Vec<SiteZoom> {
    SAVED.with(|s| {
        s.borrow()
            .iter()
            .map(|(host, zoom)| SiteZoom {
                host: host.clone(),
                zoom: *zoom,
            })
            .collect()
    })
}

/// Forgets one site's zoom, or every site's with None.
pub fn forget(host: Option<&str>) {
    SAVED.with(|s| match host {
        Some(host) => {
            s.borrow_mut().remove(host);
        }
        None => s.borrow_mut().clear(),
    });
    save();
}

/// The next zoom step in a direction ("in", "out" or "reset").
pub fn step(current: f64, direction: &str) -> f64 {
    match direction {
        "in" => STEPS
            .iter()
            .copied()
            .find(|s| *s > current + 0.001)
            .unwrap_or(current),
        "out" => STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < current - 0.001)
            .unwrap_or(current),
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::step;

    #[test]
    fn steps_like_chromium() {
        assert_eq!(step(1.0, "in"), 1.1);
        assert_eq!(step(1.0, "out"), 0.9);
        assert_eq!(step(5.0, "in"), 5.0);
        assert_eq!(step(1.37, "reset"), 1.0);
        assert_eq!(step(1.17, "out"), 1.1);
    }
}
