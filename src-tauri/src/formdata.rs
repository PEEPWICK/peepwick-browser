//! The rest of autofill's data: saved addresses, and what was typed into form fields before so
//! a field can suggest it again (autofill.rs). Card numbers are never kept, typed or imported.
//!
//! As in 0.1.x it is encrypted, here with the passwords' key (passwords.rs). formdata.json keeps
//! the addresses and the entries as two sealed values, so clearing the entries (the dialog's
//! "all time", or every time the browser closes) never needs the key and never touches the
//! addresses. The file is opened once the key is known, the first time something needs it; with
//! no file, nothing asks the keyring.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gtk::glib;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::fold;
use crate::settings::Lang;
use crate::tab::now_ms;
use crate::{passwords, paths, store};

const FILE: &str = "formdata.json";
const MAX_ENTRIES: usize = 5000;
/// The longest address field, and the longest value remembered for a form field.
const MAX_FIELD: usize = 300;
const MAX_VALUE: usize = 200;
const MAX_KEY: usize = 100;

/// An address's fields, by the names the UI and the page script use.
pub const ADDRESS_FIELDS: [&str; 9] = [
    "name",
    "organization",
    "street",
    "city",
    "state",
    "postalCode",
    "country",
    "phone",
    "email",
];

/// A saved address, as `Address` in the UI.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Address {
    pub id: String,
    pub name: String,
    pub organization: String,
    pub street: String,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
    pub phone: String,
    pub email: String,
}

impl Address {
    /// A field by its name in `ADDRESS_FIELDS`.
    pub fn field(&self, name: &str) -> &str {
        match name {
            "name" => &self.name,
            "organization" => &self.organization,
            "street" => &self.street,
            "city" => &self.city,
            "state" => &self.state,
            "postalCode" => &self.postal_code,
            "country" => &self.country,
            "phone" => &self.phone,
            "email" => &self.email,
            _ => "",
        }
    }

    fn field_mut(&mut self, name: &str) -> Option<&mut String> {
        Some(match name {
            "name" => &mut self.name,
            "organization" => &mut self.organization,
            "street" => &mut self.street,
            "city" => &mut self.city,
            "state" => &mut self.state,
            "postalCode" => &mut self.postal_code,
            "country" => &mut self.country,
            "phone" => &mut self.phone,
            "email" => &mut self.email,
            _ => return None,
        })
    }

    /// From the UI, a backup or another browser: the known fields, trimmed and cut short.
    pub fn from_value(input: &Value) -> Address {
        let mut address = Address::default();
        for name in ADDRESS_FIELDS {
            if let (Some(slot), Some(value)) = (
                address.field_mut(name),
                input.get(name).and_then(Value::as_str),
            ) {
                *slot = value.trim().chars().take(MAX_FIELD).collect();
            }
        }
        address
    }

    fn is_empty(&self) -> bool {
        ADDRESS_FIELDS
            .iter()
            .all(|name| self.field(name).is_empty())
    }

    fn same_as(&self, other: &Address) -> bool {
        ADDRESS_FIELDS
            .iter()
            .all(|name| self.field(name) == other.field(name))
    }
}

/// Something typed into a form field before: the field's key (its autocomplete token, name or
/// id), the value, how often and when it was last sent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEntry {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub last_used: u64,
}

impl FormEntry {
    /// From a backup: well-formed entries only, with their counts and dates.
    pub fn from_value(input: &Value) -> Option<FormEntry> {
        let number = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map_or(0, |n| n as u64)
        };
        Some(FormEntry {
            key: input.get("key")?.as_str()?.to_string(),
            value: input.get("value")?.as_str()?.to_string(),
            count: number("count"),
            last_used: number("lastUsed"),
        })
    }
}

