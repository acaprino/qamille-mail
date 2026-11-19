//! ADR-0007 INV-001: nothing that reaches a mail provider runs while the mail
//! lock is held. A recording gateway wraps a fake provider and reports, per call,
//! whether `MailService` held the lock of that account at that moment.
#[path = "support/lock_probe.rs"]
mod lock_probe;

use lock_probe::ProbeMail;
use qamille::{
    application::{AppError, AppResult, ServiceContext, mail::{MailService, SyncReport}, ports::*},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

/// How long an operation may take when nothing is in its way. Far more than it
/// needs, so a pass is never a race; a failure costs this much.
const PATIENCE: Duration = Duration::from_secs(2);

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
        Ok(paths.len() as u64 * 7)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

/// Holds one provider call in flight: the provider announces it reached the call
/// and then waits for `release`.
struct Gate {
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
impl Gate {
    fn pass(gate: &Mutex<Option<Gate>>) {
        let gate = gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.release.recv_timeout(Duration::from_secs(30)).expect("the test releases the call");
        }
    }
}

/// A provider that succeeds unless told otherwise. `batch` is what a sync
/// downloads; with a gate set, the next sync (or the next `apply_message`) is held
/// at the provider until the test lets it go.
#[derive(Default)]
struct Provider {
    batch: Mutex<Vec<Email>>,
    sync_gate: Mutex<Option<Gate>>,
    apply_gate: Mutex<Option<Gate>>,
    /// The next `apply_message` fails with this message.
    apply_error: Mutex<Option<String>>,
}
impl MailGateway for Provider {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        // The server's view is fixed when the download starts, not when it ends.
        let emails = self.batch.lock().unwrap().clone();
        Gate::pass(&self.sync_gate);
        Ok(SyncBatch { emails, archive_folder: None, trash_folder: None })
    }
    fn resolve_identity(&self, _: &Account, email: &Email) -> AppResult<Email> {
        Ok(Email { remote_global_id: Some(700), ..email.clone() })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn apply_message(&self, _: &Account, _: &Email, _: RemoteAction) -> AppResult<()> {
        Gate::pass(&self.apply_gate);
        match self.apply_error.lock().unwrap().take() {
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

fn gmail(id: u32) -> Account {
    Account {
        id,
        email: format!("user{id}@example.com"),
        kind: AccountKind::Gmail,
        provider_id: "gmail".into(),
        ..Default::default()
    }
}

fn inbox_message(id: u64, account_id: u32, uid: u32) -> Email {
    Email {
        id,
        account_id,
        uid: Some(uid),
        remote_uid_validity: Some(99),
        remote_folder: Some("INBOX".into()),
        message_id: format!("m{id}@example.com"),
        folder: "Inbox".into(),
        subject: format!("Message {id}"),
        from_addr: "other@example.com".into(),
        ..Default::default()
    }
}

struct Fixture {
    mail: MailService,
    probe: Arc<ProbeMail>,
    provider: Arc<Provider>,
    context: Arc<ServiceContext>,
}

fn fixture(accounts: &[u32], emails: Vec<Email>) -> Fixture {
    let mut store = Store::default();
    store.accounts = accounts.iter().map(|id| gmail(*id)).collect();
    store.emails = emails;
    let provider = Arc::new(Provider::default());
    let probe = ProbeMail::new(provider.clone());
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: probe.clone(),
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(Files),
    });
    let mail = MailService::new(context.clone());
    probe.attach(mail.clone());
    Fixture { mail, probe, provider, context }
}

#[test]
fn remote_actions_run_outside_the_mail_lock() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    f.mail.act(30, "markAsDone", None).unwrap();
    let observed = f.probe.observed();
    // Archiving moves the message: its identity is resolved, then it is moved.
    let calls: Vec<_> = observed.iter().map(|(call, _)| *call).collect();
    assert_eq!(calls, ["resolve_identity", "apply_message"], "the action must reach the provider");
    assert!(
        observed.iter().all(|(_, held)| !held),
        "provider calls made while the mail lock was held: {:?}",
        f.probe.calls_under_lock()
    );
}

#[test]
fn flag_actions_run_outside_the_mail_lock() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    f.mail.act(30, "markAsSeen", None).unwrap();
    let observed = f.probe.observed();
    assert_eq!(observed.iter().map(|(call, _)| *call).collect::<Vec<_>>(), ["apply_message"]);
    assert!(
        observed.iter().all(|(_, held)| !held),
        "provider calls made while the mail lock was held: {:?}",
        f.probe.calls_under_lock()
    );
}

