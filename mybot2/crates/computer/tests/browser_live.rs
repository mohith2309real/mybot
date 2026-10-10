//! The browser layer against a real headless Chromium. Pages are served at
//! real https origins by intercepting requests over a second CDP connection,
//! so origin checks run exactly as they would on the web. Skipped when no
//! Chromium is installed.

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use mybot_computer::boundary::{Action, Boundaries};
use mybot_computer::browser::Browser;
use mybot_computer::cdp::wait_for_cdp;
use mybot_computer::loginfill::{FillInput, FillResult, LoginFiller};
use mybot_core::approvals::{ApprovalEvent, Approvals};
use mybot_core::cancel::Cancel;
use mybot_core::db::Db;
use mybot_vault::logins::{LoginStore, NewLogin};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const SECRET: &str = "real-browser-pw-Kq7#";

fn chromium() -> Option<String> {
    let candidates = [std::env::var("MYBOT_TEST_CHROMIUM").ok(), Some("/opt/pw-browsers/chromium".into()), Some("/usr/bin/chromium".into()), Some("/usr/bin/chromium-browser".into())];
    candidates.into_iter().flatten().find(|p| std::path::Path::new(p).exists())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn page_html(path: &str) -> Option<&'static str> {
    Some(match path {
        "https://bank.example/login" => {
            r#"<!doctype html><title>Bank</title>
            <form onsubmit="event.preventDefault(); document.body.dataset.sent='1'">
              <input type="email" id="u"><input type="password" id="p">
              <input type="text" id="spoof" placeholder="Password"><button>Sign in</button>
            </form><a href="/about">About</a>"#
        }
        "https://bank.example/about" => "<!doctype html><title>About</title><h1>About the bank</h1>",
        "https://shop.example/checkout" => "<!doctype html><title>Checkout</title><p>Review your order</p><button>Place order</button>",
        "https://login.example/verify" => r#"<!doctype html><title>Verify</title><label>Verification code <input id="otp" autocomplete="one-time-code"></label>"#,
        _ => return None,
    })
}