/// A card number: 13 to 19 digits passing the Luhn check. Never kept.
pub fn looks_like_card(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .map(|c| c.to_digit(10))
        .collect::<Option<_>>()
        .unwrap_or_default();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| match (i % 2, d * 2) {
            (1, doubled) if doubled > 9 => doubled - 9,
            (1, doubled) => doubled,
            _ => d,
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Values worth remembering for a field: short, not a card number, not a long token.
pub fn rememberable(value: &str) -> bool {
    let value = value.trim();
    let token = value.len() >= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    !value.is_empty() && value.chars().count() <= MAX_VALUE && !looks_like_card(value) && !token
}

/// What the file holds once open.
#[derive(Default)]
struct Data {
    addresses: Vec<Address>,
    entries: Vec<FormEntry>,
}

impl Data {
    /// Values typed into fields with this key before, that go on from what's typed now: the
    /// most used first. Letters match in either case, the Turkish i's too.
    fn suggest(&self, key: &str, typed: &str, lang: Lang, limit: usize) -> Vec<String> {
        let needle = fold(typed.trim(), lang);
        let mut found: Vec<&FormEntry> = self
            .entries
            .iter()
            .filter(|e| {
                e.key == key && e.value != typed && fold(&e.value, lang).starts_with(&needle)
            })
            .collect();
        found.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_used.cmp(&a.last_used)));
        found
            .into_iter()
            .take(limit)
            .map(|e| e.value.clone())
            .collect()
    }

    /// Values sent with a form. Returns how many were new.
    fn remember(&mut self, sent: &[(String, String)], now: u64) -> usize {
        let mut added = 0;
        for (key, value) in sent {
            if key.is_empty() || !rememberable(value) {
                continue;
            }
            let value = value.trim();
            match self
                .entries
                .iter_mut()
                .find(|e| e.key == *key && e.value == value)
            {
                Some(entry) => {
                    entry.count += 1;
                    entry.last_used = entry.last_used.max(now);
                }
                None => {
                    self.entries.push(FormEntry {
                        key: key.clone(),
                        value: value.to_string(),
                        count: 1,
                        last_used: now,
                    });
                    added += 1;
                }
            }
        }
        self.trim_entries();
        added
    }

    /// Entries from another browser or a backup keep their counts and dates. Returns how many
    /// were new.
    fn import_entries(&mut self, incoming: Vec<FormEntry>, now: u64) -> usize {
        let mut added = 0;
        for entry in incoming {
            let key: String = entry
                .key
                .trim()
                .to_lowercase()
                .chars()
                .take(MAX_KEY)
                .collect();
            if key.is_empty() || !rememberable(&entry.value) {
                continue;
            }
            let value = entry.value.trim().to_string();
            if let Some(existing) = self
                .entries
                .iter_mut()
                .find(|e| e.key == key && e.value == value)
            {
                existing.count = existing.count.max(entry.count);
                existing.last_used = existing.last_used.max(entry.last_used);
                continue;
            }
            self.entries.push(FormEntry {
                key,
                value,
                count: entry.count.max(1),
                last_used: if entry.last_used > 0 {
                    entry.last_used
                } else {
                    now
                },
            });
            added += 1;
        }
        self.trim_entries();
        added
    }

    /// At most `MAX_ENTRIES`: the ones used longest ago go.
    fn trim_entries(&mut self) {
        if self.entries.len() > MAX_ENTRIES {
            self.entries.sort_by_key(|e| std::cmp::Reverse(e.last_used));
            self.entries.truncate(MAX_ENTRIES);
        }
    }

    /// Addresses from another browser or a backup; one equal to a saved one field for field is
    /// left out. Returns how many were new.
    fn import_addresses(&mut self, incoming: Vec<Address>) -> usize {
        let mut added = 0;
        for mut address in incoming {
            address.id = String::new();
            let address = Address {
                id: crate::bookmarks::new_id(),
                ..Address::from_value(&serde_json::to_value(&address).unwrap_or_default())
            };
            if address.is_empty() || self.addresses.iter().any(|a| a.same_as(&address)) {
                continue;
            }
            self.addresses.push(address);
            added += 1;
        }
        added
    }
}

/// formdata.json: the two parts, each sealed with the passwords' key. A part that is missing
/// is empty.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Sealed {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    addresses: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entries: Option<String>,
}

type Waiting = Box<dyn FnOnce()>;

thread_local! {
    static DATA: RefCell<Data> = RefCell::new(Data::default());
    /// The file as read: what couldn't be opened yet stays as it was.
    static SEALED: RefCell<Sealed> = RefCell::new(Sealed::default());
    static READ: Cell<bool> = const { Cell::new(false) };
    /// DATA holds what the file holds.
    static OPEN: Cell<bool> = const { Cell::new(false) };
    static SAVE_PENDING: Cell<bool> = const { Cell::new(false) };
    /// A save is waiting for the key to be asked for.
    static SAVE_ASKED: Cell<bool> = const { Cell::new(false) };
    /// What waits for the key, by number: the keyring is asked off the main thread.
    static WAITING: RefCell<HashMap<u64, Waiting>> = RefCell::new(HashMap::new());
    static NEXT_WAITING: Cell<u64> = const { Cell::new(1) };
}

