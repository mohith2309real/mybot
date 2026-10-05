//! A cancellation flag that can also be awaited: the Stop button.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct Cancel(Arc<(AtomicBool, Notify)>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.0.store(true, Ordering::SeqCst);
        self.0.1.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.0.load(Ordering::SeqCst)
    }

    /// Resolves once `cancel()` has been called (immediately if it already was).
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.1.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wakes_waiters() {
        let c = Cancel::new();
        let c2 = c.clone();
        let h = tokio::spawn(async move { c2.cancelled().await });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        c.cancel();
        h.await.unwrap();
        c.cancelled().await; // already cancelled: returns at once
    }
}
