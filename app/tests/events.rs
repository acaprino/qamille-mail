//! Task 2.2 (ADR-0007 v3 decision 5, INV-004): the writers announce what they
//! touched, once, after the commit — `queue_mail` an `UndoChanged` row,
//! `process_due` the removal plus the changed mail and counters, a sync one
//! `EmailChanged` with every id it merged per account and one `SyncProgress`
//! per phase, drafts their `DraftsChanged`, the scheduler wake and the indexer
//! targeted events. An idle pass emits nothing, and no event has an empty
//! payload: ids that no longer exist are omitted, never announced.

use qamille::{
    application::{
        AppError, AppResult, ServiceContext, connectivity,
        account_worker::{CommitOutcome, WorkerRegistry},
        contact_ports::NullContactGateway,
        contacts::ContactService,
        drafts::DraftService,
        events::AppEvent,
        indexing::MailIndexService,
        indexing_ports::{DocumentIndexReport, IndexContent, IndexPage, IndexSource},
        mail::MailService,
        mail_sync::MailSyncService,
        mail_sync_status::SyncPhase,
        ports::*,
        scheduler::SchedulerService,
        undo::UndoService,
    },
    domain::{
        indexing::FolderProgress,
        undo::{UndoAction, UndoOperation, UndoStatus},
    },
    infrastructure::{
        contact_repository::MemoryContactStore, repository::MemoryRepository,
    },
    model::*,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// `connectivity` keeps process-wide state, so the tests run one at a time: a
/// wake noted by one test must never be drained by another test's pass.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Records every emission, in order.
#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    /// The mail, undo, progress and notice events in order. The activity log's own
    /// announcements (`ActivityChanged`, centro attività) are left out: these tests pin the
    /// sequences of INV-004, and the log is covered by `all_events` and `app/tests/activity_log.rs`.
    fn events(&self) -> Vec<AppEvent> {
        self.all_events()
            .into_iter()
            .filter(|event| !matches!(event, AppEvent::ActivityChanged { .. }))
            .collect()
    }
    /// Every emission, the activity log's included.
    fn all_events(&self) -> Vec<AppEvent> {
        self.0.lock().unwrap().clone()
    }
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// The live-sync dispatch test needs a registry; no confirmation is ever
/// routed through it, so the outcome sink stays inert.
struct NoopOutcome;
impl CommitOutcome for NoopOutcome {
    fn commit_finished(&self, _: &[u64], _: &AppResult<()>) {}
}

/// The provider seam: scripted failures for the retry paths and a settable
/// download batch for the sync paths.
#[derive(Default)]
struct Gateway {
    script: Mutex<VecDeque<String>>,
    batch: Mutex<Vec<Email>>,
    applied: Mutex<Vec<(u32, u32, RemoteAction)>>,
}
impl Gateway {
    fn fail_next(&self, message: &str) {
        self.script.lock().unwrap().push_back(message.into());
    }
}
impl MailGateway for Gateway {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch {
            emails: self.batch.lock().unwrap().clone(),
            archive_folder: Some("All".into()),
            trash_folder: Some("Bin".into()),
        })
    }
    fn apply(&self, account: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        if let Some(message) = self.script.lock().unwrap().pop_front() {
            return Err(AppError::external(message));
        }
        self.applied.lock().unwrap().push((account.id, uid, action));
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
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

struct Harness {
    _guard: MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    gateway: Arc<Gateway>,
    clock: Arc<MovingClock>,
    recorder: Arc<Recorder>,
    /// The Gmail account of `fixture` and its first message (uid 10).
    account_id: u32,
    email_id: u64,
}
fn harness(store: Store) -> Harness {
    let guard = serial();
    connectivity::drain_wakes();
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let gateway = Arc::new(Gateway::default());
    let recorder = Arc::new(Recorder::default());
    let (account_id, email_id) = (store.emails[0].account_id, store.emails[0].id);
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: recorder.clone(),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: gateway.clone(),
        secrets: Arc::new(Secrets),
        clock: clock.clone(),
        attachments: Arc::new(Files),
    });
    Harness {
        _guard: guard,
        context,
        gateway,
        clock,
        recorder,
        account_id,
        email_id,
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
fn setup() -> Harness {
    harness(fixture())
}
fn undo_service(h: &Harness) -> UndoService {
    UndoService::new(
        h.context.clone(),
        MailService::new(h.context.clone()),
        DraftService::new(h.context.clone()),
    )
}
/// A download the merge cannot match anything against: a brand new message.
fn remote_mail(account_id: u32, uid: u32) -> Email {
    Email {
        account_id,
        uid: Some(uid),
        remote_uid_validity: Some(7),
        remote_folder: Some("INBOX".into()),
        folder: "Inbox".into(),
        message_id: format!("<new-{uid}@example.com>"),
        subject: format!("New arrival {uid}"),
        date: 1_999_000_000,
        ..Default::default()
    }
}

const OFFLINE: &str =
    "IMAP connect: Impossibile stabilire la connessione. Una operazione di connessione non riuscita (os error 10060)";
/// A definitive provider error: one attempt, immediate failure row (EX-010).
const AUTHENTICATION: &str =
    "IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)";

/// Step 1 of the brief: `queue_mail` emits `UndoChanged`; `process_due` after
/// the window emits `UndoRemoved` + `EmailChanged{ids:[id]}` + `CountsChanged`.
#[test]
fn done_emits_email_changed_and_undo_events() {
    let h = setup();
    let undo = undo_service(&h);
    let (email_id, account_id) = (h.email_id, h.account_id);
    let pending = undo
        .queue_mail(email_id, "markAsDone".into(), None)
        .unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::UndoChanged {
            action: pending.clone()
        }],
        "queueing announces the row the UI must show"
    );
    h.recorder.clear();
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap());
    let events = h.recorder.events();
    assert!(
        matches!(events.first(), Some(AppEvent::UndoChanged { action }) if action.status == UndoStatus::Committing),
        "the claim announces the confirming state: {events:?}"
    );
    let at = events
        .iter()
        .position(|event| matches!(event, AppEvent::UndoRemoved { .. }))
        .expect("the committed action leaves the queue");
    assert_eq!(
        &events[at..],
        &[
            AppEvent::UndoRemoved { id: pending.id },
            AppEvent::EmailChanged {
                account_id,
                ids: vec![email_id]
            },
            AppEvent::CountsChanged { account_id },
        ],
        "removal, then the touched message, then the counters"
    );
    let stored = h.context.read().unwrap();
    assert!(stored.email(email_id).unwrap().done);
    assert!(stored.undo_actions.is_empty());
}

