//! One worker per account (ADR-0007 decision 3, INV-003): every mail job of an
//! account runs on its own thread, ordered by a priority queue, so a slow sync
//! of one account never delays a user action on another (EX-004) and no job
//! executes network calls under a lock shared between accounts (INV-001 — the
//! `MailService` phase functions already keep the provider call outside the
//! account lock).
//!
//! Task 3.2 (decision 4): the worker is also the executor of the undo
//! confirmation queue. `UndoService::commit_due` claims due actions and submits
//! them as `Priority::UserAction` jobs; the worker runs them and hands the
//! result back through [`CommitOutcome`], so the queue — not the worker — owns
//! the retry policy (INV-010). Draft confirmations run on one dedicated drafts
//! worker ([`DRAFTS_ACCOUNT`]) with the same structure: a slow SMTP delivery
//! never delays a mail confirmation of any account.
//!
//! IMAP sessions: this worker does not own a session pool, and no per-account
//! connection cap is enforced yet. It calls the public `MailService` APIs, which
//! read through `MailReader`'s existing pool (≤2 sessions); the persistent IDLE
//! session (1), the live sync path's own connection (1) and the action session
//! of a confirmation (1) are opened outside it. The real current budget is
//! therefore up to 5 per account (reader ≤2 + IDLE 1 + live sync 1 + action 1),
//! within Gmail's limit of 15. Enforcing a cap (INV-003) arrives with Task 3.3,
//! when the worker owns every per-account session.
use super::{
    AppError, AppResult, ServiceContext,
    drafts::DraftService,
    events::AppEvent,
    mail::{MailQuery, MailService},
    ports::EventSink,
};
use crate::domain::model::Account;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{self, RecvTimeoutError, SyncSender, TrySendError},
};
use std::time::{Duration, Instant};

/// Queue slots per account. `submit` reports `queue_full` instead of blocking
/// the caller (typically a UI or command thread) when a worker is saturated.
const QUEUE_CAPACITY: usize = 64;
/// Priority-aware admission (controller addition 2026-11-05, review 3.1
/// Important 1): once ~75% of the channel slots hold outstanding jobs, `Sync`
/// and `Background` submissions are refused with `queue_throttled`, reserving
/// the last quarter of the queue for `UserAction` and `ReadBody`, which are
/// always admitted.
const ADMISSION_LIMIT: usize = QUEUE_CAPACITY * 3 / 4;
/// The thread wakes on channel arrival; this timeout only bounds how long an
/// idle worker waits before re-checking the stop flag, so `stop()` can join.
/// It is not a work-polling interval (INV-005 concerns deadline loops).
const IDLE_TICK: Duration = Duration::from_millis(50);
/// `stop()` lets the in-flight job finish, then joins for at most this long;
/// a longer job leaves the thread to finish on its own (detached).
const STOP_JOIN_TIMEOUT: Duration = Duration::from_secs(5);
/// Rows per page warmed by [`Job::Index`], matching the `MailService::list`
/// default page size.
const INDEX_PAGE_ROWS: usize = 60;
/// Synthetic account id of the shared drafts worker (Task 3.2). Draft
/// confirmations (`SendDraft`/`DiscardDraft`/`ScheduleDraft`) do not belong to
/// one mail account's job stream — a slow send must not delay that account's
/// mail commits — so the registry always keeps one dedicated worker under this
/// id, with the same structure as an account's. Real accounts are numbered by
/// `Store::new_id` from 1 upward and never reach `u32::MAX`.
pub const DRAFTS_ACCOUNT: u32 = u32::MAX;

/// Queue order: user actions first, then body reads, then sync, then
/// prefetch/indexing (ADR-0007 decision 3). Lower discriminant = higher
/// priority; the queue stores `Reverse` so the smallest pops first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    UserAction = 0,
    ReadBody = 1,
    Sync = 2,
    Background = 3,
    /// The history backfill (W3.1): after everything else, every time.
    History = 4,
}

