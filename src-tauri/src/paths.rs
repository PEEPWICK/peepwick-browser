//! Where the browser keeps its data (the profile folder, or PEEPWICK_PROFILE for tests) and
//! its cache, and finds the files it ships with.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::Manager;

static PROFILE: OnceLock<PathBuf> = OnceLock::new();
static CACHE: OnceLock<PathBuf> = OnceLock::new();
static RESOURCES: OnceLock<PathBuf> = OnceLock::new();

pub fn init(app: &tauri::AppHandle) -> std::io::Result<()> {
    if let Ok(dir) = app.path().resource_dir() {
        let _ = RESOURCES.set(dir);
    }
    let (dir, cache) = match std::env::var_os("PEEPWICK_PROFILE") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            let cache = dir.join("cache");
            (dir, cache)
        }
        None => {
            let base = app.path().app_data_dir().map_err(std::io::Error::other)?;
            let cache = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| base.join("cache"));
            // Development builds keep away from the installed browser's data and cache.
            if cfg!(debug_assertions) {
                (dev(&base), dev(&cache))
            } else {
                (base, cache)
            }
        }
    };
    std::fs::create_dir_all(&dir)?;
    let _ = PROFILE.set(dir);
    let _ = CACHE.set(cache);
    Ok(())
}

fn dev(dir: &Path) -> PathBuf {
    dir.with_file_name("com.peepwick.browser.dev")
}

pub fn profile() -> &'static PathBuf {
    PROFILE
        .get()
        .expect("paths::init runs before anything reads the profile")
}

pub fn user_path(name: &str) -> PathBuf {
    profile().join(name)
}

/// WebKit's cache folder: inside a test profile, and apart from the installed browser's in
/// development builds.
pub fn cache() -> &'static PathBuf {
    CACHE
        .get()
        .expect("paths::init runs before anything reads the cache folder")
}

/// A file shipped with the browser (tauri.conf.json's resources), if it's there.
pub fn resource(name: &str) -> Option<PathBuf> {
    RESOURCES
        .get()
        .map(|dir| dir.join(name))
        .filter(|path| path.exists())
}
