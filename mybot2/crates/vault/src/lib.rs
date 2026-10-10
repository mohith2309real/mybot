//! MyBot's local secrets: model API keys and your saved logins.
//!
//! Both live under `~/.mybot` (or `$MYBOT_HOME`) in the exact file format
//! MyBot 1.x writes, so moving to 2.0 needs no migration — the same passphrase
//! opens the same files from either version.
//!
//! The rule this crate exists to keep: a password leaves the store only through
//! [`logins::LoginStore::reveal_for_fill`], which is bound to the exact origin
//! the login was saved for. Every other function returns summaries.

pub mod keychain;
pub mod keys;
pub mod logins;
pub mod seal;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum VaultError {
    #[error("Could not unlock — wrong passphrase, or the file was modified.")]
    WrongPassphrase,
    #[error("Saved logins are locked. Unlock MyBot with your vault passphrase.")]
    Locked,
    #[error("{0}")]
    Invalid(String),
    #[error("vault file: {0}")]
    Format(String),
    #[error("vault crypto: {0}")]
    Crypto(String),
    #[error("vault io: {0}")]
    Io(String),
}

/// `$MYBOT_HOME`, else `~/.mybot`. Shared with MyBot 1.x on purpose.
pub fn home() -> PathBuf {
    if let Some(h) = std::env::var_os("MYBOT_HOME") {
        return PathBuf::from(h);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".mybot")
}

/// The passphrase from the environment, if one was given there.
pub fn env_passphrase() -> Option<String> {
    std::env::var("MYBOT_PASSPHRASE").ok().filter(|s| !s.is_empty())
}
