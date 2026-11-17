//! Password CSV files (what Chrome, Firefox, Bitwarden, 1Password, LastPass and KeePassXC
//! export, and Chrome's columns for our own export), the origin a login belongs to, how weak a
//! password is, and new passwords for sign-up forms. Pure functions, with the same rules as
//! src/shared/passwordcsv.ts, which the interface uses.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq)]
pub struct CsvLogin {
    pub url: String,
    pub username: String,
    pub password: String,
    pub note: String,
}

/// RFC 4180: quoted fields, doubled quotes, newlines inside quotes, a BOM at the start.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .chars()
        .peekable();
    let end_row = |row: &mut Vec<String>, rows: &mut Vec<Vec<String>>| {
        if row.iter().any(|cell| !cell.is_empty()) {
            rows.push(std::mem::take(row));
        } else {
            row.clear();
        }
    };
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' && chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else if c == '"' {
                quoted = false;
            } else {
                field.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            end_row(&mut row, &mut rows);
        } else {
            field.push(c);
        }
    }
    row.push(field);
    end_row(&mut row, &mut rows);
    rows
}

const URL_COLUMNS: [&str; 9] = [
    "url",
    "login_uri",
    "website",
    "web site",
    "login url",
    "uri",
    "origin",
    "hostname",
    "address",
];
const USERNAME_COLUMNS: [&str; 8] = [
    "username",
    "login_username",
    "user name",
    "user",
    "login",
    "email",
    "e-mail",
    "login name",
];
const PASSWORD_COLUMNS: [&str; 3] = ["password", "login_password", "pass"];
const NOTE_COLUMNS: [&str; 5] = ["note", "notes", "extra", "comment", "comments"];

fn column_of(header: &[String], names: &[&str]) -> Option<usize> {
    let normalized: Vec<String> = header.iter().map(|h| h.trim().to_lowercase()).collect();
    names
        .iter()
        .find_map(|name| normalized.iter().position(|h| h == name))
}

/// Reads logins from a CSV export; rows without an address or password are left out and
/// counted.
pub fn logins_from_csv(text: &str) -> (Vec<CsvLogin>, usize) {
    let mut rows = parse_csv(text).into_iter();
    let Some(header) = rows.next() else {
        return (Vec::new(), 0);
    };
    let rows: Vec<Vec<String>> = rows.collect();
    let (Some(url), Some(password)) = (
        column_of(&header, &URL_COLUMNS),
        column_of(&header, &PASSWORD_COLUMNS),
    ) else {
        return (Vec::new(), rows.len());
    };
    let username = column_of(&header, &USERNAME_COLUMNS);
    let note = column_of(&header, &NOTE_COLUMNS);
    let mut logins = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let cell = |at: Option<usize>| {
            at.and_then(|i| row.get(i))
                .map(|c| c.trim().to_string())
                .unwrap_or_default()
        };
        let login = CsvLogin {
            url: cell(Some(url)),
            username: cell(username),
            password: row.get(password).cloned().unwrap_or_default(),
            note: cell(note),
        };
        if login.url.is_empty() || login.password.is_empty() || origin_of(&login.url).is_none() {
            skipped += 1;
        } else {
            logins.push(login);
        }
    }
    (logins, skipped)
}

static HAS_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*://").expect("scheme rule"));

/// https://example.com for a login address; a bare host gets https://.
pub fn origin_of(url: &str) -> Option<String> {
    let text = url.trim();
    if text.is_empty() {
        return None;
    }
    let text = if HAS_SCHEME.is_match(text) {
        text.to_string()
    } else {
        format!("https://{text}")
    };
    let parsed = url::Url::parse(&text).ok()?;
    if !matches!(parsed.scheme(), "https" | "http") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

fn quote(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// One login for an export.
pub struct CsvRow<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub note: &'a str,
}

/// Chrome's columns, which every password manager reads.
pub fn logins_to_csv(logins: &[CsvRow]) -> String {
    let mut out = String::from("name,url,username,password,note\n");
    for l in logins {
        let cells = [l.name, l.url, l.username, l.password, l.note].map(quote);
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

const COMMON: [&str; 37] = [
    "123456",
    "123456789",
    "12345678",
    "password",
    "qwerty",
    "12345",
    "1234567",
    "111111",
    "123123",
    "abc123",
    "password1",
    "1234567890",
    "000000",
    "iloveyou",
    "qwerty123",
    "1q2w3e4r",
    "admin",
    "welcome",
    "letmein",
    "monkey",
    "dragon",
    "sunshine",
    "football",
    "baseball",
    "superman",
    "trustno1",
    "parola",
    "sifre",
    "şifre",
    "galatasaray",
    "fenerbahce",
    "besiktas",
    "asdasd",
    "qweasd",
    "123qwe",
    "zxcvbnm",
    "passw0rd",
];

/// Lower case the Turkish way, as `toLocaleLowerCase('tr')` does: I is ı, İ is i.
fn lower_tr(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            'I' => "ı".chars().collect::<Vec<_>>(),
            'İ' => vec!['i'],
            c => c.to_lowercase().collect(),
        })
        .collect()
}

static OTHER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}0-9]").expect("symbol rule"));

