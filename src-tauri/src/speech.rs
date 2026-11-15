//! Read aloud: speech-dispatcher, the speech service of Linux desktops, spoken to over its
//! own socket (SSIP). WebKitGTK here has no voices behind `speechSynthesis`, so the listen
//! bar asks Rust instead.
//!
//! One connection serves the browser. It opens when the listen bar first asks and closes
//! after a minute without speech. A worker thread sends the commands one at a time; a
//! reader thread takes the replies, and speech-dispatcher's notes on each paragraph
//! (begun, ended, canceled) go to the window that asked.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

/// How long a reply may take. The longest is the first list of voices: about 0.1 s for
/// espeak-ng's 13,000 entries.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// The connection closes after this long with nothing said.
const IDLE: Duration = Duration::from_secs(60);
/// The most voices kept from one speech engine.
const MAX_VOICES: usize = 300;
/// The most text one paragraph sends: the listen bar's chunks are 400 characters at most.
pub const MAX_TEXT: usize = 4000;

/// A voice of one of speech-dispatcher's output modules (speech engines).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Voice {
    pub module: String,
    pub name: String,
    pub lang: String,
}

/// A paragraph to say, from the window `owner`; its notes come back with `key`.
pub struct Utterance {
    pub owner: String,
    pub key: String,
    pub text: String,
    pub lang: String,
    /// The voice the reader picked (module, name); without one speech-dispatcher picks
    /// by language, as its settings say.
    pub voice: Option<(String, String)>,
    /// The listen bar's speed, 1 being normal.
    pub rate: f64,
}

/// What happened to a paragraph, for the listen bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Note {
    Started,
    Ended,
    /// Canceled: by the listen bar, by another window starting to read, or by
    /// speech-dispatcher (a screen reader's message comes first).
    Stopped,
    /// speech-dispatcher couldn't be reached, refused the paragraph or went away.
    Failed,
}

impl Note {
    fn name(self) -> &'static str {
        match self {
            Note::Started => "start",
            Note::Ended => "end",
            Note::Stopped => "stopped",
            Note::Failed => "error",
        }
    }
}

/// Where notes go: the windows' listen bars, or a test's channel.
type Sink = Arc<dyn Fn(&str, &str, Note) + Send + Sync>;

enum Request {
    Voices(tokio::sync::oneshot::Sender<Vec<Voice>>),
    Speak(Utterance),
    Stop(String),
    Quit(Sender<()>),
}

// ── The browser's connection ─────────────────────────────────────────────────

/// The voices speech-dispatcher offers; none when it isn't installed or doesn't answer.
pub async fn voices() -> Vec<Voice> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if worker().send(Request::Voices(tx)).is_err() {
        return Vec::new();
    }
    rx.await.unwrap_or_default()
}

/// Says a paragraph, stopping whatever this browser was saying.
pub fn speak(utterance: Utterance) {
    let _ = worker().send(Request::Speak(utterance));
}

/// Stops what the window `owner` asked to say. Another window's reading goes on.
pub fn stop(owner: &str) {
    if let Some(worker) = WORKER.get() {
        let _ = worker.send(Request::Stop(owner.to_string()));
    }
}

