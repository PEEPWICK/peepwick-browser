//! The desktop around the browser: being the system's web browser, and running the desktop's
//! own tools for it.
//!
//! The .deb installs "PEEPWICK Browser.desktop". An AppImage has none, so "make default" would
//! have nothing to point the system at: for an AppImage the browser keeps its own desktop file
//! (and icon) under ~/.local/share, as 0.1.x did, then sets it with xdg-settings and xdg-mime.

use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gtk::glib;

/// The desktop file the .deb installs; Tauri names it after the product.
const DEB_DESKTOP: &str = "PEEPWICK Browser.desktop";
/// The one an AppImage keeps in ~/.local/share/applications, under the name 0.1.x used.
const APPIMAGE_DESKTOP: &str = "peepwick-browser.desktop";
/// What a web browser opens.
const MIME: [&str; 4] = [
    "x-scheme-handler/http",
    "x-scheme-handler/https",
    "text/html",
    "application/xhtml+xml",
];
const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

/// An AppImage's own library and plugin paths. The desktop's programs are started without
/// them, as they would break those programs.
pub const APPIMAGE_ENV: [&str; 15] = [
    "LD_LIBRARY_PATH",
    "GTK_PATH",
    "GTK_EXE_PREFIX",
    "GTK_DATA_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GDK_PIXBUF_MODULE_FILE",
    "GIO_MODULE_DIR",
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_REGISTRY",
    "GST_REGISTRY_1_0",
];

/// The AppImage file this browser runs from, if it runs from one.
fn appimage() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn applications() -> PathBuf {
    glib::user_data_dir().join("applications")
}

/// Whether links open in PEEPWICK Browser. Runs the desktop's tools: not on the main thread.
pub fn is_default() -> bool {
    let limit = Duration::from_secs(3);
    run("xdg-settings", &["get", "default-web-browser"], limit)
        .or_else(|| {
            run(
                "xdg-mime",
                &["query", "default", "x-scheme-handler/https"],
                limit,
            )
        })
        .is_some_and(|id| is_ours(id.trim(), appimage().is_some()))
}

/// A desktop file that starts this browser: the .deb's, or for an AppImage also its own.
fn is_ours(id: &str, from_appimage: bool) -> bool {
    id == DEB_DESKTOP || (from_appimage && id == APPIMAGE_DESKTOP)
}

/// Makes PEEPWICK Browser the system's web browser: "done", "development" (a development
/// build, which no desktop file starts) or "failed". Runs the desktop's tools: not on the main
/// thread.
pub fn make_default() -> &'static str {
    // Tests set this to try the real thing in a development build, with their own XDG folders.
    if cfg!(debug_assertions) && std::env::var_os("PEEPWICK_TEST_DEFAULT_BROWSER").is_none() {
        return "development";
    }
    let limit = Duration::from_secs(8);
    let id = if appimage().is_some() {
        if !sync_appimage_entry(true) {
            return "failed";
        }
        run(
            "update-desktop-database",
            &[applications().as_os_str()],
            limit,
        );
        APPIMAGE_DESKTOP
    } else {
        // Pointing the system at a desktop file that isn't there would leave links opening in
        // nothing at all.
        let installed = std::iter::once(glib::user_data_dir())
            .chain(glib::system_data_dirs())
            .any(|dir| dir.join("applications").join(DEB_DESKTOP).is_file());
        if !installed {
            return "failed";
        }
        DEB_DESKTOP
    };
    run("xdg-settings", &["set", "default-web-browser", id], limit);
    let mut args = vec!["default", id];
    args.extend(MIME);
    run("xdg-mime", &args, limit);
    if is_default() { "done" } else { "failed" }
}