fn file() -> PathBuf {
    paths::user_path(FILE)
}

fn read_sealed() {
    if !READ.replace(true)
        && let Some(sealed) = store::read_json::<Sealed>(&file())
    {
        SEALED.replace(sealed);
    }
}

/// Whether opening the file needs the key, which hasn't opened it yet.
pub fn needs_key() -> bool {
    read_sealed();
    !OPEN.get()
        && SEALED.with(|s| {
            let s = s.borrow();
            s.addresses.is_some() || s.entries.is_some()
        })
}

/// Opens the file with the key as it stands; true once its contents are here. With no file
/// there's nothing to open and no key is needed.
pub fn ensure_open() -> bool {
    if OPEN.get() {
        return true;
    }
    read_sealed();
    let sealed = SEALED.with(|s| s.borrow().clone());
    // Known to be there or not: what it can't open then is lost (the key changed).
    let settled = passwords::key_settled();
    let open = |part: Option<String>| -> Result<Option<String>, ()> {
        match part {
            None => Ok(None),
            Some(text) => match passwords::open_text(&text) {
                Some(plain) => Ok(Some(plain)),
                None if settled => {
                    crate::log_error!("[formdata] saved data the key doesn't open");
                    Ok(None)
                }
                None => Err(()),
            },
        }
    };
    let (Ok(addresses), Ok(entries)) = (open(sealed.addresses), open(sealed.entries)) else {
        return false;
    };
    DATA.replace(Data {
        addresses: addresses
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        entries: entries
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
    });
    OPEN.set(true);
    true
}

/// Runs `then` on the main thread once the file is open: at once when it is (or there is
/// none), or after the key has been read, or made with `create`, for saving.
pub fn when_open(create: bool, then: impl FnOnce() + 'static) {
    if (!create || passwords::can_seal()) && ensure_open() {
        return then();
    }
    let id = NEXT_WAITING.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    WAITING.with(|waiting| waiting.borrow_mut().insert(id, Box::new(then)));
    passwords::when_ready(create, move || {
        ensure_open();
        if let Some(then) = WAITING.with(|waiting| waiting.borrow_mut().remove(&id)) {
            then();
        }
    });
}

fn save() {
    // Never over what couldn't be opened.
    if !OPEN.get() {
        return;
    }
    // Opened without asking for the key (what the fixed key sealed, on a desktop with no
    // keyring): ask once now, then save.
    if !passwords::can_seal() {
        if !SAVE_ASKED.replace(true) {
            passwords::when_ready(true, || {
                SAVE_ASKED.set(false);
                if passwords::can_seal() {
                    save();
                } else {
                    crate::log_error!("[formdata] no key to save with");
                }
            });
        }
        return;
    }
    let (addresses, entries) = DATA.with(|data| {
        let data = data.borrow();
        let part =
            |json: Result<String, serde_json::Error>, empty: bool| -> Option<Option<String>> {
                if empty {
                    return Some(None);
                }
                passwords::seal_text(&json.ok()?).map(Some)
            };
        (
            part(
                serde_json::to_string(&data.addresses),
                data.addresses.is_empty(),
            ),
            part(
                serde_json::to_string(&data.entries),
                data.entries.is_empty(),
            ),
        )
    });
    let (Some(addresses), Some(entries)) = (addresses, entries) else {
        crate::log_error!("[formdata] no key to save with");
        return;
    };
    write(Sealed {
        version: 1,
        addresses,
        entries,
    });
}

fn write(sealed: Sealed) {
    let written = serde_json::to_vec(&sealed)
        .map_err(std::io::Error::other)
        .and_then(|json| passwords::write_private(&file(), &json));
    if let Err(error) = written {
        crate::log_error!("[formdata] could not save: {error}");
    }
    SEALED.replace(sealed);
}

/// Saved a moment after a change, so a burst of changes is one write.
fn changed() {
    if !SAVE_PENDING.replace(true) {
        glib::timeout_add_local_once(Duration::from_millis(1000), flush);
    }
}

