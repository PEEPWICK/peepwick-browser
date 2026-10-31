//! PEEPWICK Browser: a calm, private browser that guards your attention.
//!
//! Tauri runs the window and the React interface (`src/renderer`); the tabs are WebKitGTK
//! views managed here. See `docs/tauri.md` for the plan of the move from Electron.

#[cfg(any(debug_assertions, feature = "harness"))]
mod harness;
mod input;
mod ipc;
mod pages;
mod paths;
mod search;
mod settings;
mod shield;
mod state;
mod store;
mod tab;
mod window;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc::pw_invoke, ipc::pw_send])
        .setup(|app| {
            paths::init(app.handle())?;
            settings::load();
            shield::start(app.handle().clone());
            window::open(app.handle(), false)?;
            #[cfg(any(debug_assertions, feature = "harness"))]
            harness::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("PEEPWICK Browser could not start");
}
