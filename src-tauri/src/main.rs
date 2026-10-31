// Keeps the release build from opening a console window (only matters on Windows).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    peepwick::run()
}
