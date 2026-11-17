//! Other browsers' saved passwords, for importing them (importers.rs).
//!
//! Chromium on Linux ("OSCrypt"): values start with "v10" (a key from the fixed password
//! "peanuts", when there was no keyring) or "v11" (a key from a secret the browser keeps in
//! the desktop's keyring); both are AES-128-CBC with a key from PBKDF2-SHA1 and an IV of
//! sixteen spaces.
//!
//! Firefox: logins.json holds usernames and passwords encrypted with a key that key4.db keeps,
//! itself encrypted under the profile's primary password (empty unless the person set one).
//! key4.db entries use PBES2 (PBKDF2-SHA256 + AES-256-CBC) in current Firefox, or NSS's older
//! SHA-1 + 3DES scheme; logins themselves are 3DES or AES-256-CBC. Everything is DER: a small
//! reader below, careful with lengths, since the files are only as sound as the disk they're on.

use std::collections::HashMap;

use openssl::hash::MessageDigest;
use openssl::pkcs5::pbkdf2_hmac;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use openssl::symm::{Cipher, Crypter, Mode, decrypt};

use crate::secrets;

// ── Chromium ──────────────────────────────────────────────────────────────────

const CHROMIUM_IV: [u8; 16] = [b' '; 16];

pub fn chromium_key(secret: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    // PBKDF2 with these parameters can't fail.
    let _ = pbkdf2_hmac(secret, b"saltysalt", 1, MessageDigest::sha1(), &mut key);
    key
}

pub struct ChromiumKeys {
    v10: [u8; 16],
    /// Keys to try for v11 values: the keyring's secret, then the empty one Chromium falls back
    /// to.
    v11: Vec<[u8; 16]>,
}

/// The keyring "application" each browser keeps its secret under.
const KEYRING_APPS: [(&str, &str); 8] = [
    ("Google Chrome", "chrome"),
    ("Google Chrome Beta", "chrome"),
    ("Google Chrome Dev", "chrome"),
    ("Chromium", "chromium"),
    ("Brave", "brave"),
    ("Microsoft Edge", "microsoft-edge"),
    ("Vivaldi", "vivaldi"),
    ("Opera", "opera"),
];

/// A browser's keys; reads its keyring secret, which the desktop may ask to unlock. Blocking.
pub fn chromium_keys(browser: &str) -> ChromiumKeys {
    let mut v11 = Vec::new();
    if let Some((_, app)) = KEYRING_APPS.iter().find(|(name, _)| *name == browser) {
        match secrets::lookup(&[("application", app)]) {
            Ok(Some(secret)) => v11.push(chromium_key(&secret)),
            Ok(None) => {}
            Err(error) => eprintln!("[import] {browser}'s keyring secret: {error}"),
        }
    }
    v11.push(chromium_key(b""));
    ChromiumKeys {
        v10: chromium_key(b"peanuts"),
        v11,
    }
}

/// Decrypts a password_value; None when no key fits.
pub fn decrypt_chromium(value: &[u8], keys: &ChromiumKeys) -> Option<Vec<u8>> {
    if value.is_empty() {
        return Some(Vec::new());
    }
    let (prefix, body) = value.split_at(value.len().min(3));
    let with = |key: &[u8; 16]| decrypt(Cipher::aes_128_cbc(), key, Some(&CHROMIUM_IV), body).ok();
    match prefix {
        b"v10" => with(&keys.v10),
        b"v11" => keys.v11.iter().find_map(with),
        // Very old profiles kept values in the clear.
        _ => Some(value.to_vec()),
    }
}

// ── DER ───────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Der<'a> {
    tag: u8,
    value: &'a [u8],
    children: Vec<Der<'a>>,
}

