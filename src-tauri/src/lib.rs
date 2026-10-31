//! PEEPWICK Browser: a calm, private browser that guards your attention.
//!
//! Tauri runs the window and the React interface (`src/renderer`); the tabs are WebKitGTK
//! views managed here. See `docs/tauri.md` for the plan of the move from Electron.

mod actions;
mod fonts;
#[cfg(any(debug_assertions, feature = "harness"))]
mod harness;
mod history;
mod i18n;
mod input;
mod ipc;
mod menu;
mod menus;
mod omnibox;
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

/// peepwick://favicon/<host>: a stored site icon, for the lists the UI draws.
fn favicon_response(uri: &tauri::http::Uri) -> tauri::http::Response<Vec<u8>> {
    let host = (uri.host() == Some("favicon")).then(|| {
        percent_encoding::percent_decode_str(uri.path().trim_start_matches('/'))
            .decode_utf8_lossy()
            .to_lowercase()
    });
    let response = tauri::http::Response::builder();
    let response = match host.and_then(|host| history::favicon_file(&host)) {
        Some((mime, bytes)) => response
            .header("content-type", mime)
            .header("cache-control", "max-age=3600")
            .body(bytes),
        None => response
            .status(404)
            .header("cache-control", "no-store")
            .body(Vec::new()),
    };
    response.unwrap_or_default()
}

pub fn run() {
    fonts::guard();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc::pw_invoke, ipc::pw_send])
        .register_uri_scheme_protocol("peepwick", |_, request| favicon_response(request.uri()))
        .setup(|app| {
            paths::init(app.handle())?;
            settings::load();
            zoom::load();
            history::open();
            shield::start(app.handle().clone());
            session::restore(app.handle(), urls_from_args())?;
            #[cfg(any(debug_assertions, feature = "harness"))]
            harness::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("PEEPWICK Browser could not start");
}