#[test]
fn sync_runs_outside_the_mail_lock() {
    let f = fixture(&[1], vec![]);
    f.provider.batch.lock().unwrap().push(Email {
        remote_only: true,
        ..inbox_message(31, 1, 11)
    });
    let report = f.mail.sync().unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.added, 1);
    let observed = f.probe.observed();
    assert_eq!(observed.iter().map(|(call, _)| *call).collect::<Vec<_>>(), ["sync_with_policy"]);
    assert!(
        observed.iter().all(|(_, held)| !held),
        "provider calls made while the mail lock was held: {:?}",
        f.probe.calls_under_lock()
    );
}

#[test]
fn an_idle_service_holds_no_lock() {
    let f = fixture(&[1, 2], vec![]);
    assert!(!f.mail.lock_held(1));
    assert!(!f.mail.lock_held(2));
    // Still free after the operations ran: no guard leaks out of them.
    f.mail.sync().unwrap();
    assert!(!f.mail.lock_held(1));
    assert!(f.context.read().is_ok());
}

#[test]
fn sync_of_every_account_runs_outside_the_mail_lock() {
    let f = fixture(&[1, 2], vec![]);
    let report = f.mail.sync().unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let observed = f.probe.observed();
    assert_eq!(
        observed.iter().map(|(call, _)| *call).collect::<Vec<_>>(),
        ["sync_with_policy", "sync_with_policy"]
    );
    assert!(
        observed.iter().all(|(_, held)| !held),
        "provider calls made while the mail lock was held: {:?}",
        f.probe.calls_under_lock()
    );
    assert!(!f.mail.lock_held(1) && !f.mail.lock_held(2), "no lock is kept between accounts");
}

/// A sync whose download is held at the provider, started on its own thread.
struct HeldSync {
    release: mpsc::Sender<()>,
    thread: thread::JoinHandle<AppResult<SyncReport>>,
}
impl HeldSync {
    fn start(f: &Fixture, account_id: u32) -> Self {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *f.provider.sync_gate.lock().unwrap() = Some(Gate { entered: entered_tx, release: release_rx });
        let mail = f.mail.clone();
        let thread = thread::spawn(move || mail.sync_account(account_id));
        entered_rx.recv_timeout(PATIENCE).expect("the sync reached the provider");
        Self { release, thread }
    }
    fn finish(self) -> SyncReport {
        self.release.send(()).unwrap();
        self.thread.join().unwrap().unwrap()
    }
}

/// Runs `work` on its own thread. `None` if it had not finished after `PATIENCE`
/// (the thread then ends whenever whatever it waits for is released).
fn within_patience<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(work());
    });
    finished.recv_timeout(PATIENCE).ok()
}

fn act_within_patience(mail: &MailService, id: u64, action: &'static str) -> Option<AppResult<()>> {
    let mail = mail.clone();
    within_patience(move || mail.act(id, action, None))
}

fn sync_within_patience(mail: &MailService, account_id: u32) -> Option<AppResult<SyncReport>> {
    let mail = mail.clone();
    within_patience(move || mail.sync_account(account_id))
}

/// An action whose provider call is held, started on its own thread.
struct HeldAction {
    release: mpsc::Sender<()>,
    thread: thread::JoinHandle<AppResult<()>>,
}
impl HeldAction {
    fn start(f: &Fixture, id: u64, action: &'static str) -> Self {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        *f.provider.apply_gate.lock().unwrap() = Some(Gate { entered: entered_tx, release: release_rx });
        let mail = f.mail.clone();
        let thread = thread::spawn(move || mail.act(id, action, None));
        entered_rx.recv_timeout(PATIENCE).expect("the action reached the provider");
        Self { release, thread }
    }
    fn finish(self) -> AppResult<()> {
        self.release.send(()).unwrap();
        self.thread.join().unwrap()
    }
}

#[test]
fn action_during_sync_does_not_wait() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let sync = HeldSync::start(&f, 1);
    // Same account as the download in flight.
    let acted = act_within_patience(&f.mail, 30, "markAsSeen");
    sync.finish();
    assert!(
        acted.as_ref().is_some_and(|result| result.is_ok()),
        "an action on the account being synced did not complete during the download: {acted:?}"
    );
}

#[test]
fn action_on_another_account_during_sync_does_not_wait() {
    let f = fixture(&[1, 2], vec![inbox_message(30, 1, 10), inbox_message(40, 2, 20)]);
    let sync = HeldSync::start(&f, 1);
    let acted = act_within_patience(&f.mail, 40, "markAsSeen");
    sync.finish();
    assert!(
        acted.as_ref().is_some_and(|result| result.is_ok()),
        "an action on another account did not complete during the download: {acted:?}"
    );
}