/// One unit of work for a single account, executed through the public
/// `MailService` phase functions of Tasks 1.2/1.3 and, for the draft variants,
/// through `DraftService`.
///
/// Queue order is decided solely by `(priority, seq)` in [`Queued`], so a job
/// never needs to be comparable itself and can carry non-comparable payloads.
pub enum Job {
    /// A user action batch: `MailService::apply_actions` (claim → plan under
    /// the account lock → provider calls with no lock → commit), which also
    /// removes `pending_ids` from the undo queue on commit.
    Act {
        email_id: u64,
        commands: Vec<(String, Option<String>)>,
        pending_ids: Vec<u64>,
    },
    /// `MailService::sync_account` for this worker's account.
    Sync,
    /// One batch of the history backfill (W3.1); the worker queues the next
    /// one itself while its queue is otherwise empty.
    Backfill,
    /// Download and cache one body: `MailService::read`.
    ReadBody { email_id: u64 },
    /// Warm bodies for a scroll selection: `MailService::prefetch`.
    Prefetch { email_ids: Vec<u64> },
    /// Warm one inbox page: `MailService::list`.
    Index { page: u32 },
    /// A queued draft delivery: `DraftService::send` (runs on the drafts
    /// worker; `pending_ids` are the undo rows the outcome settles).
    SendDraft { draft_id: u64, pending_ids: Vec<u64> },
    /// A queued draft deletion: `DraftService::discard` (drafts worker).
    DiscardDraft { draft_id: u64, pending_ids: Vec<u64> },
    /// A queued send-later: sets the draft's `send_at` (drafts worker).
    ScheduleDraft {
        draft_id: u64,
        send_at: i64,
        pending_ids: Vec<u64>,
    },
}

fn job_label(job: &Job) -> &'static str {
    match job {
        Job::Act { .. } => "action",
        Job::Sync => "sync",
        Job::Backfill => "history",
        Job::ReadBody { .. } => "read",
        Job::Prefetch { .. } => "prefetch",
        Job::Index { .. } => "index",
        Job::SendDraft { .. } => "send",
        Job::DiscardDraft { .. } => "discard",
        Job::ScheduleDraft { .. } => "schedule",
    }
}

/// The undo-queue rows a job settles. Non-empty exactly for confirmations
/// dispatched by `UndoService::commit_due`: their outcome is reported through
/// [`CommitOutcome`] instead of the generic failure `Notice`, because the
/// queue rows are what the UI reads (a retrying confirmation is not a failure
/// to toast).
fn job_pending_ids(job: &Job) -> &[u64] {
    match job {
        Job::Act { pending_ids, .. }
        | Job::SendDraft { pending_ids, .. }
        | Job::DiscardDraft { pending_ids, .. }
        | Job::ScheduleDraft { pending_ids, .. } => pending_ids,
        Job::Sync | Job::Backfill | Job::ReadBody { .. } | Job::Prefetch { .. } | Job::Index { .. } => &[],
    }
}

/// Where an undo-driven job reports its result (Task 3.2, ADR-0007 decision 4).
/// The worker executes; the confirmation queue settles: `UndoService`
/// implements this and applies its own retry policy (INV-010) to the rows it
/// claimed, so no undo state lives in the worker. Called exactly once per
/// dispatched confirmation, on the worker thread, with the job's result — or
/// with a definitive `worker_crash` error when the job panicked (a crash mid
/// mutation is a lost outcome and must never be retried, INV-008).
pub trait CommitOutcome: Send + Sync + 'static {
    fn commit_finished(&self, pending_ids: &[u64], result: &AppResult<()>);
}

/// A queued item. The queue order is `(priority, submission sequence)` alone —
/// `seq` is unique, so the order is total and FIFO holds among equal
/// priorities; the job itself never decides queue order.
struct Queued {
    priority: Priority,
    seq: u64,
    job: Job,
}
impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for Queued {}
impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Queued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.priority, self.seq).cmp(&(other.priority, other.seq))
    }
}

/// How a worker is named in notices: the drafts worker is not an account.
fn account_label(account_id: u32) -> String {
    if account_id == DRAFTS_ACCOUNT {
        "Drafts".to_string()
    } else {
        format!("Account {account_id}")
    }
}

/// The per-account executor. Cheap to keep in a registry; dropping it stops
/// the worker.
pub struct AccountWorker {
    account_id: u32,
    tx: SyncSender<Queued>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    stop: Arc<AtomicBool>,
    seq: AtomicU64,
    /// Jobs accepted into the channel but not yet pulled into the worker's
    /// heap: the occupancy gauge of the priority-aware admission in `submit`.
    outstanding: Arc<AtomicUsize>,
}

