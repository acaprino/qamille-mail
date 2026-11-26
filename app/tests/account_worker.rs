//! Task 3.1 (ADR-0007 decision 3): per-account worker with a priority queue.
//! - `user_action_preempts_queued_sync`: a queued user action runs before the
//!   second of three barrier-slowed syncs.
//! - `error_in_one_account_does_not_stop_another` (EX-004): a failing account
//!   reports once per job and keeps working; the other account is unaffected.
//! - Review Focus 3: `reconcile` without an account stops its worker and a
//!   later `submit` fails with "Account" without writing anything.
//! - `stop` drains: no queued or new job runs after it.
//!
//! Task 3.2 additions (controller 2026-11-05):
//! - Seam fix from 2.2b: `sync_selected` is the single `SyncProgress` emitter
//!   (decision 5) — exactly one Syncing + one terminal phase per Sync job.
//! - Priority-aware admission: a nearly saturated queue refuses Sync and
//!   Background jobs (`queue_throttled`) while UserAction/ReadBody still pass.
//! - `reconcile` respawns workers whose thread has finished, and keeps the
//!   dedicated drafts worker (`DRAFTS_ACCOUNT`) alive.
//! - A stopped worker announces the jobs it dropped with one notice.
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        account_worker::{CommitOutcome, DRAFTS_ACCOUNT, Job, Priority, WorkerRegistry},
        drafts::DraftService,
        events::AppEvent,
        mail::MailService,
        mail_sync_status::SyncPhase,
        ports::*,
    },
    domain::undo::{UndoAction, UndoOperation, UndoStatus},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::time::{Duration, Instant};

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok("synthetic".into())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct Files;
impl AttachmentGateway for Files {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok("text".into())
    }
}