/// The sync merge no longer runs under one long lock, so an action can commit
/// while a download that started before it is still on its way. The merge matches
/// by provider identity (`same_message`), so the stale view lands on the same
/// message and nothing is duplicated. What the user changed after the download
/// began (folder, UID, flags) must not be put back to the pre-action view; the
/// next sync, which sees the message in its new place, corrects the rest.
#[test]
fn a_message_moved_during_sync_is_reconciled_not_duplicated() {
    let local = Email { remote_global_id: Some(700), ..inbox_message(30, 1, 10) };
    let f = fixture(&[1], vec![local.clone()]);
    // What the server reports when the download starts: still in the Inbox.
    f.provider.batch.lock().unwrap().push(Email { id: 0, ..local });
    let sync = HeldSync::start(&f, 1);
    let acted = act_within_patience(&f.mail, 30, "markAsDone");
    let report = sync.finish();
    assert!(
        acted.as_ref().is_some_and(|result| result.is_ok()),
        "the action did not complete during the download: {acted:?}"
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.added, 0, "the stale view of a known message is not new mail");
    let store = f.context.read().unwrap();
    let copies: Vec<_> = store.emails.iter().filter(|email| email.message_id == "m30@example.com").collect();
    assert_eq!(copies.len(), 1, "the message must not be duplicated");
    assert!(copies[0].done, "the user's action survives the merge");
    assert_eq!(copies[0].folder, "Archive", "a stale view does not move the message back");
    assert_eq!(copies[0].uid, None, "the old Inbox UID stays invalid after the move");
    assert_eq!(copies[0].remote_global_id, Some(700));
}

#[test]
fn a_flag_changed_during_sync_survives_the_merge() {
    let local = Email { remote_global_id: Some(700), ..inbox_message(30, 1, 10) };
    assert!(!local.read);
    let f = fixture(&[1], vec![local.clone()]);
    // The download started while the message was still unread on the server.
    f.provider.batch.lock().unwrap().push(Email { id: 0, ..local });
    let sync = HeldSync::start(&f, 1);
    let acted = act_within_patience(&f.mail, 30, "markAsSeen");
    sync.finish();
    assert!(acted.as_ref().is_some_and(|result| result.is_ok()), "{acted:?}");
    let store = f.context.read().unwrap();
    assert!(store.email(30).unwrap().read, "the stale unread flag must not undo the user's action");
}

#[test]
fn a_change_made_before_the_download_is_replaced_by_the_server_view() {
    // The guard only protects what changed after the snapshot: the server stays
    // authoritative for everything else.
    let local = Email { remote_global_id: Some(700), ..inbox_message(30, 1, 10) };
    let f = fixture(&[1], vec![local.clone()]);
    f.provider.batch.lock().unwrap().push(Email { id: 0, read: true, folder: "Archive".into(), uid: Some(55), ..local });
    f.mail.sync().unwrap();
    let store = f.context.read().unwrap();
    let merged = store.email(30).unwrap();
    assert!(merged.read);
    assert_eq!(merged.folder, "Archive");
    assert_eq!(merged.uid, Some(55));
}

#[test]
fn a_download_in_flight_holds_no_lock() {
    let f = fixture(&[1, 2], vec![]);
    let sync = HeldSync::start(&f, 1);
    assert!(!f.mail.lock_held(1), "the account lock is free while the provider downloads");
    assert!(!f.mail.lock_held(2));
    sync.finish();
}

#[test]
fn sync_during_a_slow_remote_action_does_not_wait() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let action = HeldAction::start(&f, 30, "markAsSeen");
    assert!(!f.mail.lock_held(1), "the account lock is free while the provider acts");
    let synced = sync_within_patience(&f.mail, 1);
    // The local copy changes only once the provider has answered.
    let unread_meanwhile = !f.context.read().unwrap().email(30).unwrap().read;
    action.finish().unwrap();
    assert!(
        synced.as_ref().is_some_and(|result| result.is_ok()),
        "a sync did not complete during the provider action: {:?}",
        synced.as_ref().map(|result| result.as_ref().map(|report| report.added))
    );
    assert!(unread_meanwhile, "the action was committed before the provider answered");
    assert!(f.context.read().unwrap().email(30).unwrap().read);
}