/// Every element in `data`, with the children of constructed ones. None if a length runs past
/// the end, or the nesting goes deeper than any real file's.
fn parse_der(data: &[u8]) -> Option<Vec<Der<'_>>> {
    fn parse(data: &[u8], depth: usize) -> Option<Vec<Der<'_>>> {
        if depth > 16 {
            return None;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let tag = data[i];
            let mut length = usize::from(*data.get(i + 1)?);
            i += 2;
            if length & 0x80 != 0 {
                let bytes = length & 0x7f;
                if bytes == 0 || bytes > 4 {
                    return None;
                }
                length = 0;
                for _ in 0..bytes {
                    length = length.checked_mul(256)? + usize::from(*data.get(i)?);
                    i += 1;
                }
            }
            let value = data.get(i..i.checked_add(length)?)?;
            // Constructed types (SEQUENCE, SET, context-specific constructed) hold children.
            let children = if tag & 0x20 != 0 {
                parse(value, depth + 1)?
            } else {
                Vec::new()
            };
            out.push(Der {
                tag,
                value,
                children,
            });
            i += length;
        }
        Some(out)
    }
    parse(data, 0)
}

fn oid(node: &Der) -> String {
    let Some((&first, rest)) = node.value.split_first() else {
        return String::new();
    };
    let mut parts = vec![u64::from(first / 40), u64::from(first % 40)];
    let mut n: u64 = 0;
    for &b in rest {
        n = n.saturating_mul(128) + u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            parts.push(n);
            n = 0;
        }
    }
    parts
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn integer(node: &Der) -> Option<u32> {
    if node.tag != 0x02 || node.value.len() > 5 {
        return None;
    }
    node.value
        .iter()
        .try_fold(0u64, |n, b| Some(n * 256 + u64::from(*b)))
        .and_then(|n| u32::try_from(n).ok())
}

const PBES2: &str = "1.2.840.113549.1.5.13";
const SHA1_TRIPLE_DES: &str = "1.2.840.113549.1.12.5.1.3";
const AES256_CBC: &str = "2.16.840.1.101.3.4.1.42";
const TRIPLE_DES_CBC: &str = "1.2.840.113549.3.7";

// ── Firefox ───────────────────────────────────────────────────────────────────

fn unpad(mut data: Vec<u8>) -> Vec<u8> {
    let pad = usize::from(data.last().copied().unwrap_or(0));
    if (1..=16).contains(&pad) && pad <= data.len() {
        data.truncate(data.len() - pad);
    }
    data
}

/// CBC without OpenSSL's padding check: key4.db's entries are unpadded by hand, as NSS does.
fn decrypt_raw(cipher: Cipher, key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let mut crypter = Crypter::new(cipher, Mode::Decrypt, key, Some(iv)).ok()?;
    crypter.pad(false);
    let mut out = vec![0u8; data.len() + cipher.block_size()];
    let mut n = crypter.update(data, &mut out).ok()?;
    n += crypter.finalize(&mut out[n..]).ok()?;
    out.truncate(n);
    Some(unpad(out))
}

fn hmac_sha1(key: &[u8], parts: &[&[u8]]) -> Option<Vec<u8>> {
    let key = PKey::hmac(key).ok()?;
    let mut signer = Signer::new(MessageDigest::sha1(), &key).ok()?;
    for part in parts {
        signer.update(part).ok()?;
    }
    signer.sign_to_vec().ok()
}

