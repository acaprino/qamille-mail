//! ADR-0007 v3 decision 8, INV-010, EX-009/EX-010: a confirmation that never
//! reached the provider stays in the queue with an incremental backoff, survives
//! a restart and is retried early when a connection succeeds again.
//! INV-008 and EX-010 are unchanged: a definitive error (authentication) and a
//! result lost after a mutation started fail immediately, without any retry.

use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        account_worker::{CommitOutcome, WorkerRegistry},
        connectivity,
        drafts::DraftService,
        mail::MailService,
        ports::*,
        undo::UndoService,
    },
    domain::undo::{UndoAction, UndoStatus},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// `connectivity` keeps process-wide state, so these tests run one at a time: a
/// wake noted by one test must never be drained by another test's pass.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The provider seam: it fails the next `apply` calls with the queued messages
/// and records every attempt, so a test controls the exact text the source
/// produces (`infrastructure/mail.rs`) and counts the real round trips.
#[derive(Default)]
struct Gateway {
    script: Mutex<VecDeque<String>>,
    calls: AtomicU64,
    applied: Mutex<Vec<(u32, u32, RemoteAction)>>,
    sends: AtomicU64,
    /// Apply calls that succeed before the failure script is consulted.
    ok_first: AtomicU64,
}
impl Gateway {
    fn succeed_first(&self, calls: u64) {
        self.ok_first.store(calls, Ordering::SeqCst);
    }
    fn fail_next(&self, message: &str) {
        self.script.lock().unwrap().push_back(message.into());
    }
    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
    fn applied(&self) -> Vec<(u32, u32, RemoteAction)> {
        // RemoteAction has no PartialEq; keep the recorded triples for matching.
        self.applied
            .lock()
            .unwrap()
            .iter()
            .map(|(account, uid, action)| (*account, *uid, action.clone()))
            .collect()
    }
}
impl MailGateway for Gateway {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch {
            emails: vec![],
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, account: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let skip = self
            .ok_first
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if !skip {
            if let Some(message) = self.script.lock().unwrap().pop_front() {
                return Err(AppError::external(message));
            }
        }
        self.applied.lock().unwrap().push((account.id, uid, action));
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if let Some(message) = self.script.lock().unwrap().pop_front() {
            return Err(AppError::external(message));
        }
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}

struct MovingClock(AtomicU64);
impl MovingClock {
    fn advance(&self, milliseconds: u64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }
}
impl Clock for MovingClock {
    fn now(&self) -> i64 {
        self.now_ms() / 1000
    }
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst) as i64
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok("test-only".into())
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
        Ok(String::new())
    }
}

