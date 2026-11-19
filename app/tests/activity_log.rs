//! Writing the activity log (centro attività, Task 3): rows are appended best
//! effort after the fact they record, announced once, and a failure that keeps
//! repeating is one row, not one per attempt.

use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        activity as log,
        events::AppEvent,
        ports::{EventSink, StoreRepository},
    },
    domain::{
        activity::*,
        undo::{UndoAction, UndoOperation, UndoStatus},
    },
    infrastructure::{
        gateways::{GmailGateway, KeyringSecrets, LocalAttachments, SystemClock},
        repository::MemoryRepository,
    },
    model::Store,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    fn events(&self) -> Vec<AppEvent> {
        self.0.lock().unwrap().clone()
    }
}

/// A store that keeps no activity log: the trait's defaults.
struct NoLog(MemoryRepository);
impl StoreRepository for NoLog {
    fn read(&self) -> AppResult<Store> {
        self.0.read()
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.0.transaction(op)
    }
}

/// A log whose every write fails.
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

/// A log that writes but cannot be searched for the last row of a family.
struct LookupFails(MemoryRepository);
impl StoreRepository for LookupFails {
    fn read(&self) -> AppResult<Store> {
        self.0.read()
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.0.transaction(op)
    }
    fn activity_supported(&self) -> bool {
        true
    }
    fn append_activity(&self, entry: &NewActivity, at_ms: i64) -> AppResult<Option<i64>> {
        self.0.append_activity(entry, at_ms)
    }
    fn last_activity(&self, _: &[&str], _: Option<u32>) -> AppResult<Option<ActivityEntry>> {
        Err(AppError::storage("locked"))
    }
}

fn context(repository: Arc<dyn StoreRepository>) -> (ServiceContext, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let context = ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        repository,
        mail: Arc::new(GmailGateway::new(std::env::temp_dir().join("qamille-activity-log-test"))),
        secrets: Arc::new(KeyringSecrets),
        clock: Arc::new(SystemClock),
        attachments: Arc::new(LocalAttachments),
        events: recorder.clone(),
    };
    (context, recorder)
}

fn memory_context() -> (ServiceContext, Arc<Recorder>) {
    context(Arc::new(MemoryRepository::new(Store::default())))
}

fn failure(code: &str, account: Option<u32>, detail: &str) -> NewActivity {
    NewActivity {
        kind: ActivityKind::Error,
        code: code.into(),
        account_id: account,
        detail: Some(detail.into()),
        ..Default::default()
    }
}

fn rows(context: &ServiceContext) -> Vec<ActivityEntry> {
    context.repository.activity_page(None, 100, ActivityFilter::All).unwrap()
}

fn undo_action(id: u64, operation: UndoOperation, subject: &str) -> UndoAction {
    UndoAction {
        id,
        operation,
        subject: subject.into(),
        created_at_ms: 1,
        expires_at_ms: 2,
        status: UndoStatus::Pending,
        error: None,
        retry_at_ms: None,
        attempts: 0,
    }
}

#[test]
fn record_appends_and_announces() {
    let (context, recorder) = memory_context();
    let seq = log::record(&context, NewActivity { code: code::SYNC_COMPLETE.into(), ..Default::default() });
    assert_eq!(seq, Some(1));
    let stored = rows(&context);
    assert_eq!(stored.len(), 1);
    assert!(stored[0].at_ms > 0, "the context clock stamps the row");
    assert_eq!(recorder.events(), vec![AppEvent::ActivityChanged { ids: vec![1] }]);
}

#[test]
fn record_without_activity_store_announces_nothing() {
    let (context, recorder) = context(Arc::new(NoLog(MemoryRepository::new(Store::default()))));
    assert_eq!(log::record(&context, NewActivity { code: code::SYNC_COMPLETE.into(), ..Default::default() }), None);
    assert!(recorder.events().is_empty());
}