/// One merge per account is one `EmailChanged` with every id it touched, and
/// one `SyncProgress` per phase (brief step 1).
#[test]
fn sync_of_two_new_emails_emits_one_email_changed_with_both_ids() {
    let h = setup();
    let account_id = h.account_id;
    *h.gateway.batch.lock().unwrap() = vec![remote_mail(account_id, 101), remote_mail(account_id, 102)];
    let mail = MailService::new(h.context.clone());
    let report = mail.sync().unwrap();
    assert_eq!(report.added, 2);
    let events = h.recorder.events();
    let changed: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, AppEvent::EmailChanged { .. }))
        .collect();
    assert_eq!(changed.len(), 1, "one merge, one event: {events:?}");
    let stored: Vec<u64> = h
        .context
        .read()
        .unwrap()
        .emails
        .iter()
        .filter(|email| matches!(email.uid, Some(101 | 102)))
        .map(|email| email.id)
        .collect();
    assert_eq!(stored.len(), 2);
    match changed[0] {
        AppEvent::EmailChanged {
            account_id: id,
            ids,
        } => {
            assert_eq!(*id, account_id);
            assert_eq!(ids, &stored, "the event names both merged messages");
        }
        _ => unreachable!(),
    }
    let phases: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::SyncProgress {
                account_id: id,
                phase,
                ..
            } if *id == account_id => Some(*phase),
            _ => None,
        })
        .collect();
    assert_eq!(phases, vec![SyncPhase::Syncing, SyncPhase::Complete]);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AppEvent::CountsChanged { account_id: id } if *id == account_id))
            .count(),
        1
    );
    match events.iter().find(|event| {
        matches!(event, AppEvent::SyncProgress { phase: SyncPhase::Complete, added: 2, .. })
    }) {
        Some(_) => {}
        None => panic!("the closing phase reports the added count: {events:?}"),
    }
}

