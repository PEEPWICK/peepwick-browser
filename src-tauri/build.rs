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

    // The version people see (0.2.0a), from package.json and its release stage, the way
    // scripts/stage.cjs reads them for the release tags.
    let output = Command::new("node")
        .arg("-p")
        .arg("const s = require('../scripts/stage.cjs'), p = require('../package.json'); `${s.displayVersion(p)} ${s.stageOf(p)}`")
        .output()
        .expect("Node.js is needed to build PEEPWICK Browser");
    assert!(
        output.status.success(),
        "could not read the version from package.json"
    );
    let text = String::from_utf8(output.stdout).expect("the version is text");
    let (version, stage) = text.trim().split_once(' ').expect("a version and a stage");
    println!("cargo:rustc-env=PEEPWICK_VERSION={version}");
    println!("cargo:rustc-env=PEEPWICK_STAGE={stage}");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=../scripts/stage.cjs");
    tauri_build::build()
}
