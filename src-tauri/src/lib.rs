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
mod updater;
mod window;
mod zoom;

/// Addresses and files on a command line: the desktop's "open with", or a second launch.
/// Relative paths are read from `cwd`, the folder that launch started in.
fn urls_from(args: impl IntoIterator<Item = String>, cwd: &std::path::Path) -> Vec<String> {
    args.into_iter()
        .filter(|arg| !arg.starts_with('-'))
        .filter_map(|arg| {
            if ["http://", "https://", "file://"]
                .iter()
                .any(|s| arg.starts_with(s))
            {
                return Some(arg);
            }
            let path = std::fs::canonicalize(cwd.join(&arg)).ok()?;
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
    let builder = tauri::Builder::default();
    // One browser per profile: a second launch (a link opened from another app) hands its
    // addresses to the running one and quits. Test runs with their own profiles are left apart.
    #[cfg(not(any(debug_assertions, feature = "harness")))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
        let urls = urls_from(args.into_iter().skip(1), std::path::Path::new(&cwd));
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || window::open_urls(&handle, urls));
    }));
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![ipc::pw_invoke, ipc::pw_send])
        .register_uri_scheme_protocol("peepwick", |_, request| favicon_response(request.uri()))
        .setup(|app| {
            paths::init(app.handle())?;
            settings::load();
            zoom::load();
            history::open();
            shield::start(app.handle().clone());
            let cwd = std::env::current_dir().unwrap_or_default();
            session::restore(app.handle(), urls_from(std::env::args().skip(1), &cwd))?;
            updater::start(app.handle());
            #[cfg(any(debug_assertions, feature = "harness"))]
            harness::start(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("PEEPWICK Browser could not start")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                updater::install_on_exit();
            }
        });
}