/// An idle live-sync step performs no merge and emits nothing at all. The
/// downloads it dispatches run on the account workers (Task 3.3), so their
/// phases arrive asynchronously and are awaited before the idle assertion.
#[test]
fn an_idle_live_sync_step_emits_nothing() {
    let h = setup();
    let registry = Arc::new(WorkerRegistry::new(
        MailService::new(h.context.clone()),
        DraftService::new(h.context.clone()),
        h.context.clone(),
        h.recorder.clone(),
        Arc::new(NoopOutcome),
    ));
    let mut sync = MailSyncService::new(h.context.clone(), registry);
    // The first step dispatches the fallback download, one `Job::Sync` per
    // syncable account: an empty batch changes no row, so only the activity
    // phases are announced.
    let report = sync.step(Duration::from_millis(1)).unwrap();
    assert!(report.updated, "the fallback dispatches every account");
    let expected = h
        .context
        .read()
        .unwrap()
        .accounts
        .iter()
        .filter(|account| h.gateway.capabilities(account).sync)
        .count();
    let deadline = Instant::now() + Duration::from_secs(5);
    while h
        .recorder
        .events()
        .iter()
        .filter(|event| matches!(event, AppEvent::SyncProgress { phase: SyncPhase::Complete, .. }))
        .count()
        < expected
    {
        assert!(
            Instant::now() < deadline,
            "the dispatched syncs never completed: {:?}",
            h.recorder.events()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !h.recorder.events().iter().any(|event| matches!(
            event,
            AppEvent::EmailChanged { .. } | AppEvent::CountsChanged { .. }
        )),
        "an empty merge announces no change: {:?}",
        h.recorder.events()
    );
    h.recorder.clear();
    // Nothing is dirty and no deadline is due: the step waits, dispatches
    // nothing and stays silent.
    let report = sync.step(Duration::from_millis(1)).unwrap();
    assert!(!report.updated);
    assert!(
        h.recorder.events().is_empty(),
        "an idle step emits nothing: {:?}",
        h.recorder.events()
    );
}

/// Draft writes announce the draft ids they touched — save, discard and the
/// queue commit of a discard alike.
#[test]
fn draft_writes_emit_drafts_changed() {
    let h = setup();
    let drafts = DraftService::new(h.context.clone());
    let draft = drafts.create(Some(h.account_id), None, false).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::DraftsChanged {
            ids: vec![draft.id]
        }]
    );
    h.recorder.clear();
    let saved = drafts.save(draft.clone()).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::DraftsChanged {
            ids: vec![draft.id]
        }]
    );
    h.recorder.clear();
    drafts.discard(saved.id).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::DraftsChanged {
            ids: vec![draft.id]
        }]
    );
    h.recorder.clear();
    // A failed save announces nothing.
    let error = drafts.save(draft.clone()).unwrap_err();
    assert_eq!(error.code, "not_found");
    assert!(h.recorder.events().is_empty());
}

/// The queue commit of a draft action: the writer's `DraftsChanged` comes with
/// the `UndoRemoved` of the consumed queue row, in that order.
#[test]
fn a_committed_draft_discard_emits_drafts_changed_then_undo_removed() {
    let h = setup();
    let drafts = DraftService::new(h.context.clone());
    let undo = undo_service(&h);
    let draft = drafts.create(Some(h.account_id), None, false).unwrap();
    h.recorder.clear();
    let pending = undo.queue_draft(draft.id, "discard", None).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::UndoChanged {
            action: pending.clone()
        }]
    );
    h.recorder.clear();
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap());
    let events = h.recorder.events();
    let drafts_at = events
        .iter()
        .position(|event| *event == AppEvent::DraftsChanged { ids: vec![draft.id] })
        .unwrap_or_else(|| panic!("the discard announces the draft: {events:?}"));
    let removed_at = events
        .iter()
        .position(|event| *event == AppEvent::UndoRemoved { id: pending.id })
        .unwrap_or_else(|| panic!("the consumed queue row leaves the UI: {events:?}"));
    assert!(drafts_at < removed_at);
    let stored = h.context.read().unwrap();
    assert!(!stored.drafts.iter().any(|d| d.id == draft.id));
    assert!(stored.undo_actions.is_empty());
}

/// INV-010 in the UI: a retryable failure updates the row (pending, attempts,
/// backoff) instead of removing it or showing a failure.
#[test]
fn retry_scheduling_emits_undo_changed_with_the_backoff() {
    let h = setup();
    let undo = undo_service(&h);
    let pending = undo
        .queue_mail(h.email_id, "archive".into(), None)
        .unwrap();
    h.recorder.clear();
    h.gateway.fail_next(OFFLINE);
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap());
    let events = h.recorder.events();
    let retry_row = events
        .iter()
        .rev()
        .find(|event| {
            matches!(event, AppEvent::UndoChanged { action } if action.status == UndoStatus::Pending)
        })
        .unwrap_or_else(|| panic!("the scheduled retry updates the row: {events:?}"));
    match retry_row {
        AppEvent::UndoChanged { action } => {
            assert_eq!(action.id, pending.id);
            assert_eq!(action.attempts, 1);
            assert_eq!(action.retry_at_ms, Some(h.clock.now_ms() + 5_000));
            assert_eq!(action.error.as_deref(), Some(OFFLINE));
        }
        _ => unreachable!(),
    }
    assert!(
        !events.iter().any(|event| match event {
            AppEvent::UndoRemoved { .. } | AppEvent::EmailChanged { .. } => true,
            AppEvent::UndoChanged { action } => action.status == UndoStatus::Failed,
            _ => false,
        }),
        "nothing reached the provider, so nothing is removed, failed or changed: {events:?}"
    );
}

