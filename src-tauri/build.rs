use std::path::PathBuf;
use std::process::Command;

fn main() {
    // The UI's translations (src/shared/i18n.ts), as JSON for src/i18n.rs.
    let out =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("i18n.json");
    let status = Command::new("node")
        .arg("../scripts/i18n-json.mjs")
        .arg(&out)
        .status()
        .expect("Node.js is needed to build PEEPWICK Browser");
    assert!(status.success(), "could not export the translations");
    println!("cargo:rerun-if-changed=../src/shared/i18n.ts");
    println!("cargo:rerun-if-changed=../scripts/i18n-json.mjs");
    tauri_build::build()
}
