//! The undo queue writes the activity log (centro attività, Task 4): a user
//! action gets its row when it is queued, the undo window can turn it into
//! "undone", a definitive failure or a given-up retry into "failed", and a log
//! that cannot be written never blocks the action itself.

use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        account_worker::{CommitOutcome, WorkerRegistry},
        connectivity,
        drafts::DraftService,
        events::AppEvent,
        mail::MailService,
        ports::*,
        undo::UndoService,
    },
    domain::{
        activity::*,
        undo::{UndoAction, UndoStatus},
    },
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

/// `connectivity` keeps process-wide state, so these tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The provider seam: fails the next `apply` calls with the queued messages.
#[derive(Default)]
struct Gateway {
    script: Mutex<VecDeque<String>>,
}
impl Gateway {
    fn fail_next(&self, message: &str) {
        self.script.lock().unwrap().push_back(message.into());
    }
}
impl MailGateway for Gateway {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        match self.script.lock().unwrap().pop_front() {
            Some(message) => Err(AppError::external(message)),
            None => Ok(()),
        }
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
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

#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// A log whose every write fails; the store itself works.
struct BrokenLog(MemoryRepository);
impl StoreRepository for BrokenLog {
    fn read(&self) -> AppResult<Store> {
        self.0.read()
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.0.transaction(op)
    }
    fn activity_supported(&self) -> bool {
        true
    }
    fn append_activity(&self, _: &NewActivity, _: i64) -> AppResult<Option<i64>> {
        Err(AppError::storage("disk full"))
    }
    fn set_activity_outcome(&self, _: u64, _: ActivityOutcome, _: Option<&str>) -> AppResult<Option<i64>> {
        Err(AppError::storage("disk full"))
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

struct Fixture {
    _guard: std::sync::MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    gateway: Arc<Gateway>,
    clock: Arc<MovingClock>,
    undo: Arc<UndoService>,
    workers: WorkerRegistry,
    events: Arc<Recorder>,
    email_id: u64,
    account_id: u32,
}
fn setup_with(repository: impl FnOnce(Store) -> Arc<dyn StoreRepository>) -> Fixture {
    let guard = serial();
    connectivity::drain_wakes();
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let gateway = Arc::new(Gateway::default());
    let events = Arc::new(Recorder::default());
    let store = fixture();
    let (email_id, account_id) = (store.emails[0].id, store.emails[0].account_id);
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: events.clone(),
        repository: repository(store),
        mail: gateway.clone(),
        secrets: Arc::new(Secrets),
        clock: clock.clone(),
        attachments: Arc::new(Files),
    });
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
    Fixture { _guard: guard, context, gateway, clock, undo, workers, events, email_id, account_id }
}
fn setup() -> Fixture {
    setup_with(|store| Arc::new(MemoryRepository::new(store)))
}

fn rows(fixture: &Fixture) -> Vec<ActivityEntry> {
    fixture.context.repository.activity_page(None, 100, ActivityFilter::All).unwrap()
}
fn queue_archive(fixture: &Fixture) -> UndoAction {
    fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap()
}
/// `commit_due` hands claimed rows to worker threads; wait for every claim to settle.
fn settled(fixture: &Fixture) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture
        .context
        .read()
        .unwrap()
        .undo_actions
        .iter()
        .any(|a| a.status == UndoStatus::Committing)
    {
        assert!(Instant::now() < deadline, "the worker never settled the dispatched confirmation");
        std::thread::sleep(Duration::from_millis(5));
    }
}
/// The log row is written right after the queue transaction that settled the
/// action (never inside it), so a worker's failure shows in the log a moment
/// after the queue row reads `Failed`: wait for the row, not for the queue.
fn log_outcome_settles_to(fixture: &Fixture, expected: ActivityOutcome) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while rows(fixture).first().map(|row| row.outcome) != Some(expected) {
        assert!(Instant::now() < deadline, "the activity row never became {expected:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn queued_action(fixture: &Fixture) -> UndoAction {
    fixture.context.read().unwrap().undo_actions[0].clone()
}

const OFFLINE: &str =
    "IMAP connect: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)";
const AUTHENTICATION: &str = "IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)";

#[test]
fn queued_done_action_is_logged_with_account_and_subject() {
    let fixture = setup();
    let pending = fixture.undo.queue_mail(fixture.email_id, "markAsDone".into(), None).unwrap();
    let rows = rows(&fixture);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.kind, row.source, row.outcome), (ActivityKind::Action, ActivitySource::User, ActivityOutcome::Done));
    assert_eq!(row.code, "markAsDone");
    assert_eq!(row.undo_id, Some(pending.id));
    assert_eq!(row.account_id, Some(fixture.account_id));
    assert_eq!(row.subject.as_deref(), Some(pending.subject.as_str()));
    assert_eq!((row.ref_kind.as_deref(), row.ref_id), (Some("email"), Some(fixture.email_id)));
    assert!(
        fixture.events.0.lock().unwrap().contains(&AppEvent::ActivityChanged { ids: vec![row.seq as u64] }),
        "the row is announced"
    );
}

