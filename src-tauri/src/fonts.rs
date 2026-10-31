//! Guards against a broken user font cache. Cache files in ~/.cache/fontconfig linked to an
//! older format (`*.cache-9` → `*.cache-12`) make WebKit's web process spin at full CPU while
//! it starts, and pages never load. When such links are found, fontconfig is pointed at the
//! system configuration with a cache folder of the browser's own.

use std::path::PathBuf;

fn cache_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

fn has_old_cache_links(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_name().to_string_lossy().ends_with(".cache-9")
                && entry.file_type().is_ok_and(|t| t.is_symlink())
        })
    })
}

/// Runs first, before GTK or WebKit read any font; the web processes inherit the setting.
pub fn guard() {
    if std::env::var_os("FONTCONFIG_FILE").is_some() {
        return;
    }
    let Some(cache) = cache_home() else { return };
    if !has_old_cache_links(&cache.join("fontconfig")) {
        return;
    }
    let own = cache.join("peepwick-browser").join("fontconfig");
    if std::fs::create_dir_all(&own).is_err() {
        return;
    }
    let config = format!(
        r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts</dir>
  <dir>/usr/local/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir>~/.fonts</dir>
  <include ignore_missing="yes">/etc/fonts/conf.d</include>
  <cachedir>/var/cache/fontconfig</cachedir>
  <cachedir>{}</cachedir>
</fontconfig>
"#,
        own.display()
    );
    let file = own.join("fonts.conf");
    if crate::store::write_atomic(&file, config.as_bytes()).is_err() {
        return;
    }
    eprintln!(
        "[fonts] the user font cache has old links; using {}",
        own.display()
    );
    // SAFETY: nothing else runs yet; this is the first thing the browser does.
    unsafe { std::env::set_var("FONTCONFIG_FILE", &file) };
}

#[cfg(test)]
mod tests {
    use super::has_old_cache_links;

    #[test]
    fn finds_links_to_newer_cache_files() {
        let dir = std::env::temp_dir().join(format!("peepwick-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.cache-9"), b"").unwrap();
        assert!(!has_old_cache_links(&dir));
        std::os::unix::fs::symlink(dir.join("a.cache-12"), dir.join("a.cache-9")).unwrap();
        assert!(has_old_cache_links(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
