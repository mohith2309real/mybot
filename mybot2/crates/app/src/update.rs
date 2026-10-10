//! Updates over the air.
//!
//! Every release carries `latest.json` — the version, and for each platform the
//! download's URL, size and SHA-256 — and `latest.json.sig`, an Ed25519
//! signature of those exact bytes made in CI with a key only the release
//! workflow holds. MyBot fetches both and checks the signature against the
//! public key compiled in below *before* believing anything the manifest says.
//! Only then does the version, the URL, or the hash the download must match
//! mean anything. So:
//!
//! - a tampered manifest, or one signed by anyone else, is refused;
//! - a swapped or truncated download fails its hash and is thrown away;
//! - an old, genuinely signed manifest replayed later can't downgrade you,
//!   because only a strictly newer version is ever installed.
//!
//! Installing swaps the app in place (the `.app` bundle on macOS, the binary
//! elsewhere), keeping the previous version as a backup so a failed swap rolls
//! back, then relaunches. Development builds (run from a `target/` folder) never
//! replace themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The release signing key (Ed25519, public half). The private half is the
/// `MYBOT_UPDATE_SIGNING_KEY` secret of the release workflow.
pub const PUBLIC_KEY: [u8; 32] = [
    0x48, 0xdf, 0x59, 0x9c, 0x5f, 0x8c, 0x72, 0x67, 0xac, 0xe8, 0xa8, 0x1b, 0x0a, 0x81, 0x77, 0x2d, 0x40, 0x5d, 0xe9, 0x0d, 0xb1, 0x01,
    0xe6, 0x50, 0x74, 0x8c, 0xd2, 0x90, 0x23, 0x35, 0xbc, 0x43,
];

/// Where releases are published. `latest/download/<file>` always resolves to
/// the newest release, without the GitHub API (and its rate limit).
pub const DEFAULT_FEED: &str = "https://github.com/mohith2309real/mybot/releases/latest/download";

/// How often the app looks, when automatic updates are on.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    pub assets: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// A verified update, unpacked and ready to swap in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Staged {
    pub version: String,
    /// The new `MyBot.app` (macOS) or binary (elsewhere).
    pub payload: PathBuf,
    #[serde(default)]
    pub notes_url: Option<String>,
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The feed to check: the published releases, or `MYBOT_UPDATE_URL` (a
/// folder holding `latest.json` and `latest.json.sig`, for testing a release
/// before it goes out — still signature-checked like any other).
pub fn feed() -> String {
    std::env::var("MYBOT_UPDATE_URL").ok().filter(|u| !u.trim().is_empty()).map(|u| u.trim_end_matches('/').to_string()).unwrap_or_else(|| DEFAULT_FEED.to_string())
}

/// This build's key in `latest.json`, matching the release archive names.
pub fn platform() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos-x86_64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x86_64")
    } else {
        None
    }
}

/// `a` is a strictly newer version than `b` ("2.10.0" > "2.9.3"; a release
/// beats its own pre-release, "2.1.0" > "2.1.0-beta").
pub fn newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        let nums = core.split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect();
        (nums, pre)
    }
    let ((mut x, xp), (mut y, yp)) = (parts(a), parts(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    match x.cmp(&y) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => yp && !xp,
    }
}

/// Check `sig` (base64) over `bytes` with `key`, then parse the manifest.
pub fn verify_with(key: &[u8], bytes: &[u8], sig_b64: &str) -> Result<Manifest, String> {
    let sig = base64::engine::general_purpose::STANDARD.decode(sig_b64.trim()).map_err(|_| "The update signature isn't valid base64.".to_string())?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(bytes, &sig)
        .map_err(|_| "The update's signature doesn't match MyBot's release key, so it was ignored.".to_string())?;
    serde_json::from_slice(bytes).map_err(|e| format!("The update manifest is signed but unreadable: {e}"))
}

pub fn verify(bytes: &[u8], sig_b64: &str) -> Result<Manifest, String> {
    verify_with(&PUBLIC_KEY, bytes, sig_b64)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("MyBot/{}", current()))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

async fn get(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let r = client.get(url).send().await.map_err(|e| format!("Couldn't reach the update server: {e}"))?;
    if !r.status().is_success() {
        return Err(format!("The update server answered {} for {url}", r.status()));
    }
    r.bytes().await.map(|b| b.to_vec()).map_err(|e| format!("The download was cut off: {e}"))
}

/// The newest signed release, whether or not it's newer than this build.
pub async fn latest() -> Result<Manifest, String> {
    let c = client()?;
    let base = feed();
    let bytes = get(&c, &format!("{base}/latest.json")).await?;
    let sig = get(&c, &format!("{base}/latest.json.sig")).await?;
    verify(&bytes, &String::from_utf8_lossy(&sig))
}