fn fixture() -> Store {
    let mut store = sample_store();
    store.accounts[0].kind = AccountKind::Gmail;
    store.accounts[0].archive_folder = "[Gmail]/All Mail".into();
    store.accounts[0].trash_folder = "[Gmail]/Trash".into();
    store.emails[0].uid = Some(10);
    store
}
/// The queue with its worker registry, wired the way the desktop runs it
/// (Task 3.2): `commit_due` claims and dispatches to the account workers, and
/// the workers settle the results back through the same service instance.
fn services_for(context: &Arc<ServiceContext>) -> (Arc<UndoService>, WorkerRegistry) {
    let mail = MailService::new(context.clone());
    let drafts = DraftService::new(context.clone());
    let undo = Arc::new(UndoService::new(context.clone(), mail.clone(), drafts.clone()));
    let workers = WorkerRegistry::new(
        mail,
        drafts,
        context.clone(),
        Arc::new(NullSink),
        undo.clone() as Arc<dyn CommitOutcome>,
    );
    (undo, workers)
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
/// `commit_due` hands claimed rows to worker threads; wait until every claim
/// settled (no `Committing` row left) before asserting on the outcome.
fn settled(context: &Arc<ServiceContext>) {
    assert!(
        wait_until(|| !context
            .read()
            .unwrap()
            .undo_actions
            .iter()
            .any(|a| a.status == UndoStatus::Committing)),
        "the worker never settled the dispatched confirmation"
    );
}
/// Counts full-store reads and write transactions, so a test can prove the
/// idle fast path of `commit_due` keeps its hands off the store while an
/// action backs off, and that a pass that claims nothing writes nothing (each
/// real transaction rewrites and fsyncs `store.json`).
struct CountingRepository {
    inner: MemoryRepository,
    reads: AtomicU64,
    transactions: AtomicU64,
}
impl CountingRepository {
    fn new(store: Store) -> Self {
        Self {
            inner: MemoryRepository::new(store),
            reads: AtomicU64::new(0),
            transactions: AtomicU64::new(0),
        }
    }
    fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }
    fn transactions(&self) -> u64 {
        self.transactions.load(Ordering::SeqCst)
    }
}
impl StoreRepository for CountingRepository {
    fn read(&self) -> AppResult<Store> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read()
    }
    fn transaction(&self, operation: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.transactions.fetch_add(1, Ordering::SeqCst);
        self.inner.transaction(operation)
    }
}
/// Parks a thread in `wait_for_work(deadline)` the way the desktop's deadline
/// thread does; the receiver yields the instant the wait returned.
fn park_deadline_thread(
    undo: &Arc<UndoService>,
    deadline: Option<i64>,
) -> std::sync::mpsc::Receiver<Instant> {
    // The writers' signals queued so far were never consumed (no deadline
    // thread runs in these tests): consume them the way the loop's previous
    // iteration would have, so the wait below starts clean.
    undo.wait_for_work(Some(0));
    let (parked, ready) = std::sync::mpsc::channel();
    let (returned, done) = std::sync::mpsc::channel();
    let undo = undo.clone();
    std::thread::spawn(move || {
        parked.send(()).unwrap();
        undo.wait_for_work(deadline);
        let _ = returned.send(Instant::now());
    });
    ready.recv().unwrap();
    // The thread is inside the wait well before the assertions below look.
    std::thread::sleep(Duration::from_millis(30));
    done
}
fn context_with_repo(
    repository: Arc<dyn StoreRepository>,
    gateway: Arc<Gateway>,
    clock: Arc<dyn Clock>,
) -> Arc<ServiceContext> {
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository,
        mail: gateway,
        secrets: Arc::new(Secrets),
        clock,
        attachments: Arc::new(Files),
    })
}
fn context_with(store: Store, gateway: Arc<Gateway>, clock: Arc<dyn Clock>) -> Arc<ServiceContext> {
    context_with_repo(Arc::new(MemoryRepository::new(store)), gateway, clock)
}
struct Fixture {
    /// Holds the process-wide wake set for the WHOLE test body: `connectivity`
    /// is global state, and a wake noted or drained by a parallel test would
    /// corrupt the assertions (every fixture uses the same account ids).
    _guard: std::sync::MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    gateway: Arc<Gateway>,
    clock: Arc<MovingClock>,
    undo: Arc<UndoService>,
    workers: WorkerRegistry,
    repo: Arc<CountingRepository>,
    email_id: u64,
    account_id: u32,
}
fn setup() -> Fixture {
    let guard = serial();
    connectivity::drain_wakes();
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let gateway = Arc::new(Gateway::default());
    let store = fixture();
    let (email_id, account_id) = (store.emails[0].id, store.emails[0].account_id);
    let repo = Arc::new(CountingRepository::new(store));
    let context = context_with_repo(repo.clone(), gateway.clone(), clock.clone());
    let (undo, workers) = services_for(&context);
    Fixture {
        _guard: guard,
        context,
        gateway,
        clock,
        undo,
        workers,
        repo,
        email_id,
        account_id,
    }
}
/// Queue an archive action whose undo window has already expired.
fn expired_archive(fixture: &Fixture) -> UndoAction {
    let pending = fixture
        .undo
        .queue_mail(fixture.email_id, "archive".into(), None)
        .unwrap();
    fixture.clock.advance(5000);
    pending
}
fn action(fixture: &Fixture) -> UndoAction {
    fixture.context.read().unwrap().undo_actions[0].clone()
}
/// A draft queued for delivery whose undo window has already expired.
fn queued_draft(fixture: &Fixture) -> Draft {
    let drafts = DraftService::new(fixture.context.clone());
    let mut draft = drafts.create(Some(fixture.account_id), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.body = "Queued while the network was down".into();
    let draft = drafts.save(draft).unwrap();
    fixture.undo.queue_draft(draft.id, "send", None).unwrap();
    fixture.clock.advance(5000);
    draft
}

const OFFLINE: &str =
    "IMAP connect: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)";
const LOGIN_OFFLINE: &str =
    "IMAP login: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)";
const AUTHENTICATION: &str =
    "IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)";
/// `imap::Error::ConnectionLost` while a mutation command was in flight: the
/// provider may already have executed it, so the outcome is lost (INV-008).
const LOST_OUTCOME: &str = "Connection Lost";

#[test]
fn a_transient_connect_failure_stays_queued_and_commits_on_the_next_attempt() {
    let fixture = setup();
    let pending = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.gateway.fail_next(OFFLINE);
    fixture.clock.advance(5000);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "The failed attempt is a change");
    settled(&fixture.context);

    let retrying = action(&fixture);
    assert_eq!(retrying.id, pending.id);
    assert_eq!(retrying.status, UndoStatus::Pending, "INV-010: never shown as failed");
    assert_eq!(retrying.attempts, 1);
    assert_eq!(
        retrying.retry_at_ms,
        Some(fixture.clock.now_ms() + 5000),
        "First backoff is 5 s"
    );
    assert_eq!(retrying.error.as_deref(), Some(OFFLINE), "The UI keeps the reason");
    assert!(fixture.gateway.applied().is_empty());
    // The projected local state stays applied while the action exists.
    assert_eq!(
        MailService::new(fixture.context.clone())
            .read(fixture.email_id)
            .unwrap()
            .folder,
        "Archive"
    );
    // Before the deadline nothing is attempted again.
    fixture.clock.advance(4999);
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 1);
    // After the deadline the retry commits and the action leaves the queue.
    fixture.clock.advance(1);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    let applied = fixture.gateway.applied();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].0, fixture.account_id);
    assert_eq!(applied[0].1, 10);
    assert!(matches!(applied[0].2, RemoteAction::Archive));
    assert_eq!(
        fixture.context.read().unwrap().email(fixture.email_id).unwrap().folder,
        "Archive"
    );
}

