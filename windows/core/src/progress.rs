//! Cancel flags and progress throttling shared by every long-running job.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A cheap, cloneable cancel flag. The Tauri layer keeps one per job kind
/// (`cancel_duplicates`, `cancel_classify`, `cancel_model_download` call [`Cancel::cancel`]).
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    /// Clear the flag before starting a new run with the same handle.
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    /// `Err(Cancelled)` when cancelled — for `?` inside loops.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Returned (inside `anyhow::Error`) when a job stopped because its [`Cancel`] flag was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;

/// True when the error (or its cause) is [`Cancelled`].
pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.downcast_ref::<Cancelled>().is_some())
}

/// Lets a progress callback fire at most every `every` (the first call always fires).
pub(crate) struct Throttle {
    every: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub(crate) fn new(every: Duration) -> Self {
        Throttle { every, last: None }
    }
    pub(crate) fn ready(&mut self) -> bool {
        let now = Instant::now();
        match self.last {
            Some(t) if now.duration_since(t) < self.every => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// A progress callback that ignores everything (handy for tests and internal rescans).
pub fn no_progress<T: ?Sized>(_: &T) {}