/// A release newer than this build, for this platform, if there is one.
pub async fn check() -> Result<Option<Manifest>, String> {
    let m = latest().await?;
    let usable = platform().is_some_and(|p| m.assets.contains_key(p));
    Ok((usable && newer(&m.version, current())).then_some(m))
}

/// Where updates are kept: `<data>/updates`.
pub fn dir() -> PathBuf {
    mybot_vault::home().join("updates")
}

fn ready_file() -> PathBuf {
    dir().join("ready.json")
}

/// Download this platform's archive, check its size and hash against the
/// signed manifest, unpack it, and record it as ready.
pub async fn download(m: &Manifest) -> Result<Staged, String> {
    let key = platform().ok_or("There's no MyBot download for this kind of computer.")?;
    let asset = m.assets.get(key).ok_or("This release has no download for this computer.")?;
    let bytes = get(&client()?, &asset.url).await?;
    if bytes.len() as u64 != asset.size {
        return Err(format!("The download is {} bytes; the release says {}. Not installing it.", bytes.len(), asset.size));
    }
    let digest = hex::encode(Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(asset.sha256.trim()) {
        return Err("The download doesn't match the release's checksum. Not installing it.".into());
    }

    let here = dir().join(&m.version);
    let _ = std::fs::remove_dir_all(&here);
    let unpacked = here.join("unpacked");
    std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let archive = here.join(&asset.name);
    std::fs::write(&archive, &bytes).map_err(|e| e.to_string())?;
    unpack(&archive, &unpacked)?;
    let _ = std::fs::remove_file(&archive);

    let payload = find_payload(&unpacked).ok_or("The download doesn't contain MyBot.")?;
    let staged = Staged { version: m.version.clone(), payload, notes_url: m.notes_url.clone() };
    std::fs::write(ready_file(), serde_json::to_vec_pretty(&staged).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(staged)
}

/// `tar` reads both .tar.gz and (on macOS and Windows 10+) .zip, and ships
/// with every system MyBot runs on — no unpacking code of our own to get wrong.
fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let out = std::process::Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(into).output().map_err(|e| format!("Couldn't run tar to unpack the update: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("Unpacking the update failed: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

fn find_payload(root: &Path) -> Option<PathBuf> {
    let want = if cfg!(target_os = "macos") {
        "MyBot.app"
    } else if cfg!(windows) {
        "mybot2.exe"
    } else {
        "mybot2"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == want) {
                return Some(p);
            }
            if p.is_dir() && p.extension().is_none_or(|x| x != "app") {
                stack.push(p);
            }
        }
    }
    None
}

/// An update already downloaded and verified, still newer than this build.
pub fn staged() -> Option<Staged> {
    let s: Staged = serde_json::from_slice(&std::fs::read(ready_file()).ok()?).ok()?;
    (newer(&s.version, current()) && s.payload.exists()).then_some(s)
}

/// What gets replaced: the `.app` bundle on macOS, the binary elsewhere.
pub fn install_target() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let s = exe.to_string_lossy();
    if s.contains("/target/debug/") || s.contains("/target/release/") || s.contains("\\target\\") {
        return Err("This is a development build, so it doesn't update itself. Rebuild it instead.".into());
    }
    if s.contains("/AppTranslocation/") {
        return Err("macOS is running MyBot from a temporary copy. Move MyBot.app into Applications and open it from there, then updates can install.".into());
    }
    if cfg!(target_os = "macos")
        && let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|x| x == "app"))
    {
        return Ok(bundle.to_path_buf());
    }
    Ok(exe)
}

/// Swap the staged update in. Returns what to launch afterwards.
pub fn install(s: &Staged) -> Result<PathBuf, String> {
    let target = install_target()?;
    let backups = dir().join("previous");
    std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
    if target.extension().is_some_and(|x| x == "app") {
        replace_bundle(&target, &s.payload, &backups.join(format!("MyBot-{}.app", current())))?;
    } else {
        replace_binary(&target, &s.payload)?;
    }
    let _ = std::fs::remove_file(ready_file());
    Ok(target)
}

fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Different volumes: copy across, then remove the original.
        Err(_) => {
            let ok = std::process::Command::new(if cfg!(target_os = "macos") { "ditto" } else { "cp" })
                .args(if cfg!(target_os = "macos") { vec![] } else { vec!["-a"] })
                .arg(from)
                .arg(to)
                .status()?
                .success();
            if !ok {
                return Err(std::io::Error::other("copy failed"));
            }
            if from.is_dir() { std::fs::remove_dir_all(from) } else { std::fs::remove_file(from) }
        }
    }
}

