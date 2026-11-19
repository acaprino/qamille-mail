//! Coordinates push-triggered inbox downloads without holding storage locks while idling.
//!
//! Task 3.3 (ADR-0007 decision 3): this service no longer syncs inline. It
//! keeps the per-account IDLE watchers, coalesces their dirty signals and
//! dispatches one [`Job::Sync`] per affected account to that account's worker
//! in the [`WorkerRegistry`], so a slow or failing provider can never delay
//! another account's download and every socket-opening job belongs to a
//! counted worker (INV-002, INV-003). A submission the worker refuses (busy or
//! throttled queue) re-arms the dirty flag instead of dropping the account:
//! the producer respects admission and retries on a later step.
use super::{
    AppResult, ServiceContext,
    account_worker::{Job, Priority, WorkerRegistry},
};
use crate::model::{Account, AccountKind};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Timing {
    refresh_accounts: Duration,
    fallback: Duration,
    wait: Duration,
    retry_min: Duration,
    retry_max: Duration,
    session_lifetime: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            refresh_accounts: Duration::from_secs(5),
            fallback: Duration::from_secs(300),
            wait: Duration::from_secs(30),
            retry_min: Duration::from_secs(1),
            retry_max: Duration::from_secs(60),
            session_lifetime: Duration::from_secs(50 * 60),
        }
    }
}
#[derive(Default)]
struct Signals {
    dirty: AtomicBool,
    error: Mutex<Option<String>>,
}
/// One account's IDLE listener: it never downloads, it only marks the account
/// dirty and wakes the coordinator. The download runs on the account's
/// registry worker, which owns the sync connection (INV-003).
struct Watcher {
    account: Account,
    stop: Arc<AtomicBool>,
    signals: Arc<Signals>,
    thread: std::thread::JoinHandle<()>,
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct LiveSyncReport {
    /// At least one `Job::Sync` reached an account worker during this step.
    pub updated: bool,
    pub errors: Vec<String>,
}

pub struct MailSyncService {
    context: Arc<ServiceContext>,
    registry: Arc<WorkerRegistry>,
    watchers: HashMap<u32, Watcher>,
    wake_tx: mpsc::SyncSender<()>,
    wake_rx: mpsc::Receiver<()>,
    timing: Timing,
    next_accounts: Instant,
    next_fallback: Instant,
}
impl MailSyncService {
    pub fn new(context: Arc<ServiceContext>, registry: Arc<WorkerRegistry>) -> Self {
        Self::with_timing(context, registry, Timing::default())
    }
    fn with_timing(
        context: Arc<ServiceContext>,
        registry: Arc<WorkerRegistry>,
        timing: Timing,
    ) -> Self {
        let (wake_tx, wake_rx) = mpsc::sync_channel(1);
        Self {
            context,
            registry,
            watchers: HashMap::new(),
            wake_tx,
            wake_rx,
            timing,
            next_accounts: Instant::now(),
            next_fallback: Instant::now(),
        }
    }
    /// Wait for notifications, coalesce bursts and dispatch the affected
    /// accounts to their workers.
    pub fn step(&mut self, timeout: Duration) -> AppResult<LiveSyncReport> {
        if Instant::now() >= self.next_accounts {
            self.refresh_accounts()?;
            self.next_accounts = Instant::now() + self.timing.refresh_accounts;
        }
        if Instant::now() < self.next_fallback {
            let delay = timeout
                .min(self.next_accounts.saturating_duration_since(Instant::now()))
                .min(self.next_fallback.saturating_duration_since(Instant::now()));
            let _ = self.wake_rx.recv_timeout(delay);
        }
        let mut report = LiveSyncReport {
            updated: false,
            errors: Vec::new(),
        };
        let mut changed = Vec::new();
        for (&id, watcher) in &self.watchers {
            if watcher.signals.dirty.swap(false, Ordering::AcqRel) {
                changed.push(id);
            }
            if let Some(error) = watcher
                .signals
                .error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                report
                    .errors
                    .push(format!("{}: {error}", watcher.account.email));
            }
        }
        if Instant::now() >= self.next_fallback {
            self.next_fallback = Instant::now() + self.timing.fallback;
            // The 300 s recovery download is per account too (INV-002): each
            // account's own worker runs it, so one offline provider cannot
            // delay the others. Accounts whose provider cannot sync are
            // skipped: dispatching them would only fail their worker job.
            let store = self.context.read_shared()?;
            for account in store
                .accounts
                .iter()
                .filter(|a| self.context.mail.capabilities(a).sync)
            {
                if !changed.contains(&account.id) {
                    changed.push(account.id);
                }
            }
        }
        for id in changed {
            self.dispatch(id, &mut report);
        }
        Ok(report)
    }
    /// Hands one account's sync to its worker. A refusal re-arms the dirty
    /// flag and wakes a later step instead of dropping the account or forcing
    /// the queue: the producer respects admission (review 3.1 Important 1).
    fn dispatch(&mut self, id: u32, report: &mut LiveSyncReport) {
        match self.registry.submit(id, Priority::Sync, Job::Sync) {
            Ok(()) => {
                report.updated = true;
                // The history follows every sync: the walk until it is complete
                // (W3.1), then the label refresh (W3.2). The worker runs it after
                // everything else and keeps one queued at most.
                if super::history::due(&self.context, id).unwrap_or(false) {
                    let _ = self.registry.submit(id, Priority::History, Job::Backfill);
                }
            }
            // Account removal can race a queued IDLE notification. Stop that
            // watcher instead of treating it as a sync failure.
            Err(error)
                if error.code == "not_found"
                    && self
                        .context
                        .read()
                        .map(|store| store.account(id).is_none())
                        .unwrap_or(false) =>
            {
                self.watchers.remove(&id);
            }
            Err(_) => {
                if let Some(watcher) = self.watchers.get(&id) {
                    watcher.signals.dirty.store(true, Ordering::Release);
                    let _ = self.wake_tx.try_send(());
                }
            }
        }
    }
    fn refresh_accounts(&mut self) -> AppResult<()> {
        let store = self.context.read_shared()?;
        // Workers must exist before submits can route to them; reconcile also
        // stops the workers of removed accounts. Idempotent: the undo thread
        // reconciles too.
        self.registry.reconcile(&store.accounts)?;
        self.watchers.retain(|id, watcher| {
            store
                .accounts
                .iter()
                .any(|a| a.id == *id && same_connection(a, &watcher.account))
                && !watcher.thread.is_finished()
        });
        for account in store
            .accounts
            .iter()
            .cloned()
            .filter(|a| a.kind == AccountKind::Gmail)
        {
            if self.watchers.contains_key(&account.id) {
                continue;
            }
            let stop = Arc::new(AtomicBool::new(false));
            let signals = Arc::new(Signals::default());
            // A newly connected account must fetch even if the IDLE connection
            // is slow or unavailable. Do not make its first sync wait for IDLE.
            signals.dirty.store(true, Ordering::Release);
            let _ = self.wake_tx.try_send(());
            let (context, watcher_stop, watcher_signals, wake, timing, id) = (
                self.context.clone(),
                stop.clone(),
                signals.clone(),
                self.wake_tx.clone(),
                self.timing,
                account.id,
            );
            let thread = std::thread::Builder::new()
                .name(format!("gmail-idle-{id}"))
                .spawn(move || {
                    watch_account(context, id, watcher_stop, watcher_signals, wake, timing);
                })?;
            self.watchers.insert(
                id,
                Watcher {
                    account,
                    stop,
                    signals,
                    thread,
                },
            );
        }
        Ok(())
    }
}
fn same_connection(a: &Account, b: &Account) -> bool {
    a.kind == AccountKind::Gmail
        && a.email == b.email
        && a.imap_host == b.imap_host
        && a.imap_port == b.imap_port
        && a.username == b.username
        && a.oauth_client_id == b.oauth_client_id
}
fn watch_account(
    context: Arc<ServiceContext>,
    id: u32,
    stop: Arc<AtomicBool>,
    signals: Arc<Signals>,
    wake: mpsc::SyncSender<()>,
    timing: Timing,
) {
    let mut retry = timing.retry_min;
    let mut reported_failure = false;
    while !stop.load(Ordering::Relaxed) {
        let result = (|| {
            let store = context.read()?;
            let account = context.account(&store, id)?;
            let mut watch = context.mail.watch(&account)?;
            // The selected mailbox now precedes the catch-up fetch, closing the reconnect gap.
            signals.dirty.store(true, Ordering::Release);
            let _ = wake.try_send(());
            let connected = Instant::now();
            while !stop.load(Ordering::Relaxed) && connected.elapsed() < timing.session_lifetime {
                let changed = watch.wait(timing.wait)?;
                // A completed heartbeat also proves this is a healthy connection.
                retry = timing.retry_min;
                reported_failure = false;
                if changed {
                    signals.dirty.store(true, Ordering::Release);
                    let _ = wake.try_send(());
                }
            }
            Ok::<_, super::AppError>(())
        })();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = result {
            if !reported_failure {
                *signals.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!(
                    "Live Gmail sync interrupted; reconnecting automatically. {}",
                    error.message
                ));
                let _ = wake.try_send(());
                reported_failure = true;
            }
            let deadline = Instant::now() + retry;
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(100)),
                );
            }
            retry = retry.saturating_mul(2).min(timing.retry_max);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/support/live_sync.rs"]
mod tests;