/// Writes a change that is still waiting, before the browser quits.
pub fn flush() {
    if SAVE_PENDING.replace(false) {
        save();
    }
}

// ── What autofill and the Settings page use (with the file open) ────────────────

pub fn addresses() -> Vec<Address> {
    DATA.with(|data| data.borrow().addresses.clone())
}

pub fn suggest(key: &str, typed: &str) -> Vec<String> {
    let lang = crate::settings::get().language;
    DATA.with(|data| data.borrow().suggest(key, typed, lang, 6))
}

/// Remembers what a sent form held. The file is open and the key known (`when_open(true)`).
pub fn remember(sent: &[(String, String)]) {
    if !ensure_open() {
        return;
    }
    let now = now_ms();
    DATA.with(|data| data.borrow_mut().remember(sent, now));
    changed();
}

/// A new address, or changes to one; returns its id, or None when it would be empty.
pub fn save_address(input: &Value) -> Option<String> {
    if !ensure_open() {
        return None;
    }
    let mut address = Address::from_value(input);
    if address.is_empty() {
        return None;
    }
    let id = input.get("id").and_then(Value::as_str).unwrap_or("");
    let id = DATA.with(|data| {
        let mut data = data.borrow_mut();
        match data
            .addresses
            .iter_mut()
            .find(|a| !id.is_empty() && a.id == id)
        {
            Some(existing) => {
                address.id = existing.id.clone();
                *existing = address;
                existing.id.clone()
            }
            None => {
                address.id = crate::bookmarks::new_id();
                let id = address.id.clone();
                data.addresses.push(address);
                id
            }
        }
    });
    changed();
    Some(id)
}

pub fn remove_address(id: &str) {
    let removed = DATA.with(|data| {
        let mut data = data.borrow_mut();
        let before = data.addresses.len();
        data.addresses.retain(|a| a.id != id);
        data.addresses.len() != before
    });
    if removed {
        changed();
    }
}

/// How many entries were used since `since` (ms); 0 while the file is closed.
pub fn count_entries(since: u64) -> usize {
    if !ensure_open() {
        return 0;
    }
    DATA.with(|data| {
        data.borrow()
            .entries
            .iter()
            .filter(|e| e.last_used >= since)
            .count()
    })
}

/// Forgets the entries used since `since` (ms; 0 for all of them), and writes at once: the
/// browser may be closing. All of them go without asking for the key, the addresses staying
/// sealed as they were; a range needs the key.
pub fn clear_entries(since: u64) {
    if since > 0 {
        return when_open(true, move || {
            if ensure_open() {
                DATA.with(|data| data.borrow_mut().entries.retain(|e| e.last_used < since));
                SAVE_PENDING.set(false);
                save();
            }
        });
    }
    DATA.with(|data| data.borrow_mut().entries.clear());
    if OPEN.get() && passwords::can_seal() {
        SAVE_PENDING.set(false);
        return save();
    }
    read_sealed();
    let mut sealed = SEALED.with(|s| s.borrow().clone());
    if sealed.entries.take().is_some() {
        write(sealed);
    }
}

pub fn entries() -> Vec<FormEntry> {
    DATA.with(|data| data.borrow().entries.clone())
}

/// From another browser or a backup, with the file open and the key known. (addresses, entries)
pub fn import(addresses: Vec<Address>, entries: Vec<FormEntry>) -> (usize, usize) {
    if !ensure_open() {
        return (0, 0);
    }
    let now = now_ms();
    let added = DATA.with(|data| {
        let mut data = data.borrow_mut();
        (
            data.import_addresses(addresses),
            data.import_entries(entries, now),
        )
    });
    if added.0 + added.1 > 0 {
        changed();
    }
    added
}

/// The Settings page's calls, and whether each needs the key made first (for saving).
pub fn invoke_needs_key(method: &str) -> Option<bool> {
    match method {
        "addressesList" | "formEntryCount" => Some(false),
        "addressSave" => Some(true),
        _ => None,
    }
}

