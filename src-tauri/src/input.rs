//! Turning what someone typed into the address bar into a URL (or a search), as in
//! `src/shared/input.ts`.

use crate::pages::internal_page;
use crate::search::search_url;
use crate::settings::{SearchEngine, has_scheme};

const ALLOWED_SCHEMES: [&str; 6] = ["http:", "https:", "file:", "peepwick:", "about:", "data:"];

pub fn resolve_input(raw: &str, engine: SearchEngine) -> String {
    let input = raw.trim();
    if input.is_empty() {
        return String::new();
    }
    if internal_page(input).is_some() {
        return input.to_string();
    }
    let lower = input.to_lowercase();
    if has_scheme(input) || lower.starts_with("about:") || lower.starts_with("data:") {
        return if ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s)) {
            input.to_string()
        } else {
            search_url(engine, input)
        };
    }
    if input.chars().any(char::is_whitespace) {
        return search_url(engine, input);
    }
    if looks_like_host(input) {
        let host = strip_port(input.split(['/', '?', '#']).next().unwrap_or(""));
        let local = host == "localhost"
            || is_ip(host)
            || host.ends_with(".local")
            || host.ends_with(".test");
        return format!("{}://{input}", if local { "http" } else { "https" });
    }
    search_url(engine, input)
}

fn strip_port(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((name, port))
            if !port.is_empty()
                && port.len() <= 5
                && port.bytes().all(|b| b.is_ascii_digit())
                && !name.ends_with(':') =>
        {
            name
        }
        _ => host,
    }
}

fn is_ip(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>().is_ok()
        || (host.starts_with('[')
            && host.ends_with(']')
            && host[1..host.len() - 1]
                .parse::<std::net::Ipv6Addr>()
                .is_ok())
}

pub fn looks_like_host(input: &str) -> bool {
    let host = strip_port(input.split(['/', '?', '#']).next().unwrap_or(""));
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    if host == "localhost" || is_ip(host) {
        return true;
    }
    if !host
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
    {
        return false;
    }
    // The public suffix list knows ASCII (punycode) names.
    let ascii = url::Url::parse(&format!("http://{}/", host.to_lowercase()))
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    if let Some(ascii) = ascii
        && let Some(domain) = psl::domain(ascii.as_bytes())
        && domain.suffix().is_known()
    {
        return true;
    }
    host.ends_with(".local") || host.ends_with(".test") || host.ends_with(".internal")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG: SearchEngine = SearchEngine::Duckduckgo;

    #[test]
    fn resolves_what_people_type() {
        assert_eq!(resolve_input("example.com", DDG), "https://example.com");
        assert_eq!(
            resolve_input("localhost:3000/a", DDG),
            "http://localhost:3000/a"
        );
        assert_eq!(resolve_input("192.168.1.1", DDG), "http://192.168.1.1");
        assert_eq!(resolve_input("https://a.b/c", DDG), "https://a.b/c");
        assert_eq!(
            resolve_input("peepwick://settings", DDG),
            "peepwick://settings"
        );
        assert_eq!(
            resolve_input("hava durumu", DDG),
            "https://duckduckgo.com/?q=hava%20durumu"
        );
        assert_eq!(
            resolve_input("javascript://alert(1)", DDG),
            "https://duckduckgo.com/?q=javascript%3A%2F%2Falert(1)"
        );
        assert_eq!(
            resolve_input("merhaba", DDG),
            "https://duckduckgo.com/?q=merhaba"
        );
        assert_eq!(
            resolve_input("türkiye.gov.tr", DDG),
            "https://türkiye.gov.tr"
        );
        assert_eq!(resolve_input("nas.local", DDG), "http://nas.local");
        assert_eq!(
            resolve_input("file.notatld", DDG),
            "https://duckduckgo.com/?q=file.notatld"
        );
    }
}