#[test]
fn a_failed_provider_call_leaves_the_message_in_place_with_its_identity() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    *f.provider.apply_error.lock().unwrap() = Some("connection reset".into());
    let error = f.mail.act(30, "markAsDone", None).unwrap_err();
    assert!(error.message.contains("connection reset"), "{error:?}");
    let store = f.context.read().unwrap();
    let message = store.email(30).unwrap();
    assert!(!message.done);
    assert_eq!(message.folder, "Inbox");
    assert_eq!(message.uid, Some(10), "the UID stays valid when the move did not happen");
    // Saved before the destructive step, so a lost answer cannot lose the locator.
    assert_eq!(message.remote_global_id, Some(700));
    assert!(!f.mail.lock_held(1));
}

#[test]
fn a_message_deleted_during_the_provider_call_fails_cleanly() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let action = HeldAction::start(&f, 30, "markAsSeen");
    f.context
        .update(|store| {
            store.emails.retain(|email| email.id != 30);
            Ok(())
        })
        .unwrap();
    let result = action.finish();
    assert!(result.is_err(), "there is nothing left to commit the action to: {result:?}");
    assert!(f.context.read().unwrap().email(30).is_none());
    assert!(!f.mail.lock_held(1));
}

#[test]
fn two_syncs_of_one_account_do_not_duplicate_new_mail() {
    let f = fixture(&[1], vec![]);
    f.provider.batch.lock().unwrap().push(Email { remote_only: true, ..inbox_message(31, 1, 11) });
    // The first download is still on its way when a second one starts and finishes.
    let first = HeldSync::start(&f, 1);
    let second = sync_within_patience(&f.mail, 1).expect("a second sync does not wait for the first").unwrap();
    assert_eq!(second.added, 1);
    let first = first.finish();
    assert_eq!(first.added, 0, "the message the second sync saved is not new to the first");
    let store = f.context.read().unwrap();
    assert_eq!(store.emails.iter().filter(|email| email.message_id == "m31@example.com").count(), 1);
}

/// Without a provider-wide id (not every provider has one), the stale view of a
/// moved message no longer matches the local record by locator: the local copy has
/// no UID after the move. The merge finds it through the locator it had when the
/// sync started.
#[test]
fn a_moved_message_without_a_provider_id_is_not_duplicated() {
    let local = inbox_message(30, 1, 10);
    assert_eq!(local.remote_global_id, None);
    let f = fixture(&[1], vec![local.clone()]);
    f.provider.batch.lock().unwrap().push(Email { id: 0, ..local });
    let sync = HeldSync::start(&f, 1);
    let acted = act_within_patience(&f.mail, 30, "markAsDone");
    let report = sync.finish();
    assert!(acted.as_ref().is_some_and(|result| result.is_ok()), "{acted:?}");
    assert_eq!(report.added, 0);
    let store = f.context.read().unwrap();
    let copies: Vec<_> = store.emails.iter().filter(|email| email.message_id == "m30@example.com").collect();
    assert_eq!(copies.len(), 1, "the message must not be duplicated: {copies:?}");
    assert_eq!(copies[0].id, 30);
    assert_eq!(copies[0].folder, "Archive");
    assert_eq!(copies[0].uid, None);
}

/// ADR-0007: the lock is not held across the provider call, so two actions on
/// one message are kept apart by an in-flight guard: the second is refused.
#[test]
fn a_second_action_on_a_message_in_flight_is_refused() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let first = HeldAction::start(&f, 30, "markAsSeen");
    let second = act_within_patience(&f.mail, 30, "archive").expect("the second action does not wait");
    let error = second.expect_err("the message is busy");
    assert!(error.message.contains("Another action on this message is in progress"), "{error:?}");
    first.finish().unwrap();
    let store = f.context.read().unwrap();
    let message = store.email(30).unwrap();
    assert!(message.read, "the first action committed");
    assert!(!message.done && message.folder == "Inbox" && message.uid == Some(10), "the refused action left nothing behind");
    assert_eq!(f.probe.observed().iter().filter(|(call, _)| *call == "apply_message").count(), 1);
    // The guard is released: the message accepts actions again.
    f.mail.act(30, "markAsDone", None).unwrap();
}

fn queued(id: u64, email_id: u64, action: &str) -> qamille::domain::undo::UndoAction {
    use qamille::domain::undo::*;
    UndoAction {
        id,
        operation: UndoOperation::Mail { email_id, action: action.into(), value: None },
        subject: "s".into(),
        created_at_ms: 0,
        expires_at_ms: 1,
        retry_at_ms: None,
        attempts: 0,
        status: UndoStatus::Pending,
        error: None,
    }
}

