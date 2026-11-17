//! The backup file, byte for byte as the Electron version wrote it, so a backup made with
//! 0.1.x opens here and one made here opens there:
//!
//!   "PWBK" | version (1) | log2 N, r, p (scrypt) | salt (16) | IV (12) | ciphertext | tag (16)
//!
//! The payload is gzipped JSON, sealed with AES-256-GCM under a key that scrypt derives from
//! the password (in Unicode NFC). The header is the associated data, so a file whose
//! parameters were changed fails like a wrong password does. Nothing here touches GTK or the
//! profile, so the tests use it directly.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use gtk::glib;
use openssl::symm::{Cipher, decrypt_aead, encrypt_aead};

pub const EXTENSION: &str = "pwbackup";
const MAGIC: &[u8; 4] = b"PWBK";
const VERSION: u8 = 1;
const HEADER: usize = 4 + 1 + 3 + 16 + 12;
const TAG: usize = 16;
/// The most a backup may unpack to; real ones are a few megabytes.
const MAX_PLAIN: u64 = 512 * 1024 * 1024;
/// The most memory scrypt may need for a file: four times what every backup so far asked for.
/// The header is only checked after the key is derived, so a made-up file could otherwise
/// ask for gigabytes.
const MAX_SCRYPT_MEMORY: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kdf {
    pub log_n: u8,
    pub r: u8,
    pub p: u8,
}

/// About half a second and 128 MB on a laptop: slow enough to make guessing expensive. The
/// Electron version used the same.
pub const DEFAULT_KDF: Kdf = Kdf {
    log_n: 17,
    r: 8,
    p: 1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a backup, or not one this version can read.
    Format,
    /// The wrong password, or a damaged file: the two can't be told apart.
    Password,
}

impl Error {
    /// As the restore dialog names it.
    pub fn reason(self) -> &'static str {
        match self {
            Error::Format => "format",
            Error::Password => "password",
        }
    }
}

fn derive_key(password: &str, salt: &[u8], kdf: Kdf) -> Option<[u8; 32]> {
    let n = 1u64 << kdf.log_n;
    let (r, p) = (u64::from(kdf.r), u64::from(kdf.p));
    // The same text typed on another keyboard can come as other code points ("ş" or "s" and a
    // combining cedilla); Node normalized it the same way.
    let password = glib::normalize(password, glib::NormalizeMode::DefaultCompose);
    let mut key = [0u8; 32];
    openssl::pkcs5::scrypt(
        password.as_bytes(),
        salt,
        n,
        r,
        p,
        128 * n * r * 2,
        &mut key,
    )
    .ok()?;
    Some(key)
}

/// Seals a payload (JSON) with the password.
pub fn encrypt(json: &[u8], password: &str, kdf: Kdf) -> Option<Vec<u8>> {
    let mut salt = [0u8; 16];
    let mut iv = [0u8; 12];
    openssl::rand::rand_bytes(&mut salt).ok()?;
    openssl::rand::rand_bytes(&mut iv).ok()?;
    let mut header = Vec::with_capacity(HEADER);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&[VERSION, kdf.log_n, kdf.r, kdf.p]);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&iv);
    let key = derive_key(password, &salt, kdf)?;
    let mut packer = GzEncoder::new(Vec::new(), Compression::default());
    packer.write_all(json).ok()?;
    let packed = packer.finish().ok()?;
    let mut tag = [0u8; TAG];
    let sealed = encrypt_aead(
        Cipher::aes_256_gcm(),
        &key,
        Some(&iv),
        &header,
        &packed,
        &mut tag,
    )
    .ok()?;
    let mut file = header;
    file.extend_from_slice(&sealed);
    file.extend_from_slice(&tag);
    Some(file)
}

/// True when the bytes start like a backup file, before asking for the password.
pub fn looks_like_backup(data: &[u8]) -> bool {
    data.len() > HEADER + TAG && data.starts_with(MAGIC) && data[4] == VERSION
}

