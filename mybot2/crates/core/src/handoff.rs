//! The in-process human handoff: a paused run waits here until the app's
//! "I'm done" / "Skip" buttons answer, or a task row flips from another
//! process (`mybot2 resume`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;

use crate::agent::{Handoff, Pause, Resume};
use crate::cancel::Cancel;
use crate::db::Db;

#[derive(Clone, Default)]
pub struct Handoffs {
    answers: Arc<Mutex<HashMap<String, Resume>>>,
    notify: Arc<Notify>,
    db: Option<Db>,
}

impl Handoffs {
    pub fn new(db: Option<Db>) -> Self {
        Self { db, ..Default::default() }
    }

    /// The human finished the step (or declined it, with `skipped`).
    pub fn resume(&self, task_id: &str, skipped: bool) {
        self.answers.lock().unwrap().insert(task_id.to_string(), if skipped { Resume::Skipped } else { Resume::Done });
        self.notify.notify_waiters();
    }
}

#[async_trait]
impl Handoff for Handoffs {
    async fn wait(&self, task_id: Option<&str>, _pause: &Pause, timeout: Duration, cancel: &Cancel) -> Resume {
        let key = task_id.unwrap_or("-").to_string();
        self.answers.lock().unwrap().remove(&key);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(r) = self.answers.lock().unwrap().remove(&key) {
                return r;
            }
            // Another process may have resumed or cancelled the row. The obstacle
            // itself is never polled — only the human's decision.
            if let (Some(db), Some(t)) = (&self.db, task_id) {
                if let Ok(Some(row)) = db.task(t) {
                    match row.status.as_str() {
                        "running" => return Resume::Done,
                        "cancelled" | "failed" | "done" => return Resume::Cancelled,
                        _ => {}
                    }
                }
            }
            let notified = self.notify.notified();
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                _ = cancel.cancelled() => return Resume::Cancelled,
                _ = tokio::time::sleep_until(deadline) => return Resume::TimedOut,
            }
        }
    }
}