/// Task 3.4 handoff: the startup recovery announces the actions it marks, so
/// the failure rows appear without the file watcher — and, because a marked
/// action stops projecting, the rows it hid come back on screen.
#[test]
fn startup_recovery_emits_undo_changed_for_the_actions_it_marks() {
    let h = setup();
    let undo = undo_service(&h);
    let (email_id, account_id) = (h.email_id, h.account_id);
    h.context
        .update(|store| {
            store.undo_actions.push(UndoAction {
                id: 55,
                operation: UndoOperation::Mail {
                    email_id,
                    action: "archive".into(),
                    value: None,
                },
                subject: "Interrupted".into(),
                created_at_ms: 1,
                expires_at_ms: 2,
                status: UndoStatus::Committing,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            });
            Ok(())
        })
        .unwrap();
    h.recorder.clear();
    undo.recover_interrupted().unwrap();
    let events = h.recorder.events();
    assert_eq!(events.len(), 3, "{events:?}");
    match &events[0] {
        AppEvent::UndoChanged { action } => {
            assert_eq!(action.id, 55);
            assert_eq!(action.status, UndoStatus::Failed);
            assert!(action.error.is_some());
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        &events[1..],
        &[
            AppEvent::EmailChanged {
                account_id,
                ids: vec![email_id]
            },
            AppEvent::CountsChanged { account_id },
        ],
        "the row the failed action hid comes back"
    );
    // Nothing left to recover: nothing is announced.
    h.recorder.clear();
    undo.recover_interrupted().unwrap();
    assert!(h.recorder.events().is_empty());
}

/// The preview contract (`commitPreview`): a definitive failure announces the
/// failed row plus the message and counters returning from the projection.
#[test]
fn a_definitive_failure_announces_the_returning_row() {
    let h = setup();
    let undo = undo_service(&h);
    let (email_id, account_id) = (h.email_id, h.account_id);
    let pending = undo
        .queue_mail(email_id, "archive".into(), None)
        .unwrap();
    h.recorder.clear();
    h.gateway.fail_next(AUTHENTICATION);
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap());
    let events = h.recorder.events();
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::UndoChanged { action }
                if action.id == pending.id && action.status == UndoStatus::Failed
        )),
        "the failure row is announced: {events:?}"
    );
    let tail = events.len() - 2;
    assert_eq!(
        &events[tail..],
        &[
            AppEvent::EmailChanged {
                account_id,
                ids: vec![email_id]
            },
            AppEvent::CountsChanged { account_id },
        ],
        "the projected archive leaves the screen: {events:?}"
    );
}

/// INV-010 in the UI: giving up on a retrying action is the only removal that
/// brings the local state back, so the returning row is announced.
#[test]
fn giving_up_on_a_retrying_action_announces_the_returning_row() {
    let h = setup();
    let undo = undo_service(&h);
    let (email_id, account_id) = (h.email_id, h.account_id);
    let pending = undo
        .queue_mail(email_id, "archive".into(), None)
        .unwrap();
    h.gateway.fail_next(OFFLINE);
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap(), "the failure schedules a retry");
    h.recorder.clear();
    undo.dismiss(pending.id).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![
            AppEvent::UndoRemoved { id: pending.id },
            AppEvent::EmailChanged {
                account_id,
                ids: vec![email_id]
            },
            AppEvent::CountsChanged { account_id },
        ]
    );
    let rows = MailService::new(h.context.clone())
        .read_rows(vec![email_id])
        .unwrap();
    assert_eq!(rows[0].summary.folder, "Inbox", "the projection returned");
    // Dismissing the failure notice of a row already out of the projection
    // announces only the removal.
    h.recorder.clear();
    let failed = undo
        .queue_mail(email_id, "pin".into(), None)
        .unwrap();
    h.gateway.fail_next(AUTHENTICATION);
    h.clock.advance(31_000);
    assert!(undo.process_due().unwrap());
    h.recorder.clear();
    undo.dismiss(failed.id).unwrap();
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::UndoRemoved { id: failed.id }]
    );
}

/// The scheduler wake announces exactly the messages it touched, per account.
#[test]
fn scheduler_wake_emits_targeted_email_changed_and_counts() {
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 7,
        kind: AccountKind::Sample,
        email: "sample@example.com".into(),
        ..Default::default()
    });
    let now = 2_000_000_000;
    store.emails.push(Email {
        id: 11,
        account_id: 7,
        folder: "Inbox".into(),
        snoozed_until: Some(now - 5),
        ..Default::default()
    });
    store.emails.push(Email {
        id: 12,
        account_id: 7,
        folder: "Inbox".into(),
        reminder_at: Some(now - 5),
        ..Default::default()
    });
    store.emails.push(Email {
        id: 13,
        account_id: 7,
        folder: "Inbox".into(),
        snoozed_until: Some(now + 3_600),
        ..Default::default()
    });
    let h = harness(store);
    let scheduler = SchedulerService::new(h.context.clone(), DraftService::new(h.context.clone()));
    assert!(scheduler.tick().unwrap().is_empty());
    assert_eq!(
        h.recorder.events(),
        vec![
            AppEvent::EmailChanged {
                account_id: 7,
                ids: vec![11, 12]
            },
            AppEvent::CountsChanged { account_id: 7 },
        ],
        "only the due messages are announced"
    );
    let stored = h.context.read().unwrap();
    assert_eq!(stored.email(11).unwrap().snoozed_until, None);
    assert_eq!(stored.email(12).unwrap().reminder_at, None);
    assert!(stored.email(12).unwrap().pinned);
    assert_eq!(stored.email(13).unwrap().snoozed_until, Some(now + 3_600));
    drop(stored);
    // Nothing is due any more: the next tick stays silent.
    h.recorder.clear();
    scheduler.tick().unwrap();
    assert!(h.recorder.events().is_empty());
}

