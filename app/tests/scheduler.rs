//! ADR-0007 decision 4 / INV-005: the scheduler wakes on computed deadlines or
//! writer notes, never on a fixed interval, and a delivery failure can never
//! skip the snooze/reminder wake that runs before it.

use qamille::{
    application::{
        AppResult, ServiceContext,
        drafts::DraftService,
        scheduler::{SchedulerService, note_schedule_change},
        ports::*,
    },
    domain::outbox::{Delivery, DeliveryStatus},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// The scheduler waker is process-global, so these tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
struct Gateway {
    sends: AtomicU64,
}
impl MailGateway for Gateway {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch {
            emails: Vec::new(),
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn apply_message(&self, _: &Account, _: &Email, _: RemoteAction) -> AppResult<()> {
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

struct Fixture {
    _guard: std::sync::MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    clock: Arc<MovingClock>,
    scheduler: SchedulerService,
}
fn setup() -> Fixture {
    let guard = serial();
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let mut store = sample_store();
    store.accounts[0].kind = AccountKind::Gmail;
    // The sample data carries a past reminder: every computed deadline would
    // be dominated by it, so the schedule starts clean.
    for email in &mut store.emails {
        email.snoozed_until = None;
        email.reminder_at = None;
    }
    for draft in &mut store.drafts {
        draft.send_at = None;
    }
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(Gateway::default()),
        secrets: Arc::new(Secrets),
        clock: clock.clone(),
        attachments: Arc::new(Files),
    });
    let scheduler = SchedulerService::new(context.clone(), DraftService::new(context.clone()));
    Fixture {
        _guard: guard,
        context,
        clock,
        scheduler,
    }
}

#[test]
fn next_deadline_is_the_earliest_across_snooze_reminder_send_and_outbox() {
    let fixture = setup();
    let now = fixture.clock.now();
    fixture
        .context
        .update(|store| {
            store.emails[0].snoozed_until = Some(now + 300);
            store.emails[1].reminder_at = Some(now + 240);
            let mut draft = store.drafts[0].clone();
            draft.id = store.new_id();
            draft.send_at = Some(now + 120);
            draft.to = "recipient@example.com".into();
            store.drafts.push(draft.clone());
            let delivery_id = store.new_id();
            store.outbox.push(Delivery {
                id: delivery_id,
                key: "k".into(),
                request_hash: "h".into(),
                draft: draft.clone(),
                plain: String::new(),
                html: None,
                sender_name: String::new(),
                attachment_bytes: 0,
                status: DeliveryStatus::Queued,
                created_at: now,
                updated_at: now,
                send_at: now + 600,
                requested_send_at: None,
                error: None,
                owner_pid: None,
                requires_mcp_send: false,
                source_profile: None,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fixture.scheduler.next_deadline().unwrap(),
        Some((now + 120) * 1000),
        "the scheduled send is the earliest of the four deadline kinds"
    );
}

#[test]
fn a_send_failure_does_not_skip_the_wake() {
    let fixture = setup();
    let now = fixture.clock.now();
    fixture
        .context
        .update(|store| {
            store.emails[0].snoozed_until = Some(now);
            store.emails[0].folder = "Snoozed".into();
            // A scheduled draft with no recipient: its send fails validation.
            store.drafts[0].send_at = Some(now);
            store.drafts[0].to.clear();
            Ok(())
        })
        .unwrap();
    let notices = fixture.scheduler.tick().unwrap();
    assert_eq!(notices.len(), 1, "the failed send reports once: {notices:?}");
    let store = fixture.context.read().unwrap();
    assert_eq!(
        store.emails[0].snoozed_until, None,
        "the wake ran before the send failure"
    );
    assert_eq!(store.emails[0].folder, "Inbox");
    assert_eq!(store.drafts[0].send_at, None, "the claim persisted");
}

#[test]
fn a_writer_note_wakes_a_thread_sleeping_until_a_far_deadline() {
    let fixture = setup();
    let scheduler = fixture.scheduler.clone();
    let far = fixture.clock.now_ms() + 60_000;
    let waiter = std::thread::spawn(move || scheduler.wait_for_deadline(Some(far)));
    std::thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    note_schedule_change();
    waiter.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the note must wake the sleeper at once"
    );
}

#[test]
fn a_past_deadline_returns_without_waiting_and_an_empty_queue_sleeps_until_noted() {
    let fixture = setup();
    let started = Instant::now();
    fixture
        .scheduler
        .wait_for_deadline(Some(fixture.clock.now_ms() - 1));
    assert!(started.elapsed() < Duration::from_millis(500));

    let scheduler = fixture.scheduler.clone();
    let waiter = std::thread::spawn(move || scheduler.wait_for_deadline(None));
    std::thread::sleep(Duration::from_millis(150));
    assert!(!waiter.is_finished(), "an empty queue waits indefinitely");
    note_schedule_change();
    waiter.join().unwrap();
}

#[test]
fn an_outbox_enqueue_wakes_a_thread_sleeping_without_deadlines() {
    let fixture = setup();
    let scheduler = fixture.scheduler.clone();
    let waiter = std::thread::spawn(move || scheduler.wait_for_deadline(None));
    std::thread::sleep(Duration::from_millis(150));
    let now = fixture.clock.now();
    let draft_id = fixture.context.read().unwrap().drafts[0].id;
    let started = Instant::now();
    qamille::application::mcp_outbox::enqueue(
        fixture.context.clone(),
        draft_id,
        None,
        None,
        Some(now + 3_600),
        false,
        None,
    )
    .expect("the sample draft is sendable");
    waiter.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the enqueue note must wake the sleeper at once"
    );
    assert_eq!(
        fixture.scheduler.next_deadline().unwrap(),
        Some((now + 3_600) * 1000),
        "the scheduled delivery is now the earliest deadline"
    );
}