/// The payload (JSON) of a backup file.
pub fn decrypt(data: &[u8], password: &str) -> Result<Vec<u8>, Error> {
    if !looks_like_backup(data) {
        return Err(Error::Format);
    }
    let kdf = Kdf {
        log_n: data[5],
        r: data[6],
        p: data[7],
    };
    // Parameters no file of ours would have could only make us hang or run out of memory.
    if !(10..=20).contains(&kdf.log_n)
        || !(1..=16).contains(&kdf.r)
        || !(1..=4).contains(&kdf.p)
        || 128 * (1u64 << kdf.log_n) * u64::from(kdf.r) > MAX_SCRYPT_MEMORY
    {
        return Err(Error::Format);
    }
    let key = derive_key(password, &data[8..24], kdf).ok_or(Error::Format)?;
    let tag_at = data.len() - TAG;
    let packed = decrypt_aead(
        Cipher::aes_256_gcm(),
        &key,
        Some(&data[24..HEADER]),
        &data[..HEADER],
        &data[HEADER..tag_at],
        &data[tag_at..],
    )
    .map_err(|_| Error::Password)?;
    let mut plain = Vec::new();
    MultiGzDecoder::new(packed.as_slice())
        .take(MAX_PLAIN + 1)
        .read_to_end(&mut plain)
        .map_err(|_| Error::Format)?;
    if plain.len() as u64 > MAX_PLAIN {
        return Err(Error::Format);
    }
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::{Value, json};

    /// The real cost (2^17) takes half a second a call; the format is the same.
    const FAST: Kdf = Kdf {
        log_n: 10,
        r: 8,
        p: 1,
    };

    fn payload() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "format": "peepwick-backup",
            "version": 1,
            "createdAt": 1,
            "settings": { "theme": "dark" },
            "note": "ğüşiöç",
        }))
        .unwrap()
    }

    #[test]
    fn round_trips_with_the_right_password() {
        let file = encrypt(&payload(), "doğru parola", FAST).unwrap();
        assert!(looks_like_backup(&file));
        assert_eq!(decrypt(&file, "doğru parola").unwrap(), payload());
    }

    #[test]
    fn never_stores_the_contents_in_the_clear() {
        let file = encrypt(&payload(), "doğru parola", FAST).unwrap();
        let has = |needle: &[u8]| file.windows(needle.len()).any(|w| w == needle);
        assert!(!has(b"peepwick-backup"));
        assert!(!has(b"dark"));
    }

    #[test]
    fn rejects_a_wrong_password() {
        let file = encrypt(&payload(), "doğru parola", FAST).unwrap();
        assert_eq!(decrypt(&file, "yanlış parola"), Err(Error::Password));
    }

    #[test]
    fn notices_a_changed_byte_anywhere_header_included() {
        let file = encrypt(&payload(), "doğru parola", FAST).unwrap();
        for index in [10, 30, file.len() - 20, file.len() - 1] {
            let mut copy = file.clone();
            copy[index] ^= 1;
            assert!(decrypt(&copy, "doğru parola").is_err(), "byte {index}");
        }
    }

    #[test]
    fn refuses_files_that_are_not_backups_and_absurd_key_parameters() {
        assert_eq!(
            decrypt(b"not a backup at all, just some text", "x"),
            Err(Error::Format)
        );
        let file = encrypt(&payload(), "p", FAST).unwrap();
        for (index, value) in [(5, 40), (5, 9), (6, 0), (6, 17), (7, 0), (7, 5)] {
            let mut copy = file.clone();
            copy[index] = value;
            assert_eq!(
                decrypt(&copy, "p"),
                Err(Error::Format),
                "byte {index} = {value}"
            );
        }
        // Within the old limits but asking for 2 GB: refused before scrypt runs.
        let mut greedy = file.clone();
        greedy[5] = 20;
        greedy[6] = 16;
        assert_eq!(decrypt(&greedy, "p"), Err(Error::Format));
    }

    #[test]
    fn salts_every_file() {
        let a = encrypt(&payload(), "p", FAST).unwrap();
        let b = encrypt(&payload(), "p", FAST).unwrap();
        assert_ne!(a, b);
    }

    /// Made by the Electron version's own `encryptBackup` (src/main/backupfile.ts at 0.1.9a,
    /// under Node 24) with the password "s\u{327}ifre-123", that is "şifre-123" with a
    /// combining cedilla.
    const FROM_ELECTRON: &str = "UFdCSwEKCAFKwphJIRF6MCAHgN4cqS0l9ooGle4JYRhjGBMF/YuDV4C12A5NpRSMnfuNDeKzddS41pghvWvXennC2YDm1OYnf1HH4Gcv/A3fEFnNZAxbTvqvgNDkycypL6uXX2EOQPsJmH8AoL8kfnMS0Y4UMtM5btNu/MiCfGcwZ4AIfqwQFqTNjPj5w1mQ4TW23hSo7fDgR19leKEHnVjmo41p6Y7yvAp1/TQs063Nik6Ex4prthEDLw==";

    #[test]
    fn opens_a_backup_the_electron_version_made() {
        let file = base64::engine::general_purpose::STANDARD
            .decode(FROM_ELECTRON)
            .unwrap();
        assert!(looks_like_backup(&file));
        let expected = json!({
            "format": "peepwick-backup",
            "version": 1,
            "app": "0.1.9a",
            "createdAt": 1763000000000u64,
            "settings": { "theme": "light" },
            "note": "ğüşiöç",
        });
        // Typed either way, the password is the same after normalizing.
        for password in ["şifre-123", "s\u{327}ifre-123"] {
            let plain = decrypt(&file, password).unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&plain).unwrap(), expected);
        }
        assert_eq!(decrypt(&file, "sifre-123"), Err(Error::Password));
    }
}
