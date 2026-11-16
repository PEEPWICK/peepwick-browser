//! Dangerous site warnings: lists of hosts that spread malware or phish for passwords, from the
//! malware-filter project (URLhaus, OpenPhish, PhishTank and IPThreat data), downloaded every
//! 12 hours and checked on this computer. Only the lists are fetched; the addresses you visit
//! never leave the browser. The lists are kept in the profile under 0.1.x's names.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use webkit2gtk::glib;

use crate::{pages, paths, settings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threat {
    Malware,
    Phishing,
}

impl Threat {
    const ALL: [Threat; 2] = [Threat::Malware, Threat::Phishing];

    /// The name the interface knows it by.
    pub fn name(self) -> &'static str {
        match self {
            Threat::Malware => "malware",
            Threat::Phishing => "phishing",
        }
    }

    fn list(self) -> &'static str {
        match self {
            Threat::Malware => "urlhaus-filter-domains.txt",
            Threat::Phishing => "phishing-filter-domains.txt",
        }
    }

    fn path(self) -> PathBuf {
        paths::user_path(&format!("safebrowsing-{}.txt", self.name()))
    }
}

const MIRRORS: [&str; 2] = [
    "https://malware-filter.gitlab.io/malware-filter/",
    "https://curbengh.github.io/malware-filter/",
];
const REFRESH_AFTER: Duration = Duration::from_secs(12 * 60 * 60);
/// How often to look whether a list has grown old, in seconds.
const CHECK_EVERY: u32 = 60 * 60;

type Lists = Vec<(Threat, HashSet<String>)>;

thread_local! {
    static LISTS: RefCell<Lists> = const { RefCell::new(Vec::new()) };
    /// Sites opened despite the warning; forgotten when the browser closes.
    static PASSED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static UPDATING: Cell<bool> = const { Cell::new(false) };
}

/// Reads the lists kept on disk before any page opens, so restored tabs are checked too, and
/// looks every hour whether they need downloading again.
pub fn start() {
    if settings::get().safe_browsing {
        let started = Instant::now();
        let mut read = false;
        for threat in Threat::ALL {
            if let Ok(text) = std::fs::read_to_string(threat.path()) {
                put(threat, parse_host_list(&text));
                read = true;
            }
        }
        if read {
            eprintln!("[safebrowsing] lists read in {:?}", started.elapsed());
        }
    }
    refresh();
    glib::timeout_add_seconds_local(CHECK_EVERY, || {
        refresh();
        glib::ControlFlow::Continue
    });
}

/// Reads the lists kept on disk that aren't loaded yet and downloads the ones 12 hours old,
/// away from the main thread. Also when the setting is turned on.
pub fn refresh() {
    if !settings::get().safe_browsing || UPDATING.get() {
        return;
    }
    let loaded: Vec<Threat> = LISTS.with(|lists| lists.borrow().iter().map(|(t, _)| *t).collect());
    let work: Vec<(Threat, bool)> = Threat::ALL
        .into_iter()
        .map(|threat| (threat, !loaded.contains(&threat)))
        .filter(|(threat, unloaded)| *unloaded || !is_fresh(&threat.path()))
        .collect();
    if work.is_empty() {
        return;
    }
    UPDATING.set(true);
    std::thread::spawn(move || {
        let mut client = None;
        for (threat, unloaded) in work {
            let path = threat.path();
            if unloaded && let Ok(text) = std::fs::read_to_string(&path) {
                install(threat, parse_host_list(&text));
            }
            if is_fresh(&path) {
                continue;
            }
            let client = client.get_or_insert_with(|| {
                reqwest::blocking::Client::builder()
                    .user_agent(concat!("PEEPWICK-Browser/", env!("CARGO_PKG_VERSION")))
                    .timeout(Duration::from_secs(60))
                    .build()
            });
            let Ok(client) = client else { break };
            match download(client, threat.list()) {
                Some(text) => {
                    if let Err(error) = crate::store::write_atomic(&path, text.as_bytes()) {
                        crate::log_error!(
                            "[safebrowsing] could not keep the {} list: {error}",
                            threat.name()
                        );
                    }
                    install(threat, parse_host_list(&text));
                }
                None => crate::log_error!(
                    "[safebrowsing] could not download the {} list",
                    threat.name()
                ),
            }
        }
        glib::MainContext::default().invoke(|| UPDATING.set(false));
    });
}

