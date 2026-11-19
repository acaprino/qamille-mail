//! Background storage maintenance (ADR-0007 decision 2).
//!
//! `reconcile_storage` — index recovery, space accounting and the queued mail
//! cache cleanup — is a job of its own: it runs on its own schedule, under its
//! own lock, and never inside a user write (`ServiceContext::update` does not
//! call it). The desktop runs one thread that loops
//! `wait_until_due(); run_once();`; an eviction `Notice` may call `run_once()`
//! directly from any thread — the pass lock serialises the two.

use super::{AppResult, ServiceContext};
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Normal cadence of the maintenance job: one pass a minute, plus a pass on
/// demand when a write reports evicted content (ADR-0007 decision 2).
pub const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
/// Every how-many-th pass also folds the write-ahead log, returns free pages
/// and prunes the change feed (Task 4.3 wiring of review 4.2 minor 3): with
/// the default cadence that is roughly every ten minutes, far away from any
/// user write, and a failing attempt is simply the next pass's job.
const CHECKPOINT_EVERY: u64 = 10;

pub struct MaintenanceService {
    context: Arc<ServiceContext>,
    interval: Duration,
    /// One pass at a time: the job thread and an on-demand pass (an eviction
    /// notice) never run `reconcile_storage` concurrently. This lock is owned
    /// by maintenance alone; no user write ever waits on it.
    pass: Mutex<()>,
    /// When the last pass finished, successful or not: a failing pass must not
    /// turn into a hot retry loop, the store keeps its durable pending marker
    /// and the next scheduled pass picks it up. `None` until the first pass,
    /// so one is due as soon as the service starts.
    completed: Mutex<Option<Instant>>,
    /// Passes run by this service instance, for the checkpoint cadence.
    passes: AtomicU64,
}

impl MaintenanceService {
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self::with_interval(context, MAINTENANCE_INTERVAL)
    }

    pub fn with_interval(context: Arc<ServiceContext>, interval: Duration) -> Self {
        Self {
            context,
            interval,
            pass: Mutex::new(()),
            completed: Mutex::new(None),
            passes: AtomicU64::new(0),
        }
    }

    /// One maintenance pass: `reconcile_storage` including the pending mail
    /// cache cleanup (`ServiceContext::cleanup_mail_cache` is the same call
    /// with the user-facing error wording), under the maintenance lock and
    /// outside of any write transaction. A failure is returned to the caller
    /// and retried by the next pass; it never marks anything lost. Every
    /// `CHECKPOINT_EVERY`-th pass also runs the store's log checkpoint; that
    /// is best-effort and never changes the pass's outcome.
    pub fn run_once(&self) -> AppResult<()> {
        let _pass = self.pass_lock();
        let result = self.context.cleanup_mail_cache();
        let passes = self.passes.fetch_add(1, Ordering::AcqRel) + 1;
        if passes % CHECKPOINT_EVERY == 0 {
            let _ = self.context.repository.storage_checkpoint();
        }
        *self.completed_lock() = Some(Instant::now());
        result
    }

    /// When the next pass is due: `interval` after the last one. `None` means
    /// a pass is due now (nothing ran yet, or the interval has elapsed), so
    /// the maintenance thread sleeps exactly until this instant instead of
    /// polling at a fixed rate (ADR-0007 decision 4: deadlines, not polling).
    pub fn next_due(&self) -> Option<Instant> {
        let due = (*self.completed_lock())? + self.interval;
        (due > Instant::now()).then_some(due)
    }

    /// Sleeps until a pass is due; returns immediately when one already is.
    pub fn wait_until_due(&self) {
        while let Some(due) = self.next_due() {
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    }

    fn pass_lock(&self) -> MutexGuard<'_, ()> {
        self.pass.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn completed_lock(&self) -> MutexGuard<'_, Option<Instant>> {
        self.completed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