#[test]
fn a_successful_connection_wakes_the_queue_before_a_long_backoff_expires() {
    let fixture = setup();
    expired_archive(&fixture);
    fixture.gateway.fail_next(LOGIN_OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert_eq!(action(&fixture).attempts, 1);
    // A long pushback: without a wake the queue would wait five minutes.
    fixture.context.update(|store| {
        store.undo_actions[0].attempts = 4;
        store.undo_actions[0].retry_at_ms = Some(fixture.clock.now_ms() + 300_000);
        Ok(())
    }).unwrap();
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 1);

    connectivity::note_success(fixture.account_id);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "The wake retries immediately");
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.gateway.applied().len(), 1);
    // The wake is consumed once and another account is not woken by it.
    assert!(connectivity::drain_wakes().is_empty());
}

#[test]
fn a_wake_counts_as_an_attempt_and_keeps_the_backoff_when_it_fails_again() {
    let fixture = setup();
    expired_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    fixture.undo.commit_due(&fixture.workers).unwrap();
    settled(&fixture.context);
    fixture.context.update(|store| {
        store.undo_actions[0].attempts = 4;
        store.undo_actions[0].retry_at_ms = Some(fixture.clock.now_ms() + 300_000);
        Ok(())
    }).unwrap();
    fixture.gateway.fail_next(OFFLINE);
    connectivity::note_success(fixture.account_id);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let retrying = action(&fixture);
    assert_eq!(retrying.status, UndoStatus::Pending);
    assert_eq!(retrying.attempts, 5, "The woken attempt still counts");
    assert_eq!(
        retrying.retry_at_ms,
        Some(fixture.clock.now_ms() + 600_000),
        "The backoff is capped at 600 s"
    );
    // The consumed wake does not keep retrying before the new deadline.
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 2);
}