/// For an AppImage: keeps a desktop file that starts this very AppImage. Called at startup too,
/// so that moving or updating the AppImage doesn't leave the system pointing at a file that is
/// gone. Rewrites only a file that is already ours; `create` makes one.
pub fn sync_appimage_entry(create: bool) -> bool {
    let Some(exec) = appimage()
        .as_deref()
        .and_then(|p| p.to_str())
        .and_then(exec_quote)
    else {
        return false;
    };
    let file = applications().join(APPIMAGE_DESKTOP);
    let wanted = desktop_entry(&exec);
    match std::fs::read(&file) {
        Ok(current) if current == wanted.as_bytes() => return true,
        Ok(current) => {
            let ours = String::from_utf8_lossy(&current)
                .lines()
                .any(|line| line == "Name=PEEPWICK Browser");
            if !ours {
                return false;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {}
        Err(_) => return false,
    }
    let written = (|| {
        std::fs::create_dir_all(applications())?;
        crate::store::write_atomic(&file, wanted.as_bytes())?;
        let icons = glib::user_data_dir().join("icons/hicolor/256x256/apps");
        std::fs::create_dir_all(&icons)?;
        crate::store::write_atomic(&icons.join("peepwick-browser.png"), ICON)
    })();
    match written {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[desktop] could not write the desktop file: {error}");
            false
        }
    }
}

/// The AppImage's desktop file. It lists about: and unknown: too, which xdg-settings sets on
/// GNOME: a local desktop file without them gets rewritten, with a four-second wait for each.
fn desktop_entry(exec: &str) -> String {
    [
        "[Desktop Entry]",
        "Type=Application",
        "Name=PEEPWICK Browser",
        "GenericName=Web Browser",
        "GenericName[tr]=Web Tarayıcısı",
        "Comment=A calm, private browser that guards your attention.",
        "Comment[tr]=Dikkatinizi koruyan, sakin ve gizli bir tarayıcı.",
        &format!("Exec={exec} %U"),
        "Icon=peepwick-browser",
        "Terminal=false",
        "StartupNotify=true",
        "StartupWMClass=peepwick-browser",
        "Categories=Network;WebBrowser;",
        "MimeType=text/html;application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;x-scheme-handler/about;x-scheme-handler/unknown;",
        "Keywords=web;browser;internet;tarayıcı;",
        "",
    ]
    .join("\n")
}

/// A program path as a desktop file's Exec value: quoted, with `"`, `` ` ``, `$` and `\`
/// escaped for the quotes and then once more as a string value, as the Desktop Entry
/// specification asks. None for a path no desktop file can hold.
fn exec_quote(path: &str) -> Option<String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str(r"\\");
                out.push(c);
            }
            '\\' => out.push_str(r"\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

/// Runs one of the desktop's tools for at most `limit`: what it printed, if it succeeded.
fn run<S: AsRef<OsStr>>(program: &str, args: &[S], limit: Duration) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if appimage().is_some() {
        for name in APPIMAGE_ENV {
            command.env_remove(name);
        }
    }
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                eprintln!("[desktop] {program} did not finish");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_exec_paths_as_the_spec_asks() {
        assert_eq!(
            exec_quote("/home/user/Masaüstü/PEEPWICK Browser.AppImage").as_deref(),
            Some("\"/home/user/Masaüstü/PEEPWICK Browser.AppImage\"")
        );
        assert_eq!(
            exec_quote(r#"/a/"b"/$c/`d`/e\f/100%"#).as_deref(),
            Some(r#""/a/\\"b\\"/\\$c/\\`d\\`/e\\\\f/100%%""#)
        );
        assert_eq!(exec_quote("/a/b\nc"), None);
        assert_eq!(exec_quote(""), None);
    }

    #[test]
    fn desktop_entry_lists_what_xdg_settings_sets() {
        let entry = desktop_entry("\"/x.AppImage\"");
        assert!(entry.contains("\nExec=\"/x.AppImage\" %U\n"));
        assert!(entry.lines().any(|line| line == "Name=PEEPWICK Browser"));
        let types = entry
            .lines()
            .find_map(|line| line.strip_prefix("MimeType="))
            .unwrap();
        for mime in MIME
            .iter()
            .chain(&["x-scheme-handler/about", "x-scheme-handler/unknown"])
        {
            assert!(types.split(';').any(|t| t == *mime), "{mime}");
        }
    }

    #[test]
    fn knows_its_own_desktop_files() {
        assert!(is_ours("PEEPWICK Browser.desktop", false));
        assert!(is_ours("PEEPWICK Browser.desktop", true));
        assert!(is_ours("peepwick-browser.desktop", true));
        // An old 0.1.x file may still be named so, but a .deb install doesn't start from it.
        assert!(!is_ours("peepwick-browser.desktop", false));
        assert!(!is_ours("firefox.desktop", true));
        assert!(!is_ours("", true));
    }
}