/// Controller contract for Task 2.3's `applyRows`: summary rows with the three
/// membership flags as keys even when unset, the inbox date, requested order,
/// no ghost rows.
#[test]
fn read_rows_carries_the_membership_flags_and_omits_unknown_ids() {
    let h = setup();
    let mail = MailService::new(h.context.clone());
    let store = h.context.read().unwrap();
    let first = store.emails[0].clone();
    let second = store.emails[1].clone();
    drop(store);
    let now = h.clock.now();
    let rows = mail
        .read_rows(vec![second.id, first.id, 999_999, second.id])
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.summary.id).collect::<Vec<_>>(),
        vec![second.id, first.id],
        "requested order, one row per id, unknown ids omitted"
    );
    let value = serde_json::to_value(&rows[0]).unwrap();
    for key in ["snoozed_until", "set_aside", "reminder_at"] {
        assert!(
            value.get(key).is_some(),
            "{key} must be a key of every row, null when unset: {value}"
        );
    }
    assert_eq!(value["snoozed_until"], serde_json::Value::Null);
    assert_eq!(value["reminder_at"], serde_json::Value::Null);
    assert_eq!(value["set_aside"], serde_json::Value::Bool(false));
    assert_eq!(value["date"].as_i64(), Some(second.inbox_date()));
    // Once set, the flags are the stored values: a just-snoozed message can
    // never look like an Inbox row again (review 2.3, Important 2).
    h.context
        .update(|store| {
            let email = store.email_mut(first.id).unwrap();
            email.snoozed_until = Some(now + 86_400);
            email.set_aside = true;
            email.reminder_at = Some(now + 3_600);
            Ok(())
        })
        .unwrap();
    let value = serde_json::to_value(&mail.read_rows(vec![first.id]).unwrap()[0]).unwrap();
    assert_eq!(value["snoozed_until"].as_i64(), Some(now + 86_400));
    assert_eq!(value["set_aside"], serde_json::Value::Bool(true));
    assert_eq!(value["reminder_at"].as_i64(), Some(now + 3_600));
    // A message the store no longer holds is omitted, never a ghost row.
    h.context
        .update(|store| {
            store.emails.retain(|email| email.id != first.id);
            Ok(())
        })
        .unwrap();
    assert!(mail.read_rows(vec![first.id]).unwrap().is_empty());
}

/// The page cap of `list_mail` applies to the requested ids, and the row date
/// is the inbox date (`returned_at` when the message came back from a snooze).
#[test]
fn read_rows_caps_at_one_hundred_and_uses_the_inbox_date() {
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 1,
        email: "sample@example.com".into(),
        ..Default::default()
    });
    for id in 1..=120u64 {
        store.emails.push(Email {
            id,
            account_id: 1,
            subject: format!("Message {id}"),
            date: id as i64,
            ..Default::default()
        });
    }
    store.emails[0].returned_at = Some(500);
    let h = harness(store);
    let mail = MailService::new(h.context.clone());
    let rows = mail.read_rows((1..=120).collect()).unwrap();
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[0].summary.date, 500, "returned_at is the inbox date");
    assert_eq!(rows[0].snoozed_until, None);
    assert!(!rows[0].set_aside);
    assert_eq!(rows[0].reminder_at, None);
    assert_eq!(mail.read_rows(vec![7, 7, 7]).unwrap().len(), 1);
    assert!(mail.read_rows(vec![]).unwrap().is_empty());
}

/// A search engine that is installed and indexes every document without warnings.
struct ReadySearch;
impl SearchGateway for ReadySearch {
    fn index_document(
        &self,
        _: &Store,
        _: &IndexContent,
        _: &AtomicBool,
    ) -> AppResult<DocumentIndexReport> {
        Ok(DocumentIndexReport {
            indexed: true,
            warnings: vec![],
        })
    }
    fn search(&self, _: &Store, _: &str, _: usize) -> AppResult<Vec<SearchHit>> {
        Ok(vec![])
    }
    fn index(&self, _: &Store) -> AppResult<String> {
        Ok(String::new())
    }
    fn installed(&self) -> bool {
        true
    }
}