/// Fake gateway with the seams this suite needs: a per-call log, a barrier that
/// makes one account's syncs slow until the test releases them, a hard failure
/// for one account (EX-004), and a panic mode for the crash-resilience test.
struct Gateway {
    log: Mutex<Vec<String>>,
    barrier: Barrier,
    slow_account: Option<u32>,
    fail_account: Option<u32>,
    panic_sync: AtomicBool,
    sync_calls: AtomicU32,
    /// History batches wait on the barrier (EX-10).
    slow_history: AtomicBool,
    history_calls: AtomicU32,
}
impl Gateway {
    fn new(slow_account: Option<u32>, fail_account: Option<u32>) -> Arc<Self> {
        Arc::new(Self {
            log: Mutex::new(Vec::new()),
            barrier: Barrier::new(2),
            slow_account,
            fail_account,
            panic_sync: AtomicBool::new(false),
            sync_calls: AtomicU32::new(0),
            slow_history: AtomicBool::new(false),
            history_calls: AtomicU32::new(0),
        })
    }
    fn logged(&self, entry: &str) -> bool {
        self.log.lock().unwrap().iter().any(|line| line == entry)
    }
}
impl MailGateway for Gateway {
    fn sync(&self, account: &Account) -> AppResult<SyncBatch> {
        if self.panic_sync.load(Ordering::SeqCst) {
            panic!("test: the sync job crashed");
        }
        if self.fail_account == Some(account.id) {
            return Err(AppError::external("network down"));
        }
        let call = self.sync_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let id = account.id;
        self.log.lock().unwrap().push(format!("sync-start-{id}-{call}"));
        if self.slow_account == Some(account.id) {
            // Reusable rendezvous: the test releases each slow sync in turn.
            self.barrier.wait();
        }
        self.log.lock().unwrap().push(format!("sync-end-{id}-{call}"));
        Ok(SyncBatch {
            emails: vec![],
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn apply_message(&self, _: &Account, email: &Email, _: RemoteAction) -> AppResult<()> {
        self.log.lock().unwrap().push(format!("act-{}", email.id));
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
    /// A 1,000-message All Mail without rows: two batches of 500 UIDs.
    fn history_batch(&self, account: &Account, request: &HistoryRequest) -> AppResult<HistoryBatch> {
        let call = self.history_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.log.lock().unwrap().push(format!("history-start-{call}"));
        if self.fail_account == Some(account.id) {
            return Err(AppError::external("All Mail unavailable"));
        }
        if self.slow_history.load(Ordering::SeqCst) {
            self.barrier.wait();
        }
        let mut progress = request.progress.clone();
        if progress.uid_validity != Some(1) {
            progress.restart(1);
        }
        let range = progress.plan(1001, request.batch_uids);
        Ok(HistoryBatch { folder: "[Gmail]/All Mail".into(), uid_validity: 1, uid_next: 1001, exists: 1000, range, ..Default::default() })
    }
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<AppEvent>>,
}
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.seen.lock().unwrap().push(event);
    }
}
impl Recorder {
    fn count(&self, matches: impl Fn(&AppEvent) -> bool) -> usize {
        self.seen.lock().unwrap().iter().filter(|e| matches(e)).count()
    }
    fn notices(&self) -> usize {
        self.count(|e| matches!(e, AppEvent::Notice { .. }))
    }
    fn notice_texts(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AppEvent::Notice { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
    fn complete(&self, account_id: u32) -> usize {
        self.count(|e| {
            matches!(
                e,
                AppEvent::SyncProgress { account_id: id, phase: SyncPhase::Complete, .. } if *id == account_id
            )
        })
    }
    fn progress(&self, account_id: u32) -> usize {
        self.count(|e| {
            matches!(e, AppEvent::SyncProgress { account_id: id, .. } if *id == account_id)
        })
    }
}

/// These suites drive workers directly, without an undo queue to settle
/// against: dispatched confirmations settle through this no-op.
struct NoOutcome;
impl CommitOutcome for NoOutcome {
    fn commit_finished(&self, _: &[u64], _: &AppResult<()>) {}
}

fn account(id: u32) -> Account {
    Account {
        id,
        email: format!("user{id}@example.com"),
        kind: AccountKind::Gmail,
        provider_id: "gmail".into(),
        ..Default::default()
    }
}
fn email(id: u64, account_id: u32) -> Email {
    Email {
        id,
        account_id,
        uid: Some(id as u32),
        remote_uid_validity: Some(99),
        remote_folder: Some("INBOX".into()),
        message_id: format!("m{id}@example.com"),
        folder: "Inbox".into(),
        subject: format!("Message {id}"),
        from_addr: "other@example.com".into(),
        ..Default::default()
    }
}
fn queued_action(id: u64, email_id: u64, action: &str) -> UndoAction {
    UndoAction {
        id,
        operation: UndoOperation::Mail {
            email_id,
            action: action.into(),
            value: None,
        },
        subject: "Message".into(),
        created_at_ms: 2_000_000_000_000,
        expires_at_ms: 2_000_000_100_000,
        status: UndoStatus::Pending,
        error: None,
        retry_at_ms: None,
        attempts: 0,
    }
}
fn context(store: Store, gateway: Arc<Gateway>, events: Arc<dyn EventSink>) -> Arc<ServiceContext> {
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events,
        repository: Arc::new(MemoryRepository::new(store)),
        mail: gateway,
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(Files),
    })
}
fn wait_until(mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn act(email_id: u64, action: &str, pending_ids: Vec<u64>) -> Job {
    Job::Act {
        email_id,
        commands: vec![(action.into(), None)],
        pending_ids,
    }
}
fn spawn_worker(account_id: u32, context: &Arc<ServiceContext>) -> qamille::application::account_worker::AccountWorker {
    qamille::application::account_worker::AccountWorker::spawn(
        account_id,
        MailService::new(context.clone()),
        DraftService::new(context.clone()),
        context.clone(),
        context.events.clone(),
        Arc::new(NoOutcome),
    )
    .unwrap()
}
fn registry(context: &Arc<ServiceContext>) -> WorkerRegistry {
    WorkerRegistry::new(
        MailService::new(context.clone()),
        DraftService::new(context.clone()),
        context.clone(),
        context.events.clone(),
        Arc::new(NoOutcome),
    )
}

#[test]
fn user_action_preempts_queued_sync() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(30, 1));
    store
        .undo_actions
        .push(queued_action(100, 30, "markAsSeen"));
    let gateway = Gateway::new(Some(1), None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let worker = spawn_worker(1, &context);

    // The first sync is inside the barrier before anything else is queued, so
    // the remaining jobs compete only among themselves.
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| gateway.log.lock().unwrap().iter().any(|line| line.starts_with("sync-start-1-"))),
        "worker never started the first sync"
    );
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    worker
        .submit(Priority::UserAction, act(30, "markAsSeen", vec![100]))
        .unwrap();
    gateway.barrier.wait(); // release sync 1

    // The action must fully commit (store written, queued undo action removed)
    // before the second sync is released.
    assert!(
        wait_until(|| {
            let store = context.read().unwrap();
            store.email(30).is_some_and(|e| e.read) && store.undo_actions.is_empty()
        }),
        "the queued user action did not complete before the second sync"
    );
    gateway.barrier.wait(); // release sync 2
    gateway.barrier.wait(); // release sync 3
    assert!(wait_until(|| gateway.logged("sync-end-1-3")), "syncs did not finish");
    worker.stop();

