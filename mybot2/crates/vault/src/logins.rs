//! Saved logins: `logins.enc`, file-compatible with MyBot 1.x.
//!
//! Same three rules as 1.x:
//!   1. The model never sees a password — only [`LoginStore::reveal_for_fill`]
//!      returns one, and only to the browser fill.
//!   2. A login fills only on the exact origin it was saved for (https, or
//!      http on loopback).
//!   3. Every use is approved by the human unless they chose "always" for that
//!      one login. (The approval itself lives in `mybot-core`.)

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::VaultError;
use crate::seal::{open, read_sealed, write_sealed};

/// One saved login. Field names match 1.x's JSON exactly.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLogin {
    pub id: String,
    pub origin: String,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub always_allow: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
}

impl Drop for SavedLogin {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

impl std::fmt::Debug for SavedLogin {
    // A derived Debug would print the password into any log that formats one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedLogin")
            .field("id", &self.id)
            .field("origin", &self.origin)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Everything but the password. The only shape that leaves this module freely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSummary {
    pub id: String,
    pub origin: String,
    pub username: String,
    pub label: Option<String>,
    pub always_allow: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

impl From<&SavedLogin> for LoginSummary {
    fn from(l: &SavedLogin) -> Self {
        Self {
            id: l.id.clone(),
            origin: l.origin.clone(),
            username: l.username.clone(),
            label: l.label.clone(),
            always_allow: l.always_allow,
            created_at: l.created_at.clone(),
            last_used_at: l.last_used_at.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct LoginsFile {
    #[serde(default)]
    logins: Vec<SavedLogin>,
}

/// What a fill gets: the username and the password, wiped on drop.
pub struct Credential {
    pub username: String,
    pub password: Zeroizing<String>,
}

// ---------------------------------------------------------------------------
// Origins
// ---------------------------------------------------------------------------

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}

/// "github.com", "https://GitHub.com/login?x=1" → "https://github.com".
/// https only; plain http for loopback alone, where nothing crosses a network.
pub fn normalize_origin(input: &str) -> Result<String, VaultError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(VaultError::Invalid(
            "Give the address of the sign-in page, e.g. https://github.com/login".into(),
        ));
    }
    let has_scheme = raw
        .split_once("://")
        .map(|(s, _)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)))
        .unwrap_or(false);
    let with_scheme = if has_scheme { raw.to_string() } else { format!("https://{raw}") };
    let url = Url::parse(&with_scheme).map_err(|_| VaultError::Invalid(format!("\"{input}\" is not a web address.")))?;
    let host = url.host_str().unwrap_or("");
    if host.is_empty() {
        return Err(VaultError::Invalid(format!("\"{input}\" is not a web address.")));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(host) => {}
        other => {
            return Err(VaultError::Invalid(format!(
                "Saved logins only work on https pages (got {other}://{host})."
            )));
        }
    }
    Ok(url.origin().ascii_serialization())
}

/// The origin of a page, or `None` if no login may ever fill there.
pub fn page_origin(url: &str) -> Option<String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    normalize_origin(url).ok()
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct NewLogin<'a> {
    pub url: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub label: Option<&'a str>,
}

/// The store, with its passphrase held in memory once unlocked.
pub struct LoginStore {
    path: PathBuf,
    passphrase: Mutex<Option<Zeroizing<String>>>,
}

impl Default for LoginStore {
    fn default() -> Self {
        Self::at(crate::home().join("logins.enc"))
    }
}