/// Decrypts one key4.db entry: the password check, or a key.
fn decrypt_key4_entry(der: &[u8], global_salt: &[u8], primary: &str) -> Option<Vec<u8>> {
    let root = parse_der(der)?.into_iter().next()?;
    let [algorithm, encrypted] = root.children.as_slice() else {
        return None;
    };
    let [algorithm_oid, params, ..] = algorithm.children.as_slice() else {
        return None;
    };
    let base = openssl::sha::sha1(&[global_salt, primary.as_bytes()].concat());
    match oid(algorithm_oid).as_str() {
        PBES2 => {
            let [kdf, cipher, ..] = params.children.as_slice() else {
                return None;
            };
            let kdf_params = kdf.children.get(1)?;
            let salt = kdf_params.children.first()?;
            let iterations = integer(kdf_params.children.get(1)?)?;
            let length = kdf_params.children.get(2).and_then(integer).unwrap_or(32) as usize;
            if length != 32 || iterations == 0 || iterations > 10_000_000 {
                return None;
            }
            let mut key = [0u8; 32];
            pbkdf2_hmac(
                &base,
                salt.value,
                iterations as usize,
                MessageDigest::sha256(),
                &mut key,
            )
            .ok()?;
            // NSS keeps the 14-byte IV DER-encoded; the real IV is that encoding (04 0e + bytes).
            let iv = [&[0x04, 0x0e][..], cipher.children.get(1)?.value].concat();
            if iv.len() != 16 {
                return None;
            }
            decrypt_raw(Cipher::aes_256_cbc(), &key, &iv, encrypted.value)
        }
        SHA1_TRIPLE_DES => {
            let es = params.children.first()?.value;
            let mut pes = es.to_vec();
            pes.resize(pes.len().max(20), 0);
            let chp = openssl::sha::sha1(&[&base[..], es].concat());
            let k1 = hmac_sha1(&chp, &[&pes, es])?;
            let tk = hmac_sha1(&chp, &[&pes])?;
            let k2 = hmac_sha1(&chp, &[&tk, es])?;
            let k = [k1, k2].concat();
            decrypt_raw(
                Cipher::des_ede3_cbc(),
                &k[..24],
                &k[k.len() - 8..],
                encrypted.value,
            )
        }
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub enum FirefoxError {
    /// The profile has a primary password, and this isn't it.
    PrimaryPassword,
}

/// The login keys from key4.db's rows (item1, item2 of the password row; a11, a102 of each
/// private key), by their id. Err when the primary password is needed or wrong.
pub fn login_keys(
    item1: &[u8],
    item2: &[u8],
    privates: &[(Vec<u8>, Vec<u8>)],
    primary: &str,
) -> Result<HashMap<Vec<u8>, Vec<u8>>, FirefoxError> {
    let check = decrypt_key4_entry(item2, item1, primary).ok_or(FirefoxError::PrimaryPassword)?;
    if !check.starts_with(b"password-check") {
        return Err(FirefoxError::PrimaryPassword);
    }
    Ok(privates
        .iter()
        .filter_map(|(a11, a102)| {
            // A key that can't be read is left out; the logins that need it are too.
            Some((a102.clone(), decrypt_key4_entry(a11, item1, primary)?))
        })
        .collect())
}

/// Decrypts an encryptedUsername or encryptedPassword (base64 DER).
pub fn decrypt_login_field(base64: &str, keys: &HashMap<Vec<u8>, Vec<u8>>) -> Option<String> {
    use base64::Engine;
    let der = base64::engine::general_purpose::STANDARD
        .decode(base64.trim())
        .ok()?;
    let root = parse_der(&der)?.into_iter().next()?;
    let [key_id, algorithm, encrypted] = root.children.as_slice() else {
        return None;
    };
    let [cipher_oid, iv, ..] = algorithm.children.as_slice() else {
        return None;
    };
    let key = keys.get(key_id.value).or_else(|| keys.values().next())?;
    let plain = match oid(cipher_oid).as_str() {
        AES256_CBC => decrypt(
            Cipher::aes_256_cbc(),
            key.get(..32)?,
            Some(iv.value),
            encrypted.value,
        ),
        TRIPLE_DES_CBC => decrypt(
            Cipher::des_ede3_cbc(),
            key.get(..24)?,
            Some(iv.value),
            encrypted.value,
        ),
        _ => return None,
    }
    .ok()?;
    String::from_utf8(plain).ok()
}

// ── Writing, for tests ─────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod write {
    use super::*;
    use openssl::symm::encrypt;

    fn length(n: usize) -> Vec<u8> {
        if n < 0x80 {
            return vec![n as u8];
        }
        let bytes: Vec<u8> = n
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        [vec![0x80 | bytes.len() as u8], bytes].concat()
    }

    pub fn node(tag: u8, value: &[u8]) -> Vec<u8> {
        [vec![tag], length(value.len()), value.to_vec()].concat()
    }

    pub fn seq(items: &[Vec<u8>]) -> Vec<u8> {
        node(0x30, &items.concat())
    }

    pub fn octets(value: &[u8]) -> Vec<u8> {
        node(0x04, value)
    }

    pub fn int(n: u32) -> Vec<u8> {
        let mut bytes: Vec<u8> = n
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        if bytes.first().is_none_or(|b| b & 0x80 != 0) {
            bytes.insert(0, 0);
        }
        node(0x02, &bytes)
    }

    pub fn oid(dotted: &str) -> Vec<u8> {
        let parts: Vec<u64> = dotted.split('.').map(|p| p.parse().unwrap()).collect();
        let mut bytes = vec![(parts[0] * 40 + parts[1]) as u8];
        for &n in &parts[2..] {
            let mut chunk = vec![(n & 0x7f) as u8];
            let mut v = n >> 7;
            while v > 0 {
                chunk.insert(0, ((v & 0x7f) as u8) | 0x80);
                v >>= 7;
            }
            bytes.extend(chunk);
        }
        node(0x06, &bytes)
    }

    /// A PBES2 key4.db entry, the way current Firefox writes one.
    pub fn key4_entry(
        plain: &[u8],
        global_salt: &[u8],
        primary: &str,
        salt: &[u8],
        iv14: &[u8],
    ) -> Vec<u8> {
        let base = openssl::sha::sha1(&[global_salt, primary.as_bytes()].concat());
        let mut key = [0u8; 32];
        pbkdf2_hmac(&base, salt, 1000, MessageDigest::sha256(), &mut key).unwrap();
        let iv = [&[0x04, 0x0e][..], iv14].concat();
        let encrypted = encrypt(Cipher::aes_256_cbc(), &key, Some(&iv), plain).unwrap();
        seq(&[
            seq(&[
                oid(PBES2),
                seq(&[
                    seq(&[
                        oid("1.2.840.113549.1.5.12"),
                        seq(&[
                            octets(salt),
                            int(1000),
                            int(32),
                            seq(&[oid("1.2.840.113549.2.9")]),
                        ]),
                    ]),
                    seq(&[oid(AES256_CBC), octets(iv14)]),
                ]),
            ]),
            octets(&encrypted),
        ])
    }

    /// A login field the way NSS's SDR writes it (3DES or AES-256-CBC).
    pub fn login_field(text: &str, key_id: &[u8], key: &[u8], iv: &[u8], aes: bool) -> String {
        use base64::Engine;
        let (cipher, key, algorithm) = if aes {
            (Cipher::aes_256_cbc(), &key[..32], AES256_CBC)
        } else {
            (Cipher::des_ede3_cbc(), &key[..24], TRIPLE_DES_CBC)
        };
        let encrypted = encrypt(cipher, key, Some(iv), text.as_bytes()).unwrap();
        let der = seq(&[
            octets(key_id),
            seq(&[oid(algorithm), octets(iv)]),
            octets(&encrypted),
        ]);
        base64::engine::general_purpose::STANDARD.encode(der)
    }

    pub fn chromium(plain: impl AsRef<[u8]>, key: &[u8; 16], prefix: &str) -> Vec<u8> {
        let body = encrypt(
            Cipher::aes_128_cbc(),
            key,
            Some(&CHROMIUM_IV),
            plain.as_ref(),
        )
        .unwrap();
        [prefix.as_bytes(), &body].concat()
    }

    /// A browser's keys without its keyring: the fixed v10 key and the empty v11 one.
    pub fn keys() -> ChromiumKeys {
        ChromiumKeys {
            v10: chromium_key(b"peanuts"),
            v11: vec![chromium_key(b"")],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random(n: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; n];
        openssl::rand::rand_bytes(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn reads_chromium_v10_and_v11() {
        let keys = ChromiumKeys {
            v10: chromium_key(b"peanuts"),
            v11: vec![chromium_key(b"keyring-secret"), chromium_key(b"")],
        };
        let v10 = write::chromium("hunter2", &chromium_key(b"peanuts"), "v10");
        assert_eq!(decrypt_chromium(&v10, &keys).unwrap(), b"hunter2");
        let v11 = write::chromium("şifre-ğü", &chromium_key(b"keyring-secret"), "v11");
        assert_eq!(
            decrypt_chromium(&v11, &keys).unwrap(),
            "şifre-ğü".as_bytes()
        );
        // A secret that isn't there: given up cleanly.
        let other = write::chromium("secret", &chromium_key(b"another-secret"), "v11");
        let without = ChromiumKeys {
            v10: keys.v10,
            v11: vec![chromium_key(b"")],
        };
        assert_eq!(decrypt_chromium(&other, &without), None);
        assert_eq!(decrypt_chromium(b"", &keys).unwrap(), b"");
    }

    #[test]
    fn decrypts_firefox_logins_with_3des_and_aes() {
        let global_salt = random(20);
        let key_id = random(16);
        let login_key = random(24);
        let check = key4_check(&global_salt, "");
        let privates = vec![(
            write::key4_entry(&login_key, &global_salt, "", &random(32), &random(14)),
            key_id.clone(),
        )];
        let keys = login_keys(&global_salt, &check, &privates, "").unwrap();
        let field = write::login_field("ada@example.com", &key_id, &login_key, &random(8), false);
        assert_eq!(
            decrypt_login_field(&field, &keys).as_deref(),
            Some("ada@example.com")
        );
        let aes_key = random(32);
        let aes_keys = login_keys(
            &global_salt,
            &check,
            &[(
                write::key4_entry(&aes_key, &global_salt, "", &random(32), &random(14)),
                key_id.clone(),
            )],
            "",
        )
        .unwrap();
        let field = write::login_field("parolam", &key_id, &aes_key, &random(16), true);
        assert_eq!(
            decrypt_login_field(&field, &aes_keys).as_deref(),
            Some("parolam")
        );
    }

    fn key4_check(global_salt: &[u8], primary: &str) -> Vec<u8> {
        write::key4_entry(
            b"password-check\x02\x02",
            global_salt,
            primary,
            &random(32),
            &random(14),
        )
    }

    #[test]
    fn asks_for_the_primary_password() {
        let global_salt = random(20);
        let key_id = random(16);
        let login_key = random(24);
        let check = key4_check(&global_salt, "ana-parola");
        let privates = vec![(
            write::key4_entry(
                &login_key,
                &global_salt,
                "ana-parola",
                &random(32),
                &random(14),
            ),
            key_id.clone(),
        )];
        assert_eq!(
            login_keys(&global_salt, &check, &privates, "").unwrap_err(),
            FirefoxError::PrimaryPassword
        );
        let keys = login_keys(&global_salt, &check, &privates, "ana-parola").unwrap();
        let field = write::login_field("x", &key_id, &login_key, &random(8), false);
        assert_eq!(decrypt_login_field(&field, &keys).as_deref(), Some("x"));
    }

    #[test]
    fn reads_nss_old_sha1_3des_entries() {
        // NSS's older scheme, written here the way NSS wrote it.
        let global_salt = random(20);
        let entry_salt = random(20);
        let plain = b"password-check\x02\x02";
        let base = openssl::sha::sha1(&global_salt);
        let mut pes = entry_salt.clone();
        pes.resize(20, 0);
        let chp = openssl::sha::sha1(&[&base[..], &entry_salt].concat());
        let k1 = hmac_sha1(&chp, &[&pes, &entry_salt]).unwrap();
        let tk = hmac_sha1(&chp, &[&pes]).unwrap();
        let k2 = hmac_sha1(&chp, &[&tk, &entry_salt]).unwrap();
        let k = [k1, k2].concat();
        let encrypted = openssl::symm::encrypt(
            Cipher::des_ede3_cbc(),
            &k[..24],
            Some(&k[k.len() - 8..]),
            plain,
        )
        .unwrap();
        let der = write::seq(&[
            write::seq(&[
                write::oid(SHA1_TRIPLE_DES),
                write::seq(&[write::octets(&entry_salt), write::int(1)]),
            ]),
            write::octets(&encrypted),
        ]);
        let check = decrypt_key4_entry(&der, &global_salt, "").unwrap();
        assert!(check.starts_with(b"password-check"));
    }

    #[test]
    fn refuses_broken_der() {
        assert!(parse_der(&[0x30, 0x05, 0x04]).is_none());
        assert!(parse_der(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]).is_none());
        assert!(parse_der(&[0x04]).is_none());
        assert!(decrypt_login_field("not base64", &HashMap::new()).is_none());
        assert!(decrypt_key4_entry(&[0x30, 0x00], b"salt", "").is_none());
        // Nested too deep.
        let mut deep = vec![0x05, 0x00];
        for _ in 0..40 {
            deep = write::node(0x30, &deep);
        }
        assert!(parse_der(&deep).is_none());
    }
}
