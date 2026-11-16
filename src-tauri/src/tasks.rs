//! The task manager (Shift+Esc): the browser's processes with their memory and processor
//! use, tabs grouped by the process they share, and ending a tab's process when a page hangs
//! or eats memory. WebKit names no process: the browser's children come from /proc, and each
//! tab's page, and reader view, says which process it lives in as it commits a load
//! (webext/peepwick-webext.c, `Tab::ask_pid`).

use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

use crate::i18n::t;
use crate::window;

/// /proc counts processor time in these per second (USER_HZ, 100 wherever Linux runs a
/// desktop).
const TICKS_PER_SECOND: f64 = 100.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTab {
    pub id: u32,
    pub title: String,
    pub url: String,
    pub favicon: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pid: i32,
    kind: &'static str,
    /// For services: what runs there.
    name: String,
    tabs: Vec<TaskTab>,
    /// Bytes: the process's own memory and its share of what it shares with others.
    memory: u64,
    /// Percent of one processor core since the last look.
    cpu: f64,
}

/// One of the browser's processes as /proc shows it.
#[derive(Debug, PartialEq)]
struct Process {
    pid: i32,
    ppid: i32,
    name: String,
    /// Processor time used so far.
    ticks: u64,
    zombie: bool,
}

/// The last look at each process's processor time, for the use since then.
static LAST: Mutex<Option<(Instant, HashMap<i32, u64>)>> = Mutex::new(None);

/// Which tabs live in which process, from the main thread. A reader view has a process of
/// its own. Tabs whose process isn't known yet are asked again, for the next look.
pub fn tab_processes() -> HashMap<i32, Vec<TaskTab>> {
    let mut map: HashMap<i32, Vec<TaskTab>> = HashMap::new();
    for win in window::all() {
        for tab in win.all_tabs() {
            let entry = TaskTab {
                id: tab.id,
                title: tab.display_title(),
                url: tab.display_url(),
                favicon: tab.favicon(),
            };
            let (page, reader) = tab.processes();
            if let Some(pid) = page {
                map.entry(pid).or_default().push(entry.clone());
            }
            if let Some(pid) = reader {
                map.entry(pid).or_default().push(TaskTab {
                    title: format!("{} · {}", t("menu.reader"), entry.title),
                    ..entry
                });
            }
        }
    }
    map
}

/// Ends a tab's process, or closes a reader view; their tabs show the "page stopped" screen
/// until reloaded. Only tabs' processes: the UI can't end anything else.
pub fn end(pid: i32) {
    for win in window::all() {
        if win.all_tabs().iter().any(|tab| tab.end_process(pid)) {
            return;
        }
    }
}

/// Every process of the browser with what it does, its memory and processor use; off the
/// main thread.
pub fn rows(tabs: HashMap<i32, Vec<TaskTab>>) -> Vec<TaskRow> {
    let own = std::process::id() as i32;
    let family = family(own, &all_processes());
    let now = Instant::now();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let before = last.take();
    let mut ticks = HashMap::new();
    let mut rows: Vec<TaskRow> = family
        .into_iter()
        .filter(|p| !p.zombie)
        .map(|p| {
            ticks.insert(p.pid, p.ticks);
            let cpu = before
                .as_ref()
                .and_then(|(at, seen)| Some((now.duration_since(*at), *seen.get(&p.pid)?)))
                .map(|(elapsed, earlier)| {
                    cpu_percent(p.ticks.saturating_sub(earlier), elapsed.as_secs_f64())
                })
                .unwrap_or(0.0);
            let shown = tabs.get(&p.pid).cloned().unwrap_or_default();
            let (kind, name) = kind_of(&p, own, !shown.is_empty(), || has_extension(p.pid));
            TaskRow {
                pid: p.pid,
                kind,
                name,
                tabs: if kind == "tab" { shown } else { Vec::new() },
                memory: memory(p.pid),
                cpu,
            }
        })
        .collect();
    *last = Some((now, ticks));
    drop(last);
    sort(&mut rows);
    rows
}

fn cpu_percent(ticks: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    ticks as f64 / TICKS_PER_SECOND / seconds * 100.0
}

/// What a process does. WebKit's web processes hold tabs, the browser's interface, or pages
/// kept for going back; only the tabs' and reader views' load the browser's extension.
fn kind_of(
    process: &Process,
    own: i32,
    has_tabs: bool,
    has_extension: impl FnOnce() -> bool,
) -> (&'static str, String) {
    let name = process.name.as_str();
    if process.pid == own {
        ("browser", String::new())
    } else if name.starts_with("WebKitWebProces") {
        if has_tabs {
            ("tab", String::new())
        } else if !has_extension() {
            ("ui", String::new())
        } else {
            ("other", String::new())
        }
    } else if name.starts_with("WebKitNetworkPr") {
        ("service", "network".into())
    } else if name.starts_with("WebKitGPUProces") {
        ("gpu", String::new())
    } else if name.starts_with("speech-dispatch") {
        ("service", "speech-dispatcher".into())
    } else {
        ("other", name.to_string())
    }
}

/// The browser first, then the interface, then the rest by memory.
fn sort(rows: &mut [TaskRow]) {
    let rank = |row: &TaskRow| match row.kind {
        "browser" => 0,
        "gpu" => 1,
        "ui" => 2,
        _ => 3,
    };
    rows.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.memory.cmp(&a.memory)));
}