    let log = gateway.log.lock().unwrap().clone();
    let act_at = log.iter().position(|line| line == "act-30").unwrap();
    let second_sync_at = log.iter().position(|line| line == "sync-start-1-2").unwrap();
    assert!(
        act_at < second_sync_at,
        "action ran after the second sync: {log:?}"
    );
    assert_eq!(log.iter().filter(|l| l.starts_with("sync-")).count(), 6, "{log:?}");
    // Seam fix (2.2b): `sync_selected` is the single `SyncProgress` emitter —
    // exactly one Syncing + one Complete per Sync job, never a second pair
    // from the worker; a clean run emits no notices.
    assert_eq!(recorder.complete(1), 3);
    assert_eq!(
        recorder.progress(1),
        6,
        "expected exactly one SyncProgress pair per Sync job"
    );
    assert_eq!(recorder.notices(), 0);
}

#[test]
fn error_in_one_account_does_not_stop_another() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.accounts.push(account(2));
    let gateway = Gateway::new(None, Some(2));
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let registry = registry(&context);
    registry.reconcile(&[account(1), account(2)]).unwrap();

    // EX-004: account 2 fails, account 1 completes; the failure is announced once.
    registry.submit(2, Priority::Sync, Job::Sync).unwrap();
    registry.submit(1, Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| recorder.notices() == 1 && recorder.complete(1) == 1),
        "account 1 did not sync cleanly while account 2 failed"
    );
    // Single emitter (decision 5): the failed account's progress is exactly
    // one Syncing + one Error, both from the sync writer.
    assert_eq!(recorder.progress(2), 2, "{:?}", recorder.notice_texts());
    assert_eq!(
        recorder.count(|e| matches!(e, AppEvent::SyncProgress { account_id: 2, phase: SyncPhase::Error, .. })),
        1
    );
    let notice = recorder
        .notice_texts()
        .into_iter()
        .next()
        .unwrap();
    assert!(notice.contains("network down") && notice.contains('2'), "{notice}");

    // Both workers are still alive: a second round runs, and the failure of
    // account 2 is reported once per failed job, not repeated by itself.
    registry.submit(2, Priority::Sync, Job::Sync).unwrap();
    registry.submit(1, Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| recorder.notices() == 2 && recorder.complete(1) == 2),
        "a worker died after the account error"
    );
}

#[test]
fn reconcile_removal_stops_the_worker_and_later_submits_fail() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.accounts.push(account(2));
    store.emails.push(email(31, 1));
    store.emails.push(email(32, 2));
    let gateway = Gateway::new(None, None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let registry = registry(&context);
    registry.reconcile(&[account(1), account(2)]).unwrap();

    // Worker 2 is live before the removal.
    registry
        .submit(2, Priority::UserAction, act(32, "markAsSeen", vec![]))
        .unwrap();
    assert!(
        wait_until(|| context.read().unwrap().email(32).is_some_and(|e| e.read)),
        "worker 2 never ran the action"
    );

    // Review Focus 3: reconcile without account 2 stops its worker; a later
    // submit fails with "Account" and nothing is written.
    registry.reconcile(&[account(1)]).unwrap();
    let error = registry
        .submit(2, Priority::UserAction, act(32, "markAsUnseen", vec![]))
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(error.message.contains("Account"), "{error}");
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        context.read().unwrap().email(32).is_some_and(|e| e.read),
        "the rejected job still wrote to the store"
    );

    // Worker 1 survived the reconcile.
    registry
        .submit(1, Priority::UserAction, act(31, "markAsSeen", vec![]))
        .unwrap();
    assert!(
        wait_until(|| context.read().unwrap().email(31).is_some_and(|e| e.read)),
        "worker 1 stopped after the reconcile"
    );
}

#[test]
fn stop_drains_the_queue_and_rejects_new_jobs() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(31, 1));
    let gateway = Gateway::new(None, None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let worker = spawn_worker(1, &context);
    worker.stop();
    let error = worker
        .submit(Priority::UserAction, act(31, "markAsSeen", vec![]))
        .unwrap_err();
    assert_eq!(error.code, "worker_stopped");
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        context.read().unwrap().email(31).is_some_and(|e| !e.read),
        "a job ran after stop"
    );
}