#[test]
fn a_queued_action_on_a_message_in_flight_is_refused() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let first = HeldAction::start(&f, 30, "markAsSeen");
    let pending = [queued(900, 30, "markAsDone")];
    let mail = f.mail.clone();
    let result = within_patience(move || mail.act_queued(&pending)).expect("the queued action does not wait");
    let error = result.expect_err("the message is busy");
    assert!(error.message.contains("Another action on this message is in progress"), "{error:?}");
    first.finish().unwrap();
    assert_eq!(f.probe.observed().iter().filter(|(call, _)| *call == "apply_message").count(), 1);
}

/// A queued action refused by the in-flight guard is not lost: it goes back to
/// Pending with a short defer and is applied once the other action is done.
#[test]
fn a_queued_action_refused_by_the_guard_is_requeued_not_failed() {
    use qamille::application::{drafts::DraftService, undo::UndoService};
    use qamille::domain::undo::UndoStatus;
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    f.context.update(|store| { store.undo_actions.push(queued(900, 30, "markAsDone")); Ok(()) }).unwrap();
    let undo = UndoService::new(f.context.clone(), f.mail.clone(), DraftService::new(f.context.clone()));
    let expires = f.context.read().unwrap().undo_actions[0].expires_at_ms;
    let first = HeldAction::start(&f, 30, "markAsSeen");
    let busy = undo.clone();
    within_patience(move || busy.process_due()).expect("the queue does not wait").unwrap();
    {
        let store = f.context.read().unwrap();
        let action = store.undo_actions.iter().find(|a| a.id == 900).expect("the action is still queued");
        assert_eq!(action.status, UndoStatus::Pending, "{action:?}");
        assert!(action.error.is_none());
        assert!(action.retry_at_ms.is_some_and(|at| at > f.context.clock.now_ms()), "deferred, not retried in a tight loop");
        assert_eq!(action.attempts, 0, "a deferral is not a failed attempt");
        assert_eq!(action.expires_at_ms, expires, "the undo window is untouched");
    }
    first.finish().unwrap();
    f.context.update(|store| { store.undo_actions[0].retry_at_ms = Some(1); Ok(()) }).unwrap();
    // The clock is fixed, so a fresh service stands in for the 3 s passing.
    let later = UndoService::new(f.context.clone(), f.mail.clone(), DraftService::new(f.context.clone()));
    later.process_due().unwrap();
    let store = f.context.read().unwrap();
    assert!(store.undo_actions.is_empty(), "{:?}", store.undo_actions);
    assert_eq!(store.email(30).unwrap().folder, "Archive");
    assert!(store.email(30).unwrap().read);
    assert_eq!(f.probe.observed().iter().filter(|(call, _)| *call == "apply_message").count(), 2);
}
#[test]
fn a_user_action_during_a_queued_move_is_refused() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *f.provider.apply_gate.lock().unwrap() = Some(Gate { entered: entered_tx, release: release_rx });
    let mail = f.mail.clone();
    let pending = [queued(900, 30, "markAsDone")];
    let queued_run = thread::spawn(move || mail.act_queued(&pending));
    entered_rx.recv_timeout(PATIENCE).expect("the queued move reached the provider");
    let second = act_within_patience(&f.mail, 30, "moveToTrash").expect("does not wait");
    assert!(second.is_err());
    release.send(()).unwrap();
    queued_run.join().unwrap().unwrap();
    let store = f.context.read().unwrap();
    assert_eq!(store.email(30).unwrap().folder, "Archive");
}

/// The provider moved the message, then the commit could not be applied. The
/// undo queue must not be left holding the action: replaying a move that already
/// happened would conflict with the provider (INV-007).
#[test]
fn a_move_whose_commit_fails_leaves_no_retryable_action() {
    let f = fixture(&[1], vec![inbox_message(30, 1, 10)]);
    f.context
        .update(|store| {
            store.undo_actions.push(queued(900, 30, "markAsDone"));
            Ok(())
        })
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *f.provider.apply_gate.lock().unwrap() = Some(Gate { entered: entered_tx, release: release_rx });
    let mail = f.mail.clone();
    let pending = [queued(900, 30, "markAsDone")];
    let run = thread::spawn(move || mail.act_queued(&pending));
    entered_rx.recv_timeout(PATIENCE).expect("the move reached the provider");
    // The message disappears while the provider moves it.
    f.context
        .update(|store| {
            store.emails.retain(|email| email.id != 30);
            Ok(())
        })
        .unwrap();
    release.send(()).unwrap();
    let error = run.join().unwrap().expect_err("the commit cannot apply");
    assert!(error.message.contains("provider"), "the error names the mismatch: {error:?}");
    let store = f.context.read().unwrap();
    assert!(store.undo_actions.iter().all(|a| a.id != 900), "no pending action is left to retry the move");
    assert!(!f.mail.lock_held(1));
}