impl AccountWorker {
    /// Start the worker thread for one account (or the drafts worker under
    /// [`DRAFTS_ACCOUNT`]). Jobs are submitted through
    /// [`submit`](Self::submit); undo confirmations report their result to
    /// `outcome`, other failures are announced on `events`. `mail` and
    /// `drafts` must be the process-shared service instances so the per
    /// account locks and the draft operation lock stay unique.
    pub fn spawn(
        account_id: u32,
        mail: MailService,
        drafts: DraftService,
        context: Arc<ServiceContext>,
        events: Arc<dyn EventSink>,
        outcome: Arc<dyn CommitOutcome>,
    ) -> AppResult<Self> {
        let (tx, rx) = mpsc::sync_channel::<Queued>(QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let outstanding = Arc::new(AtomicUsize::new(0));
        let handle = std::thread::Builder::new()
            .name(format!("qamille-worker-{account_id}"))
            .spawn({
                let stop = stop.clone();
                let outstanding = outstanding.clone();
                move || run(account_id, mail, drafts, context, events, outcome, rx, stop, outstanding)
            })
            .map_err(|error| AppError::new("worker_spawn", error.to_string()))?;
        Ok(Self {
            account_id,
            tx,
            handle: Mutex::new(Some(handle)),
            stop,
            seq: AtomicU64::new(0),
            outstanding,
        })
    }
    /// The account this worker serves (INV-003: one thread, one account;
    /// [`DRAFTS_ACCOUNT`] for the shared drafts worker).
    pub fn account_id(&self) -> u32 {
        self.account_id
    }
    /// Whether the worker thread has exited — stopped on purpose, or killed by
    /// something that escaped the loop's panic guards. `reconcile` respawns
    /// such workers (controller addition 2026-11-05).
    fn is_finished(&self) -> bool {
        self.handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|handle| handle.is_finished())
    }
    /// Enqueue a job. Fails without writing anything when the worker is
    /// stopping, has stopped, or the queue is full. `Sync` and `Background`
    /// are additionally refused with `queue_throttled` once ~75% of the
    /// channel is outstanding, keeping headroom for `UserAction`/`ReadBody`.
    pub fn submit(&self, priority: Priority, job: Job) -> AppResult<()> {
        if self.stop.load(Ordering::Acquire) {
            return Err(AppError::new(
                "worker_stopped",
                "This account worker is stopping; the job was not queued",
            ));
        }
        if matches!(priority, Priority::Sync | Priority::Background | Priority::History)
            && self.outstanding.load(Ordering::Acquire) >= ADMISSION_LIMIT
        {
            return Err(AppError::new(
                "queue_throttled",
                "This account's mail queue is busy; background work is deferred so user actions keep their headroom",
            ));
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(Queued { priority, seq, job }) {
            Ok(()) => {
                self.outstanding.fetch_add(1, Ordering::Release);
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err(AppError::new(
                "queue_full",
                "The mail queue of this account is full; retry shortly",
            )),
            Err(TrySendError::Disconnected(_)) => Err(AppError::new(
                "worker_stopped",
                "This account worker is no longer running",
            )),
        }
    }
    /// Stop the worker: queued jobs are drained without executing (announced
    /// by one notice), the in-flight job is left to finish, and the thread is
    /// joined for at most [`STOP_JOIN_TIMEOUT`]. Idempotent.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        let Some(handle) = self
            .handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        // std has no timed join: wait bounded, then detach. The stop flag
        // already guarantees no further job executes.
        let deadline = Instant::now() + STOP_JOIN_TIMEOUT;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
}

impl Drop for AccountWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Registry keyed by account: `reconcile` keeps one live worker per account
/// plus the drafts worker, `submit` routes a job to its account's worker or
/// fails with "Account".
pub struct WorkerRegistry {
    mail: MailService,
    drafts: DraftService,
    context: Arc<ServiceContext>,
    events: Arc<dyn EventSink>,
    outcome: Arc<dyn CommitOutcome>,
    /// `Arc` so `stop_worker` can join outside the registry lock.
    workers: Mutex<HashMap<u32, Arc<AccountWorker>>>,
}

impl WorkerRegistry {
    /// The registry spawns workers with clones of these; `mail` and `drafts`
    /// must be the shared service instances of the process so per-account
    /// locks and the draft operation lock stay unique, and `outcome` is the
    /// `UndoService` the dispatched confirmations settle against.
    pub fn new(
        mail: MailService,
        drafts: DraftService,
        context: Arc<ServiceContext>,
        events: Arc<dyn EventSink>,
        outcome: Arc<dyn CommitOutcome>,
    ) -> Self {
        Self {
            mail,
            drafts,
            context,
            events,
            outcome,
            workers: Mutex::new(HashMap::new()),
        }
    }
    /// Spawn a worker for every account that has none, respawn workers whose
    /// thread has finished (crashed or stopped), keep the drafts worker alive,
    /// and stop the workers of accounts no longer listed. Stops run outside
    /// the registry lock where they can block: joining an in-flight job must
    /// not block submits to other accounts.
    pub fn reconcile(&self, accounts: &[Account]) -> AppResult<()> {
        let stale = {
            let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
            let wanted: HashSet<u32> = accounts.iter().map(|account| account.id).collect();
            let stale_ids: Vec<u32> = workers
                .keys()
                .copied()
                .filter(|id| *id != DRAFTS_ACCOUNT && !wanted.contains(id))
                .collect();
            let stale: Vec<Arc<AccountWorker>> = stale_ids
                .into_iter()
                .filter_map(|id| workers.remove(&id))
                .collect();
            for id in wanted
                .iter()
                .copied()
                .chain(std::iter::once(DRAFTS_ACCOUNT))
            {
                if workers.get(&id).is_some_and(|worker| !worker.is_finished()) {
                    continue;
                }
                // A finished thread joins at once, so replacing it here cannot
                // block submits to the other accounts.
                if let Some(dead) = workers.remove(&id) {
                    dead.stop();
                }
                let worker = AccountWorker::spawn(
                    id,
                    self.mail.clone(),
                    self.drafts.clone(),
                    self.context.clone(),
                    self.events.clone(),
                    self.outcome.clone(),
                )?;
                workers.insert(id, Arc::new(worker));
            }
            stale
        };
        for worker in stale {
            worker.stop();
        }
        Ok(())
    }
    /// Route a job to the worker of `account_id` ([`DRAFTS_ACCOUNT`] for the
    /// drafts worker). Unknown or removed accounts fail with `not_found`
    /// ("Account …") and nothing is written. `try_send` inside keeps this
    /// non-blocking, so the registry lock is never held while a queue is full.
    pub fn submit(&self, account_id: u32, p: Priority, job: Job) -> AppResult<()> {
        let workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        let worker = workers
            .get(&account_id)
            .ok_or_else(|| AppError::missing("Account"))?;
        worker.submit(p, job)
    }
    /// Stop the worker of one account in place, keeping its entry: the thread
    /// finishes and the next `reconcile` respawns a fresh one. Recycling seam
    /// for a worker whose thread died; returns false when the account has no
    /// worker. The bounded join runs outside the registry lock.
    pub fn stop_worker(&self, account_id: u32) -> bool {
        let worker = self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&account_id)
            .cloned();
        match worker {
            Some(worker) => {
                worker.stop();
                true
            }
            None => false,
        }
    }
}

/// The worker loop: pull the best queued job, run it, report the result, and
/// never exit on a job error or panic (EX-004). Returns on the stop flag or a
/// disconnected channel; jobs still queued are then dropped without executing
/// and counted for one closing notice.
#[allow(clippy::too_many_arguments)]
fn run(
    account_id: u32,
    mail: MailService,
    drafts: DraftService,
    context: Arc<ServiceContext>,
    events: Arc<dyn EventSink>,
    outcome: Arc<dyn CommitOutcome>,
    rx: mpsc::Receiver<Queued>,
    stop: Arc<AtomicBool>,
    outstanding: Arc<AtomicUsize>,
) {
    let mut heap: BinaryHeap<Reverse<Queued>> = BinaryHeap::new();
    // A job popped for execution but abandoned because the stop flag was set.
    let mut popped = 0;
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if heap.is_empty() {
            match rx.recv_timeout(IDLE_TICK) {
                Ok(item) => {
                    outstanding.fetch_sub(1, Ordering::AcqRel);
                    heap.push(Reverse(item));
                }
                Err(RecvTimeoutError::Timeout) => continue,
                // All senders are gone: the worker value was dropped.
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Move everything that arrived into the heap so the highest-priority
        // job wins, not the oldest.
        while let Ok(item) = rx.try_recv() {
            outstanding.fetch_sub(1, Ordering::AcqRel);
            heap.push(Reverse(item));
        }
        let Some(Reverse(queued)) = heap.pop() else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            popped = 1;
            break;
        }
        let job = queued.job;
        let backfill = matches!(job, Job::Backfill);
        // One history batch at a time: a duplicate already queued is enough.
        if backfill && heap.iter().any(|Reverse(q)| matches!(q.job, Job::Backfill)) {
            continue;
        }
        let label = job_label(&job);
        let pending = job_pending_ids(&job).to_vec();
        // A failed or even panicking job never kills this account's worker.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_job(account_id, &mail, &drafts, &context, job)
        }));
        let where_ = account_label(account_id);
        // The next history batch waits behind any other work (EX-10).
        if matches!(result, Ok(Ok(true))) {
            if !heap.iter().any(|Reverse(q)| matches!(q.job, Job::Backfill)) {
                heap.push(Reverse(Queued { priority: Priority::History, seq: queued.seq, job: Job::Backfill }));
            }
        }
        match result {
            Ok(result) => {
                if pending.is_empty() {
                    if let Err(error) = result {
                        announce(&events, format!("{where_}: {label} failed: {}", error.message));
                    }
                } else {
                    // The confirmation queue settles its own rows and announces
                    // them through its events; a guarded call, so a panicking
                    // handler cannot take the worker down either.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        outcome.commit_finished(&pending, &result.map(|_| ()))
                    }));
                }
            }
            Err(_) if pending.is_empty() => announce(
                &events,
                format!("{where_}: {label} crashed and was skipped; the worker continues"),
            ),
            Err(_) => {
                // A crash mid-confirmation is a lost outcome: settled as
                // definitive, never retried (INV-008).
                let crash = Err(AppError::new(
                    "worker_crash",
                    format!(
                        "The queued {label} stopped unexpectedly; its result is unknown. \
                         Check the provider before trying again."
                    ),
                ));
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    outcome.commit_finished(&pending, &crash)
                }));
            }
        }
    }
    // Jobs still queued at the stop are dropped without executing; say so once
    // instead of losing them silently (controller addition 2026-11-05).
    let mut dropped = heap.len() + popped;
    heap.clear();
    while rx.try_recv().is_ok() {
        outstanding.fetch_sub(1, Ordering::AcqRel);
        dropped += 1;
    }
    if dropped > 0 {
        announce(
            &events,
            format!("{}: stopped with {dropped} queued job(s) dropped", account_label(account_id)),
        );
    }
}

