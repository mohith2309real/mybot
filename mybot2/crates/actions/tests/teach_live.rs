//! Teach-a-task against a real headless Chromium: the recorder sees clicks,
//! typing and navigation, and a typed password never reaches MyBot. Skipped
//! when no Chromium is installed.

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use mybot_actions::teach::{Recorder, actions_of};
use mybot_computer::browser::Browser;
use mybot_computer::cdp::wait_for_cdp;
use mybot_core::db::Db;

const SECRET: &str = "teach-pw-Zr9!x";

fn chromium() -> Option<String> {
    let candidates = [std::env::var("MYBOT_TEST_CHROMIUM").ok(), Some("/opt/pw-browsers/chromium".into()), Some("/usr/bin/chromium".into())];
    candidates.into_iter().flatten().find(|p| std::path::Path::new(p).exists())
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

const FORM: &str = "data:text/html,<title>Shop</title><form onsubmit='event.preventDefault()'><label>Search <input id=q name=q></label><label>Password <input type=password id=p></label><button id=go>Find</button></form>";

#[tokio::test(flavor = "multi_thread")]
async fn recorder_sees_the_demo_but_not_the_password() {
    let Some(exe) = chromium() else {
        eprintln!("skipped: no Chromium found");
        return;
    };
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let profile = tempfile::tempdir().unwrap();
    let _child = Kill(
        Command::new(exe)
            .args(["--headless=new", "--no-sandbox", "--disable-gpu", "--no-first-run", &format!("--remote-debugging-port={port}"), &format!("--user-data-dir={}", profile.path().display()), "about:blank"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for_cdp(port, Duration::from_secs(30)).await.unwrap();
    let b = Arc::new(Browser::connect(port).await.unwrap());
    b.navigate(FORM).await.unwrap();

    let rec = Recorder::start(b.clone(), "find shoes").await.unwrap();
    let snap = b.snapshot(4000).await.unwrap();
    let find = |needle: &str| -> u32 {
        snap.elements
            .lines()
            .find(|l| l.contains(needle))
            .and_then(|l| l.trim().trim_start_matches('[').split(']').next()?.parse().ok())
            .unwrap_or_else(|| panic!("no {needle} in\n{}", snap.elements))
    };
    b.type_text(find("input:text"), "running shoes", false).await.unwrap();
    b.type_text(find("input:password"), SECRET, false).await.unwrap();
    b.click(find("button")).await.unwrap();
    // A later document is covered by the init script.
    b.navigate("data:text/html,<title>Results</title><a href='#top'>Top</a>").await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    let db = Db::in_memory().unwrap();
    let stored = rec.stop(&db).await.unwrap();
    let actions = actions_of(&stored);
    let log = serde_json::to_string(&actions).unwrap();
    assert!(!log.contains(SECRET), "the password reached the recording: {log}");
    assert!(log.contains("<redacted:password>"), "{log}");
    assert!(actions.iter().any(|a| a.kind == "input" && a.value.as_deref() == Some("running shoes")), "{log}");
    assert!(actions.iter().any(|a| a.kind == "click" && a.target.as_ref().is_some_and(|t| t.tag == "button")), "{log}");
    assert!(actions.iter().any(|a| a.kind == "navigate" && a.target.as_ref().is_some_and(|t| t.description == "Results")), "{log}");

    // Disarmed: nothing more is recorded after stop.
    let before = actions.len();
    b.navigate("data:text/html,<title>After</title>").await.unwrap();
    assert!(!b.eval("!!(window.__mybotTeach && window.__mybotTeach.on)").await.unwrap().as_bool().unwrap());
    assert_eq!(actions_of(&db.recording(&stored.id).unwrap().unwrap()).len(), before);
}