/// Replace an app bundle, keeping the old one at `backup` until the new one is
/// in place. A running app can be moved on macOS; the process carries on.
pub fn replace_bundle(target: &Path, payload: &Path, backup: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(backup);
    move_path(target, backup).map_err(|e| format!("Couldn't move the old MyBot aside ({e}). Is it in a folder you can write to?"))?;
    if let Err(e) = move_path(payload, target) {
        let _ = move_path(backup, target); // roll back
        return Err(format!("Couldn't put the new MyBot in place ({e}); kept the old one."));
    }
    Ok(())
}

/// Replace a binary. A running executable can't be overwritten on Windows but
/// can be renamed, so: rename it to `.old`, put the new one in its place.
pub fn replace_binary(target: &Path, payload: &Path) -> Result<(), String> {
    let old = old_path(target);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(target, &old).map_err(|e| format!("Couldn't move the old MyBot aside ({e})."))?;
    if let Err(e) = std::fs::copy(payload, target) {
        let _ = std::fs::rename(&old, target);
        return Err(format!("Couldn't put the new MyBot in place ({e}); kept the old one."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Start the freshly installed MyBot (the caller then quits).
pub fn relaunch(target: &Path) -> Result<(), String> {
    let mut cmd = if target.extension().is_some_and(|x| x == "app") {
        let mut c = std::process::Command::new("open");
        c.arg("-n").arg(target);
        c
    } else {
        std::process::Command::new(target)
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("Updated, but couldn't restart MyBot: {e}. Open it again yourself."))
}

/// Tidy up after an update: the `.old` binary left by the swap, and staged
/// downloads that are no longer newer than this build.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() && name.chars().next().is_some_and(|c| c.is_ascii_digit()) && !newer(&name, current()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    if std::fs::read(ready_file()).is_ok() && staged().is_none() {
        let _ = std::fs::remove_file(ready_file());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn keypair() -> ring::signature::Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn manifest(version: &str) -> Vec<u8> {
        serde_json::to_vec(&Manifest {
            version: version.into(),
            tag: format!("v{version}"),
            published: None,
            notes_url: None,
            assets: [("macos-arm64".to_string(), Asset { name: "a.tar.gz".into(), url: "https://example.invalid/a.tar.gz".into(), size: 3, sha256: "00".into() })].into(),
        })
        .unwrap()
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("2.1.0", "2.0.0"));
        assert!(newer("2.10.0", "2.9.9"));
        assert!(newer("v3.0", "2.99.99"));
        assert!(newer("2.1.0", "2.1.0-beta"));
        assert!(!newer("2.1.0-beta", "2.1.0"));
        assert!(!newer("2.1.0", "2.1.0"));
        assert!(!newer("2.0.9", "2.1.0"), "never a downgrade");
    }

    #[test]
    fn only_the_release_key_is_believed() {
        let kp = keypair();
        let bytes = manifest("9.9.9");
        let sig = base64::engine::general_purpose::STANDARD.encode(kp.sign(&bytes).as_ref());
        let m = verify_with(kp.public_key().as_ref(), &bytes, &sig).expect("genuine manifest verifies");
        assert_eq!(m.version, "9.9.9");

        let mut tampered = bytes.clone();
        let at = tampered.iter().position(|&b| b == b'9').unwrap();
        tampered[at] = b'8';
        assert!(verify_with(kp.public_key().as_ref(), &tampered, &sig).is_err(), "an edited manifest is refused");

        let other = keypair();
        assert!(verify_with(other.public_key().as_ref(), &bytes, &sig).is_err(), "someone else's key is refused");
        assert!(verify(&bytes, &sig).is_err(), "a test-signed manifest doesn't pass the real release key");
    }

    #[test]
    fn swapping_a_bundle_keeps_a_backup_and_rolls_back() {
        let t = std::env::temp_dir().join(format!("mybot-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let target = t.join("Applications/MyBot.app");
        let payload = t.join("staged/MyBot.app");
        let backup = t.join("previous/MyBot-old.app");
        std::fs::create_dir_all(target.join("Contents")).unwrap();
        std::fs::write(target.join("Contents/version"), "old").unwrap();
        std::fs::create_dir_all(payload.join("Contents")).unwrap();
        std::fs::write(payload.join("Contents/version"), "new").unwrap();
        std::fs::create_dir_all(t.join("previous")).unwrap();

        replace_bundle(&target, &payload, &backup).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("Contents/version")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(backup.join("Contents/version")).unwrap(), "old");

        // A payload that isn't there: the swap fails and the app is untouched.
        let err = replace_bundle(&target, &t.join("staged/missing.app"), &t.join("previous/again.app"));
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(target.join("Contents/version")).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn swapping_a_binary_leaves_the_old_one_as_dot_old() {
        let t = std::env::temp_dir().join(format!("mybot-update-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        let target = t.join("mybot2");
        let payload = t.join("new-mybot2");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&payload, "new").unwrap();
        replace_binary(&target, &payload).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(old_path(&target)).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&t);
    }
}