/// Indexing announces its percent from memory, once per visible movement (and as a
/// heartbeat while it holds, see below); the durable per-page checkpoint stays in the store.
#[test]
fn indexing_emits_index_progress_when_the_percent_moves() {
    struct NoSource;
    impl IndexSource for NoSource {
        fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
            Ok(vec![])
        }
        fn next(
            &self,
            _: &Account,
            folder: &FolderProgress,
            _: &AtomicBool,
        ) -> AppResult<IndexPage> {
            Ok(IndexPage {
                progress: folder.clone(),
                email: None,
            })
        }
        fn content(
            &self,
            _: &Account,
            email: &Email,
            _: &AtomicBool,
        ) -> AppResult<IndexContent> {
            Ok(IndexContent {
                email: email.clone(),
                media: vec![],
                warnings: vec![],
            })
        }
    }
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 3,
        kind: AccountKind::Sample,
        email: "sample@example.com".into(),
        ..Default::default()
    });
    store.emails.push(Email {
        id: 9,
        account_id: 3,
        subject: "To index".into(),
        date: 1,
        ..Default::default()
    });
    let h = harness(store);
    let indexer = MailIndexService::new(
        h.context.clone(),
        Arc::new(NoSource),
        Arc::new(ReadySearch),
    );
    assert!(indexer.step().unwrap());
    assert_eq!(
        h.recorder.events(),
        vec![AppEvent::IndexProgress {
            account_id: 3,
            percent: 100
        }]
    );
    // The percent is remembered in memory: the idle pass does not repeat it.
    h.recorder.clear();
    indexer.step().unwrap();
    indexer.step().unwrap();
    assert!(
        h.recorder.events().is_empty(),
        "an unchanged percent is not announced again: {:?}",
        h.recorder.events()
    );
}

/// The mailbox of an account the indexer walks: `messages` are what the server
/// holds; `gap` is the UID span one empty page skips (stretches of deleted
/// mail), 0 for a mailbox without gaps.
struct Archive {
    messages: Vec<Email>,
    gap: u32,
}
const GAP_END: u32 = 100_000;
impl IndexSource for Archive {
    fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
        Ok(vec![archive_folder()])
    }
    fn next(
        &self,
        _: &Account,
        folder: &FolderProgress,
        _: &AtomicBool,
    ) -> AppResult<IndexPage> {
        let email = self
            .messages
            .iter()
            .filter(|email| email.uid.unwrap() > folder.after_uid)
            .min_by_key(|email| email.uid)
            .cloned();
        let mut progress = folder.clone();
        progress.uid_validity = Some(7);
        if let Some(email) = &email {
            progress.after_uid = email.uid.unwrap();
        } else if folder.after_uid < GAP_END {
            progress.after_uid += self.gap;
        }
        progress.complete = email.is_none() && (self.gap == 0 || progress.after_uid >= GAP_END);
        Ok(IndexPage { progress, email })
    }
    fn content(&self, _: &Account, email: &Email, _: &AtomicBool) -> AppResult<IndexContent> {
        Ok(IndexContent {
            email: email.clone(),
            media: vec![],
            warnings: vec![],
        })
    }
}
fn archive_folder() -> FolderProgress {
    FolderProgress {
        name: "Archive".into(),
        role: "Archive".into(),
        ..Default::default()
    }
}
fn archive_message(uid: u32) -> Email {
    Email {
        account_id: 3,
        uid: Some(uid),
        remote_uid_validity: Some(7),
        remote_folder: Some("Archive".into()),
        folder: "Archive".into(),
        remote_only: true,
        message_id: format!("3-{uid}@example.com"),
        subject: format!("Mail {uid}"),
        date: i64::from(uid),
        ..Default::default()
    }
}
/// A Gmail account with a million messages and a known folder, so the percent
/// stays at 0 however many messages the tests index, and reading the folder
/// list again keeps the census.
fn big_mailbox_store() -> Store {
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 3,
        kind: AccountKind::Gmail,
        email: "big@example.com".into(),
        search_index: qamille::domain::indexing::IndexProgress {
            inventory_total: Some(1_000_000),
            inventory_attempted: true,
            folders: vec![archive_folder()],
            ..Default::default()
        },
        ..Default::default()
    });
    store.emails.push(Email {
        id: 9,
        account_id: 3,
        subject: "Known".into(),
        date: 1,
        ..Default::default()
    });
    store.next_id = 9;
    store
}
fn zero_percent() -> AppEvent {
    AppEvent::IndexProgress {
        account_id: 3,
        percent: 0,
    }
}