impl LoginStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), passphrase: Mutex::new(None) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Hold the passphrase for this process, after proving it opens the file.
    pub fn unlock(&self, passphrase: &str) -> Result<(), VaultError> {
        self.read_with(passphrase)?;
        *self.passphrase.lock().unwrap() = Some(Zeroizing::new(passphrase.to_string()));
        Ok(())
    }

    pub fn lock(&self) {
        *self.passphrase.lock().unwrap() = None;
    }

    pub fn is_unlocked(&self) -> bool {
        self.passphrase.lock().unwrap().is_some() || crate::env_passphrase().is_some()
    }

    fn pass(&self) -> Result<Zeroizing<String>, VaultError> {
        if let Some(p) = self.passphrase.lock().unwrap().as_ref() {
            return Ok(p.clone());
        }
        crate::env_passphrase().map(Zeroizing::new).ok_or(VaultError::Locked)
    }

    fn read_with(&self, passphrase: &str) -> Result<LoginsFile, VaultError> {
        match read_sealed(&self.path)? {
            None => Ok(LoginsFile::default()),
            Some(s) => open(passphrase, &s),
        }
    }

    fn read(&self) -> Result<(LoginsFile, Zeroizing<String>), VaultError> {
        let pass = self.pass()?;
        Ok((self.read_with(&pass)?, pass))
    }

    fn write(&self, pass: &str, file: &LoginsFile) -> Result<(), VaultError> {
        write_sealed(&self.path, pass, file)
    }

    pub fn list(&self) -> Result<Vec<LoginSummary>, VaultError> {
        let (file, _) = self.read()?;
        let mut out: Vec<LoginSummary> = file.logins.iter().map(LoginSummary::from).collect();
        out.sort_by(|a, b| a.origin.cmp(&b.origin).then(a.username.cmp(&b.username)));
        Ok(out)
    }

    /// Save a login. The same origin + username again replaces the password.
    pub fn add(&self, new: NewLogin<'_>) -> Result<LoginSummary, VaultError> {
        let origin = normalize_origin(new.url)?;
        let username = new.username.trim();
        if username.is_empty() {
            return Err(VaultError::Invalid("A login needs a username or email.".into()));
        }
        if new.password.is_empty() {
            return Err(VaultError::Invalid("A login needs a password.".into()));
        }
        let label = new.label.map(str::trim).filter(|s| !s.is_empty()).map(String::from);

        let (mut file, pass) = self.read()?;
        if let Some(existing) = file.logins.iter_mut().find(|l| l.origin == origin && l.username == username) {
            existing.password.zeroize();
            existing.password = new.password.to_string();
            if new.label.is_some() {
                existing.label = label;
            }
            let summary = LoginSummary::from(&*existing);
            self.write(&pass, &file)?;
            return Ok(summary);
        }

        let login = SavedLogin {
            id: uuid::Uuid::new_v4().to_string(),
            origin,
            username: username.to_string(),
            password: new.password.to_string(),
            label,
            always_allow: false,
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            last_used_at: None,
        };
        let summary = LoginSummary::from(&login);
        file.logins.push(login);
        self.write(&pass, &file)?;
        Ok(summary)
    }

    /// By id; by origin (every account there); or by "origin username".
    pub fn remove(&self, selector: &str) -> Result<Vec<LoginSummary>, VaultError> {
        let (mut file, pass) = self.read()?;
        let m = Matcher::new(selector);
        let removed: Vec<LoginSummary> = file.logins.iter().filter(|l| m.hit(l)).map(LoginSummary::from).collect();
        if removed.is_empty() {
            return Err(VaultError::Invalid(format!("No saved login matches \"{selector}\".")));
        }
        file.logins.retain(|l| !m.hit(l));
        self.write(&pass, &file)?;
        Ok(removed)
    }

    pub fn set_always_allow(&self, selector: &str, always: bool) -> Result<Vec<LoginSummary>, VaultError> {
        let (mut file, pass) = self.read()?;
        let m = Matcher::new(selector);
        let mut hit = Vec::new();
        for l in file.logins.iter_mut().filter(|l| m.hit(l)) {
            l.always_allow = always;
            hit.push(LoginSummary::from(&*l));
        }
        if hit.is_empty() {
            return Err(VaultError::Invalid(format!("No saved login matches \"{selector}\".")));
        }
        self.write(&pass, &file)?;
        Ok(hit)
    }

    /// Exact (origin, username) — used when a human answers "always".
    pub fn set_always_for(&self, origin: &str, username: &str, always: bool) -> Result<(), VaultError> {
        let (mut file, pass) = self.read()?;
        if let Some(l) = file.logins.iter_mut().find(|l| l.origin == origin && l.username == username) {
            l.always_allow = always;
            self.write(&pass, &file)?;
        }
        Ok(())
    }

    /// Summaries for one exact origin — the most a bot may be told.
    pub fn logins_for(&self, origin: &str) -> Result<Vec<LoginSummary>, VaultError> {
        let (file, _) = self.read()?;
        Ok(file.logins.iter().filter(|l| l.origin == origin).map(LoginSummary::from).collect())
    }

    /// The one place a password leaves the store, and only into a browser fill.
    /// The origin is checked again here because this function is the boundary.
    pub fn reveal_for_fill(&self, id: &str, origin: &str) -> Result<Credential, VaultError> {
        let (mut file, pass) = self.read()?;
        let login = file
            .logins
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or_else(|| VaultError::Invalid("That saved login no longer exists.".into()))?;
        if login.origin != origin {
            return Err(VaultError::Invalid("Saved login does not belong to this site.".into()));
        }
        login.last_used_at = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        let cred = Credential { username: login.username.clone(), password: Zeroizing::new(login.password.clone()) };
        self.write(&pass, &file)?;
        Ok(cred)
    }
}

