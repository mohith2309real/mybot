//! The on-disk envelope, byte-compatible with MyBot 1.x (`src/secrets/vault.ts`).
//!
//! AES-256-GCM under a scrypt-derived key (N = 2^15, r = 8, p = 1, 32 bytes),
//! 16-byte salt and 12-byte IV, fresh on every write. Node keeps the GCM tag
//! separate from the ciphertext; the `aes-gcm` crate appends it. That is the one
//! translation this module exists to get right, and the cross-compat test
//! decrypts a file Node wrote.

use std::fs;
use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rand::RngCore;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::VaultError;

const SCRYPT_LOG_N: u8 = 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// The JSON file: `{"v":1,"salt":…,"iv":…,"tag":…,"data":…}`, all base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sealed {
    pub v: u32,
    pub salt: String,
    pub iv: String,
    pub tag: String,
    pub data: String,
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, VaultError> {
    let params = scrypt::Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, KEY_LEN)
        .map_err(|e| VaultError::Crypto(e.to_string()))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, key.as_mut())
        .map_err(|e| VaultError::Crypto(e.to_string()))?;
    Ok(key)
}

/// Encrypt any serialisable value.
pub fn seal<T: Serialize>(passphrase: &str, value: &T) -> Result<Sealed, VaultError> {
    let mut salt = [0u8; SALT_LEN];
    let mut iv = [0u8; IV_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut iv);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| VaultError::Crypto(e.to_string()))?;
    let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|e| VaultError::Format(e.to_string()))?);
    let mut out = cipher
        .encrypt(Nonce::from_slice(&iv), plain.as_slice())
        .map_err(|e| VaultError::Crypto(e.to_string()))?;

    let tag = out.split_off(out.len() - TAG_LEN);
    Ok(Sealed {
        v: 1,
        salt: B64.encode(salt),
        iv: B64.encode(iv),
        tag: B64.encode(tag),
        data: B64.encode(out),
    })
}

/// Decrypt. A wrong passphrase and a tampered file are the same error: both
/// mean "you can't read this", and telling them apart leaks nothing useful.
pub fn open<T: DeserializeOwned>(passphrase: &str, sealed: &Sealed) -> Result<T, VaultError> {
    if sealed.v != 1 {
        return Err(VaultError::Format(format!("unknown vault version {}", sealed.v)));
    }
    let bad = |_| VaultError::Format("vault file is not valid base64".into());
    let salt = B64.decode(&sealed.salt).map_err(bad)?;
    let iv = B64.decode(&sealed.iv).map_err(bad)?;
    let tag = B64.decode(&sealed.tag).map_err(bad)?;
    let mut data = B64.decode(&sealed.data).map_err(bad)?;
    if iv.len() != IV_LEN || tag.len() != TAG_LEN {
        return Err(VaultError::Format("vault file has the wrong shape".into()));
    }
    data.extend_from_slice(&tag);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|e| VaultError::Crypto(e.to_string()))?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&iv), data.as_slice())
            .map_err(|_| VaultError::WrongPassphrase)?,
    );
    serde_json::from_slice(&plain).map_err(|e| VaultError::Format(e.to_string()))
}

pub fn read_sealed(path: &Path) -> Result<Option<Sealed>, VaultError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(serde_json::from_str(&text).map_err(|e| VaultError::Format(e.to_string()))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(VaultError::Io(e.to_string())),
    }
}

/// Seal and write: directory 0700, file 0600, replaced atomically so a crash
/// mid-write never leaves half a vault.
pub fn write_sealed<T: Serialize>(path: &Path, passphrase: &str, value: &T) -> Result<(), VaultError> {
    let sealed = seal(passphrase, value)?;
    let text = serde_json::to_string(&sealed).map_err(|e| VaultError::Format(e.to_string()))?;
    let dir = path.parent().ok_or_else(|| VaultError::Io("vault path has no directory".into()))?;
    create_private_dir(dir)?;

    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    {
        let mut f = open_private(&tmp)?;
        f.write_all(text.as_bytes()).map_err(|e| VaultError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| VaultError::Io(e.to_string()))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        VaultError::Io(e.to_string())
    })
}

fn create_private_dir(dir: &Path) -> Result<(), VaultError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if !dir.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| VaultError::Io(e.to_string()))?;
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir).map_err(|e| VaultError::Io(e.to_string()))?;
    Ok(())
}

fn open_private(path: &Path) -> Result<fs::File, VaultError> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path).map_err(|e| VaultError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_passphrase() {
        let sealed = seal("pw", &serde_json::json!({"a": "secret-value"})).unwrap();
        let back: serde_json::Value = open("pw", &sealed).unwrap();
        assert_eq!(back["a"], "secret-value");
        assert!(matches!(open::<serde_json::Value>("nope", &sealed), Err(VaultError::WrongPassphrase)));
    }

    #[test]
    fn fresh_salt_and_iv_every_time() {
        let a = seal("pw", &1).unwrap();
        let b = seal("pw", &1).unwrap();
        assert_ne!(a.salt, b.salt);
        assert_ne!(a.iv, b.iv);
        assert_ne!(a.data, b.data);
    }

    #[test]
    fn tampering_fails_closed() {
        let mut sealed = seal("pw", &"hello").unwrap();
        let mut data = B64.decode(&sealed.data).unwrap();
        data[0] ^= 1;
        sealed.data = B64.encode(data);
        assert!(matches!(open::<String>("pw", &sealed), Err(VaultError::WrongPassphrase)));
    }
}