#[test]
fn an_authentication_failure_is_definitive_and_is_attempted_once() {
    let fixture = setup();
    let pending = expired_archive(&fixture);
    fixture.gateway.fail_next(AUTHENTICATION);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);

    let failed = action(&fixture);
    assert_eq!(failed.id, pending.id);
    assert_eq!(failed.status, UndoStatus::Failed, "EX-010: immediate failure row");
    assert_eq!(failed.attempts, 0, "A definitive error is not a retry");
    assert!(failed.retry_at_ms.is_none());
    assert_eq!(failed.error.as_deref(), Some(AUTHENTICATION));
    // The local projection returns, and no further attempt is ever made.
    assert_eq!(
        MailService::new(fixture.context.clone())
            .read(fixture.email_id)
            .unwrap()
            .folder,
        "Inbox"
    );
    fixture.clock.advance(600_000);
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 1);
    fixture.undo.dismiss(pending.id).unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
}

#[test]
fn a_result_lost_during_a_mutation_command_is_never_retried() {
    for message in [
        LOST_OUTCOME,
        "No Response: [TRYCREATE] UID MOVE failed",
        "Bad Response: b'NO [CANNOT] Message not found'",
    ] {
        let fixture = setup();
        let pending = expired_archive(&fixture);
        fixture.gateway.fail_next(message);
        assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "{message}");
        settled(&fixture.context);
        let failed = action(&fixture);
        assert_eq!(failed.status, UndoStatus::Failed, "INV-008: {message}");
        assert_eq!(failed.attempts, 0, "{message}");
        assert!(failed.retry_at_ms.is_none(), "{message}");
        assert_eq!(failed.error.as_deref(), Some(message), "{message}");
        fixture.clock.advance(3_600_000);
        assert!(!fixture.undo.commit_due(&fixture.workers).unwrap(), "{message}");
        assert_eq!(fixture.gateway.calls(), 1, "{message}");
        fixture.undo.dismiss(pending.id).unwrap();
    }
}

#[test]
fn an_uncertain_delivery_is_never_retried_even_with_a_network_like_message() {
    let fixture = setup();
    let draft = queued_draft(&fixture);
    fixture
        .gateway
        .fail_next("network error: connection reset by peer (os error 10054)");
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let failed = action(&fixture);
    assert_eq!(
        failed.status,
        UndoStatus::Failed,
        "INV-008: an unknown send is never retried"
    );
    assert_eq!(failed.attempts, 0);
    assert!(failed.retry_at_ms.is_none());
    fixture.clock.advance(3_600_000);
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.sends.load(Ordering::SeqCst), 1);
    // The draft is still there, so the user can decide after checking the provider.
    assert!(fixture
        .context
        .read()
        .unwrap()
        .drafts
        .iter()
        .any(|d| d.id == draft.id));
}

#[test]
fn a_queued_send_keeps_its_real_reason_because_the_outbox_owns_reconciliation() {
    let fixture = setup();
    let draft = queued_draft(&fixture);
    // Even a provably pre-contact SMTP failure cannot be replayed by this queue:
    // the outbox recorded the delivery as uncertain, so a second `send` returns
    // `delivery_unknown` without ever contacting the provider again. Failing at
    // once keeps the real reason on screen instead of that second answer.
    fixture
        .gateway
        .fail_next("Connection error: dns error: failed to lookup address information (os error 11001)");
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let failed = action(&fixture);
    assert_eq!(failed.status, UndoStatus::Failed);
    assert_eq!(failed.attempts, 0);
    assert!(failed.retry_at_ms.is_none());
    assert!(
        failed.error.as_deref().unwrap().starts_with("Connection error:"),
        "{:?}",
        failed.error
    );
    fixture.clock.advance(3_600_000);
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.sends.load(Ordering::SeqCst), 1);
    assert!(fixture
        .context
        .read()
        .unwrap()
        .drafts
        .iter()
        .any(|d| d.id == draft.id));
}