/// Answers them on the main thread, once `passwords::ready` has run.
pub fn invoke(method: &str, args: &[Value]) -> Value {
    if !ensure_open() {
        return match method {
            "addressesList" => Value::Array(Vec::new()),
            "formEntryCount" => Value::from(0),
            _ => Value::Null,
        };
    }
    match method {
        "addressesList" => serde_json::to_value(addresses()).unwrap_or_default(),
        "formEntryCount" => Value::from(count_entries(0)),
        "addressSave" => {
            save_address(args.first().unwrap_or(&Value::Null)).map_or(Value::Null, Value::from)
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn knows_card_numbers() {
        assert!(looks_like_card("4111 1111 1111 1111"));
        assert!(looks_like_card("5500-0000-0000-0004"));
        assert!(!looks_like_card("4111 1111 1111 1112"));
        assert!(!looks_like_card("0532 123 45 67"));
        assert!(!looks_like_card("12345"));
    }

    #[test]
    fn remembers_only_short_ordinary_values() {
        assert!(rememberable("Ada Lovelace"));
        assert!(!rememberable("   "));
        assert!(!rememberable("4111111111111111"));
        assert!(!rememberable("a8F3kQ9zL2mX7pR4tY6wV1bN5cJ0hG8d"));
        assert!(!rememberable(&"x".repeat(201)));
    }

    #[test]
    fn suggests_the_most_used_values_that_go_on_from_what_is_typed() {
        let mut data = Data::default();
        let sent = |key: &str, value: &str| vec![(key.to_string(), value.to_string())];
        data.remember(&sent("city", "İstanbul"), 1);
        data.remember(&sent("city", "İzmir"), 2);
        data.remember(&sent("city", "İzmir"), 3);
        data.remember(&sent("city", "Ankara"), 4);
        data.remember(&sent("email", "ada@example.com"), 5);
        data.remember(&sent("cc-number", "4111 1111 1111 1111"), 6);
        assert_eq!(data.entries.len(), 4);
        let tr = Lang::Tr;
        assert_eq!(
            data.suggest("city", "", tr, 6),
            ["İzmir", "Ankara", "İstanbul"]
        );
        assert_eq!(data.suggest("city", "iz", tr, 6), ["İzmir"]);
        assert_eq!(data.suggest("city", "İs", tr, 6), ["İstanbul"]);
        assert_eq!(data.suggest("city", "İzmir", tr, 6), Vec::<String>::new());
        assert_eq!(
            data.suggest("email", "ADA", Lang::En, 6),
            ["ada@example.com"]
        );
    }

    #[test]
    fn imports_keep_counts_and_skip_what_is_here() {
        let mut data = Data::default();
        data.remember(&[("email".into(), "ada@example.com".into())], 10);
        let incoming = vec![
            FormEntry::from_value(
                &json!({ "key": "Email", "value": "ada@example.com", "count": 7, "lastUsed": 20 }),
            )
            .unwrap(),
            FormEntry::from_value(
                &json!({ "key": "city", "value": "İzmir", "count": 2, "lastUsed": 0 }),
            )
            .unwrap(),
            FormEntry::from_value(&json!({ "key": "card", "value": "4111111111111111" })).unwrap(),
        ];
        assert!(FormEntry::from_value(&json!({ "key": 1, "value": "x" })).is_none());
        assert_eq!(data.import_entries(incoming, 99), 1);
        let email = data.entries.iter().find(|e| e.key == "email").unwrap();
        assert_eq!((email.count, email.last_used), (7, 20));
        let city = data.entries.iter().find(|e| e.key == "city").unwrap();
        assert_eq!((city.count, city.last_used), (2, 99));
    }

    #[test]
    fn imports_addresses_once() {
        let mut data = Data::default();
        let ada = Address::from_value(
            &json!({ "name": " Ada Lovelace ", "city": "London", "extra": "x" }),
        );
        assert_eq!(ada.name, "Ada Lovelace");
        assert_eq!(
            data.import_addresses(vec![ada.clone(), ada.clone(), Address::default()]),
            1
        );
        assert!(!data.addresses[0].id.is_empty());
        assert_eq!(data.import_addresses(vec![ada]), 0);
    }

    #[test]
    fn keeps_at_most_five_thousand_entries() {
        let mut data = Data::default();
        let sent: Vec<(String, String)> = (0..MAX_ENTRIES + 10)
            .map(|i| ("k".to_string(), format!("value {i}")))
            .collect();
        data.remember(&sent, 1);
        assert_eq!(data.entries.len(), MAX_ENTRIES);
    }
}