/// Controller addition 2026-11-05 (review 3.1, Important 1): once ~75% of the
/// 64-slot channel is outstanding, `Sync`/`Background` are refused with a
/// typed error while `UserAction`/`ReadBody` keep their reserved headroom.
#[test]
fn background_jobs_are_refused_while_user_actions_keep_their_headroom() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(30, 1));
    let gateway = Gateway::new(Some(1), None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let worker = spawn_worker(1, &context);

    // The worker blocks inside the slow sync, so nothing is pulled from the
    // channel while the test fills it.
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| gateway.log.lock().unwrap().iter().any(|line| line.starts_with("sync-start-1-"))),
        "worker never entered the slow sync"
    );

    // 48 background jobs = 75% of the 64 slots: the admission limit.
    for page in 0..48u32 {
        worker
            .submit(Priority::Background, Job::Index { page })
            .unwrap();
    }
    let throttled = worker
        .submit(Priority::Background, Job::Index { page: 48 })
        .unwrap_err();
    assert_eq!(throttled.code, "queue_throttled");
    let throttled = worker.submit(Priority::Sync, Job::Sync).unwrap_err();
    assert_eq!(
        throttled.code, "queue_throttled",
        "sync yields to user actions as well"
    );
    // The reserved headroom stays open for user-facing work.
    worker
        .submit(Priority::UserAction, act(30, "markAsSeen", vec![]))
        .unwrap();
    worker
        .submit(Priority::ReadBody, Job::ReadBody { email_id: 30 })
        .unwrap();

    gateway.barrier.wait(); // release the slow sync
    assert!(
        wait_until(|| context.read().unwrap().email(30).is_some_and(|e| e.read)),
        "the admitted user action never ran"
    );
    worker.stop();
}

/// A panicking job is skipped with one notice and never kills the worker
/// (EX-004) — the property `reconcile`'s respawn backs up defensively.
#[test]
fn a_panicking_job_is_skipped_and_the_worker_continues() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(30, 1));
    let gateway = Gateway::new(None, None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let worker = spawn_worker(1, &context);

    gateway.panic_sync.store(true, Ordering::SeqCst);
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| recorder
            .notice_texts()
            .iter()
            .any(|text| text.contains("crashed and was skipped"))),
        "the panic was not announced: {:?}",
        recorder.notice_texts()
    );
    gateway.panic_sync.store(false, Ordering::SeqCst);
    worker
        .submit(Priority::UserAction, act(30, "markAsSeen", vec![]))
        .unwrap();
    assert!(
        wait_until(|| context.read().unwrap().email(30).is_some_and(|e| e.read)),
        "the worker died with the panicking job"
    );
    worker.stop();
}

/// Controller addition 2026-11-05: `reconcile` respawns workers whose thread
/// has finished. Job panics are caught (previous test), so a finished handle
/// comes from a crash that escaped the guards or an explicit stop; both leave
/// the same `is_finished()` state this test drives through `stop_worker`.
#[test]
fn reconcile_respawns_a_worker_whose_thread_finished() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(31, 1));
    let gateway = Gateway::new(None, None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let registry = registry(&context);
    registry.reconcile(&[account(1)]).unwrap();

    registry
        .submit(1, Priority::UserAction, act(31, "markAsSeen", vec![]))
        .unwrap();
    assert!(
        wait_until(|| context.read().unwrap().email(31).is_some_and(|e| e.read)),
        "the first worker never ran the action"
    );

    // The thread finishes; the entry stays, so submits fail as "stopping"...
    assert!(registry.stop_worker(1));
    let error = registry
        .submit(1, Priority::UserAction, act(31, "markAsUnseen", vec![]))
        .unwrap_err();
    assert_eq!(error.code, "worker_stopped");

    // ...and the next reconcile replaces the finished worker with a fresh one.
    registry.reconcile(&[account(1)]).unwrap();
    context
        .update(|store| {
            store.email_mut(31).unwrap().read = false;
            Ok(())
        })
        .unwrap();
    registry
        .submit(1, Priority::UserAction, act(31, "markAsSeen", vec![]))
        .unwrap();
    assert!(
        wait_until(|| context.read().unwrap().email(31).is_some_and(|e| e.read)),
        "the respawned worker never ran the action"
    );
}