fn all_processes() -> Vec<Process> {
    let Ok(dir) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| parse_stat(pid, &fs::read_to_string(format!("/proc/{pid}/stat")).ok()?))
        .collect()
}

/// /proc/<pid>/stat. The name sits in parentheses and may hold spaces and parentheses itself.
fn parse_stat(pid: i32, stat: &str) -> Option<Process> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    // From the state on: state, ppid, … utime is the 14th field, stime the 15th.
    let rest: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some(Process {
        pid,
        ppid: rest.get(1)?.parse().ok()?,
        name,
        ticks: utime + stime,
        zombie: rest.first() == Some(&"Z"),
    })
}

/// The browser and everything it started, their children too.
fn family(root: i32, all: &[Process]) -> Vec<Process> {
    let mut children: HashMap<i32, Vec<&Process>> = HashMap::new();
    let mut own = None;
    for p in all {
        if p.pid == root {
            own = Some(p);
        }
        children.entry(p.ppid).or_default().push(p);
    }
    let mut found = Vec::new();
    let mut next: Vec<&Process> = own.into_iter().collect();
    while let Some(p) = next.pop() {
        if let Some(kids) = children.get(&p.pid) {
            next.extend(kids.iter().copied().filter(|k| k.pid != root));
        }
        found.push(Process {
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.clone(),
            ticks: p.ticks,
            zombie: p.zombie,
        });
    }
    found
}

/// The process's own memory and its share of the pages it shares with others (the
/// proportional set size), so the rows add up to what the browser takes. Older kernels give
/// the resident size.
fn memory(pid: i32) -> u64 {
    let field = |file: &str, key: &str| {
        let text = fs::read_to_string(format!("/proc/{pid}/{file}")).ok()?;
        kilobytes(&text, key)
    };
    field("smaps_rollup", "Pss:")
        .or_else(|| field("status", "VmRSS:"))
        .unwrap_or(0)
        * 1024
}

fn kilobytes(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Whether a web process loaded the browser's extension: the tabs' and reader views' do, the
/// interface's doesn't.
fn has_extension(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/maps"))
        .is_ok_and(|maps| maps.contains("libpeepwick-webext"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: i32, ppid: i32, name: &str) -> Process {
        Process {
            pid,
            ppid,
            name: name.into(),
            ticks: 0,
            zombie: false,
        }
    }

    #[test]
    fn reads_proc_stat() {
        let stat = "4242 (Web (Content) x) S 4000 4242 4000 0 -1 4194560 2370 0 0 0 150 37 0 0 20 0 24 0 312 \
                    1150287872 45376 18446744073709551615";
        assert_eq!(
            parse_stat(4242, stat),
            Some(Process {
                pid: 4242,
                ppid: 4000,
                name: "Web (Content) x".into(),
                ticks: 187,
                zombie: false,
            })
        );
        assert!(parse_stat(1, "1 (init) Z 0 1").is_none());
        assert!(
            parse_stat(7, "7 (x) Z 1 7 7 0 -1 0 0 0 0 0 3 4 0")
                .unwrap()
                .zombie
        );
        assert_eq!(
            kilobytes("Rss:    243200 kB\nPss:    136331 kB\n", "Pss:"),
            Some(136331)
        );
        assert_eq!(kilobytes("Rss: 1 kB\n", "Pss:"), None);
        assert_eq!(cpu_percent(50, 2.0), 25.0);
        assert_eq!(cpu_percent(50, 0.0), 0.0);
    }

    #[test]
    fn finds_the_browsers_processes() {
        let all = [
            process(1, 0, "systemd"),
            process(10, 1, "peepwick-browse"),
            process(11, 10, "WebKitNetworkPr"),
            process(12, 10, "bwrap"),
            process(13, 12, "WebKitWebProces"),
            process(14, 1, "WebKitWebProces"),
        ];
        let mut pids: Vec<i32> = family(10, &all).iter().map(|p| p.pid).collect();
        pids.sort();
        assert_eq!(pids, [10, 11, 12, 13]);
        assert!(family(99, &all).is_empty());
    }

    #[test]
    fn names_what_each_process_does() {
        let web = process(20, 10, "WebKitWebProces");
        let kind = |p: &Process, tabs: bool, extension: bool| kind_of(p, 10, tabs, || extension);
        assert_eq!(
            kind(&process(10, 1, "peepwick-browse"), false, false).0,
            "browser"
        );
        assert_eq!(kind(&web, true, true).0, "tab");
        assert_eq!(kind(&web, false, false).0, "ui");
        assert_eq!(kind(&web, false, true).0, "other");
        assert_eq!(
            kind(&process(21, 10, "WebKitNetworkPr"), false, false),
            ("service", "network".to_string())
        );
        assert_eq!(
            kind(&process(22, 10, "WebKitGPUProces"), false, false).0,
            "gpu"
        );
        assert_eq!(
            kind(&process(23, 10, "xdg-settings"), false, false),
            ("other", "xdg-settings".to_string())
        );

        let row = |pid, kind, memory| TaskRow {
            pid,
            kind,
            name: String::new(),
            tabs: Vec::new(),
            memory,
            cpu: 0.0,
        };
        let mut rows = vec![
            row(1, "tab", 50),
            row(2, "service", 70),
            row(3, "ui", 10),
            row(4, "browser", 5),
            row(5, "tab", 90),
        ];
        sort(&mut rows);
        assert_eq!(
            rows.iter().map(|r| r.pid).collect::<Vec<_>>(),
            [4, 3, 5, 2, 1]
        );
    }
}
