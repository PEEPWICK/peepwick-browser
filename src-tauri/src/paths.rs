//! Where the browser keeps its data: the profile folder, or PEEPWICK_PROFILE for tests.

use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::Manager;

static PROFILE: OnceLock<PathBuf> = OnceLock::new();

pub fn init(app: &tauri::AppHandle) -> std::io::Result<()> {
    let dir = match std::env::var_os("PEEPWICK_PROFILE") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let base = app.path().app_data_dir().map_err(std::io::Error::other)?;
            // Development builds keep away from the installed browser's data.
            if cfg!(debug_assertions) {
                base.with_file_name("com.peepwick.browser.dev")
            } else {
                base
            }
        }
    };
    std::fs::create_dir_all(&dir)?;
    let _ = PROFILE.set(dir);
    Ok(())
}

pub fn profile() -> &'static PathBuf {
    PROFILE
        .get()
        .expect("paths::init runs before anything reads the profile")
}

pub fn user_path(name: &str) -> PathBuf {
    profile().join(name)
}
