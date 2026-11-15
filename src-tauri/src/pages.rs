//! Built-in pages (drawn by the UI) and what the address says about a page's security,
//! as in `src/shared/url.ts`.

const INTERNAL_PAGES: [&str; 8] = [
    "newtab",
    "settings",
    "history",
    "extensions",
    "bookmarks",
    "downloads",
    "passwords",
    "reading",
];

pub const NEW_TAB_URL: &str = "peepwick://newtab";

/// What follows `peepwick://`, written in any case.
fn after_scheme(url: &str) -> Option<&str> {
    let url = url.trim();
    url.get(..11)
        .filter(|s| s.eq_ignore_ascii_case("peepwick://"))
        .map(|_| &url[11..])
}

/// `peepwick://settings/privacy` → "settings".
pub fn internal_page(url: &str) -> Option<&'static str> {
    let rest = after_scheme(url)?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_lowercase();
    INTERNAL_PAGES.iter().copied().find(|page| *page == name)
}

/// `peepwick://settings/privacy` → "privacy", also from the old `peepwick://settings#privacy`
/// form, as `internalSubpage` in `src/shared/url.ts`.
pub fn internal_subpage(url: &str) -> String {
    fn word(s: &str) -> &str {
        let end = s
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '-'))
            .unwrap_or(s.len());
        &s[..end]
    }
    let Some(rest) = after_scheme(url) else {
        return String::new();
    };
    let after_page = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    if after_page.len() == rest.len() {
        return String::new();
    }
    let after_page = after_page.strip_prefix('/').unwrap_or(after_page);
    let mut subpage = word(after_page);
    if subpage.is_empty() {
        subpage = after_page.strip_prefix('#').map(word).unwrap_or_default();
    }
    subpage.to_ascii_lowercase()
}

pub fn internal_url(page: &str, subpage: &str) -> String {
    if subpage.is_empty() {
        format!("peepwick://{page}")
    } else {
        format!("peepwick://{page}/{subpage}")
    }
}

/// Hosts on this computer or the local network, which can't get a certificate from a public
/// authority.
pub fn is_local_host(host: &str) -> bool {
    let h = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_lowercase();
    if !h.contains('.') && !h.contains(':') {
        return true;
    }
    const LOCAL_SUFFIXES: [&str; 10] = [
        ".localhost",
        ".local",
        ".lan",
        ".home",
        ".test",
        ".internal",
        ".intranet",
        ".corp",
        ".home.arpa",
        ".example",
    ];
    if LOCAL_SUFFIXES.iter().any(|s| h.ends_with(s)) || h.ends_with(".invalid") {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        return a == 127
            || a == 10
            || a == 0
            || (a == 169 && b == 254)
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || (a == 100 && (64..=127).contains(&b));
    }
    if let Ok(ip) = h.parse::<std::net::Ipv6Addr>() {
        let first = ip.segments()[0];
        return ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80;
    }
    false
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The security shown before a certificate check result is known.
pub fn security_of(url: &str) -> &'static str {
    if url.starts_with("https:") {
        "secure"
    } else if url.starts_with("http:") {
        if is_local_host(&host_of(url)) {
            "local"
        } else {
            "insecure"
        }
    } else if url.starts_with("file:") {
        "local"
    } else {
        "internal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_internal_pages() {
        assert_eq!(
            internal_page("peepwick://settings/privacy"),
            Some("settings")
        );
        assert_eq!(internal_page("PEEPWICK://NewTab"), Some("newtab"));
        assert_eq!(internal_page("peepwick://saved/abc"), None);
        assert_eq!(internal_page("https://settings"), None);
    }

    #[test]
    fn finds_subpages() {
        for (url, subpage) in [
            ("peepwick://settings/about", "about"),
            (" PEEPWICK://Settings/Site-Data ", "site-data"),
            ("peepwick://settings#privacy", "privacy"),
            ("peepwick://settings/#privacy", "privacy"),
            ("peepwick://settings/about#privacy", "about"),
            ("peepwick://settings/about?from=menu", "about"),
            ("peepwick://settings", ""),
            ("peepwick://settings/", ""),
            ("peepwick://settings#", ""),
            ("peepwick:///about", ""),
            ("https://settings/about", ""),
        ] {
            assert_eq!(internal_subpage(url), subpage, "{url}");
        }
        assert_eq!(
            internal_subpage(&internal_url("settings", "about")),
            "about"
        );
    }

    #[test]
    fn knows_local_hosts() {
        for host in [
            "localhost",
            "printer",
            "nas.local",
            "10.0.0.8",
            "172.20.1.1",
            "192.168.1.1",
            "[::1]",
            "fd12:3456::1",
        ] {
            assert!(is_local_host(host), "{host}");
        }
        for host in ["example.com", "8.8.8.8", "172.32.0.1", "local.com"] {
            assert!(!is_local_host(host), "{host}");
        }
        assert_eq!(security_of("http://192.168.1.1/"), "local");
        assert_eq!(security_of("http://example.com/"), "insecure");
    }
}
