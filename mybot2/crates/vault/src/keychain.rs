//! "Remember on this computer": the vault passphrase in the OS keychain.
//!
//! Opt-in. With it, MyBot opens without asking; without it, the passphrase is
//! asked once per launch and held only in memory. Either way it is never
//! written to a MyBot file.

const SERVICE: &str = "MyBot";
const ACCOUNT: &str = "vault-passphrase";

#[cfg(feature = "keychain")]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
}

/// Store the passphrase in the OS keychain.
pub fn remember(passphrase: &str) -> Result<(), String> {
    #[cfg(feature = "keychain")]
    {
        entry()?.set_password(passphrase).map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "keychain"))]
    {
        let _ = passphrase;
        Err("this build has no keychain support".into())
    }
}

/// The remembered passphrase, if any.
pub fn recall() -> Option<String> {
    #[cfg(feature = "keychain")]
    {
        entry().ok()?.get_password().ok()
    }
    #[cfg(not(feature = "keychain"))]
    {
        None
    }
}

pub fn forget() -> Result<(), String> {
    #[cfg(feature = "keychain")]
    {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    #[cfg(not(feature = "keychain"))]
    {
        Ok(())
    }
}

pub fn available() -> bool {
    cfg!(feature = "keychain")
}

#[allow(dead_code)]
const _NAMES: (&str, &str) = (SERVICE, ACCOUNT);