/// The browser is closing: nothing it asked for goes on being said.
pub fn shutdown() {
    if let Some(worker) = WORKER.get() {
        let (tx, rx) = mpsc::channel();
        if worker.send(Request::Quit(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }
}

static WORKER: OnceLock<Sender<Request>> = OnceLock::new();

fn worker() -> &'static Sender<Request> {
    WORKER.get_or_init(|| start(Address::from_env(), Arc::new(to_window)))
}

/// A note for the listen bar of the window `owner`. It goes as an event of its own: a UI
/// command would take the keyboard from the page.
fn to_window(owner: &str, key: &str, note: Note) {
    let (owner, key) = (owner.to_string(), key.to_string());
    glib::MainContext::default().invoke(move || {
        if let Some(win) = crate::window::find(&owner) {
            win.emit("speech", json!({ "key": key, "event": note.name() }));
        }
    });
}

/// Where speech-dispatcher listens.
#[derive(Clone, Debug, PartialEq)]
struct Address {
    path: PathBuf,
    /// The usual socket, which speech-dispatcher may be started on when nothing listens.
    /// An address from SPEECHD_ADDRESS is used as it is.
    usual: bool,
}

impl Address {
    /// SPEECHD_ADDRESS when it is set, read as speech-dispatcher's own library reads it
    /// (unix sockets only); otherwise the socket in the user's runtime folder.
    fn from_env() -> Option<Address> {
        match std::env::var("SPEECHD_ADDRESS") {
            Ok(address) if !address.trim().is_empty() => Address::parse(&address),
            _ => Some(Address::usual()),
        }
    }

    fn parse(address: &str) -> Option<Address> {
        let address = address.trim();
        if address == "unix_socket" {
            return Some(Address::usual());
        }
        let path = address.strip_prefix("unix_socket:")?;
        (!path.is_empty()).then(|| Address {
            path: PathBuf::from(path),
            usual: false,
        })
    }

    fn usual() -> Address {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Address {
            path: runtime.join("speech-dispatcher").join("speechd.sock"),
            usual: true,
        }
    }

    fn connect(&self) -> io::Result<UnixStream> {
        let error = match UnixStream::connect(&self.path) {
            Ok(stream) => return Ok(stream),
            Err(error) if self.usual => error,
            Err(error) => return Err(error),
        };
        // Desktops with systemd start speech-dispatcher on the first connection. Without
        // that, it is started the way its own library does: as a daemon on this socket.
        let mut child = Command::new("speech-dispatcher")
            .args([
                "--spawn",
                "--communication-method",
                "unix_socket",
                "--socket-path",
            ])
            .arg(&self.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| error)?;
        let mut stream = Err(io::Error::from(io::ErrorKind::NotFound));
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            stream = UnixStream::connect(&self.path);
            if stream.is_ok() {
                break;
            }
        }
        std::thread::spawn(move || child.wait());
        stream
    }
}

// ── The worker ───────────────────────────────────────────────────────────────

fn start(address: Option<Address>, sink: Sink) -> Sender<Request> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("peepwick-speech".into())
        .spawn(move || work(address, rx, sink));
    if let Err(error) = spawned {
        eprintln!("[speech] could not start: {error}");
    }
    tx
}

