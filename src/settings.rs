//! User settings, in settings.json next to the token (see `auth`). For now
//! just the repos and owners left out of every list:
//!
//! ```json
//! { "ignored_repositories": ["some-org", "owner/repo"] }
//! ```

use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// An owner ("some-org", a user too) or one repo ("owner/repo").
    pub ignored_repositories: Vec<String>,
}

/// What every search leaves out, read by `github::full_query` on any thread.
static IGNORED: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// macOS: ~/Library/Application Support/github-prs/settings.json
/// Linux: ~/.config/github-prs/settings.json
/// Windows: %APPDATA%\github-prs\settings.json
pub fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("github-prs").join("settings.json"))
}

/// A missing or broken file means no settings.
pub fn load() -> Settings {
    let s: Settings = path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    *IGNORED.write().unwrap() = s.ignored_repositories.clone();
    s
}

pub fn save(s: &Settings) -> Result<(), String> {
    *IGNORED.write().unwrap() = s.ignored_repositories.clone();
    let path = path().ok_or("no config folder")?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| e.to_string())
}

/// Search qualifiers that leave out the ignored repos: " -user:org -repo:o/r".
/// `-user:` covers organizations and personal accounts alike.
pub fn exclusions() -> String {
    qualifiers(&IGNORED.read().unwrap())
}

fn qualifiers(ignored: &[String]) -> String {
    ignored
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty() && !e.contains(char::is_whitespace))
        .map(|e| if e.contains('/') { format!(" -repo:{e}") } else { format!(" -user:{e}") })
        .collect()
}

impl Settings {
    /// GitHub names ignore case.
    pub fn is_ignored(&self, entry: &str) -> bool {
        self.ignored_repositories.iter().any(|e| e.trim().eq_ignore_ascii_case(entry))
    }

    pub fn set_ignored(&mut self, entry: &str, on: bool) {
        self.ignored_repositories.retain(|e| !e.trim().eq_ignore_ascii_case(entry));
        if on {
            self.ignored_repositories.push(entry.to_string());
            self.ignored_repositories.sort_by_key(|e| e.to_lowercase());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners_and_repos_become_qualifiers() {
        let ignored = ["ZipRecruiter-Test".to_string(), " owner/repo ".to_string(), "".to_string(), "bad entry".to_string()];
        assert_eq!(qualifiers(&ignored), " -user:ZipRecruiter-Test -repo:owner/repo");
    }

    #[test]
    fn toggling_ignores_case() {
        let mut s = Settings::default();
        s.set_ignored("ZipRecruiter-Test", true);
        assert!(s.is_ignored("ziprecruiter-test"));
        s.set_ignored("ziprecruiter-test", false);
        assert!(s.ignored_repositories.is_empty());
    }
}
