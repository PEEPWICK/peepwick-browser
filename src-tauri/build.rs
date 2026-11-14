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

    // The web process extension (webext/peepwick-webext.c). It lands in gen/, where
    // tauri.conf.json's resources take it from: so it has to be there before tauri_build runs.
    build_web_extension();
    tauri_build::build()
}

fn build_web_extension() {
    let source = "webext/peepwick-webext.c";
    println!("cargo:rerun-if-changed={source}");
    let built =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("webext.so");
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "webkit2gtk-web-extension-4.1"])
        .output()
        .expect("pkg-config is needed to build PEEPWICK Browser");
    assert!(
        flags.status.success(),
        "webkit2gtk-web-extension-4.1 not found: install libwebkit2gtk-4.1-dev"
    );
    let flags = String::from_utf8(flags.stdout).expect("pkg-config prints text");
    let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let status = Command::new(&compiler)
        .args([
            "-shared",
            "-fPIC",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Wno-unused-parameter",
        ])
        .args(["-Wl,--as-needed", "-s", "-o"])
        .arg(&built)
        .arg(source)
        .args(flags.split_whitespace())
        .status()
        .unwrap_or_else(|error| panic!("could not run the C compiler ({compiler}): {error}"));
    assert!(status.success(), "could not build {source}");
    // tauri_build watches the resource: rewriting the same bytes would rebuild every time.
    let shipped = PathBuf::from("gen/webext/libpeepwick-webext.so");
    let bytes = std::fs::read(&built).expect("the compiler wrote the extension");
    if std::fs::read(&shipped).ok().as_deref() != Some(bytes.as_slice()) {
        std::fs::create_dir_all("gen/webext").expect("could not create gen/webext");
        std::fs::write(&shipped, bytes).expect("could not write gen/webext");
    }
}