fn work(address: Option<Address>, requests: Receiver<Request>, sink: Sink) {
    let mut connection: Option<Connection> = None;
    // Kept for the browser's run: speech engines don't change their voices meanwhile.
    let mut voices: Option<Vec<Voice>> = None;
    loop {
        let request = if connection.is_some() {
            match requests.recv_timeout(IDLE) {
                Ok(request) => request,
                Err(RecvTimeoutError::Timeout) => {
                    if connection.as_ref().is_some_and(|open| !open.speaking()) {
                        if let Some(open) = connection.take() {
                            open.quit();
                        }
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match requests.recv() {
                Ok(request) => request,
                Err(_) => break,
            }
        };
        // speech-dispatcher went away (restarted, or a crash): the next request reconnects.
        if connection.as_ref().is_some_and(Connection::closed) {
            connection = None;
        }
        match request {
            Request::Voices(reply) => {
                if voices.is_none() {
                    if let Some(open) = connect(&address, &sink, &mut connection) {
                        match open.voices() {
                            Ok(list) if !list.is_empty() => voices = Some(list),
                            Ok(_) | Err(Failure::Refused) => {}
                            Err(Failure::Broken) => connection = None,
                        }
                    }
                }
                let _ = reply.send(voices.clone().unwrap_or_default());
            }
            Request::Speak(utterance) => {
                // A picked voice speaks in its own language.
                let voice_lang = utterance.voice.as_ref().and_then(|(module, name)| {
                    voices
                        .iter()
                        .flatten()
                        .find(|voice| voice.module == *module && voice.name == *name)
                        .map(|voice| voice.lang.clone())
                });
                let Some(open) = connect(&address, &sink, &mut connection) else {
                    sink(&utterance.owner, &utterance.key, Note::Failed);
                    continue;
                };
                if let Err(Failure::Broken) = open.speak(&utterance, voice_lang) {
                    connection = None;
                }
            }
            Request::Stop(owner) => {
                if let Some(open) = connection.as_mut() {
                    if open.speaking_for(&owner) {
                        if let Err(Failure::Broken) = open.command("CANCEL self") {
                            connection = None;
                        }
                    }
                }
            }
            Request::Quit(done) => {
                if let Some(mut open) = connection.take() {
                    let _ = open.command("CANCEL self");
                    open.quit();
                }
                let _ = done.send(());
                break;
            }
        }
    }
}

/// The open connection, or a new one.
fn connect<'a>(
    address: &Option<Address>,
    sink: &Sink,
    connection: &'a mut Option<Connection>,
) -> Option<&'a mut Connection> {
    if connection.is_none() {
        *connection = address
            .as_ref()
            .and_then(|address| Connection::open(address, sink.clone()).ok());
    }
    connection.as_mut()
}

// ── The connection ───────────────────────────────────────────────────────────

/// A reply: its code and the lines before the last ("NNN-…").
#[derive(Debug)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

#[derive(Debug)]
enum Failure {
    /// speech-dispatcher said no; the connection is fine.
    Refused,
    /// The connection is out of step or gone.
    Broken,
}

/// Who asked for a paragraph speech-dispatcher has queued.
#[derive(Clone, Debug)]
struct Ticket {
    owner: String,
    key: String,
}

#[derive(Default)]
struct Shared {
    /// The paragraph whose "message queued" reply comes next.
    pending: Option<Ticket>,
    /// Paragraphs speech-dispatcher has queued and not yet finished, by its message id.
    queued: HashMap<u64, Ticket>,
    /// The reader thread saw the connection end.
    closed: bool,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

struct Connection {
    stream: UnixStream,
    replies: Receiver<Reply>,
    shared: Arc<Mutex<Shared>>,
    sink: Sink,
    /// What a paragraph without a picked voice goes back to.
    usual_module: Option<String>,
    usual_voice_type: Option<String>,
}

impl Drop for Connection {
    /// Ends the reader thread too: it holds a copy of the socket.
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

impl Connection {
    fn open(address: &Address, sink: Sink) -> io::Result<Connection> {
        let stream = address.connect()?;
        stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
        let reading = stream.try_clone()?;
        let (tx, replies) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (state, notes) = (shared.clone(), sink.clone());
        std::thread::Builder::new()
            .name("peepwick-speech-read".into())
            .spawn(move || read(reading, tx, state, notes))?;
        let mut connection = Connection {
            stream,
            replies,
            shared,
            sink,
            usual_module: None,
            usual_voice_type: None,
        };
        let refused = |_: Failure| io::Error::from(io::ErrorKind::ConnectionRefused);
        connection
            .command("SET self CLIENT_NAME user:peepwick:read_aloud")
            .map_err(refused)?;
        connection
            .command("SET self NOTIFICATION ALL on")
            .map_err(refused)?;
        connection.usual_module = connection.get("OUTPUT_MODULE");
        connection.usual_voice_type = connection.get("VOICE_TYPE");
        Ok(connection)
    }

    fn command(&mut self, line: &str) -> Result<Reply, Failure> {
        let line = line.replace(['\r', '\n'], " ");
        if self
            .stream
            .write_all(format!("{line}\r\n").as_bytes())
            .is_err()
        {
            return Err(Failure::Broken);
        }
        self.reply()
    }

    fn reply(&mut self) -> Result<Reply, Failure> {
        match self.replies.recv_timeout(REPLY_TIMEOUT) {
            Ok(reply) if reply.code < 300 => Ok(reply),
            Ok(_) => Err(Failure::Refused),
            // A late reply would answer the next command: the connection starts again.
            Err(_) => Err(Failure::Broken),
        }
    }

    fn get(&mut self, what: &str) -> Option<String> {
        let reply = self.command(&format!("GET {what}")).ok()?;
        reply
            .lines
            .into_iter()
            .next()
            .filter(|value| !value.is_empty())
    }

    /// Every speech engine's voices, without espeak-ng's thousands of "+variant" copies.
    fn voices(&mut self) -> Result<Vec<Voice>, Failure> {
        let modules = self.command("LIST OUTPUT_MODULES")?.lines;
        let mut voices = Vec::new();
        for module in modules {
            // The dummy module only says that speech isn't set up.
            if module == "dummy" || module.trim().is_empty() {
                continue;
            }
            match self.command(&format!("SET self OUTPUT_MODULE {module}")) {
                Ok(_) => {}
                Err(Failure::Refused) => continue,
                Err(Failure::Broken) => return Err(Failure::Broken),
            }
            match self.command("LIST SYNTHESIS_VOICES") {
                Ok(reply) => voices.extend(parse_voices(&module, &reply.lines)),
                Err(Failure::Refused) => {}
                Err(Failure::Broken) => return Err(Failure::Broken),
            }
        }
        if let Some(module) = self.usual_module.clone() {
            self.set("OUTPUT_MODULE", &module)?;
        }
        Ok(voices)
    }

    fn speak(&mut self, utterance: &Utterance, voice_lang: Option<String>) -> Result<(), Failure> {
        let result = self.try_speak(utterance, voice_lang);
        if result.is_err() {
            // It never got queued, so no note will come for it from speech-dispatcher.
            lock(&self.shared).pending = None;
            (self.sink)(&utterance.owner, &utterance.key, Note::Failed);
        }
        result
    }

    fn try_speak(
        &mut self,
        utterance: &Utterance,
        voice_lang: Option<String>,
    ) -> Result<(), Failure> {
        // One paragraph at a time: what was being said stops, and its window hears so.
        self.tell("CANCEL self")?;
        match &utterance.voice {
            Some((module, name)) => {
                let lang = voice_lang.unwrap_or_else(|| utterance.lang.clone());
                self.set("LANGUAGE", &lang)?;
                self.set("OUTPUT_MODULE", module)?;
                self.set("SYNTHESIS_VOICE", name)?;
            }
            None => {
                if let Some(module) = self.usual_module.clone() {
                    self.set("OUTPUT_MODULE", &module)?;
                }
                self.set("LANGUAGE", &utterance.lang)?;
                // A voice type puts aside the voice picked for an earlier paragraph.
                let voice_type = self.usual_voice_type.clone();
                self.set("VOICE_TYPE", voice_type.as_deref().unwrap_or("MALE1"))?;
            }
        }
        self.set("RATE", &ssip_rate(utterance.rate).to_string())?;
        lock(&self.shared).pending = Some(Ticket {
            owner: utterance.owner.clone(),
            key: utterance.key.clone(),
        });
        self.command("SPEAK")?;
        let data = format!("{}\r\n.\r\n", escape(&utterance.text));
        if self.stream.write_all(data.as_bytes()).is_err() {
            return Err(Failure::Broken);
        }
        self.reply().map(|_| ())
    }

    /// A setting for the next paragraph.
    fn set(&mut self, what: &str, value: &str) -> Result<(), Failure> {
        self.tell(&format!("SET self {what} {value}"))
    }

    /// A command whose refusal is let be (a voice of an engine that went away): only a
    /// broken connection counts.
    fn tell(&mut self, line: &str) -> Result<(), Failure> {
        match self.command(line) {
            Ok(_) | Err(Failure::Refused) => Ok(()),
            Err(Failure::Broken) => Err(Failure::Broken),
        }
    }

    fn speaking(&self) -> bool {
        let shared = lock(&self.shared);
        shared.pending.is_some() || !shared.queued.is_empty()
    }

    fn speaking_for(&self, owner: &str) -> bool {
        lock(&self.shared)
            .queued
            .values()
            .any(|ticket| ticket.owner == owner)
    }

    fn closed(&self) -> bool {
        lock(&self.shared).closed
    }

    fn quit(mut self) {
        let _ = self.command("QUIT");
    }
}

/// Reads what speech-dispatcher sends: replies go to the worker, notes on paragraphs to
/// the windows that asked for them.
fn read(stream: UnixStream, replies: Sender<Reply>, shared: Arc<Mutex<Shared>>, sink: Sink) {
    let mut reader = BufReader::new(stream);
    let mut raw = Vec::new();
    let (mut reply, mut note) = (Vec::new(), Vec::new());
    loop {
        raw.clear();
        match reader.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&raw);
        let Some((code, last, text)) = parse_line(line.trim_end_matches(['\r', '\n'])) else {
            continue;
        };
        // Notes (7xx) come whenever a paragraph begins or ends, between replies too.
        if (700..800).contains(&code) {
            if !last {
                note.push(text);
                continue;
            }
            // The message's id, then the client's.
            let lines = std::mem::take(&mut note);
            let id = lines.first().and_then(|id| id.trim().parse::<u64>().ok());
            let event = match code {
                701 => Note::Started,
                702 => Note::Ended,
                703 => Note::Stopped,
                _ => continue,
            };
            let ticket = id.and_then(|id| {
                let mut shared = lock(&shared);
                match event {
                    Note::Started => shared.queued.get(&id).cloned(),
                    _ => shared.queued.remove(&id),
                }
            });
            if let Some(ticket) = ticket {
                sink(&ticket.owner, &ticket.key, event);
            }
            continue;
        }
        if !last {
            reply.push(text);
            continue;
        }
        let reply = Reply {
            code,
            lines: std::mem::take(&mut reply),
        };
        // A paragraph queued: its id ties the notes that follow to the window that asked.
        // That happens here, before any of them is read.
        if code == 225 {
            let mut shared = lock(&shared);
            let id = reply
                .lines
                .first()
                .and_then(|id| id.trim().parse::<u64>().ok());
            if let (Some(ticket), Some(id)) = (shared.pending.take(), id) {
                shared.queued.insert(id, ticket);
            }
        }
        if replies.send(reply).is_err() {
            break;
        }
    }
    // What was being said won't end on its own now.
    let lost: Vec<Ticket> = {
        let mut shared = lock(&shared);
        shared.closed = true;
        shared.queued.drain().map(|(_, ticket)| ticket).collect()
    };
    for ticket in lost {
        sink(&ticket.owner, &ticket.key, Note::Failed);
    }
}

/// "NNN-text" (more lines follow) or "NNN text" (the last line).
fn parse_line(line: &str) -> Option<(u16, bool, String)> {
    let code = line.get(..3)?.parse::<u16>().ok()?;
    let last = match line.as_bytes().get(3) {
        Some(b'-') => false,
        Some(b' ') | None => true,
        _ => return None,
    };
    Some((code, last, line.get(4..).unwrap_or("").to_string()))
}

/// "name<TAB>language<TAB>variant" lines. espeak-ng lists each voice again with every
/// variant ("Turkish+Adam"); those copies are left out where the plain voice is there.
fn parse_voices(module: &str, lines: &[String]) -> Vec<Voice> {
    let all: Vec<Voice> = lines
        .iter()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            let lang = fields.next().unwrap_or("").trim();
            (!name.is_empty()).then(|| Voice {
                module: module.to_string(),
                name: name.to_string(),
                lang: lang.to_string(),
            })
        })
        .collect();
    let plain: HashSet<&str> = all
        .iter()
        .filter(|voice| !voice.name.contains('+'))
        .map(|voice| voice.name.as_str())
        .collect();
    all.iter()
        .filter(|voice| match voice.name.split_once('+') {
            Some((base, _)) => !plain.contains(base),
            None => true,
        })
        .take(MAX_VOICES)
        .cloned()
        .collect()
}

