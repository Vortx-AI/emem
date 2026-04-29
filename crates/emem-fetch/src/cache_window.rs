//! In-flight coalesce window for source fetches.
//!
//! Two agents asking for the same source tile within a short window
//! share a single in-flight fetch. Concurrent callers `await` on the
//! same `tokio::sync::Notify` and observe the cached bytes when the
//! winner returns.
//!
//! The coalesce key is intentionally caller-provided (typically the
//! resolved source URL) so the dispatcher can decide whether two
//! fetches are equivalent (e.g. ignoring caller-specific query strings).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

use crate::FetchResponse;

/// Coalesce window. Holds a single in-flight fetch per key.
pub struct CoalesceWindow {
    inner: Mutex<HashMap<String, Slot>>,
}

struct Slot {
    notify: Arc<Notify>,
    result: Option<Result<FetchResponse, String>>,
}

impl CoalesceWindow {
    /// Build a fresh empty window.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Register interest in `key`. If a fetch is already in-flight the
    /// returned `Coalesced::Waiter` blocks until the winner publishes.
    /// Otherwise the returned `Coalesced::Owner` indicates the caller
    /// is the winner and must perform the fetch and call `publish` when
    /// it's done.
    pub async fn enter(&self, key: &str) -> Coalesced {
        let mut g = self.inner.lock().await;
        if let Some(slot) = g.get(key) {
            let notify = slot.notify.clone();
            return Coalesced::Waiter {
                key: key.to_string(),
                notify,
            };
        }
        let notify = Arc::new(Notify::new());
        g.insert(
            key.to_string(),
            Slot {
                notify: notify.clone(),
                result: None,
            },
        );
        Coalesced::Owner {
            key: key.to_string(),
            notify,
        }
    }

    /// Publish the fetch result, wake all waiters, and clear the slot.
    pub async fn publish(&self, key: &str, result: Result<FetchResponse, String>) {
        let mut g = self.inner.lock().await;
        if let Some(slot) = g.get_mut(key) {
            slot.result = Some(result);
            slot.notify.notify_waiters();
        }
        // We deliberately do not remove the slot here — waiters that arrive
        // *after* publish but before we clear should still see the cached
        // result (handled by the Waiter::wait path). Removal happens in the
        // owner's drop guard via `release`.
    }

    /// Release the slot (the owner calls this once all waiters have woken).
    pub async fn release(&self, key: &str) {
        let mut g = self.inner.lock().await;
        g.remove(key);
    }

    /// Try to get the cached result for a waiter without holding the
    /// outer mutex while awaiting the notify.
    pub async fn cached(&self, key: &str) -> Option<Result<FetchResponse, String>> {
        let g = self.inner.lock().await;
        g.get(key).and_then(|s| s.result.clone())
    }
}

impl Default for CoalesceWindow {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of [`CoalesceWindow::enter`].
pub enum Coalesced {
    /// First caller for this key. Perform the fetch and call `publish`.
    Owner { key: String, notify: Arc<Notify> },
    /// Subsequent caller. `wait` on the returned future.
    Waiter { key: String, notify: Arc<Notify> },
}

impl Coalesced {
    /// Key this coalesce slot represents.
    pub fn key(&self) -> &str {
        match self {
            Coalesced::Owner { key, .. } => key,
            Coalesced::Waiter { key, .. } => key,
        }
    }
    /// Notify handle to await on (waiter) or signal on (owner).
    pub fn notify(&self) -> Arc<Notify> {
        match self {
            Coalesced::Owner { notify, .. } => notify.clone(),
            Coalesced::Waiter { notify, .. } => notify.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_caller_is_owner_second_is_waiter() {
        let cw = CoalesceWindow::new();
        let a = cw.enter("k").await;
        let b = cw.enter("k").await;
        assert!(matches!(a, Coalesced::Owner { .. }));
        assert!(matches!(b, Coalesced::Waiter { .. }));
    }
}
