//! fill_login: sign in with a login you saved, after you say yes.
//!
//!   1. Read the page. The origin comes from the browser, never the model — a
//!      bot cannot claim it is on github.com.
//!   2. Check the refs point at the right kinds of field.
//!   3. Find a login saved for that exact origin. None → hand to the human.
//!   4. Ask (or apply "always"). No answer → nothing fills.
//!   5. Fill, re-checking origin and field types against the live page.
//!
//! The model gets back a sentence. The password exists in this process for
//! one fill call, in a zeroizing buffer, and goes nowhere else.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mybot_core::approvals::{Approvals, Outcome};
use mybot_core::cancel::Cancel;
use mybot_vault::logins::{LoginStore, LoginSummary, page_origin};

use crate::browser::{Browser, FillError, Snapshot};

#[async_trait]
pub trait FillPage: Send + Sync {
    async fn snapshot(&self) -> Result<Snapshot, String>;
    async fn fill(
        &self,
        origin: &str,
        password_ref: Option<u32>,
        username_ref: Option<u32>,
        username: &str,
        password: &str,
        submit: bool,
    ) -> Result<bool, FillError>;
}

#[async_trait]
impl FillPage for Browser {
    async fn snapshot(&self) -> Result<Snapshot, String> {
        Browser::snapshot(self, 6000).await
    }
    async fn fill(&self, origin: &str, p: Option<u32>, u: Option<u32>, user: &str, pass: &str, submit: bool) -> Result<bool, FillError> {
        self.fill_credential(origin, p, u, user, pass, submit).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct FillInput {
    pub password_ref: Option<u32>,
    pub username_ref: Option<u32>,
    pub account: Option<String>,
    pub submit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FillResult {
    Filled(String),
    /// The model asked for something wrong; it can correct and retry.
    Error(String),
    /// No saved login (or locked): a human signs in instead.
    Handoff { text: String, reason: String },
    /// The human said no, or did not answer.
    Denied(String),
}

/// One approval covers both pages of a two-step sign-in in the same task.
const GRANT: Duration = Duration::from_secs(300);

pub struct LoginFiller {
    pub logins: Arc<LoginStore>,
    pub approvals: Approvals,
    pub wait: Duration,
    grants: Mutex<HashMap<String, Instant>>,
}

const USERNAME_KINDS: [&str; 4] = ["input:text", "input:email", "input:tel", "input:search"];

impl LoginFiller {
    pub fn new(logins: Arc<LoginStore>, approvals: Approvals) -> Self {
        Self { logins, approvals, wait: mybot_core::approvals::DEFAULT_WAIT, grants: Mutex::new(HashMap::new()) }
    }

    pub fn clear_grants(&self) {
        self.grants.lock().unwrap().clear();
    }

    pub async fn fill(&self, page: &dyn FillPage, input: FillInput, bot: &str, task_id: Option<&str>, cancel: &Cancel) -> FillResult {
        if input.password_ref.is_none() && input.username_ref.is_none() {
            return FillResult::Error("fill_login needs password_ref, username_ref, or both. Call page_read first.".into());
        }
        let snap = match page.snapshot().await {
            Ok(s) => s,
            Err(e) => return FillResult::Error(format!("Could not read the page: {e}")),
        };
        let Some(origin) = page_origin(&snap.url) else {
            return FillResult::Error(format!(
                "Saved logins are only filled on https pages. This page is {}. Nothing was filled.",
                if snap.url.is_empty() { "blank" } else { &snap.url }
            ));
        };
        if let Some(r) = input.password_ref {
            if snap.kind(r) != Some("input:password") {
                return FillResult::Error(format!("Ref {r} is not a password field on this page. Call page_read and check."));
            }
        }
        if let Some(r) = input.username_ref {
            if !snap.kind(r).is_some_and(|k| USERNAME_KINDS.contains(&k)) {
                return FillResult::Error(format!("Ref {r} is not a username or email field on this page. Call page_read and check."));
            }
        }

        let handoff = |reason: String, text: &str| FillResult::Handoff { text: text.into(), reason };
        if !self.logins.exists() {
            return handoff(format!("No saved login for {origin}. Sign in here, then hand control back."), "No logins are saved. Handing over to the human to sign in.");
        }
        if !self.logins.is_unlocked() {
            return handoff(
                format!("Saved logins are locked, so sign in to {origin} yourself, then hand control back."),
                "Saved logins are locked in this session. Handing over to the human to sign in.",
            );
        }
        let candidates: Vec<LoginSummary> = match self.logins.logins_for(&origin) {
            Ok(c) => c,
            Err(e) => {
                return handoff(format!("Could not open saved logins ({e}). Sign in to {origin} yourself."), "Saved logins could not be opened. Handing over to the human to sign in.");
            }
        };
        if candidates.is_empty() {
            return handoff(
                format!("No saved login for {origin}. Sign in here, then hand control back."),
                &format!("No login is saved for {origin}. Handing over to the human to sign in."),
            );
        }

        let names = || candidates.iter().map(|l| l.username.as_str()).collect::<Vec<_>>().join(", ");
        let login = match &input.account {
            Some(a) => match candidates.iter().find(|l| l.username.eq_ignore_ascii_case(a.trim())) {
                Some(l) => l.clone(),
                None => return FillResult::Error(format!("No saved login for \"{a}\" on {origin}. Saved accounts here: {}.", names())),
            },
            None if candidates.len() == 1 => candidates[0].clone(),
            None => {
                return FillResult::Error(format!(
                    "More than one login is saved for {origin}: {}. Call fill_login again with account set to the one the task needs. \
                     If the task does not say, call request_human.",
                    names()
                ));
            }
        };

        let key = format!("{}|{origin}|{}", task_id.unwrap_or("-"), login.id);
        let granted = self.grants.lock().unwrap().get(&key).is_some_and(|t| t.elapsed() < GRANT);
        if !granted {
            let outcome = self.approvals.request(bot, task_id, &login, self.wait, cancel).await;
            if !outcome.allowed() {
                let why = match outcome {
                    Outcome::TimedOut => "The human did not answer in time, so nothing was filled.",
                    Outcome::Cancelled => "The task was stopped while waiting, so nothing was filled.",
                    _ => "The human said no to using the saved login.",
                };
                return FillResult::Denied(format!(
                    "{why} Do not call fill_login again for this site in this task, and never try to type a password yourself. \
                     If signing in is still required, call request_human; otherwise carry on with what you can do without it."
                ));
            }
            self.grants.lock().unwrap().insert(key, Instant::now());
        }

        let cred = match self.logins.reveal_for_fill(&login.id, &origin) {
            Ok(c) => c,
            Err(e) => return FillResult::Error(format!("{e} Nothing was filled.")),
        };
        match page.fill(&origin, input.password_ref, input.username_ref, &cred.username, &cred.password, input.submit).await {
            Ok(submitted) => {
                let what = match (input.password_ref, input.username_ref) {
                    (Some(_), Some(_)) => "username and password",
                    (Some(_), None) => "password",
                    _ => "username",
                };
                FillResult::Filled(format!(
                    "Filled the saved {what} for {} on {origin}{}. Call page_read to see the result. \
                     If the site now asks for a one-time code, that is the human's to enter.",
                    login.username,
                    if submitted { " and submitted the form" } else { "" }
                ))
            }
            Err(FillError(msg)) => FillResult::Error(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mybot_core::db::Db;
    use mybot_vault::logins::NewLogin;
    use tokio::sync::Mutex as AsyncMutex;

    const SECRET: &str = "hunter2-Zq9!unique-pw";
    const FORM: &str = "  [1] input:email \"Email\"\n  [2] input:password \"Password\"\n  [3] button \"Sign in\"";

    struct FakePage {
        url: String,
        elements: String,
        fills: AsyncMutex<Vec<(String, String, String)>>,
    }

    #[async_trait]
    impl FillPage for FakePage {
        async fn snapshot(&self) -> Result<Snapshot, String> {
            Ok(Snapshot { url: self.url.clone(), elements: self.elements.clone(), ..Default::default() })
        }
        async fn fill(&self, origin: &str, _: Option<u32>, _: Option<u32>, user: &str, pass: &str, submit: bool) -> Result<bool, FillError> {
            self.fills.lock().await.push((origin.into(), user.into(), pass.into()));
            Ok(submit)
        }
    }

    fn page(url: &str, elements: &str) -> FakePage {
        FakePage { url: url.into(), elements: elements.into(), fills: AsyncMutex::new(vec![]) }
    }

    fn setup() -> (tempfile::TempDir, LoginFiller, Approvals) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(LoginStore::at(dir.path().join("logins.enc")));
        store.unlock("pw").unwrap();
        store.add(NewLogin { url: "https://github.com/login", username: "alice@example.com", password: SECRET, label: None }).unwrap();
        let approvals = Approvals::new(Db::in_memory().unwrap(), store.clone());
        let mut f = LoginFiller::new(store, approvals.clone());
        f.wait = Duration::from_secs(5);
        (dir, f, approvals)
    }

    /// Answer the next request as soon as it shows up.
    fn answer(a: &Approvals, allow: bool) {
        let a = a.clone();
        let mut rx = a.subscribe();
        tokio::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                if let mybot_core::approvals::ApprovalEvent::Requested(r) = ev {
                    if allow { a.allow(&r.id, false).unwrap(); } else { a.deny(&r.id).unwrap(); }
                    break;
                }
            }
        });
    }

    #[tokio::test]
    async fn approved_fill_right_site_right_account() {
        let (_d, f, a) = setup();
        let p = page("https://github.com/login", FORM);
        answer(&a, true);
        let r = f.fill(&p, FillInput { password_ref: Some(2), username_ref: Some(1), submit: true, ..Default::default() }, "scout", Some("t1"), &Cancel::new()).await;
        let FillResult::Filled(text) = r else { panic!("{r:?}") };
        assert!(!text.contains(SECRET));
        let fills = p.fills.lock().await;
        assert_eq!(fills[0], ("https://github.com".into(), "alice@example.com".into(), SECRET.into()));
    }

    #[tokio::test]
    async fn lookalikes_subdomains_and_http_get_nothing() {
        let (_d, f, a) = setup();
        for url in ["https://github.com.login-check.example/login", "https://gist.github.com/login"] {
            let p = page(url, FORM);
            assert!(matches!(f.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", None, &Cancel::new()).await, FillResult::Handoff { .. }));
            assert!(p.fills.lock().await.is_empty());
        }
        let p = page("http://github.com/login", FORM);
        assert!(matches!(f.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", None, &Cancel::new()).await, FillResult::Error(_)));
        assert!(a.recent(10).is_empty(), "nobody was even asked");
    }

    #[tokio::test]
    async fn spoofed_label_is_not_a_password_field() {
        let (_d, f, _a) = setup();
        let p = page("https://github.com/login", "  [2] input:text \"input:password Password\"");
        assert!(matches!(f.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", None, &Cancel::new()).await, FillResult::Error(_)));
    }

    #[tokio::test]
    async fn denied_fills_nothing_and_two_step_asks_once() {
        let (_d, f, a) = setup();
        let p = page("https://github.com/login", FORM);
        answer(&a, false);
        assert!(matches!(f.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", Some("t2"), &Cancel::new()).await, FillResult::Denied(_)));
        assert!(p.fills.lock().await.is_empty());

        answer(&a, true);
        assert!(matches!(f.fill(&p, FillInput { username_ref: Some(1), ..Default::default() }, "s", Some("t3"), &Cancel::new()).await, FillResult::Filled(_)));
        // Same task, password page: no second question (a second ask would time out → Denied).
        let mut quick = LoginFiller::new(f.logins.clone(), a.clone());
        quick.wait = Duration::from_millis(50);
        *quick.grants.lock().unwrap() = f.grants.lock().unwrap().clone();
        assert!(matches!(quick.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", Some("t3"), &Cancel::new()).await, FillResult::Filled(_)));
        // A different task is asked again.
        assert!(matches!(quick.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", Some("t4"), &Cancel::new()).await, FillResult::Denied(_)));
    }

    #[tokio::test]
    async fn locked_store_hands_over_without_asking() {
        let (_d, f, a) = setup();
        f.logins.lock();
        if mybot_vault::env_passphrase().is_none() {
            let p = page("https://github.com/login", FORM);
            assert!(matches!(f.fill(&p, FillInput { password_ref: Some(2), ..Default::default() }, "s", None, &Cancel::new()).await, FillResult::Handoff { .. }));
            assert!(a.recent(5).is_empty());
        }
    }
}