#[test]
fn record_survives_a_failing_log() {
    let (context, recorder) = context(Arc::new(BrokenLog(MemoryRepository::new(Store::default()))));
    assert_eq!(log::record(&context, NewActivity { code: code::ARCHIVE.into(), ..Default::default() }), None);
    log::set_undo_outcome(&context, 5, ActivityOutcome::Undone, None);
    assert!(recorder.events().is_empty(), "a log that cannot be written announces nothing and never panics");
}

#[test]
fn failure_is_recorded_once_until_success_or_new_message() {
    let (context, _) = memory_context();
    for _ in 0..3 {
        log::record_failure(&context, failure(code::SYNC_FAILED, Some(1), "timeout"), code::SYNC_COMPLETE);
    }
    assert_eq!(rows(&context).len(), 1, "the same failure three times is one row");

    log::record_failure(&context, failure(code::SYNC_FAILED, Some(1), "auth"), code::SYNC_COMPLETE);
    assert_eq!(rows(&context).len(), 2, "a new message is a new row");

    log::record(&context, NewActivity { code: code::SYNC_COMPLETE.into(), account_id: Some(1), ..Default::default() });
    log::record_failure(&context, failure(code::SYNC_FAILED, Some(1), "auth"), code::SYNC_COMPLETE);
    assert_eq!(rows(&context).len(), 4, "a success in between lets the same failure be recorded again");

    log::record_failure(&context, failure(code::SYNC_FAILED, Some(2), "auth"), code::SYNC_COMPLETE);
    assert_eq!(rows(&context).len(), 5, "another account is its own family");
}

#[test]
fn a_failure_is_still_recorded_when_the_family_lookup_fails() {
    let (context, recorder) = context(Arc::new(LookupFails(MemoryRepository::new(Store::default()))));
    let seq = log::record_failure(&context, failure(code::MODEL_FAILED, None, "offline"), code::MODEL_INSTALLED);
    assert_eq!(seq, Some(1), "an unknown history records the error: a repeated row beats a lost one");
    assert_eq!(recorder.events(), vec![AppEvent::ActivityChanged { ids: vec![1] }]);
}

#[test]
fn only_logged_mail_actions_map_to_entries() {
    let mail = |action: &str| UndoOperation::Mail { email_id: 41, action: action.into(), value: None };

    let done = log::for_undo_action(&undo_action(9, mail("markAsDone"), "Cyber Resilience Act")).unwrap();
    assert_eq!(done.kind, ActivityKind::Action);
    assert_eq!(done.code, "markAsDone");
    assert_eq!(done.source, ActivitySource::User);
    assert_eq!(done.subject.as_deref(), Some("Cyber Resilience Act"));
    assert_eq!((done.ref_kind.as_deref(), done.ref_id), (Some("email"), Some(41)));
    assert_eq!(done.undo_id, Some(9));
    assert_eq!(done.outcome, ActivityOutcome::Done);
    assert_eq!(done.account_id, None, "the caller fills the account from the store");

    for action in ["archive", "snooze", "moveToTrash"] {
        assert!(log::for_undo_action(&undo_action(1, mail(action), "x")).is_some(), "{action}");
    }
    assert!(log::for_undo_action(&undo_action(1, mail("markAsSeen"), "x")).is_none());
    assert!(log::for_undo_action(&undo_action(1, mail("pin"), "x")).is_none());

    let send = log::for_undo_action(&undo_action(2, UndoOperation::SendDraft { draft_id: 8 }, "Re: invoice")).unwrap();
    assert_eq!((send.code.as_str(), send.ref_kind.as_deref(), send.ref_id), ("send", Some("draft"), Some(8)));
    let discard = log::for_undo_action(&undo_action(3, UndoOperation::DiscardDraft { draft_id: 8 }, "")).unwrap();
    assert_eq!(discard.code, "discardDraft");
    assert_eq!(discard.subject, None, "an empty subject is no subject");
    let schedule = log::for_undo_action(&undo_action(4, UndoOperation::ScheduleDraft { draft_id: 8, send_at: 99 }, "x")).unwrap();
    assert_eq!(schedule.code, "scheduleDraft");
}