/// One notice emission that a failing sink can never turn into a dead worker
/// (controller addition 2026-11-05: the post-failure emit is itself guarded).
fn announce(events: &Arc<dyn EventSink>, text: String) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        events.emit(AppEvent::Notice { text });
    }));
}

/// Execute one job through the public service APIs. The worker holds no lock
/// here: `apply_actions` and `sync_account` take the per-account lock only for
/// their short store phases, `DraftService` its operation lock (INV-001).
fn run_job(
    account_id: u32,
    mail: &MailService,
    drafts: &DraftService,
    context: &Arc<ServiceContext>,
    job: Job,
) -> AppResult<bool> {
    match job {
        // The sync writer (`MailService::sync_selected`) is the single emitter
        // of `SyncProgress` (ADR-0007 decision 5; controller seam fix
        // 2026-11-05): the worker adds nothing, so progress is never announced
        // twice per sync. Collected provider errors still surface as the job's
        // failure, which the loop announces once.
        Job::Backfill => return super::history::run_batch(context, account_id),
        Job::Sync => mail.sync_account(account_id).and_then(|report| {
            if report.errors.is_empty() {
                Ok(())
            } else {
                Err(AppError::external(report.errors.join("; ")))
            }
        }),
        Job::Act {
            email_id,
            commands,
            pending_ids,
        } => {
            let refs: Vec<(&str, Option<&str>)> = commands
                .iter()
                .map(|(action, value)| (action.as_str(), value.as_deref()))
                .collect();
            mail.apply_actions(email_id, &refs, &pending_ids)
        }
        Job::ReadBody { email_id } => mail.read(email_id).map(|_| ()),
        Job::Prefetch { email_ids } => mail.prefetch(email_ids),
        Job::Index { page } => mail
            .list(MailQuery {
                account_id: Some(account_id),
                offset: page as usize * INDEX_PAGE_ROWS,
                limit: INDEX_PAGE_ROWS,
                ..Default::default()
            })
            .map(|_| ()),
        Job::SendDraft { draft_id, .. } => drafts.send(draft_id),
        Job::DiscardDraft { draft_id, .. } => drafts.discard(draft_id),
        Job::ScheduleDraft {
            draft_id, send_at, ..
        } => {
            let committed = context.update_with_events(|store| {
                let draft = store
                    .drafts
                    .iter_mut()
                    .find(|d| d.id == draft_id)
                    .ok_or_else(|| AppError::missing("Draft"))?;
                draft.send_at = Some(send_at);
                draft.updated = context.clock.now().max(draft.updated + 1);
                Ok((
                    (),
                    vec![AppEvent::DraftsChanged {
                        ids: vec![draft_id],
                    }],
                ))
            });
            if committed.is_ok() {
                // The settled send-later is a new scheduler deadline (INV-005).
                super::scheduler::note_schedule_change();
            }
            committed
        }
    }?;
    Ok(false)
}