/// SSIP ends a message with a line holding a dot, so a line that starts with one gets
/// another.
fn escape(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let line = line.trim_end_matches('\r');
            if line.starts_with('.') {
                format!(".{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The listen bar's speed (1 normal, 2 twice as fast) on speech-dispatcher's scale of
/// -100 to 100, mapped as Chromium maps it (a third of normal is -100, three times is
/// 100), so 0.1.x's speeds sound the same.
fn ssip_rate(rate: f64) -> i32 {
    if !(rate.is_finite() && rate > 0.0) {
        return 0;
    }
    (100.0 * rate.log10() / 3f64.log10())
        .round()
        .clamp(-100.0, 100.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    #[test]
    fn reads_speechd_addresses() {
        let at = |path: &str| {
            Some(Address {
                path: PathBuf::from(path),
                usual: false,
            })
        };
        assert_eq!(
            Address::parse("unix_socket:/run/speech.sock"),
            at("/run/speech.sock")
        );
        assert_eq!(
            Address::parse(" unix_socket:/a b/s.sock "),
            at("/a b/s.sock")
        );
        assert!(Address::parse("unix_socket").is_some_and(|address| address.usual));
        assert_eq!(Address::parse("unix_socket:"), None);
        assert_eq!(Address::parse("inet_socket:127.0.0.1:6560"), None);
    }

    #[test]
    fn maps_speeds_as_chromium_did() {
        let rates: Vec<i32> = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0]
            .into_iter()
            .map(ssip_rate)
            .collect();
        assert_eq!(rates, [-26, 0, 20, 37, 51, 63]);
        assert_eq!(ssip_rate(0.1), -100);
        assert_eq!(ssip_rate(10.0), 100);
        assert_eq!(ssip_rate(f64::NAN), 0);
        assert_eq!(ssip_rate(-1.0), 0);
    }

    #[test]
    fn escapes_lines_that_start_with_a_dot() {
        assert_eq!(escape(".a\nb\r\n.\n..c"), "..a\r\nb\r\n..\r\n...c");
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn reads_reply_lines() {
        assert_eq!(
            parse_line("249-Turkish\ttr\tnone"),
            Some((249, false, "Turkish\ttr\tnone".into()))
        );
        assert_eq!(
            parse_line("225 OK MESSAGE QUEUED"),
            Some((225, true, "OK MESSAGE QUEUED".into()))
        );
        assert_eq!(parse_line("231"), Some((231, true, String::new())));
        assert_eq!(parse_line("2x5 OK"), None);
        assert_eq!(parse_line("225+1"), None);
    }

    #[test]
    fn lists_voices_without_variant_copies() {
        let lines: Vec<String> = [
            "Turkish\ttr\tnone",
            "Turkish+Adam\ttr\tAdam",
            "English (America)\ten-US\tnone",
            "English (America)+Andy\ten-US\tAndy",
            "Solo+Voice\tde\tnone",
            "\tfr\tnone",
        ]
        .iter()
        .map(|line| line.to_string())
        .collect();
        let names: Vec<String> = parse_voices("espeak-ng", &lines)
            .into_iter()
            .map(|voice| format!("{} {} {}", voice.module, voice.name, voice.lang))
            .collect();
        assert_eq!(
            names,
            [
                "espeak-ng Turkish tr",
                "espeak-ng English (America) en-US",
                "espeak-ng Solo+Voice de"
            ]
        );
    }

    /// A stand-in for speech-dispatcher on a socket of its own. It answers what the
    /// browser sends and logs it, says nothing out loud, and sends notes on paragraphs:
    /// one with "(end)" in it ends at once, "(vanish)" takes the server away
    /// mid-paragraph, any other goes on until canceled.
    fn fake_server(path: &Path) -> Receiver<String> {
        fn send(out: &mut UnixStream, text: &str) {
            let _ = out.write_all(text.replace('\n', "\r\n").as_bytes());
        }
        let listener = UnixListener::bind(path).unwrap();
        let (log, logged) = mpsc::channel();
        std::thread::spawn(move || {
            let mut next_id = 1u64;
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut out = stream.try_clone().unwrap();
                let mut lines = BufReader::new(stream).lines();
                let mut module = String::from("espeak-ng");
                let mut active: Vec<u64> = Vec::new();
                while let Some(Ok(line)) = lines.next() {
                    let _ = log.send(line.clone());
                    let reply = if let Some(name) = line.strip_prefix("SET self OUTPUT_MODULE ") {
                        module = name.to_string();
                        "216 OK OUTPUT MODULE SET\n".to_string()
                    } else if let Some(rate) = line.strip_prefix("SET self RATE ") {
                        match rate.parse::<i32>() {
                            Ok(rate) if (-100..=100).contains(&rate) => "203 OK RATE SET\n".into(),
                            _ => "409 ERR RATE TOO HIGH\n".into(),
                        }
                    } else if line.starts_with("SET self ") {
                        "201 OK SET\n".into()
                    } else if line == "GET OUTPUT_MODULE" {
                        "251-espeak-ng\n251 OK GET RETURNED\n".into()
                    } else if line == "GET VOICE_TYPE" {
                        "251-FEMALE1\n251 OK GET RETURNED\n".into()
                    } else if line == "LIST OUTPUT_MODULES" {
                        "250-espeak-ng\n250-dummy\n250-openjtalk\n250 OK MODULE LIST SENT\n".into()
                    } else if line == "LIST SYNTHESIS_VOICES" {
                        match module.as_str() {
                            "espeak-ng" => "249-Turkish\ttr\tnone\n249-Turkish+Adam\ttr\tAdam\n249-English (America)\ten-US\tnone\n249 OK VOICE LIST SENT\n",
                            "openjtalk" => "249-Default\tja\tnone\n249 OK VOICE LIST SENT\n",
                            _ => "411 ERR NO VOICES\n",
                        }
                        .into()
                    } else if line == "CANCEL self" {
                        // The notes may come before the reply.
                        let mut text: String = active
                            .drain(..)
                            .map(|id| format!("703-{id}\n703-1\n703 CANCELED\n"))
                            .collect();
                        text.push_str("213 OK CANCELED\n");
                        text
                    } else if line == "SPEAK" {
                        send(&mut out, "230 OK RECEIVING DATA\n");
                        let mut said = Vec::new();
                        while let Some(Ok(data)) = lines.next() {
                            if data == "." {
                                break;
                            }
                            said.push(data.strip_prefix('.').map(str::to_string).unwrap_or(data));
                        }
                        let said = said.join("\n");
                        let _ = log.send(format!("SAY {said}"));
                        let id = next_id;
                        next_id += 1;
                        let mut text = format!(
                            "225-{id}\n225 OK MESSAGE QUEUED\n701-{id}\n701-1\n701 BEGIN\n"
                        );
                        if said.contains("(end)") {
                            text.push_str(&format!("702-{id}\n702-1\n702 END\n"));
                        } else {
                            active.push(id);
                        }
                        send(&mut out, &text);
                        if said.contains("(vanish)") {
                            let _ = out.shutdown(std::net::Shutdown::Both);
                            break;
                        }
                        continue;
                    } else if line == "QUIT" {
                        send(&mut out, "231 HAPPY HACKING\n");
                        break;
                    } else {
                        "500 ERR INVALID COMMAND\n".into()
                    };
                    send(&mut out, &reply);
                }
            }
        });
        logged
    }

    /// A folder for the test's socket that goes away with the test, even a failed one.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    type Heard = (String, String, Note);

    fn heard(owner: &str, key: &str, note: Note) -> Heard {
        (owner.into(), key.into(), note)
    }

    fn say(owner: &str, key: &str, text: &str, voice: Option<(&str, &str)>, rate: f64) -> Request {
        Request::Speak(Utterance {
            owner: owner.into(),
            key: key.into(),
            text: text.into(),
            lang: "tr".into(),
            voice: voice.map(|(module, name)| (module.into(), name.into())),
            rate,
        })
    }

    fn list_voices(worker: &Sender<Request>) -> Vec<Voice> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        worker.send(Request::Voices(tx)).unwrap();
        rx.blocking_recv().unwrap()
    }

    #[test]
    fn reads_aloud_through_speech_dispatcher() {
        let dir = TempDir(
            std::env::temp_dir().join(format!("peepwick-speech-test-{}", std::process::id())),
        );
        std::fs::create_dir_all(&dir.0).unwrap();
        let path = dir.0.join("speechd.sock");
        let log = fake_server(&path);
        let (tx, notes) = mpsc::channel::<Heard>();
        let sink: Sink = Arc::new(move |owner, key, note| {
            let _ = tx.send(heard(owner, key, note));
        });
        let worker = start(Some(Address { path, usual: false }), sink);
        let next = || notes.recv_timeout(Duration::from_secs(5)).expect("a note");
        let sent = || log.try_iter().collect::<Vec<String>>();

        // Voices of every engine but the dummy one, without the variant copies.
        let voice = |module: &str, name: &str, lang: &str| Voice {
            module: module.into(),
            name: name.into(),
            lang: lang.into(),
        };
        assert_eq!(
            list_voices(&worker),
            [
                voice("espeak-ng", "Turkish", "tr"),
                voice("espeak-ng", "English (America)", "en-US"),
                voice("openjtalk", "Default", "ja")
            ]
        );
        let setup = sent();
        assert_eq!(
            setup[..2],
            [
                "SET self CLIENT_NAME user:peepwick:read_aloud",
                "SET self NOTIFICATION ALL on"
            ]
        );
        assert!(
            !setup
                .iter()
                .any(|line| line == "SET self OUTPUT_MODULE dummy")
        );
        assert_eq!(
            setup.last().map(String::as_str),
            Some("SET self OUTPUT_MODULE espeak-ng")
        );

        // Without a picked voice: the language, and the voice type speech-dispatcher had.
        worker
            .send(say("w1", "1", ".Merhaba (end)", None, 2.0))
            .unwrap();
        assert_eq!(next(), heard("w1", "1", Note::Started));
        assert_eq!(next(), heard("w1", "1", Note::Ended));
        assert_eq!(
            sent(),
            [
                "CANCEL self",
                "SET self OUTPUT_MODULE espeak-ng",
                "SET self LANGUAGE tr",
                "SET self VOICE_TYPE FEMALE1",
                "SET self RATE 63",
                "SPEAK",
                "SAY .Merhaba (end)"
            ]
        );

        // A picked voice speaks in its own language; this paragraph goes on.
        worker
            .send(say(
                "w1",
                "2",
                "Uzun paragraf",
                Some(("openjtalk", "Default")),
                1.0,
            ))
            .unwrap();
        assert_eq!(next(), heard("w1", "2", Note::Started));
        assert_eq!(
            sent()[1..4],
            [
                "SET self LANGUAGE ja",
                "SET self OUTPUT_MODULE openjtalk",
                "SET self SYNTHESIS_VOICE Default"
            ]
        );

        // Another window's stop leaves it; its own stops it.
        worker.send(Request::Stop("w2".into())).unwrap();
        list_voices(&worker);
        assert!(sent().is_empty());
        worker.send(Request::Stop("w1".into())).unwrap();
        assert_eq!(next(), heard("w1", "2", Note::Stopped));
        assert_eq!(sent(), ["CANCEL self"]);

        // A second window starting to read stops the first.
        worker.send(say("w1", "3", "Üçüncü", None, 1.0)).unwrap();
        assert_eq!(next(), heard("w1", "3", Note::Started));
        worker
            .send(say("w2", "a", "Öteki (end)", None, 1.0))
            .unwrap();
        assert_eq!(next(), heard("w1", "3", Note::Stopped));
        assert_eq!(next(), heard("w2", "a", Note::Started));
        assert_eq!(next(), heard("w2", "a", Note::Ended));

        // speech-dispatcher going away mid-paragraph; the next one connects again.
        worker
            .send(say("w1", "4", "Gidiyor (vanish)", None, 1.0))
            .unwrap();
        assert_eq!(next(), heard("w1", "4", Note::Started));
        assert_eq!(next(), heard("w1", "4", Note::Failed));
        worker
            .send(say("w1", "5", "Geri döndü (end)", None, 1.0))
            .unwrap();
        assert_eq!(next(), heard("w1", "5", Note::Started));
        assert_eq!(next(), heard("w1", "5", Note::Ended));

        // Closing the browser stops what is being said.
        worker.send(say("w1", "6", "Son", None, 1.0)).unwrap();
        assert_eq!(next(), heard("w1", "6", Note::Started));
        sent();
        let (done, quit) = mpsc::channel();
        worker.send(Request::Quit(done)).unwrap();
        quit.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(next(), heard("w1", "6", Note::Stopped));
        assert_eq!(sent(), ["CANCEL self", "QUIT"]);
    }

    #[test]
    fn says_when_speech_dispatcher_is_missing() {
        let (tx, notes) = mpsc::channel::<Heard>();
        let sink: Sink = Arc::new(move |owner, key, note| {
            let _ = tx.send(heard(owner, key, note));
        });
        let address = Address {
            path: std::env::temp_dir()
                .join(format!("peepwick-speech-none-{}.sock", std::process::id())),
            usual: false,
        };
        let worker = start(Some(address), sink);
        assert!(list_voices(&worker).is_empty());
        worker.send(say("w1", "1", "Merhaba", None, 1.0)).unwrap();
        assert_eq!(
            notes.recv_timeout(Duration::from_secs(5)).unwrap(),
            heard("w1", "1", Note::Failed)
        );
    }
}