/// The heartbeat of an unchanged percent means "the indexer worked", not "the
/// indexer was called". A big mailbox held at 0% keeps announcing it while
/// messages are indexed; a caught-up account, rescanned every minute, stays
/// silent however long that lasts, or the UI would show an idle account as busy
/// for ever.
#[test]
fn the_index_heartbeat_follows_work_not_calls() {
    let h = harness(big_mailbox_store());
    let archive = Archive {
        messages: (1..=100).map(archive_message).collect(),
        gap: 0,
    };
    let indexer = MailIndexService::new(h.context.clone(), Arc::new(archive), Arc::new(ReadySearch));
    indexer.step().unwrap(); // The first pass re-reads the folder list.
    for _ in 0..4 {
        // One batch of 25 messages per step, ten seconds apart: the 4th lands on 30 s.
        indexer.step().unwrap();
        h.clock.advance(10_000);
    }
    assert_eq!(
        h.recorder.events(),
        vec![zero_percent(), zero_percent()],
        "the first announce, then one heartbeat after 30 s of work"
    );
    // Caught up: every minute the folders are rescanned and nothing is found.
    for _ in 0..6 {
        h.clock.advance(61_000);
        indexer.step().unwrap();
        indexer.step().unwrap();
    }
    assert_eq!(
        h.recorder.events(),
        vec![zero_percent(), zero_percent()],
        "a caught-up account sends no heartbeat"
    );
}

/// A stretch of deleted mail holds no message, but walking it moves the folder
/// cursor: that is work too, so the heartbeat goes on and a long walk is not
/// shown as stuck.
#[test]
fn walking_a_gap_of_deleted_mail_keeps_the_heartbeat() {
    let h = harness(big_mailbox_store());
    let archive = Archive {
        messages: vec![],
        gap: 500,
    };
    let indexer = MailIndexService::new(h.context.clone(), Arc::new(archive), Arc::new(ReadySearch));
    indexer.step().unwrap(); // The first pass re-reads the folder list.
    for _ in 0..4 {
        indexer.step().unwrap();
        h.clock.advance(10_000);
    }
    assert_eq!(
        h.recorder.events(),
        vec![zero_percent(), zero_percent()],
        "empty pages that move the cursor are work: first announce, then a heartbeat"
    );
}
// ---------------------------------------------------------------------------
// Review 2.2b, Minor 2 (ADR-0007 v3 decisions 5/6, INV-004): the sync merge
// announces only the messages whose guarded writes actually wrote a different
// value. A routine re-sync of identical headers must not produce one oversized
// `EmailChanged` that the UI turns into a full list reload.
// ---------------------------------------------------------------------------

/// The fixture with an IMAP locator on every message of the Gmail account, so
/// a batch cloned from the store matches its rows by identity the way a real
/// header download does.
fn located_fixture() -> Store {
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    let mut uid = 100;
    for email in store
        .emails
        .iter_mut()
        .filter(|email| email.account_id == account_id)
    {
        email.uid = Some(uid);
        email.remote_uid_validity = Some(7);
        email.remote_folder = Some("INBOX".into());
        uid += 1;
    }
    store
}
fn account_mails(store: &Store) -> Vec<Email> {
    let account_id = store.accounts[0].id;
    store
        .emails
        .iter()
        .filter(|email| email.account_id == account_id)
        .cloned()
        .collect()
}

/// (a) A metadata sync whose batch matches known messages and writes back the
/// very same values announces nothing: no oversized `EmailChanged`, and no
/// `CountsChanged` because no count moved.
#[test]
fn sync_of_identical_known_headers_emits_no_email_changed() {
    let store = located_fixture();
    let batch = account_mails(&store);
    let rows = store.emails.len();
    let h = harness(store);
    let account_id = h.account_id;
    *h.gateway.batch.lock().unwrap() = batch;
    let mail = MailService::new(h.context.clone());
    let report = mail.sync().unwrap();
    assert_eq!(report.added, 0, "identical headers merge into the known rows");
    assert_eq!(h.context.read().unwrap().emails.len(), rows);
    let events = h.recorder.events();
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AppEvent::EmailChanged { .. } | AppEvent::CountsChanged { .. }
        )),
        "a metadata-only re-sync of account {account_id} announces no change: {events:?}"
    );
}