#[test]
fn a_retrying_action_survives_a_restart_and_is_rescheduled() {
    let fixture = setup();
    expired_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let retrying = action(&fixture);
    assert_eq!(retrying.attempts, 1);

    // A restart: the same data, a brand new service over a new repository.
    let serialized = serde_json::to_string(&fixture.context.read().unwrap()).unwrap();
    let restarted = context_with(
        serde_json::from_str(&serialized).unwrap(),
        fixture.gateway.clone(),
        fixture.clock.clone(),
    );
    let (resumed, workers2) = services_for(&restarted);
    resumed.recover_interrupted().unwrap();
    let restored = restarted.read().unwrap().undo_actions[0].clone();
    assert_eq!(restored.id, retrying.id);
    assert_eq!(restored.status, UndoStatus::Pending);
    assert_eq!(restored.attempts, 1);
    assert_eq!(restored.retry_at_ms, retrying.retry_at_ms);
    assert_eq!(restored.error.as_deref(), Some(OFFLINE));

    // Nothing happens before the saved deadline, then the queue resumes.
    assert!(!resumed.commit_due(&workers2).unwrap());
    assert_eq!(fixture.gateway.calls(), 1);
    fixture.clock.advance(5000);
    assert!(resumed.commit_due(&workers2).unwrap());
    settled(&restarted);
    assert!(restarted.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.gateway.applied().len(), 1);
}

#[test]
fn the_backoff_grows_5_15_60_300_then_stays_at_600_seconds() {
    let fixture = setup();
    expired_archive(&fixture);
    let mut expected = 5_000;
    for attempt in 1..=6u32 {
        fixture.gateway.fail_next(OFFLINE);
        fixture.clock.advance(600_000);
        assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "attempt {attempt}");
        settled(&fixture.context);
        let retrying = action(&fixture);
        assert_eq!(retrying.attempts, attempt);
        assert_eq!(retrying.status, UndoStatus::Pending);
        assert_eq!(
            retrying.retry_at_ms,
            Some(fixture.clock.now_ms() + expected),
            "attempt {attempt}"
        );
        expected = match attempt {
            1 => 15_000,
            2 => 60_000,
            3 => 300_000,
            _ => 600_000,
        };
    }
    assert_eq!(fixture.gateway.calls(), 6);
}

#[test]
fn giving_up_on_a_retrying_action_removes_it_and_restores_the_local_view() {
    let fixture = setup();
    let pending = expired_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert_eq!(
        MailService::new(fixture.context.clone())
            .read(fixture.email_id)
            .unwrap()
            .folder,
        "Archive",
        "The projection stays applied while the queue retries"
    );
    fixture.undo.dismiss(pending.id).unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(
        MailService::new(fixture.context.clone())
            .read(fixture.email_id)
            .unwrap()
            .folder,
        "Inbox",
        "Rinuncia is the only case where the local state returns"
    );
    // The queue is empty: no attempt is made later.
    fixture.clock.advance(600_000);
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 1);
    assert_eq!(fixture.gateway.sends.load(Ordering::SeqCst), 0);
}

#[test]
fn a_backing_off_queue_reads_nothing_until_a_deadline_or_a_wake() {
    let fixture = setup();
    expired_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "The failure schedules a retry");
    settled(&fixture.context);
    assert_eq!(action(&fixture).attempts, 1);

    // The worker-thread settlement invalidated the schedule cache, so one pass
    // recomputes it; the store reads measured below start after that.
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());

    // Idle ticks while the backoff runs: the cached deadline and the absent
    // wake must keep `commit_due` off the store entirely (probe: the counting
    // repository wrapper around `MemoryRepository`).
    let reads = fixture.repo.reads();
    fixture.clock.advance(1000);
    for _ in 0..5 {
        assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    }
    assert_eq!(fixture.repo.reads(), reads, "Idle ticks perform no store reads");
    assert_eq!(fixture.gateway.calls(), 1);

    // The deadline thread sleeps on the backoff deadline: nothing polls while
    // it waits (no store read), and a connection that succeeds ends the wait
    // through the notifier, not through a fixed-interval re-check (INV-005).
    let deadline = fixture.undo.next_deadline().unwrap();
    assert_eq!(deadline, action(&fixture).retry_at_ms, "the backoff is the next wake");
    let reads = fixture.repo.reads();
    let writes = fixture.repo.transactions();
    let done = park_deadline_thread(&fixture.undo, deadline);
    assert!(done.try_recv().is_err(), "the wait holds until a wake or the deadline");
    assert_eq!(fixture.repo.reads(), reads, "the wait itself never reads the store");
    let noted = Instant::now();
    connectivity::note_success(fixture.account_id);
    let returned = done
        .recv_timeout(Duration::from_secs(2))
        .expect("a connectivity wake must end the deadline wait");
    assert!(
        returned.duration_since(noted) < Duration::from_millis(120),
        "the wake is delivered by the notifier, not found by a later poll: {:?}",
        returned.duration_since(noted)
    );
    assert_eq!(fixture.repo.reads(), reads, "the wake reads nothing by itself");
    assert_eq!(fixture.repo.transactions(), writes);

    // The woken pass is a full one: early retry, commit.
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap(), "The wake forces a full pass");
    assert!(fixture.repo.reads() > reads, "The woken pass reads the store");
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.gateway.applied().len(), 1);
}