#[test]
fn queued_unlogged_action_writes_nothing() {
    let fixture = setup();
    fixture.undo.queue_mail(fixture.email_id, "markAsSeen".into(), None).unwrap();
    assert!(rows(&fixture).is_empty());
}

#[test]
fn undo_marks_the_row_undone() {
    let fixture = setup();
    let pending = queue_archive(&fixture);
    fixture.undo.undo(pending.id).unwrap();
    let rows = rows(&fixture);
    assert_eq!(rows.len(), 1, "undo corrects the row, it does not add one");
    assert_eq!(rows[0].outcome, ActivityOutcome::Undone);
}

#[test]
fn definitive_failure_marks_the_row_failed_with_the_error() {
    let fixture = setup();
    let pending = queue_archive(&fixture);
    fixture.gateway.fail_next(AUTHENTICATION);
    fixture.clock.advance(5000);
    fixture.undo.commit_due(&fixture.workers).unwrap();
    settled(&fixture);

    let failed = queued_action(&fixture);
    assert_eq!(failed.status, UndoStatus::Failed);
    log_outcome_settles_to(&fixture, ActivityOutcome::Failed);
    let rows = rows(&fixture);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, ActivityOutcome::Failed);
    assert_eq!(rows[0].detail, failed.error, "the row keeps the reason the notice shows");
    assert!(rows[0].detail.is_some());

    fixture.undo.dismiss(pending.id).unwrap();
    let after = self::rows(&fixture);
    assert_eq!(after.len(), 1, "reading and dismissing the failure notice adds nothing");
    assert_eq!(after[0].outcome, ActivityOutcome::Failed);
}

#[test]
fn retryable_failure_leaves_the_row_done() {
    let fixture = setup();
    queue_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    fixture.clock.advance(5000);
    fixture.undo.commit_due(&fixture.workers).unwrap();
    settled(&fixture);

    let retrying = queued_action(&fixture);
    assert_eq!((retrying.status, retrying.attempts), (UndoStatus::Pending, 1));
    assert_eq!(rows(&fixture)[0].outcome, ActivityOutcome::Done, "a backoff is not a failure");
}

#[test]
fn giving_up_on_a_retrying_action_marks_the_row_failed() {
    let fixture = setup();
    let pending = queue_archive(&fixture);
    fixture.gateway.fail_next(OFFLINE);
    fixture.clock.advance(5000);
    fixture.undo.commit_due(&fixture.workers).unwrap();
    settled(&fixture);
    let reason = queued_action(&fixture).error;
    assert_eq!(reason.as_deref(), Some(OFFLINE));

    fixture.undo.dismiss(pending.id).unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty(), "Rinuncia removes the row from the queue");
    let rows = rows(&fixture);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, ActivityOutcome::Failed);
    assert_eq!(rows[0].detail, reason);
}

#[test]
fn interrupted_commit_marks_row_failed() {
    let fixture = setup();
    let pending = queue_archive(&fixture);
    fixture
        .context
        .repository
        .transaction(&mut |store| {
            store.undo_actions[0].status = UndoStatus::Committing;
            Ok(())
        })
        .unwrap();

    fixture.undo.recover_interrupted().unwrap();
    let recovered = queued_action(&fixture);
    assert_eq!((recovered.id, recovered.status), (pending.id, UndoStatus::Failed));
    let rows = rows(&fixture);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, ActivityOutcome::Failed);
    assert_eq!(rows[0].detail, recovered.error);
}

#[test]
fn queue_succeeds_when_activity_write_fails() {
    let fixture = setup_with(|store| Arc::new(BrokenLog(MemoryRepository::new(store))));
    let pending = queue_archive(&fixture);
    assert_eq!(fixture.context.read().unwrap().undo_actions.len(), 1, "the action is queued");
    fixture.undo.undo(pending.id).expect("undo works with a log that cannot be written");
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    let events = fixture.events.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(event, AppEvent::UndoChanged { .. })), "the toast still shows");
    assert!(
        !events.iter().any(|event| matches!(event, AppEvent::ActivityChanged { .. })),
        "nothing was logged, so nothing is announced"
    );
}