/// Serve page_html() for every https request on every page target.
async fn intercept(port: u16) {
    // CDP answers before the first tab is listed; on a busy CI machine that gap
    // is long enough to see an empty list, so wait for the page to show up.
    let mut page = None;
    for _ in 0..100 {
        let list: Vec<Value> = reqwest::get(format!("http://127.0.0.1:{port}/json/list")).await.unwrap().json().await.unwrap();
        page = list.into_iter().find(|t| t["type"] == "page");
        if page.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let page = page.expect("Chromium listed no page target within 10 s");
    let ws_url = page["webSocketDebuggerUrl"].as_str().unwrap().to_string();
    let (ws, _) = tokio_tungstenite::connect_async(ws_url).await.unwrap();
    let (mut sink, mut stream) = ws.split();
    sink.send(Message::Text(json!({"id": 1, "method": "Fetch.enable", "params": {"patterns": [{"urlPattern": "https://*"}]}}).to_string().into())).await.unwrap();
    tokio::spawn(async move {
        let mut id = 10;
        while let Some(Ok(Message::Text(t))) = stream.next().await {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["method"] != "Fetch.requestPaused" {
                continue;
            }
            let req_id = v["params"]["requestId"].as_str().unwrap().to_string();
            let url = v["params"]["request"]["url"].as_str().unwrap().split('?').next().unwrap().to_string();
            id += 1;
            let msg = match page_html(&url) {
                Some(body) => json!({"id": id, "method": "Fetch.fulfillRequest", "params": {
                    "requestId": req_id, "responseCode": 200,
                    "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                    "body": base64::engine::general_purpose::STANDARD.encode(body)}}),
                None => json!({"id": id, "method": "Fetch.failRequest", "params": {"requestId": req_id, "errorReason": "Failed"}}),
            };
            if sink.send(Message::Text(msg.to_string().into())).await.is_err() {
                break;
            }
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn real_browser() {
    let Some(exe) = chromium() else {
        eprintln!("skipped: no Chromium found");
        return;
    };
    let port = free_port();
    let profile = tempfile::tempdir().unwrap();
    let _child = Kill(
        Command::new(exe)
            .args([
                "--headless=new",
                "--no-sandbox",
                "--disable-gpu",
                "--no-first-run",
                &format!("--remote-debugging-port={port}"),
                &format!("--user-data-dir={}", profile.path().display()),
                "about:blank",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for_cdp(port, Duration::from_secs(30)).await.unwrap();
    intercept(port).await;

    let b = Browser::connect(port).await.unwrap();
    let (url, title) = b.navigate("https://bank.example/login").await.unwrap();
    assert_eq!((url.as_str(), title.as_str()), ("https://bank.example/login", "Bank"));

    // --- snapshot and refs
    let snap = b.snapshot(6000).await.unwrap();
    let find = |kind: &str, label: &str| -> u32 {
        snap.elements
            .lines()
            .find(|l| l.contains(kind) && l.contains(label))
            .and_then(|l| l.trim().trim_start_matches('[').split(']').next()?.parse().ok())
            .unwrap_or_else(|| panic!("no {kind} {label} in\n{}", snap.elements))
    };
    let user_ref = find("input:email", "");
    let pw_ref = find("input:password", "");
    let spoof_ref = find("input:text", "Password");

    // --- the fill's own checks against the live page
    assert!(b.fill_credential("https://github.com", Some(pw_ref), None, "a", SECRET, false).await.unwrap_err().0.contains("no longer on"));
    assert!(b.fill_credential("https://bank.example", Some(spoof_ref), None, "a", SECRET, false).await.unwrap_err().0.contains("not a password field"));
    assert_eq!(b.eval("document.getElementById('spoof').value").await.unwrap(), "");

    // --- the whole fill_login path: approval answered, then filled and submitted
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LoginStore::at(dir.path().join("logins.enc")));
    store.unlock("pw").unwrap();
    store.add(NewLogin { url: "https://bank.example", username: "alice@bank.example", password: SECRET, label: None }).unwrap();
    let approvals = Approvals::new(Db::in_memory().unwrap(), store.clone());
    let filler = LoginFiller::new(store, approvals.clone());
    let mut rx = approvals.subscribe();
    let a2 = approvals.clone();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if let ApprovalEvent::Requested(r) = ev {
                a2.allow(&r.id, false).unwrap();
                break;
            }
        }
    });
    let r = filler
        .fill(&b, FillInput { password_ref: Some(pw_ref), username_ref: Some(user_ref), account: None, submit: true }, "scout", Some("t"), &Cancel::new())
        .await;
    assert!(matches!(&r, FillResult::Filled(t) if !t.contains(SECRET)), "{r:?}");
    assert_eq!(b.eval("document.getElementById('p').value").await.unwrap(), SECRET);
    assert_eq!(b.eval("document.getElementById('u').value").await.unwrap(), "alice@bank.example");
    assert_eq!(b.eval("document.body.dataset.sent").await.unwrap(), "1");

    // --- the next page_read never shows it
    let after = b.snapshot(6000).await.unwrap();
    assert!(!after.elements.contains(SECRET) && !after.text.contains(SECRET), "leak:\n{}", after.elements);

    // --- typing, clicking, history
    b.type_text(spoof_ref, "hello", false).await.unwrap();
    assert_eq!(b.eval("document.getElementById('spoof').value").await.unwrap(), "hello");
    b.type_text(spoof_ref, "replaced", false).await.unwrap();
    assert_eq!(b.eval("document.getElementById('spoof').value").await.unwrap(), "replaced");
    let about = find("a", "About");
    b.click(about).await.unwrap();
    assert_eq!(b.url().await, "https://bank.example/about");
    b.history_step(-1).await.unwrap();
    assert_eq!(b.url().await, "https://bank.example/login");
    assert!(b.click(999).await.unwrap_err().contains("No element with ref 999"));

    // --- boundaries on real pages
    let bounds = Boundaries::default();
    b.navigate("https://shop.example/checkout").await.unwrap();
    let p = bounds.detect(&b, Action::Navigate, None, None).await.unwrap();
    assert_eq!(p.kind, "payment");
    let snap = b.snapshot(6000).await.unwrap();
    let order = snap.elements.lines().position(|l| l.contains("Place order")).unwrap() as u32 + 1;
    bounds.acknowledge(&p);
    assert_eq!(bounds.detect(&b, Action::Click, Some(order), Some(&snap)).await.unwrap().kind, "payment", "commit buttons always stop");
    b.navigate("https://login.example/verify").await.unwrap();
    assert_eq!(bounds.detect(&b, Action::Read, None, None).await.unwrap().kind, "two_factor");

    // --- tabs
    b.new_tab("").await.unwrap();
    assert_eq!(b.tabs().await.unwrap().len(), 2);
    b.close_tab(1).await.unwrap();
    assert_eq!(b.tabs().await.unwrap().len(), 1);
    assert!(!b.screenshot_png().await.unwrap().is_empty());
}