#[test]
fn set_undo_outcome_announces_only_when_a_row_changed() {
    let (context, recorder) = memory_context();
    log::record(
        &context,
        NewActivity { kind: ActivityKind::Action, code: code::ARCHIVE.into(), undo_id: Some(9), ..Default::default() },
    );
    log::set_undo_outcome(&context, 9, ActivityOutcome::Undone, None);
    log::set_undo_outcome(&context, 404, ActivityOutcome::Failed, Some("nope".into()));
    assert_eq!(rows(&context)[0].outcome, ActivityOutcome::Undone);
    assert_eq!(
        recorder.events(),
        vec![
            AppEvent::ActivityChanged { ids: vec![1] },
            AppEvent::ActivityChanged { ids: vec![1] },
        ],
        "the append and the undo announce; the unknown undo id does not"
    );
}

#[test]
fn activity_changed_serializes_like_the_typescript_mirror() {
    assert_eq!(
        serde_json::to_string(&AppEvent::ActivityChanged { ids: vec![4] }).unwrap(),
        r#"{"kind":"activity-changed","ids":[4]}"#
    );
    assert!(AppEvent::ActivityChanged { ids: vec![] }.is_empty_change(), "no id, no announcement");
    assert!(!AppEvent::ActivityChanged { ids: vec![4] }.is_empty_change());
}

#[test]
fn mark_seen_stores_now_and_clears_the_unseen_errors() {
    let (context, _) = memory_context();
    log::record(&context, failure(code::SYNC_FAILED, Some(1), "timeout"));
    assert_eq!(context.repository.unseen_activity_errors().unwrap(), 1);
    // The clock keeps moving: a failure recorded after marking is unseen again.
    log::mark_seen(&context).unwrap();
    assert!(context.repository.activity_seen_ms().unwrap() > 0);
    assert_eq!(context.repository.unseen_activity_errors().unwrap(), 0);
}

#[test]
fn mark_seen_without_an_activity_store_is_a_no_op() {
    let (context, _) = context(Arc::new(NoLog(MemoryRepository::new(Store::default()))));
    log::mark_seen(&context).unwrap();
    assert_eq!(context.repository.activity_seen_ms().unwrap(), 0);
}

#[test]
fn a_repeated_failure_is_recorded_again_after_a_long_quiet_while() {
    // The same outage at 02:00 and again at 14:00, with a quiet recovery in between that wrote no
    // success row (a sync with nothing new is not history): the second outage must not vanish.
    let (context, _) = memory_context();
    let now = context.clock.now_ms();
    let hour = 3_600_000;
    context
        .repository
        .append_activity(&failure(code::SYNC_FAILED, Some(1), "timeout"), now - 12 * hour)
        .unwrap();
    log::record_failure(&context, failure(code::SYNC_FAILED, Some(1), "timeout"), code::SYNC_COMPLETE);
    assert_eq!(rows(&context).len(), 2, "twelve hours later the same message is news again");
    // Within the window it is still one row.
    log::record_failure(&context, failure(code::SYNC_FAILED, Some(1), "timeout"), code::SYNC_COMPLETE);
    assert_eq!(rows(&context).len(), 2, "a failure that keeps repeating stays one row");
}

#[test]
fn a_failure_just_inside_the_window_is_still_deduplicated() {
    let (context, _) = memory_context();
    let now = context.clock.now_ms();
    context
        .repository
        .append_activity(&failure(code::MODEL_FAILED, None, "offline"), now - FAILURE_REPEAT_MS + 60_000)
        .unwrap();
    log::record_failure(&context, failure(code::MODEL_FAILED, None, "offline"), code::MODEL_INSTALLED);
    assert_eq!(rows(&context).len(), 1);
}
