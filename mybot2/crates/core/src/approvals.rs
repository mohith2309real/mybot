//! Asking you before a saved login is used.
//!
//! Every request leaves a row — including the ones an "always" rule answered —
//! so there is an audit trail of every time a password was used. The row holds
//! the site and username, never the password. Silence is a no: an unanswered
//! request expires and nothing fills. Answers can come from this process (the
//! app's approval card) or another (`mybot2 logins allow` in a terminal), so
//! the wait listens for both.

use std::sync::Arc;
use std::time::Duration;

use mybot_vault::logins::{LoginStore, LoginSummary};
use tokio::sync::{Notify, broadcast};

use crate::cancel::Cancel;
use crate::db::{Db, LoginRequest};

pub const DEFAULT_WAIT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalEvent {
    Requested(LoginRequest),
    Settled(LoginRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AllowedByHuman,
    AllowedByRule,
    Denied,
    TimedOut,
    Cancelled,
}

impl Outcome {
    pub fn allowed(self) -> bool {
        matches!(self, Outcome::AllowedByHuman | Outcome::AllowedByRule)
    }
}

#[derive(Clone)]
pub struct Approvals {
    db: Db,
    logins: Arc<LoginStore>,
    events: broadcast::Sender<ApprovalEvent>,
    settled: Arc<Notify>,
}

impl Approvals {
    pub fn new(db: Db, logins: Arc<LoginStore>) -> Self {
        let (events, _) = broadcast::channel(64);
        Self { db, logins, events, settled: Arc::new(Notify::new()) }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApprovalEvent> {
        self.events.subscribe()
    }

    pub fn pending(&self) -> Vec<LoginRequest> {
        self.db.pending_login_requests().unwrap_or_default()
    }

    pub fn recent(&self, n: usize) -> Vec<LoginRequest> {
        self.db.recent_login_requests(n).unwrap_or_default()
    }

    fn settle(&self, id: &str, status: &str, by: &str, always: bool) -> bool {
        let changed = self.db.settle_login_request(id, status, by, always).unwrap_or(false);
        if changed {
            if let Ok(Some(r)) = self.db.login_request(id) {
                let _ = self.events.send(ApprovalEvent::Settled(r));
            }
            self.settled.notify_waiters();
        }
        changed
    }

    /// You said yes. With `always`, this login fills on its site from now on
    /// without asking.
    pub fn allow(&self, id: &str, always: bool) -> Result<LoginRequest, String> {
        let req = self.db.login_request(id).map_err(|e| e.to_string())?.ok_or("No such sign-in request.")?;
        if req.status != "pending" {
            return Err(format!("That request was already {}.", req.status));
        }
        if always {
            self.logins.set_always_for(&req.origin, &req.username, true).map_err(|e| e.to_string())?;
        }
        self.settle(id, "allowed", "human", always);
        Ok(self.db.login_request(id).ok().flatten().unwrap_or(req))
    }

    pub fn deny(&self, id: &str) -> Result<LoginRequest, String> {
        let req = self.db.login_request(id).map_err(|e| e.to_string())?.ok_or("No such sign-in request.")?;
        if req.status != "pending" {
            return Err(format!("That request was already {}.", req.status));
        }
        self.settle(id, "denied", "human", false);
        Ok(self.db.login_request(id).ok().flatten().unwrap_or(req))
    }

    /// Ask, and wait for the answer.
    pub async fn request(
        &self,
        bot: &str,
        task_id: Option<&str>,
        login: &LoginSummary,
        wait: Duration,
        cancel: &Cancel,
    ) -> Outcome {
        let Ok(id) = self.db.insert_login_request(task_id, bot, &login.origin, &login.username) else {
            return Outcome::Denied;
        };
        if login.always_allow {
            self.settle(&id, "allowed", "rule", true);
            return Outcome::AllowedByRule;
        }
        if let Ok(Some(r)) = self.db.login_request(&id) {
            let _ = self.events.send(ApprovalEvent::Requested(r));
        }

        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Ok(Some(r)) = self.db.login_request(&id) {
                match r.status.as_str() {
                    "allowed" => {
                        return if r.decided_by.as_deref() == Some("rule") { Outcome::AllowedByRule } else { Outcome::AllowedByHuman };
                    }
                    "denied" => return Outcome::Denied,
                    "expired" => return Outcome::TimedOut,
                    _ => {}
                }
            }
            let notified = self.settled.notified();
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(POLL) => {}
                _ = cancel.cancelled() => {
                    self.settle(&id, "denied", "human", false);
                    return Outcome::Cancelled;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    self.settle(&id, "expired", "timeout", false);
                    return Outcome::TimedOut;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mybot_vault::logins::NewLogin;

    async fn next_requested(rx: &mut broadcast::Receiver<ApprovalEvent>) -> LoginRequest {
        loop {
            if let ApprovalEvent::Requested(r) = rx.recv().await.unwrap() {
                return r;
            }
        }
    }

    fn setup() -> (tempfile::TempDir, Approvals, LoginSummary) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(LoginStore::at(dir.path().join("logins.enc")));
        store.unlock("pw").unwrap();
        let login = store.add(NewLogin { url: "github.com", username: "alice", password: "pw1", label: None }).unwrap();
        (dir, Approvals::new(Db::in_memory().unwrap(), store), login)
    }

    #[tokio::test]
    async fn allow_deny_timeout_cancel_always() {
        let (_d, a, login) = setup();
        let cancel = Cancel::new();

        // Silence is a no.
        assert_eq!(a.request("scout", None, &login, Duration::from_millis(50), &cancel).await, Outcome::TimedOut);
        assert_eq!(a.recent(1)[0].status, "expired");

        // Allowed from "the app" while waiting.
        let mut rx = a.subscribe();
        let a2 = a.clone();
        let l2 = login.clone();
        let wait = tokio::spawn(async move { a2.request("scout", None, &l2, Duration::from_secs(5), &Cancel::new()).await });
        let req = next_requested(&mut rx).await;
        a.allow(&req.id, false).unwrap();
        assert_eq!(wait.await.unwrap(), Outcome::AllowedByHuman);
        assert!(a.deny(&req.id).is_err(), "settles once");

        // Denied.
        let a3 = a.clone();
        let l3 = login.clone();
        let wait = tokio::spawn(async move { a3.request("scout", None, &l3, Duration::from_secs(5), &Cancel::new()).await });
        let req = next_requested(&mut rx).await;
        a.deny(&req.id).unwrap();
        assert_eq!(wait.await.unwrap(), Outcome::Denied);

        // Stop withdraws it.
        let c = Cancel::new();
        let c2 = c.clone();
        let a4 = a.clone();
        let l4 = login.clone();
        let wait = tokio::spawn(async move { a4.request("scout", None, &l4, Duration::from_secs(5), &c2).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        c.cancel();
        assert_eq!(wait.await.unwrap(), Outcome::Cancelled);

        // An answer written straight to the row (another process) is seen.
        let a5 = a.clone();
        let l5 = login.clone();
        let wait = tokio::spawn(async move { a5.request("scout", None, &l5, Duration::from_secs(5), &Cancel::new()).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let id = a.pending()[0].id.clone();
        a.db.settle_login_request(&id, "allowed", "human", false).unwrap();
        assert_eq!(wait.await.unwrap(), Outcome::AllowedByHuman);

        // "Always": next time nobody is asked, and it is still audited.
        let mut always = login.clone();
        always.always_allow = true;
        assert_eq!(a.request("scout", None, &always, Duration::from_secs(5), &cancel).await, Outcome::AllowedByRule);
        assert_eq!(a.recent(1)[0].decided_by.as_deref(), Some("rule"));
    }
}
