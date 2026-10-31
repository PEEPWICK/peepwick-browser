//! Small helpers for the JSON files in the profile.

use std::fs;
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;

/// Writes through a temporary file, so a crash never leaves half a file behind.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