/// Hands a list read on another thread to the main thread, where the checks happen.
fn install(threat: Threat, hosts: HashSet<String>) {
    glib::MainContext::default().invoke(move || put(threat, hosts));
}

fn put(threat: Threat, hosts: HashSet<String>) {
    eprintln!(
        "[safebrowsing] {} list: {} hosts",
        threat.name(),
        hosts.len()
    );
    LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        lists.retain(|(t, _)| *t != threat);
        lists.push((threat, hosts));
        lists.sort_by_key(|(t, _)| Threat::ALL.iter().position(|a| a == t));
    });
}

fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < REFRESH_AFTER)
}

fn download(client: &reqwest::blocking::Client, file: &str) -> Option<String> {
    MIRRORS.iter().find_map(|base| {
        let text = client
            .get(format!("{base}{file}"))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text())
            .ok()?;
        // A captive portal or an error page isn't a list.
        text.starts_with("# Title:").then_some(text)
    })
}

fn parse_host_list(text: &str) -> HashSet<String> {
    text.lines()
        .map(|line| line.trim().to_ascii_lowercase())
        .filter(|host| !host.is_empty() && !host.starts_with('#') && !host.starts_with('!'))
        .collect()
}

/// Which list a host is on, looking at the host itself and the domains above it.
fn listed_threat(host: &str, lists: &Lists) -> Option<Threat> {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let address = host.contains(':') || host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let mut name = host.as_str();
    loop {
        if let Some((threat, _)) = lists.iter().find(|(_, hosts)| hosts.contains(name)) {
            return Some(*threat);
        }
        // Addresses are only listed whole, and a bare top-level domain is never looked up.
        match name.split_once('.') {
            Some((_, rest)) if !address && rest.contains('.') => name = rest,
            _ => return None,
        }
    }
}

/// The list a page's address is on, unless the warnings are off or the site was opened anyway.
pub fn threat_for(url: &str) -> Option<Threat> {
    if !settings::get().safe_browsing || !(url.starts_with("http:") || url.starts_with("https:")) {
        return None;
    }
    let host = pages::host_of(url);
    if host.is_empty() || PASSED.with(|passed| passed.borrow().contains(&host)) {
        return None;
    }
    LISTS.with(|lists| listed_threat(&host, &lists.borrow()))
}

/// "Continue anyway": the site opens without the warning until the browser closes.
pub fn pass(url: &str) {
    PASSED.with(|passed| passed.borrow_mut().insert(pages::host_of(url)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists() -> Lists {
        vec![
            (
                Threat::Malware,
                parse_host_list("# Title: malware\nbad.example\n203.0.113.9\n"),
            ),
            (
                Threat::Phishing,
                parse_host_list("# Title: phishing\n! comment\n\n  Login-Bank.Example.com  \n"),
            ),
        ]
    }

    #[test]
    fn reads_host_lists() {
        let hosts = parse_host_list("# Title: x\n! note\n\n Foo.Example \nbar.example\r\n");
        assert_eq!(hosts.len(), 2);
        assert!(hosts.contains("foo.example") && hosts.contains("bar.example"));
    }

    #[test]
    fn finds_hosts_and_the_domains_above_them() {
        let lists = lists();
        assert_eq!(listed_threat("bad.example", &lists), Some(Threat::Malware));
        assert_eq!(
            listed_threat("cdn.bad.example", &lists),
            Some(Threat::Malware)
        );
        assert_eq!(listed_threat("BAD.example.", &lists), Some(Threat::Malware));
        assert_eq!(
            listed_threat("login-bank.example.com", &lists),
            Some(Threat::Phishing)
        );
        assert_eq!(
            listed_threat("www.login-bank.example.com", &lists),
            Some(Threat::Phishing)
        );
        // Neither a domain below a listed one nor its neighbours.
        assert_eq!(listed_threat("example.com", &lists), None);
        assert_eq!(listed_threat("bank.example.com", &lists), None);
        assert_eq!(listed_threat("notbad.example", &lists), None);
    }

    #[test]
    fn looks_up_addresses_whole_and_never_a_top_level_domain() {
        let mut lists = lists();
        assert_eq!(listed_threat("203.0.113.9", &lists), Some(Threat::Malware));
        assert_eq!(listed_threat("1.203.0.113.9", &lists), None);
        lists[0].1.insert("example".into());
        assert_eq!(listed_threat("site.example", &lists), None);
        assert_eq!(listed_threat("[::1]", &lists), None);
    }
}
