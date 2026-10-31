//! PEEPWICK Browser: a calm, private browser that guards your attention.
//!
//! Tauri runs the window and the React interface (`src/renderer`); the tabs are WebKitGTK
//! views managed here. See `docs/tauri.md` for the plan of the move from Electron.

mod actions;
mod fonts;
#[cfg(any(debug_assertions, feature = "harness"))]
mod harness;
mod i18n;
mod input;
mod ipc;
mod menu;
mod menus;
mod pages;
mod paths;
mod search;
mod session;
mod settings;
mod shield;
mod shortcuts;
mod state;
mod store;
mod tab;
mod window;
mod zoom;

/// Addresses and files given on the command line (the desktop's "open with").
fn urls_from_args() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .filter_map(|arg| {
            if ["http://", "https://", "file://"]
                .iter()
                .any(|s| arg.starts_with(s))
            {
                return Some(arg);
            }
            let path = std::fs::canonicalize(&arg).ok()?;
            url::Url::from_file_path(path).ok().map(String::from)
        })
        .take(20)
        .collect()
}

pub fn run() {
    fonts::guard();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc::pw_invoke, ipc::pw_send])
        .setup(|app| {
            paths::init(app.handle())?;
            settings::load();
            zoom::load();
            shield::start(app.handle().clone());
            session::restore(app.handle(), urls_from_args())?;
            #[cfg(any(debug_assertions, feature = "harness"))]
            harness::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("PEEPWICK Browser could not start");
}
