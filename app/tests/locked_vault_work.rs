//! Credentials the vault cannot open right now (phase 3, Task 7: a data folder from another
//! Windows user or computer, a key Windows cannot open at the moment) hold the work that needs
//! them, and never turn it into a failure: a scheduled send stays armed, a send or an archive left
//! in the undo queue stays queued. The vault refuses before any provider is contacted, so once it
//! opens (`vault_unlock`, `credentials_opened`) the same work leaves at once.
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        drafts::DraftService,
        error::vault_codes,
        events::AppEvent,
        mail::MailService,
        ports::*,
        scheduler::SchedulerService,
        undo::UndoService,
    },
    domain::undo::{UndoAction, UndoStatus},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const LOCKED: &str = "The credentials key belongs to another Windows user or another computer: enter the transfer passphrase in Settings, or sign in to the accounts again.";

/// The scheduler's waker and the queue's connection wakes are process-wide: one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// The vault as `KeyringSecrets` reports it while the key does not open here: every read refused
/// with `vault_locked`, until the transfer passphrase opens it.
#[derive(Default)]
struct Vault {
    locked: AtomicBool,
    reads: AtomicU64,
    /// Reads that still open before the lock applies: the vault closing between two reads.
    open_reads: Mutex<u64>,
}
impl Vault {
    fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }
    fn lock(&self) {
        self.locked.store(true, Ordering::SeqCst);
    }
    fn unlock(&self) {
        self.locked.store(false, Ordering::SeqCst);
    }
}
impl SecretStore for Vault {
    fn get(&self, _: &str) -> AppResult<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let still_open = {
            let mut left = self.open_reads.lock().unwrap();
            let open = *left > 0;
            *left = left.saturating_sub(1);
            open
        };
        if self.locked.load(Ordering::SeqCst) && !still_open {
            return Err(AppError::new(vault_codes::LOCKED, LOCKED));
        }
        Ok("test-only".into())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
/// The provider: it only counts what reached it.
#[derive(Default)]
struct Provider {
    sends: AtomicU64,
    applied: AtomicU64,
}
impl MailGateway for Provider {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    fn send_failures(&self) -> usize {
        self.0.lock().unwrap().iter().filter(|e| matches!(e, AppEvent::ScheduledSendFailed { .. })).count()
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
struct Files;
impl AttachmentGateway for Files {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

struct Fixture {
    _serial: std::sync::MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    clock: Arc<MovingClock>,
    vault: Arc<Vault>,
    provider: Arc<Provider>,
    events: Arc<Recorder>,
    scheduler: SchedulerService,
    undo: UndoService,
    draft_id: u64,
    email_id: u64,
}
fn setup() -> Fixture {
    let serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    qamille::application::connectivity::drain_wakes();
    let mut store = sample_store();
    // A Gmail account: its credentials come from the vault (the sample provider needs none).
    store.accounts[0].kind = AccountKind::Gmail;
    store.accounts[0].archive_folder = "[Gmail]/All Mail".into();
    store.accounts[0].trash_folder = "[Gmail]/Trash".into();
    store.settings.undo_seconds = 5;
    for email in &mut store.emails {
        email.snoozed_until = None;
        email.reminder_at = None;
    }
    store.emails[0].uid = Some(10);
    for draft in &mut store.drafts {
        draft.send_at = None;
    }
    let draft_id = store.drafts[0].id;
    let email_id = store.emails[0].id;
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let vault = Arc::new(Vault::default());
    let provider = Arc::new(Provider::default());
    let events = Arc::new(Recorder::default());
    let context = Arc::new(ServiceContext {
        compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: events.clone(),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: provider.clone(),
        secrets: vault.clone(),
        clock: clock.clone(),
        attachments: Arc::new(Files),
    });
    let drafts = DraftService::new(context.clone());
    let scheduler = SchedulerService::new(context.clone(), drafts.clone());
    let undo = UndoService::new(context.clone(), MailService::new(context.clone()), drafts);
    Fixture { _serial: serial, context, clock, vault, provider, events, scheduler, undo, draft_id, email_id }
}
impl Fixture {
    fn draft(&self) -> Option<Draft> {
        self.context.read().unwrap().drafts.into_iter().find(|d| d.id == self.draft_id)
    }
    fn schedule_now(&self) -> i64 {
        let now = self.clock.now();
        self.context
            .update(|store| {
                store.drafts.iter_mut().find(|d| d.id == self.draft_id).unwrap().send_at = Some(now);
                Ok(())
            })
            .unwrap();
        now
    }
    fn only_row(&self) -> UndoAction {
        let rows = self.context.read().unwrap().undo_actions;
        assert_eq!(rows.len(), 1, "{rows:?}");
        rows[0].clone()
    }
    fn sends(&self) -> u64 {
        self.provider.sends.load(Ordering::SeqCst)
    }
}

#[test]
fn a_scheduled_send_waits_for_the_vault_and_leaves_once_it_opens() {
    let fixture = setup();
    fixture.vault.lock();
    let due = fixture.schedule_now();
    assert!(fixture.scheduler.tick().unwrap().is_empty(), "no «non riuscito» notice");
    assert_eq!(fixture.events.send_failures(), 0, "no Windows notification of a failed send");
    assert_eq!(fixture.sends(), 0);
    assert_eq!(fixture.draft().unwrap().send_at, Some(due), "the send stays armed");
    assert!(fixture.context.read().unwrap().outbox.is_empty(), "no delivery was started");
    // Held, not spinning: the thread sleeps past the overdue time, and reads nothing before then.
    let recheck = fixture.scheduler.next_deadline().unwrap().expect("a held send is rechecked");
    assert!(recheck > fixture.clock.now_ms(), "a deadline in the past would spin the scheduler thread");
    let reads = fixture.vault.reads();
    assert!(fixture.scheduler.tick().unwrap().is_empty());
    assert_eq!(fixture.vault.reads(), reads, "a held send does not ask the vault again before its recheck");
    // Its recheck asks again, and holds it again while the vault stays shut.
    fixture.clock.advance((recheck - fixture.clock.now_ms()) as u64);
    assert!(fixture.scheduler.tick().unwrap().is_empty());
    assert!(fixture.vault.reads() > reads);
    assert_eq!(fixture.draft().unwrap().send_at, Some(due));
    // The transfer passphrase opens the vault: the send leaves now, not at the next recheck.
    fixture.vault.unlock();
    fixture.scheduler.credentials_opened();
    assert!(fixture.scheduler.next_deadline().unwrap().is_some_and(|at| at <= fixture.clock.now_ms()));
    assert!(fixture.scheduler.tick().unwrap().is_empty());
    assert_eq!(fixture.sends(), 1);
    assert!(fixture.draft().is_none(), "a sent draft leaves the drafts");
    assert_eq!(fixture.events.send_failures(), 0);
}

#[test]
fn a_vault_that_shuts_between_the_check_and_the_send_puts_the_schedule_back() {
    let fixture = setup();
    fixture.vault.lock();
    // The scheduler's own check still opens; the send's read, right after its claim, does not.
    *fixture.vault.open_reads.lock().unwrap() = 1;
    let due = fixture.schedule_now();
    assert!(fixture.scheduler.tick().unwrap().is_empty(), "no «non riuscito» notice");
    assert_eq!(fixture.events.send_failures(), 0);
    assert_eq!(fixture.sends(), 0);
    assert_eq!(fixture.draft().unwrap().send_at, Some(due), "the claim is undone: the send is armed again");
    assert!(fixture.context.read().unwrap().outbox.is_empty(), "no delivery was started");
    assert!(fixture.scheduler.next_deadline().unwrap().is_some_and(|at| at > fixture.clock.now_ms()), "held");
    fixture.vault.unlock();
    fixture.scheduler.credentials_opened();
    fixture.scheduler.tick().unwrap();
    assert_eq!(fixture.sends(), 1);
}

#[test]
fn a_send_left_in_the_undo_queue_waits_for_the_vault_and_leaves_once_it_opens() {
    let fixture = setup();
    fixture.undo.queue_draft(fixture.draft_id, "send", None).unwrap();
    // The queue's leftover meets a vault that does not open here (the folder moved to another PC).
    fixture.vault.lock();
    fixture.clock.advance(6_000);
    fixture.undo.process_due().unwrap();
    let row = fixture.only_row();
    assert_eq!(row.status, UndoStatus::Pending, "held, never failed: {row:?}");
    assert_eq!(row.error.as_deref(), Some(LOCKED), "the notice says why it waits");
    assert!(row.retry_at_ms.is_some_and(|at| at > fixture.clock.now_ms()), "not due again at once: {row:?}");
    assert_eq!(fixture.sends(), 0);
    assert!(fixture.context.read().unwrap().outbox.is_empty(), "no delivery was started");
    assert!(fixture.undo.next_deadline().unwrap().is_some_and(|at| at > fixture.clock.now_ms()));
    // Opened: due now, and the send leaves on the next pass.
    fixture.vault.unlock();
    fixture.undo.credentials_opened().unwrap();
    assert_eq!(fixture.undo.next_deadline().unwrap(), Some(fixture.clock.now_ms()));
    fixture.undo.process_due().unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.sends(), 1);
}

#[test]
fn an_archive_left_in_the_undo_queue_waits_for_the_vault_instead_of_failing() {
    let fixture = setup();
    fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    fixture.vault.lock();
    fixture.clock.advance(6_000);
    fixture.undo.process_due().unwrap();
    let row = fixture.only_row();
    assert_eq!(row.status, UndoStatus::Pending, "held, never failed: {row:?}");
    assert_eq!(row.error.as_deref(), Some(LOCKED));
    assert_eq!(fixture.provider.applied.load(Ordering::SeqCst), 0);
    fixture.vault.unlock();
    fixture.undo.credentials_opened().unwrap();
    fixture.undo.process_due().unwrap();
    assert!(fixture.context.read().unwrap().undo_actions.is_empty());
    assert_eq!(fixture.provider.applied.load(Ordering::SeqCst), 1);
}

#[test]
fn opening_the_vault_with_nothing_held_writes_nothing() {
    let fixture = setup();
    fixture.undo.queue_mail(fixture.email_id, "archive".into(), None).unwrap();
    let before = fixture.context.read().unwrap().undo_actions;
    fixture.undo.credentials_opened().unwrap();
    fixture.scheduler.credentials_opened();
    // A row still inside its undo window stays undoable: opening the vault sends nothing early.
    assert_eq!(fixture.context.read().unwrap().undo_actions, before);
}