struct Matcher {
    raw: String,
    origin: Option<String>,
    user: Option<String>,
}

impl Matcher {
    fn new(selector: &str) -> Self {
        let raw = selector.trim().to_string();
        let mut parts = raw.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or("");
        let user = parts.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Self { origin: normalize_origin(first).ok(), user, raw }
    }

    fn hit(&self, l: &SavedLogin) -> bool {
        if l.id == self.raw {
            return true;
        }
        match &self.origin {
            Some(o) => &l.origin == o && self.user.as_deref().is_none_or(|u| l.username == u),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "hunter2-Zq9!unique-pw";

    fn store() -> (tempfile::TempDir, LoginStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = LoginStore::at(dir.path().join("logins.enc"));
        s.unlock("pw").unwrap();
        (dir, s)
    }

    #[test]
    fn origins() {
        assert_eq!(normalize_origin("github.com").unwrap(), "https://github.com");
        assert_eq!(normalize_origin("https://GitHub.com/login?r=/x").unwrap(), "https://github.com");
        assert_eq!(normalize_origin("https://github.com:443/").unwrap(), "https://github.com");
        assert_eq!(normalize_origin("https://a.example.com:8443/x").unwrap(), "https://a.example.com:8443");
        assert_eq!(normalize_origin("http://localhost:3000/login").unwrap(), "http://localhost:3000");
        assert!(normalize_origin("http://github.com").is_err());
        assert!(normalize_origin("ftp://github.com").is_err());
        assert!(normalize_origin("javascript:alert(1)").is_err());
        assert!(normalize_origin("").is_err());
        assert_eq!(page_origin("about:blank"), None);
        assert_eq!(page_origin("https://github.com/login"), Some("https://github.com".into()));
    }

    #[test]
    fn locked_until_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let s = LoginStore::at(dir.path().join("logins.enc"));
        // SAFETY of the env read: tests in this crate never set MYBOT_PASSPHRASE.
        if crate::env_passphrase().is_none() {
            assert_eq!(s.list().unwrap_err(), VaultError::Locked);
        }
    }

    #[test]
    fn add_list_encrypted_reveal() {
        let (_d, s) = store();
        let gh = s.add(NewLogin { url: "https://github.com/login", username: "alice@example.com", password: SECRET, label: Some("Work") }).unwrap();
        s.add(NewLogin { url: "bank.example", username: "alice", password: "bank-pw", label: None }).unwrap();
        let listed = s.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert!(!serde_json::to_string(&listed).unwrap().contains(SECRET));

        let raw = std::fs::read_to_string(s.path()).unwrap();
        for leak in [SECRET, "alice@example.com", "github.com"] {
            assert!(!raw.contains(leak), "{leak} on disk in plaintext");
        }

        // Re-add replaces, keeps the id.
        let again = s.add(NewLogin { url: "github.com", username: "alice@example.com", password: "new-pw", label: None }).unwrap();
        assert_eq!(again.id, gh.id);
        assert_eq!(again.label.as_deref(), Some("Work"));
        assert_eq!(s.reveal_for_fill(&gh.id, "https://github.com").unwrap().password.as_str(), "new-pw");
        assert!(s.reveal_for_fill(&gh.id, "https://github.com.evil.example").is_err());

        // A second unlock with the wrong passphrase fails closed.
        assert_eq!(s.unlock("wrong").unwrap_err(), VaultError::WrongPassphrase);
    }

    #[test]
    fn remove_and_always() {
        let (_d, s) = store();
        s.add(NewLogin { url: "github.com", username: "a", password: "1", label: None }).unwrap();
        s.add(NewLogin { url: "github.com", username: "b", password: "2", label: None }).unwrap();
        assert_eq!(s.set_always_allow("github.com b", true).unwrap().len(), 1);
        assert!(s.logins_for("https://github.com").unwrap().iter().any(|l| l.username == "b" && l.always_allow));
        assert_eq!(s.remove("https://github.com a").unwrap().len(), 1);
        assert_eq!(s.remove("github.com").unwrap().len(), 1);
        assert!(s.remove("github.com").is_err());
    }

    #[test]
    fn debug_never_prints_password() {
        let l = SavedLogin {
            id: "1".into(),
            origin: "https://x.example".into(),
            username: "u".into(),
            password: SECRET.into(),
            label: None,
            always_allow: false,
            created_at: String::new(),
            last_used_at: None,
        };
        assert!(!format!("{l:?}").contains(SECRET));
    }
}