#[test]
fn an_unattempted_confirmation_cannot_be_dismissed() {
    let fixture = setup();
    let pending = expired_archive(&fixture);
    // Expired but never attempted: the first commit belongs to the queue, and
    // dropping it would rewind the projection without any failure to show.
    fixture.undo.dismiss(pending.id).unwrap();
    assert_eq!(fixture.context.read().unwrap().undo_actions.len(), 1);
    // After one attempt the same call is "Rinuncia" and does remove it.
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert_eq!(action(&fixture).attempts, 1);
    fixture.undo.dismiss(pending.id).unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(
        MailService::new(fixture.context.clone())
            .read(fixture.email_id)
            .unwrap()
            .folder,
        "Inbox"
    );
}

#[test]
fn an_expired_sibling_does_not_drag_a_retrying_action_into_an_early_attempt() {
    let fixture = setup();
    let retrying = expired_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    fixture.context.update(|store| {
        let action = store.undo_actions.iter_mut().find(|a| a.id == retrying.id).unwrap();
        action.retry_at_ms = Some(fixture.clock.now_ms() + 300_000);
        Ok(())
    }).unwrap();
    // A second action on the same message expires while the first one is backing off.
    fixture.undo.queue_mail(fixture.email_id, "pin".into(), None).unwrap();
    fixture.clock.advance(5000);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let calls = fixture.gateway.applied();
    assert_eq!(calls.len(), 1, "Only the due action is committed");
    assert!(matches!(calls[0].2, RemoteAction::Flag(true)));
    let store = fixture.context.read().unwrap();
    assert_eq!(store.undo_actions.len(), 1);
    assert_eq!(store.undo_actions[0].id, retrying.id);
    assert_eq!(store.undo_actions[0].status, UndoStatus::Pending);
    // Its own deadline is still honoured: the failed attempt is the only one so far.
    assert_eq!(
        fixture.gateway.calls(),
        2,
        "The retrying action was not attempted before its deadline"
    );
    fixture.clock.advance(300_000);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
}

#[test]
fn stored_actions_without_the_retry_fields_still_load() {
    let _guard = serial();
    let legacy = r#"{
        "id": 7,
        "operation": { "kind": "mail", "email_id": 3, "action": "archive", "value": null },
        "subject": "Q4 proposal",
        "created_at_ms": 1000,
        "expires_at_ms": 6000,
        "status": "pending",
        "error": null
    }"#;
    let action: UndoAction = serde_json::from_str(legacy).unwrap();
    assert_eq!(action.attempts, 0);
    assert_eq!(action.retry_at_ms, None);
    // The new fields are written, so a restart keeps the schedule.
    let json = serde_json::to_string(&action).unwrap();
    assert!(json.contains("\"attempts\":0"), "{json}");
    assert!(json.contains("\"retry_at_ms\":null"), "{json}");
}

/// The batch invariant behind retrying a whole claim after "IMAP connect:":
/// flags first, move last, flags idempotent. The move's own connect fails
/// after the flag already reached the server; the retry re-sends the flag and
/// the move lands exactly once.
#[test]
fn a_move_connect_failure_after_flags_retries_the_batch_and_moves_once() {
    let fixture = setup();
    fixture.undo.queue_mail(fixture.email_id, "markAsSeen".into(), None).unwrap();
    fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.clock.advance(5000);
    fixture.gateway.succeed_first(1);
    fixture.gateway.fail_next(OFFLINE);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert_eq!(fixture.gateway.calls(), 2, "Flag sent, then the move failed to connect");
    let queued = fixture.context.read().unwrap().undo_actions.clone();
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().all(|a| a.status == UndoStatus::Pending && a.attempts == 1));

    fixture.clock.advance(5000);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    let applied = fixture.gateway.applied();
    let flags = applied.iter().filter(|a| matches!(a.2, RemoteAction::Seen(true))).count();
    let moves = applied.iter().filter(|a| matches!(a.2, RemoteAction::Archive)).count();
    assert_eq!(flags, 2, "The idempotent flag is re-sent with the retried batch");
    assert_eq!(moves, 1, "The move is applied exactly once");
}

