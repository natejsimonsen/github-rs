//! Finds a GitHub token. Uses the OS credential store through the `keyring`
//! crate (macOS Keychain, Windows Credential Manager, Linux Secret Service)
//! and plain file reads. It never starts other programs.
//!
//! Reading gh's credential can make the OS ask for permission, and macOS asks
//! again whenever the app binary changes. So after one successful read we
//! copy the token to a file only your user can read, and use that from then on.

use base64::Engine;
use std::io::Write;
use std::path::PathBuf;

/// The credential name the `gh` CLI uses when it stores its login.
const GH_SERVICE: &str = "gh:github.com";

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Env,
    SavedFile,
    GhKeyring,
    GhConfigFile,
    Pasted,
}

impl Source {
    pub fn describe(self) -> &'static str {
        match self {
            Source::Env => "GH_TOKEN / GITHUB_TOKEN",
            Source::SavedFile => "saved token",
            Source::GhKeyring => "gh CLI login",
            Source::GhConfigFile => "gh CLI login (hosts.yml)",
            Source::Pasted => "pasted token",
        }
    }
}

/// Try each source in order. The first token found wins.
pub fn find_token() -> Option<(String, Source)> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            if !t.trim().is_empty() {
                return Some((t.trim().to_string(), Source::Env));
            }
        }
    }
    if let Some(t) = read_token_file() {
        return Some((t, Source::SavedFile));
    }
    let gh = read_gh_hosts();
    if let Some(t) = gh.token {
        return Some((t, Source::GhConfigFile));
    }
    // gh saves one item per user, and older versions also save one with an
    // empty account name for the active user. Try both.
    let mut accounts: Vec<String> = gh.user.into_iter().collect();
    accounts.push(String::new());
    for account in accounts {
        if let Some(t) = read_gh_keyring(&account) {
            let t = decode_go_keyring(&t);
            let _ = save_token(&t);
            return Some((t, Source::GhKeyring));
        }
    }
    None
}

fn read_gh_keyring(account: &str) -> Option<String> {
    // gh uses the go-keyring library. On Windows it names the credential
    // "service:user"; on macOS and Linux it stores service and user separately.
    #[cfg(target_os = "windows")]
    let entry = keyring::Entry::new_with_target(&format!("{GH_SERVICE}:{account}"), GH_SERVICE, account);
    #[cfg(not(target_os = "windows"))]
    let entry = keyring::Entry::new(GH_SERVICE, account);
    let t = entry.ok()?.get_password().ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// go-keyring may wrap the value as `go-keyring-base64:<base64>` or
/// `go-keyring-encoded:<hex>`.
fn decode_go_keyring(raw: &str) -> String {
    if let Some(b64) = raw.strip_prefix("go-keyring-base64:") {
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
            if let Ok(s) = String::from_utf8(bytes) {
                return s.trim().to_string();
            }
        }
    }
    if let Some(hex) = raw.strip_prefix("go-keyring-encoded:") {
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()))
            .collect();
        if let Some(s) = bytes.and_then(|b| String::from_utf8(b).ok()) {
            return s.trim().to_string();
        }
    }
    raw.to_string()
}

/// macOS: ~/Library/Application Support/github-prs/token
/// Linux: ~/.config/github-prs/token
/// Windows: %APPDATA%\github-prs\token
fn token_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("github-prs").join("token"))
}

fn read_token_file() -> Option<String> {
    let t = std::fs::read_to_string(token_path()?).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Write the token so only the current user can read it (0600 on macOS and
/// Linux; on Windows the user profile folder is already private).
pub fn save_token(token: &str) -> Result<(), String> {
    let path = token_path().ok_or("no config folder")?;
    let dir = path.parent().unwrap();
    let mut db = std::fs::DirBuilder::new();
    db.recursive(true);
    let mut oo = std::fs::OpenOptions::new();
    oo.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        db.mode(0o700);
        oo.mode(0o600);
    }
    db.create(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join("token.tmp");
    let mut f = oo.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(token.as_bytes()).map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn forget_token() {
    if let Some(p) = token_path() {
        let _ = std::fs::remove_file(p);
    }
}

#[derive(Default)]
struct GhHosts {
    user: Option<String>,
    token: Option<String>,
}

/// Minimal reader for gh's `hosts.yml`. We only need the active `user:` and,
/// if gh was set up with plain-text storage, `oauth_token:`.
fn read_gh_hosts() -> GhHosts {
    let mut out = GhHosts::default();
    let dir = std::env::var("GH_CONFIG_DIR").ok().map(PathBuf::from).or_else(|| {
        if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(x).join("gh"));
        }
        if cfg!(windows) {
            return dirs::config_dir().map(|d| d.join("GitHub CLI"));
        }
        dirs::home_dir().map(|h| h.join(".config").join("gh"))
    });
    let Some(dir) = dir else { return out };
    let Ok(text) = std::fs::read_to_string(dir.join("hosts.yml")) else {
        return out;
    };
    let mut in_github = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            in_github = line.trim_end_matches(':').trim() == "github.com";
            continue;
        }
        // Only look at keys directly under the host.
        let indent = line.len() - line.trim_start().len();
        if !in_github || indent > 4 {
            continue;
        }
        let t = line.trim();
        if let Some(v) = t.strip_prefix("user:") {
            out.user = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        } else if let Some(v) = t.strip_prefix("oauth_token:") {
            out.token = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_go_keyring_formats() {
        assert_eq!(decode_go_keyring("go-keyring-base64:Z2hvX2FiYw=="), "gho_abc");
        assert_eq!(decode_go_keyring("go-keyring-encoded:67686f5f616263"), "gho_abc");
        assert_eq!(decode_go_keyring("gho_plain"), "gho_plain");
    }
}
