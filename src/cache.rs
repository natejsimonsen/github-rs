//! Small JSON disk cache in the OS cache folder (e.g. ~/Library/Caches/github-prs). On launch the app
//! shows cached data instantly, then refreshes it in the background.

use serde::{Serialize, de::DeserializeOwned};
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("github-prs"))
}

/// Turn any key into a safe file name.
fn path(key: &str) -> Option<PathBuf> {
    let name: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(150)
        .collect();
    Some(dir()?.join(format!("{name}.json")))
}

pub fn load<T: DeserializeOwned>(key: &str) -> Option<T> {
    let bytes = std::fs::read(path(key)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write to a temp file and rename, so a crash never leaves half a file.
pub fn store<T: Serialize>(key: &str, value: &T) {
    let (Some(dir), Some(p)) = (dir(), path(key)) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(bytes) = serde_json::to_vec(value) else { return };
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}