/// Review 3.2 Important 1: a due row whose same-target sibling is still
/// `Committing` (provider call stalled) must not make the deadline loop re-arm
/// immediately: the next wait is floored ~250 ms ahead while that state holds,
/// and the floor clears as soon as a pass claims again.
#[test]
fn a_due_row_blocked_by_a_committing_sibling_floors_the_next_deadline() {
    let fixture = setup();
    let a = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    let b = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.clock.advance(5000);
    // Simulate the dispatched sibling whose provider call is stalled: B stays
    // Committing while A is due again.
    fixture
        .context
        .update(|store| {
            for row in &mut store.undo_actions {
                if row.id == b.id {
                    row.status = UndoStatus::Committing;
                }
            }
            Ok(())
        })
        .unwrap();

    // First blocked pass: nothing claimed, the floor is armed.
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.gateway.calls(), 0, "a blocked row attempts nothing");
    let now = fixture.clock.now_ms();
    let floored = fixture.undo.next_deadline().unwrap().unwrap();
    assert!(
        (now + 200..=now + 300).contains(&floored),
        "the blocked state waits on the 250 ms floor, got {floored} vs now {now}"
    );

    // While the sibling stalls, every re-arm stays floored: the loop can iterate
    // at most ~4 times per simulated second, never unbounded.
    for _ in 0..8 {
        fixture.clock.advance(250);
        assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
        let now = fixture.clock.now_ms();
        let deadline = fixture.undo.next_deadline().unwrap().unwrap();
        assert!(
            deadline > now,
            "a blocked pass must never re-arm into the past, got {deadline} <= {now}"
        );
        assert!(deadline <= now + 300);
    }
    assert_eq!(fixture.gateway.calls(), 0);

    // The sibling settles: the very next pass claims A at once (no floor left)
    // and the queue drains.
    fixture
        .context
        .update(|store| {
            store.undo_actions.retain(|row| row.id != b.id);
            Ok(())
        })
        .unwrap();
    fixture.clock.advance(250);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let left = fixture.context.read().unwrap();
    assert!(
        left.undo_actions.iter().all(|row| row.id != a.id),
        "the blocked row commits once the sibling is gone"
    );
    assert_eq!(left.undo_actions.len(), 0);
}

/// Audit C1 (busy-spin): a due row whose same-target sibling was queued later
/// and is still inside its undo window cannot be claimed. That is a known wait
/// with a known end — the sibling's expiry — so the pass must neither claim
/// (every store transaction rewrites and fsyncs `store.json`) nor re-arm into
/// the past (two store reads plus a transaction per iteration, 100% CPU).
#[test]
fn a_due_row_blocked_by_an_unexpired_sibling_waits_for_its_window_without_spinning() {
    let fixture = setup();
    let a = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    // A is due; star-then-archive style: B on the same message opens a fresh window.
    fixture.clock.advance(5000);
    let b = fixture.undo.queue_mail(fixture.email_id, "pin".into(), None).unwrap();
    let now = fixture.clock.now_ms();
    assert!(a.expires_at_ms <= now && b.expires_at_ms > now);

    let writes = fixture.repo.transactions();
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap(), "nothing can be claimed");
    assert_eq!(fixture.gateway.calls(), 0, "a blocked row attempts nothing");
    assert_eq!(
        fixture.repo.transactions(),
        writes,
        "a blocked pass writes nothing: the blocker is read off the snapshot"
    );
    assert_eq!(
        fixture.undo.next_deadline().unwrap(),
        Some(b.expires_at_ms),
        "the next wake is the blocking sibling's expiry, never the past"
    );

    // Inside B's window the loop stays quiet: later passes (the loop firing
    // anyway) take the idle fast path — no store read, no store write.
    let reads = fixture.repo.reads();
    for _ in 0..10 {
        fixture.clock.advance(300);
        assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
        let now = fixture.clock.now_ms();
        let deadline = fixture.undo.next_deadline().unwrap().unwrap();
        assert!(deadline > now, "re-armed into the past: {deadline} <= {now}");
        assert_eq!(deadline, b.expires_at_ms);
    }
    assert_eq!(fixture.repo.reads(), reads, "no store read while the window runs");
    assert_eq!(fixture.repo.transactions(), writes, "no store write while the window runs");
    assert_eq!(fixture.gateway.calls(), 0);

    // At B's expiry both rows are due: one pass claims them as one batch.
    fixture.clock.advance(2000);
    assert_eq!(fixture.clock.now_ms(), b.expires_at_ms);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.gateway.applied().len(), 2, "both actions reach the provider once");
}

