//! Updates from GitHub Releases through Tauri's updater: checked at startup and every four
//! hours, downloaded in the background and installed on restart, as 0.1.x did. Packages are
//! signed, and nothing is installed unless its signature matches the key built into the
//! browser, for the version the list announces.
//!
//! The list of the newest version (latest.json) lives in the `updates` branch, because GitHub's
//! "latest release" skips pre-releases.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gtk::glib;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::state::UpdateState;
use crate::{paths, session, window};

const CHECK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);

static STATE: Mutex<UpdateState> = Mutex::new(UpdateState {
    status: "idle",
    version: None,
    progress: 0.0,
    error: None,
});

/// A downloaded and verified .deb: installed when the browser restarts or quits, because
/// installing asks for the system password.
static PENDING_DEB: Mutex<Option<Vec<u8>>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq)]
enum Installer {
    AppImage,
    Deb,
}

/// How this copy was installed; development builds and copies run from elsewhere don't update.
fn installer() -> Option<Installer> {
    if cfg!(debug_assertions) || cfg!(feature = "harness") {
        return None;
    }
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::AppImage) if std::env::var_os("APPIMAGE").is_some() => {
            Some(Installer::AppImage)
        }
        Some(BundleType::Deb) => Some(Installer::Deb),
        _ => None,
    }
}

pub fn state() -> UpdateState {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Changes the state and lets every window know; callable from any thread.
fn set(app: &AppHandle, change: impl FnOnce(&mut UpdateState)) {
    change(
        &mut STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    let _ = app.run_on_main_thread(window::global_changed);
}

fn fail(app: &AppHandle, error: String) {
    eprintln!("[updater] {error}");
    set(app, |s| {
        s.status = "error";
        s.error = Some(error);
    });
}

pub fn start(app: &AppHandle) {
    if installer().is_none() {
        set(app, |s| s.status = "unsupported");
        return;
    }
    let first = app.clone();
    glib::timeout_add_local_once(Duration::from_secs(15), move || check(&first));
    let every = app.clone();
    glib::timeout_add_local(CHECK_EVERY, move || {
        check(&every);
        glib::ControlFlow::Continue
    });
}

/// Looks for a newer version, and downloads it if there is one. Also run by "Check for
/// updates" in Settings.
pub fn check(app: &AppHandle) {
    if installer().is_none() {
        set(app, |s| s.status = "unsupported");
        return;
    }
    if matches!(state().status, "checking" | "downloading" | "ready") {
        return;
    }
    set(app, |s| {
        s.status = "checking";
        s.error = None;
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let found = match app.updater() {
            Ok(updater) => updater.check().await.map_err(|e| e.to_string()),
            Err(error) => Err(error.to_string()),
        };
        match found {
            Ok(Some(update)) => download(app, update).await,
            Ok(None) => set(&app, |s| {
                s.status = "latest";
                s.progress = 0.0;
            }),
            Err(error) => fail(&app, error),
        }
    });
}

/// 0.2.2 → 0.2.2a: the release workflow writes the version people see into the list.
fn shown_version(update: &Update) -> String {
    match update
        .raw_json
        .get("displayVersion")
        .and_then(|v| v.as_str())
    {
        Some(shown) => shown.to_string(),
        None => update.version.clone(),
    }
}

async fn download(app: AppHandle, update: Update) {
    let version = shown_version(&update);
    set(&app, |s| {
        s.status = "downloading";
        s.version = Some(version);
        s.progress = 0.0;
    });
    let (mut received, mut shown) = (0u64, 0u64);
    let progress = app.clone();
    let downloaded = update
        .download(
            move |chunk, total| {
                received += chunk as u64;
                let Some(total) = total.filter(|t| *t > 0) else {
                    return;
                };
                let percent = (received * 100 / total).min(100);
                if percent != shown {
                    shown = percent;
                    set(&progress, |s| s.progress = percent as f64);
                }
            },
            || {},
        )
        .await;
    // `download` has checked the signature.
    let bytes = match downloaded {
        Ok(bytes) => bytes,
        Err(error) => return fail(&app, error.to_string()),
    };
    match installer() {
        Some(Installer::AppImage) => {
            // The AppImage file is replaced now; the running browser keeps its copy until it
            // restarts.
            let installed =
                tauri::async_runtime::spawn_blocking(move || update.install(&bytes)).await;
            match installed {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return fail(&app, error.to_string()),
                Err(error) => return fail(&app, error.to_string()),
            }
        }
        _ => {
            *PENDING_DEB
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(bytes)
        }
    }
    set(&app, |s| {
        s.status = "ready";
        s.progress = 100.0;
    });
}

/// Installs a .deb with the system's password prompt (pkexec), from a folder only this user
/// can open.
fn install_deb(bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    let dir: PathBuf = paths::user_path("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| e.to_string())?;
    let file = dir.join("peepwick-browser.deb");
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("pkexec")
        .arg("dpkg")
        .arg("-i")
        .arg(&file)
        .status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err("the update was not installed".into()),
        Err(error) => Err(error.to_string()),
    }
}

/// "Restart to update": a waiting .deb is installed first, then the browser starts again with
/// the new version and the same windows.
pub fn install_and_restart(app: &AppHandle) {
    if state().status != "ready" {
        return;
    }
    let deb = PENDING_DEB
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(bytes) = deb
            && let Err(error) = install_deb(&bytes)
        {
            // Kept for another try, or for when the browser quits.
            *PENDING_DEB
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(bytes);
            eprintln!("[updater] {error}");
            return;
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || relaunch(&handle));
    });
}

/// Starts the new version and quits. Unlike `AppHandle::restart`, the addresses this copy was
/// opened with aren't passed on: the restored session already has them. For an AppImage the
/// path is the AppImage file, which now holds the new version.
fn relaunch(app: &AppHandle) {
    session::save_before_exit();
    app.cleanup_before_exit();
    if let Ok(path) = tauri::process::current_binary(&app.env()) {
        let _ = std::process::Command::new(path).spawn();
    }
    std::process::exit(0);
}

/// The browser is quitting with a .deb waiting: install it now, as 0.1.x did.
pub fn install_on_exit() {
    let deb = PENDING_DEB
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(bytes) = deb
        && let Err(error) = install_deb(&bytes)
    {
        eprintln!("[updater] {error}");
    }
}