/// (b) A sync that flips `read` on exactly one known message announces an
/// `EmailChanged` naming exactly that message — decision 6 targeting.
#[test]
fn sync_flipping_one_read_announces_only_that_message() {
    let store = located_fixture();
    let mut batch = account_mails(&store);
    let flipped = batch.len() - 1;
    let flipped_id = batch[flipped].id;
    let original_read = batch[flipped].read;
    batch[flipped].read = !original_read;
    let h = harness(store);
    let account_id = h.account_id;
    *h.gateway.batch.lock().unwrap() = batch;
    let mail = MailService::new(h.context.clone());
    let report = mail.sync().unwrap();
    assert_eq!(report.added, 0);
    let events = h.recorder.events();
    let changed: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, AppEvent::EmailChanged { .. }))
        .collect();
    assert_eq!(changed.len(), 1, "one merge, one event: {events:?}");
    match changed[0] {
        AppEvent::EmailChanged {
            account_id: id,
            ids,
        } => {
            assert_eq!(*id, account_id);
            assert_eq!(
                ids,
                &vec![flipped_id],
                "only the message whose read state moved: {events:?}"
            );
        }
        _ => unreachable!(),
    }
    assert!(
        events.iter().any(|event| matches!(
            event,
            AppEvent::CountsChanged { account_id: id } if *id == account_id
        )),
        "the unread count truly moved: {events:?}"
    );
    assert_eq!(
        h.context.read().unwrap().email(flipped_id).unwrap().read,
        !original_read,
        "the flip is stored"
    );
}

/// (c) A body taken over from a `remote_only` row is a real change: the merge
/// announces that message.
#[test]
fn sync_taking_over_a_remote_only_body_announces_that_message() {
    let mut store = located_fixture();
    let target = store.emails[0].id;
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    store.emails[0].attachments.clear();
    let mut batch = account_mails(&store);
    // The download brings the full payload for the header-only row.
    batch[0].remote_only = false;
    batch[0].body = "Full body taken over from the provider".into();
    let h = harness(store);
    *h.gateway.batch.lock().unwrap() = batch;
    let mail = MailService::new(h.context.clone());
    let report = mail.sync().unwrap();
    assert_eq!(report.added, 0);
    let events = h.recorder.events();
    match events
        .iter()
        .find(|event| matches!(event, AppEvent::EmailChanged { .. }))
    {
        Some(AppEvent::EmailChanged { ids, .. }) => {
            assert_eq!(ids, &vec![target], "the takeover is announced: {events:?}")
        }
        other => panic!("the body takeover announces the message: {events:?} ({other:?})"),
    }
    let stored = h.context.read().unwrap();
    assert!(!stored.email(target).unwrap().remote_only);
    assert_eq!(
        stored.email(target).unwrap().body,
        "Full body taken over from the provider"
    );
}

/// Task 2.4: the contacts sync appears on the single indicator — one
/// `ContactsSync` bracket around the whole `sync_all` run, per-book events that
/// only move the activity to the account being read, and a standalone
/// `sync_book` bracketed by its own account.
#[test]
fn contacts_sync_announces_the_run_and_each_book() {
    let h = setup();
    let service = ContactService::new(
        h.context.clone(),
        Arc::new(MemoryContactStore::default()),
        Arc::new(NullContactGateway),
    );
    let account_id = h.account_id;
    service.sync_all().unwrap();
    let events = h.recorder.events();
    assert_eq!(
        events.first(),
        Some(&AppEvent::ContactsSync {
            account_id: None,
            running: true
        }),
        "the run opens the activity: {events:?}"
    );
    assert_eq!(
        events.last(),
        Some(&AppEvent::ContactsSync {
            account_id: None,
            running: false
        }),
        "the run closes it, even with a gateway that answers nothing: {events:?}"
    );
    assert!(
        events.iter().any(|event| *event
            == AppEvent::ContactsSync {
                account_id: Some(account_id),
                running: true
            }),
        "each book moves the activity to its account: {events:?}"
    );
    assert!(
        !events[1..events.len() - 1]
            .iter()
            .any(|event| matches!(event, AppEvent::ContactsSync { running: false, .. })),
        "the end of a book is not the end of the run: {events:?}"
    );
    // A standalone book read brackets itself with its own account.
    h.recorder.clear();
    let _ = service.sync_book(&format!("google:{account_id}"));
    assert_eq!(
        h.recorder.events(),
        vec![
            AppEvent::ContactsSync {
                account_id: Some(account_id),
                running: true
            },
            AppEvent::ContactsSync {
                account_id: Some(account_id),
                running: false
            },
        ]
    );
}

/// The activity log (centro attività) announces its row once, after the events of the action it
/// records: the toast first, the history right behind it.
#[test]
fn a_logged_action_announces_its_history_row_after_its_own_events() {
    let h = setup();
    let undo = undo_service(&h);
    let pending = undo.queue_mail(h.email_id, "markAsDone".into(), None).unwrap();
    assert_eq!(
        h.recorder.all_events(),
        vec![
            AppEvent::UndoChanged { action: pending },
            AppEvent::ActivityChanged { ids: vec![1] },
        ]
    );
    h.recorder.clear();
    // An action the history does not keep announces no row.
    undo.queue_mail(h.email_id, "markAsSeen".into(), None).unwrap();
    assert!(
        !h.recorder
            .all_events()
            .iter()
            .any(|event| matches!(event, AppEvent::ActivityChanged { .. })),
        "read state is not history"
    );
}