/// The blocker leaving its window early (undo) frees the due row at once: the
/// wake is the writer's signal, so waiting on the sibling's expiry never
/// strands the row.
#[test]
fn undoing_the_unexpired_sibling_frees_the_blocked_due_row_at_once() {
    let fixture = setup();
    let a = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.clock.advance(5000);
    let b = fixture.undo.queue_mail(fixture.email_id, "pin".into(), None).unwrap();
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert_eq!(fixture.undo.next_deadline().unwrap(), Some(b.expires_at_ms));

    let done = park_deadline_thread(&fixture.undo, fixture.undo.next_deadline().unwrap());
    assert!(done.try_recv().is_err(), "the thread sleeps while B holds A");
    fixture.undo.undo(b.id).unwrap();
    done.recv_timeout(Duration::from_secs(2))
        .expect("removing the blocker signals the deadline thread");
    let now = fixture.clock.now_ms();
    assert!(
        fixture.undo.next_deadline().unwrap().unwrap() <= now,
        "A is due again as soon as B is gone"
    );
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    let left = fixture.context.read().unwrap();
    assert!(left.undo_actions.iter().all(|row| row.id != a.id));
    assert_eq!(fixture.gateway.applied().len(), 1, "only A was confirmed");
}

/// Another message's deadline is never delayed by a blocked row: the wake is
/// the earliest of the blocked row's wait and every other row's own deadline.
#[test]
fn a_blocked_row_does_not_delay_an_unrelated_earlier_deadline() {
    let fixture = setup();
    let other_email = fixture.context.read().unwrap().emails[1].id;
    let a = fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.clock.advance(1000);
    let c = fixture.undo.queue_mail(other_email, "archive".into(), None).unwrap();
    fixture.clock.advance(1000);
    let b = fixture.undo.queue_mail(fixture.email_id, "pin".into(), None).unwrap();
    fixture.clock.advance(3000); // t0+5000: A due, C (t0+6000) and B (t0+7000) not
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    assert!(a.expires_at_ms <= fixture.clock.now_ms() && c.expires_at_ms < b.expires_at_ms);
    assert_eq!(
        fixture.undo.next_deadline().unwrap(),
        Some(c.expires_at_ms),
        "C's own deadline comes before the sibling B that holds A"
    );
}

/// The floor is state, not a blanket delay: an unblocked queue keeps exact
/// deadlines (a row simply not due yet waits precisely until its expiry).
#[test]
fn an_unblocked_queue_keeps_exact_deadlines() {
    let fixture = setup();
    fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    // A pass with nothing due must not arm the floor.
    assert!(!fixture.undo.commit_due(&fixture.workers).unwrap());
    let now = fixture.clock.now_ms();
    let deadline = fixture.undo.next_deadline().unwrap().unwrap();
    assert_eq!(deadline, now + 5000, "the undo window stays the exact deadline");
    fixture.clock.advance(4999);
    let deadline = fixture.undo.next_deadline().unwrap().unwrap();
    assert!(
        deadline <= fixture.clock.now_ms() + 20,
        "no floor pushes an unblocked deadline out, got {deadline}"
    );
    fixture.clock.advance(1);
    assert!(fixture.undo.commit_due(&fixture.workers).unwrap());
    settled(&fixture.context);
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
}