/// The dedicated drafts worker (Task 3.2): reconcile always keeps it alive
/// under the synthetic `DRAFTS_ACCOUNT` id, and it runs the draft jobs with
/// the same structure as an account's.
#[test]
fn the_drafts_worker_runs_queued_draft_jobs() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.drafts.push(Draft {
        id: 7,
        account_id: 1,
        to: "recipient@example.com".into(),
        updated: 1,
        ..Default::default()
    });
    let gateway = Gateway::new(None, None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let registry = registry(&context);
    // No accounts at all: the drafts worker still exists.
    registry.reconcile(&[]).unwrap();
    registry
        .submit(
            DRAFTS_ACCOUNT,
            Priority::UserAction,
            Job::ScheduleDraft {
                draft_id: 7,
                send_at: 2_000_100_000,
                pending_ids: vec![],
            },
        )
        .unwrap();
    assert!(
        wait_until(|| context
            .read()
            .unwrap()
            .drafts
            .iter()
            .find(|d| d.id == 7)
            .is_some_and(|d| d.send_at == Some(2_000_100_000))),
        "the drafts worker never scheduled the draft"
    );
}

/// Controller addition 2026-11-05: jobs still queued when the worker stops are
/// dropped without executing and announced with exactly one notice.
#[test]
fn stop_announces_the_jobs_it_dropped() {
    let mut store = Store::default();
    store.accounts.push(account(1));
    let gateway = Gateway::new(Some(1), None);
    let recorder = Arc::new(Recorder::default());
    let context = context(store, gateway.clone(), recorder.clone());
    let worker = spawn_worker(1, &context);

    worker.submit(Priority::Sync, Job::Sync).unwrap();
    assert!(
        wait_until(|| gateway.log.lock().unwrap().iter().any(|line| line.starts_with("sync-start-1-"))),
        "worker never entered the slow sync"
    );
    worker
        .submit(Priority::Background, Job::Index { page: 1 })
        .unwrap();
    worker
        .submit(Priority::Background, Job::Index { page: 2 })
        .unwrap();

    // stop() sets the flag first, then joins bounded: while the worker is
    // blocked in the barrier the join times out and detaches, so the flag is
    // guaranteed set before the sync is released and the worker drains.
    worker.stop();
    gateway.barrier.wait();

    assert!(
        wait_until(|| recorder.notices() == 1),
        "the dropped jobs were not announced: {:?}",
        recorder.notice_texts()
    );
    let notices = recorder.notice_texts();
    assert!(
        notices[0].contains("stopped with 2 queued job(s) dropped"),
        "{notices:?}"
    );
}

#[test]
fn work_queued_during_a_history_batch_runs_before_the_next_one() {
    // EX-10: the backfill always yields to the user's actions and to syncs.
    let mut store = Store::default();
    store.accounts.push(account(1));
    store.emails.push(email(30, 1));
    store.undo_actions.push(queued_action(100, 30, "markAsSeen"));
    let gateway = Gateway::new(None, None);
    gateway.slow_history.store(true, Ordering::SeqCst);
    let context = context(store, gateway.clone(), Arc::new(Recorder::default()));
    let worker = spawn_worker(1, &context);
    worker.submit(Priority::History, Job::Backfill).unwrap();
    assert!(wait_until(|| gateway.logged("history-start-1")), "the first batch never started");
    worker.submit(Priority::History, Job::Backfill).unwrap();
    worker.submit(Priority::Sync, Job::Sync).unwrap();
    worker.submit(Priority::UserAction, act(30, "markAsSeen", vec![100])).unwrap();
    gateway.barrier.wait(); // release batch 1
    assert!(wait_until(|| gateway.logged("history-start-2")), "the walk did not continue");
    gateway.barrier.wait(); // release batch 2
    assert!(wait_until(|| context.read().unwrap().accounts[0].history.complete), "the walk did not finish");
    worker.stop();
    let log = gateway.log.lock().unwrap().clone();
    let at = |entry: &str| log.iter().position(|line| line == entry).unwrap_or_else(|| panic!("{entry} missing: {log:?}"));
    assert!(at("act-30") < at("history-start-2") && at("sync-end-1-1") < at("history-start-2"), "{log:?}");
    assert_eq!(gateway.history_calls.load(Ordering::SeqCst), 2, "one batch at a time, no duplicate: {log:?}");
}

#[test]
fn repeated_history_failure_waits_for_another_sync() {
    let mut store = Store::default();
    let mut mailbox = account(1);
    mailbox.history.last_error = Some("All Mail unavailable".into());
    store.accounts.push(mailbox);
    let gateway = Gateway::new(None, Some(1));
    let context = context(store, gateway.clone(), Arc::new(Recorder::default()));
    let worker = spawn_worker(1, &context);
    worker.submit(Priority::History, Job::Backfill).unwrap();
    assert!(wait_until(|| gateway.logged("history-start-1")));
    std::thread::sleep(Duration::from_millis(150));
    worker.stop();
    assert_eq!(gateway.history_calls.load(Ordering::SeqCst), 1);
}