/// Short, common, or a single kind of character and not long enough to make up for it.
pub fn is_weak_password(password: &str) -> bool {
    // Counted in UTF-16 units, as the interface counts.
    let length = password.encode_utf16().count();
    if length < 8 || COMMON.contains(&lower_tr(password).as_str()) {
        return true;
    }
    let lower = password
        .chars()
        .any(|c| c.is_ascii_lowercase() || "çğıöşü".contains(c));
    let upper = password
        .chars()
        .any(|c| c.is_ascii_uppercase() || "ÇĞİÖŞÜ".contains(c));
    let digit = password.chars().any(|c| c.is_ascii_digit());
    let kinds = [lower, upper, digit, OTHER.is_match(password)]
        .iter()
        .filter(|k| **k)
        .count();
    if kinds == 1 && length < 14 {
        return true;
    }
    let mut chars = password.chars();
    let first = chars.next();
    first.is_some() && chars.clone().next().is_some() && chars.all(|c| Some(c) == first)
}

/// A strong random password for sign-up forms: 16 characters from four kinds, no look-alikes.
/// None if the system's random generator fails.
pub fn generate_password() -> Option<String> {
    generate_with(|buffer| openssl::rand::rand_bytes(buffer).is_ok())
}

/// `generate_password` with the random bytes from `random`, which says whether it gave any.
fn generate_with(mut random: impl FnMut(&mut [u8]) -> bool) -> Option<String> {
    const SETS: [&str; 4] = [
        "abcdefghjkmnpqrstuvwxyz",
        "ABCDEFGHJKLMNPQRSTUVWXYZ",
        "23456789",
        "-_.!?#",
    ];
    const LENGTH: usize = 16;
    let all: Vec<char> = SETS.concat().chars().collect();
    // Bytes above the last whole multiple of the alphabet are skipped, so every character is
    // equally likely.
    let limit = 256 - (256 % all.len());
    loop {
        let mut out = Vec::with_capacity(LENGTH);
        while out.len() < LENGTH {
            let mut bytes = [0u8; LENGTH];
            if !random(&mut bytes) {
                return None;
            }
            for b in bytes.map(usize::from) {
                if b < limit && out.len() < LENGTH {
                    out.push(all[b % all.len()]);
                }
            }
        }
        if SETS.iter().all(|set| out.iter().any(|c| set.contains(*c))) {
            return Some(out.into_iter().collect());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_commas_newlines_and_a_bom() {
        assert_eq!(
            parse_csv("\u{feff}a,\"b,c\",\"d \"\"e\"\"\"\r\n1,\"2\n3\",4\n"),
            vec![vec!["a", "b,c", "d \"e\""], vec!["1", "2\n3", "4"]]
        );
    }

    #[test]
    fn reads_chrome_firefox_and_bitwarden_exports() {
        let (chrome, _) = logins_from_csv(
            "name,url,username,password,note\nGitHub,https://github.com/login,ada,pw1,\n",
        );
        assert_eq!(
            chrome,
            vec![CsvLogin {
                url: "https://github.com/login".into(),
                username: "ada".into(),
                password: "pw1".into(),
                note: String::new(),
            }]
        );
        let (firefox, _) = logins_from_csv(
            "\"url\",\"username\",\"password\",\"httpRealm\"\n\"https://example.com\",\"bob\",\"pw2\",\"\"\n",
        );
        assert_eq!(
            (
                firefox[0].url.as_str(),
                firefox[0].username.as_str(),
                firefox[0].password.as_str()
            ),
            ("https://example.com", "bob", "pw2")
        );
        let (bitwarden, skipped) = logins_from_csv(
            "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,X,n,,0,example.org,cem,pw3,\n,,note,Y,,,0,,,,\n",
        );
        assert_eq!(
            bitwarden,
            vec![CsvLogin {
                url: "example.org".into(),
                username: "cem".into(),
                password: "pw3".into(),
                note: "n".into(),
            }]
        );
        assert_eq!(skipped, 1);
    }

    #[test]
    fn writes_what_it_reads() {
        let csv = logins_to_csv(&[CsvRow {
            name: "a, \"b\"",
            url: "https://a.example",
            username: "u",
            password: "p,\"q\"",
            note: "line1\nline2",
        }]);
        assert_eq!(
            logins_from_csv(&csv).0[0],
            CsvLogin {
                url: "https://a.example".into(),
                username: "u".into(),
                password: "p,\"q\"".into(),
                note: "line1\nline2".into(),
            }
        );
    }

    #[test]
    fn turns_addresses_into_origins() {
        assert_eq!(
            origin_of("github.com").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin_of("http://localhost:8080/login").as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(
            origin_of("HTTPS://Example.COM:443/x").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(origin_of("ftp://x"), None);
        assert_eq!(origin_of("android://abc@com.app/"), None);
        assert_eq!(origin_of("  "), None);
    }

    #[test]
    fn flags_short_common_and_one_kind_passwords() {
        assert!(is_weak_password("123456"));
        assert!(is_weak_password("galatasaray"));
        assert!(is_weak_password("GALATASARAY"));
        assert!(is_weak_password("abcdefghij"));
        assert!(is_weak_password("zzzzzzzzzzzzzzzz"));
        assert!(!is_weak_password("Kq7#vT9!mZ2x"));
        assert!(!is_weak_password("correcthorsebatterystaple"));
    }

    #[test]
    fn generates_sixteen_characters_of_every_kind() {
        assert_eq!(generate_with(|_| false), None);
        for _ in 0..50 {
            let password = generate_password().expect("random bytes");
            assert_eq!(password.chars().count(), 16);
            assert!(!is_weak_password(&password));
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(!password.contains(['l', 'I', 'O', '0', '1', 'o', 'i']));
        }
    }
}
