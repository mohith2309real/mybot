//! MyBot 1.x (Node) and 2.0 (Rust) must open each other's files with the same
//! passphrase. Runs the real 1.x code through `node`; skipped if node or the
//! 1.x sources are not around (e.g. a standalone checkout of this workspace).

use std::path::PathBuf;
use std::process::Command;

use mybot_vault::keys::KeyStore;
use mybot_vault::logins::{LoginStore, NewLogin};

fn repo_root() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    root.join("src/secrets/logins.ts").exists().then_some(root)
}

fn node(root: &PathBuf, home: &std::path::Path, script: &str) -> Option<String> {
    let out = Command::new("node")
        .current_dir(root)
        .env("MYBOT_HOME", home)
        .env("MYBOT_DB", home.join("compat.db"))
        .env_remove("MYBOT_PASSPHRASE")
        .args(["--no-warnings", "--input-type=module", "-e", script])
        .output()
        .ok()?;
    assert!(out.status.success(), "node failed: {}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[test]
fn node_writes_rust_reads_and_back() {
    let Some(root) = repo_root() else {
        eprintln!("skipped: MyBot 1.x sources not found");
        return;
    };
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipped: node not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    // 1.x writes a key and a login.
    let wrote = node(
        &root,
        home,
        r#"
        const v = await import('./src/secrets/vault.ts');
        const l = await import('./src/secrets/logins.ts');
        v.writeVault('shared-pass', { anthropic: 'sk-ant-FROM-NODE' });
        l.unlock('shared-pass');
        const s = l.addLogin({ url: 'https://github.com/login', username: 'alice@example.com', password: 'pw-from-node', label: 'Work' });
        console.log(s.id);
        "#,
    );
    let Some(node_id) = wrote else { return };

    // 2.0 reads them.
    let keys = KeyStore::at(home.join("keys.enc"));
    assert_eq!(keys.read("shared-pass").unwrap()["anthropic"], "sk-ant-FROM-NODE");
    let logins = LoginStore::at(home.join("logins.enc"));
    logins.unlock("shared-pass").unwrap();
    let listed = logins.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, node_id);
    assert_eq!(listed[0].label.as_deref(), Some("Work"));
    let cred = logins.reveal_for_fill(&node_id, "https://github.com").unwrap();
    assert_eq!(cred.password.as_str(), "pw-from-node");

    // 2.0 writes; 1.x reads.
    logins
        .add(NewLogin { url: "https://bank.example", username: "bob", password: "pw-from-rust", label: None })
        .unwrap();
    logins.set_always_allow("https://github.com", true).unwrap();
    keys.set("shared-pass", "openai", "sk-FROM-RUST").unwrap();

    let read_back = node(
        &root,
        home,
        r#"
        const v = await import('./src/secrets/vault.ts');
        const l = await import('./src/secrets/logins.ts');
        l.unlock('shared-pass');
        const list = l.listLogins();
        const bank = list.find((x) => x.origin === 'https://bank.example');
        const gh = list.find((x) => x.origin === 'https://github.com');
        console.log(JSON.stringify({
          key: v.readVault('shared-pass').openai,
          pw: l.revealForFill(bank.id, 'https://bank.example').password,
          always: gh.alwaysAllow,
          used: Boolean(gh.lastUsedAt),
        }));
        "#,
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&read_back).unwrap();
    assert_eq!(v["key"], "sk-FROM-RUST");
    assert_eq!(v["pw"], "pw-from-rust");
    assert_eq!(v["always"], true);
    assert_eq!(v["used"], true);
}
