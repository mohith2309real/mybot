//! Model-provider API keys: `keys.enc`, a sealed `{"anthropic": "...", ...}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::VaultError;
use crate::seal::{open, read_sealed, write_sealed};

pub type KeyMap = BTreeMap<String, String>;

pub struct KeyStore {
    path: PathBuf,
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::at(crate::home().join("keys.enc"))
    }
}

impl KeyStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    pub fn read(&self, passphrase: &str) -> Result<KeyMap, VaultError> {
        match read_sealed(&self.path)? {
            None => Ok(KeyMap::new()),
            Some(s) => open(passphrase, &s),
        }
    }

    pub fn write(&self, passphrase: &str, keys: &KeyMap) -> Result<(), VaultError> {
        write_sealed(&self.path, passphrase, keys)
    }

    pub fn set(&self, passphrase: &str, provider: &str, key: &str) -> Result<(), VaultError> {
        let mut keys = self.read(passphrase)?;
        keys.insert(provider.to_string(), key.to_string());
        self.write(passphrase, &keys)
    }

    pub fn remove(&self, passphrase: &str, provider: &str) -> Result<bool, VaultError> {
        let mut keys = self.read(passphrase)?;
        let had = keys.remove(provider).is_some();
        self.write(passphrase, &keys)?;
        Ok(had)
    }

    /// Environment variables win over the vault, so CI and throwaway shells
    /// need no passphrase at all.
    pub fn resolve(&self, provider: &str, passphrase: Option<&str>) -> Option<Zeroizing<String>> {
        if let Some(v) = env_key(provider) {
            return Some(Zeroizing::new(v));
        }
        let pass = passphrase?;
        self.read(pass).ok()?.get(provider).cloned().map(Zeroizing::new)
    }
}

pub fn env_var_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        _ => "",
    }
}

fn env_key(provider: &str) -> Option<String> {
    let primary = std::env::var(env_var_for(provider)).ok().filter(|s| !s.is_empty());
    if primary.is_some() || provider != "gemini" {
        return primary;
    }
    std::env::var("GOOGLE_API_KEY").ok().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_read_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::at(dir.path().join("keys.enc"));
        store.set("pw", "anthropic", "sk-ant-FAKE").unwrap();
        assert_eq!(store.read("pw").unwrap()["anthropic"], "sk-ant-FAKE");
        assert!(store.read("wrong").is_err());
        assert!(!std::fs::read_to_string(store.path()).unwrap().contains("sk-ant-FAKE"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(store.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(store.remove("pw", "anthropic").unwrap());
        assert!(store.read("pw").unwrap().is_empty());
    }
}
