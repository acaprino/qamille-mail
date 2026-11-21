//! The three facts the desktop turns into Windows notifications (phase 3, Task 3; docs/aspetto-nativo.md,
//! rule 28): the writer that knows the fact announces it once, as a typed event the desktop sink keeps
//! away from the page. New mail only from a live sync of an account that already synced in this process
//! (never its first sync, never what arrived while Qamille was closed, never older mail the sync is still
//! bringing in); a scheduled send that failed, once; a sign-in that no longer works, once until it works
//! again. And the unread count of the taskbar is the sidebar's.
#[path = "support/sync_fakes.rs"]
mod fakes;

use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        drafts::DraftService,
        events::AppEvent,
        mail::MailService,
        ports::*,
        scheduler::SchedulerService,
        workspace::WorkspaceService,
    },
    domain::undo::{UndoAction, UndoOperation, UndoStatus},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};

/// The scheduler's waker and the connectivity state are process-wide: one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());
fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    /// Only the three system facts, in order.
    fn system(&self) -> Vec<AppEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(event, AppEvent::MailArrived { .. } | AppEvent::ScheduledSendFailed { .. } | AppEvent::SignInExpired { .. })
            })
            .cloned()
            .collect()
    }
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// The provider: the next download, or the error the next sync fails with.
#[derive(Default)]
struct Gateway {
    batch: Mutex<Vec<Email>>,
    failure: Mutex<Option<(&'static str, String)>>,
}
impl MailGateway for Gateway {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        if let Some((code, message)) = self.failure.lock().unwrap().clone() {
            return Err(AppError::new(code, message));
        }
        Ok(SyncBatch { emails: self.batch.lock().unwrap().clone(), archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}

struct Clock(AtomicU64);
impl qamille::application::ports::Clock for Clock {
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

/// Now, in seconds: the clock of every test.
const NOW: i64 = 2_000_000_000;

struct Harness {
    _guard: MutexGuard<'static, ()>,
    context: Arc<ServiceContext>,
    gateway: Arc<Gateway>,
    recorder: Arc<Recorder>,
    account_id: u32,
}
/// The sample workspace with its first account on Gmail; `last_sync` is when it last synced before
/// this process (`None`: a mailbox just added).
fn harness(last_sync: Option<i64>) -> Harness {
    let guard = serial();
    let mut store = sample_store();
    store.accounts[0].kind = AccountKind::Gmail;
    store.accounts[0].last_sync = last_sync;
    for email in &mut store.emails {
        email.snoozed_until = None;
        email.reminder_at = None;
    }
    for draft in &mut store.drafts {
        draft.send_at = None;
    }
    let account_id = store.accounts[0].id;
    let gateway = Arc::new(Gateway::default());
    let recorder = Arc::new(Recorder::default());
    let context = Arc::new(ServiceContext {
        compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: recorder.clone(),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: gateway.clone(),
        secrets: Arc::new(Secrets),
        clock: Arc::new(Clock(AtomicU64::new((NOW * 1000) as u64))),
        attachments: Arc::new(Files),
    });
    Harness { _guard: guard, context, gateway, recorder, account_id }
}
impl Harness {
    /// The next download: the messages, as the provider describes them.
    fn download(&self, emails: Vec<Email>) {
        *self.gateway.batch.lock().unwrap() = emails;
    }
    fn fail_with(&self, failure: Option<(&'static str, &str)>) {
        *self.gateway.failure.lock().unwrap() = failure.map(|(code, message)| (code, message.to_string()));
    }
    /// The stored id of the message the provider knows as `uid`.
    fn id_of(&self, uid: u32) -> u64 {
        self.context.read().unwrap().emails.iter().find(|email| email.uid == Some(uid)).expect("stored").id
    }
}

/// A message as the provider describes it, received by Gmail at `date`.
fn message(account_id: u32, uid: u32, folder: &str, read: bool, date: i64) -> Email {
    Email {
        account_id,
        uid: Some(uid),
        remote_uid_validity: Some(7),
        remote_folder: Some(if folder == "Inbox" { "INBOX".into() } else { "[Gmail]/All Mail".into() }),
        folder: folder.into(),
        message_id: format!("<arrival-{uid}@example.com>"),
        from_name: format!("Sender {uid}"),
        from_addr: format!("sender{uid}@example.com"),
        subject: format!("Subject {uid}"),
        body: format!("The body of {uid}, never in a notification"),
        read,
        date,
        ..Default::default()
    }
}

#[test]
fn mail_arrived_names_only_new_unread_inbox_messages() {
    let h = harness(Some(NOW - 600));
    let mail = MailService::new(h.context.clone());
    let a = h.account_id;
    // The first pass of this process sets the baseline.
    h.download(vec![]);
    mail.sync().unwrap();
    assert!(h.recorder.system().is_empty());

    let arrivals = vec![
        message(a, 201, "Inbox", false, NOW - 5),
        // Read elsewhere already: nothing to announce.
        message(a, 202, "Inbox", true, NOW - 4),
        // Not in the Inbox (filtered to All Mail by a Gmail rule).
        message(a, 203, "Archive", false, NOW - 3),
        // Weeks old: the sync is still bringing in older mail, not new mail.
        message(a, 204, "Inbox", false, NOW - 30 * 24 * 3600),
        message(a, 205, "Inbox", false, NOW - 2),
    ];
    h.download(arrivals.clone());
    mail.sync().unwrap();
    assert_eq!(
        h.recorder.system(),
        vec![AppEvent::MailArrived { account_id: a, ids: vec![h.id_of(201), h.id_of(205)] }],
        "one event per arrival, naming only the new unread messages of the Inbox"
    );

    // The same messages again (an update), and one more: only that one is new.
    h.recorder.clear();
    let mut next = arrivals;
    next.push(message(a, 206, "Inbox", false, NOW - 1));
    h.download(next);
    mail.sync().unwrap();
    assert_eq!(h.recorder.system(), vec![AppEvent::MailArrived { account_id: a, ids: vec![h.id_of(206)] }]);

    // Nothing new: nothing announced, not even an empty event.
    h.recorder.clear();
    mail.sync().unwrap();
    assert!(h.recorder.system().is_empty());
}

#[test]
fn mail_arrived_is_silent_on_the_first_sync_of_an_account() {
    {
        // A mailbox just added: its first sync brings the whole Inbox, which is no arrival.
        let h = harness(None);
        let a = h.account_id;
        let mail = MailService::new(h.context.clone());
        h.download(vec![message(a, 301, "Inbox", false, NOW - 5), message(a, 302, "Inbox", false, NOW - 4)]);
        mail.sync().unwrap();
        assert!(h.recorder.system().is_empty(), "the first sync of an account");
        // From the next sync on, new mail is announced.
        h.download(vec![message(a, 303, "Inbox", false, NOW - 1)]);
        mail.sync().unwrap();
        assert_eq!(h.recorder.system(), vec![AppEvent::MailArrived { account_id: a, ids: vec![h.id_of(303)] }]);
    }
    {
        // A mailbox that synced before Qamille was closed: what arrived meanwhile comes with the first
        // sync of this process, and is not announced either.
        let h = harness(Some(NOW - 3600));
        let mail = MailService::new(h.context.clone());
        h.download(vec![message(h.account_id, 401, "Inbox", false, NOW - 60)]);
        mail.sync().unwrap();
        assert!(h.recorder.system().is_empty(), "mail that arrived with Qamille closed");
    }
    {
        // A first sync that failed does not count: the first one that works is still silent.
        let h = harness(Some(NOW - 3600));
        let mail = MailService::new(h.context.clone());
        h.fail_with(Some(("external", "IMAP connect: timed out")));
        let _ = mail.sync();
        h.fail_with(None);
        h.download(vec![message(h.account_id, 501, "Inbox", false, NOW - 60)]);
        mail.sync().unwrap();
        assert!(h.recorder.system().is_empty(), "the first sync that worked sets the baseline");
    }
}

#[test]
fn scheduled_send_failed_is_announced_once() {
    let h = harness(Some(NOW - 600));
    let scheduler = SchedulerService::new(h.context.clone(), DraftService::new(h.context.clone()));
    let (draft_id, subject) = h
        .context
        .update(|store| {
            // A scheduled draft without recipients: its send fails.
            let draft = &mut store.drafts[0];
            draft.send_at = Some(NOW);
            draft.to.clear();
            draft.subject = "Offerta Q4".into();
            Ok((draft.id, draft.subject.clone()))
        })
        .unwrap();
    let notices = scheduler.tick().unwrap();
    assert_eq!(notices.len(), 1, "the notice in the app stays: {notices:?}");
    assert_eq!(h.recorder.system(), vec![AppEvent::ScheduledSendFailed { draft_id, subject }]);
    h.recorder.clear();
    scheduler.tick().unwrap();
    scheduler.tick().unwrap();
    assert!(h.recorder.system().is_empty(), "announced once");
}

#[test]
fn sign_in_expired_comes_from_the_error_code() {
    let h = harness(Some(NOW - 600));
    let a = h.account_id;
    let mail = MailService::new(h.context.clone());
    // A network error is not an expired sign-in.
    h.fail_with(Some(("external", "IMAP connect: timed out")));
    let _ = mail.sync();
    assert!(h.recorder.system().is_empty(), "a network error");
    // The credential is gone from the vault, or Google refuses it (`invalid_grant`).
    h.fail_with(Some(("sign_in_required", "Reconnect this mail account in Settings")));
    let _ = mail.sync();
    let _ = mail.sync();
    assert_eq!(h.recorder.system(), vec![AppEvent::SignInExpired { account_id: Some(a) }], "once per account");
    h.recorder.clear();
    h.fail_with(Some(("sign_in_expired", qamille::oauth::SIGN_IN_EXPIRED)));
    let _ = mail.sync();
    assert!(h.recorder.system().is_empty(), "still the same expired sign-in");
    // It works again: the next expiry is announced again.
    h.fail_with(None);
    mail.sync().unwrap();
    h.fail_with(Some(("sign_in_expired", qamille::oauth::SIGN_IN_EXPIRED)));
    let _ = mail.sync();
    assert_eq!(h.recorder.system(), vec![AppEvent::SignInExpired { account_id: Some(a) }]);

    // Google's refusal travels as text through the IMAP and SMTP errors, and is read back as its code.
    let wrapped = qamille::oauth::provider_error(format!("IMAP login: {}", qamille::oauth::SIGN_IN_EXPIRED));
    assert_eq!(wrapped.code, "sign_in_expired");
    assert_eq!(qamille::oauth::provider_error("IMAP connect: timed out".into()).code, "external");
    drop(mail);
    drop(h);

    // The Qamille server no longer knows this computer (`unauthorized` of the sync pass).
    let _guard = serial();
    let server = Arc::new(fakes::FakeSync::default());
    let app = fakes::App::new(&server);
    app.sign_in("u1");
    app.consent();
    let expired = || app.events.0.lock().unwrap().iter().filter(|e| e.starts_with("SignInExpired")).cloned().collect::<Vec<_>>();
    server.fail_next("unreachable");
    let t0 = std::time::Instant::now();
    assert!(app.sync.tick(t0).is_err());
    assert!(expired().is_empty(), "an unreachable server is not an expired sign-in");
    server.fail_next("unauthorized");
    assert_eq!(app.sync.sync_now().unwrap_err().code, "unauthorized");
    server.fail_next("unauthorized");
    assert!(app.sync.sync_now().is_err());
    assert_eq!(expired(), vec!["SignInExpired { account_id: None }".to_string()], "once for the server");
    app.sync.sync_now().unwrap();
    server.fail_next("unauthorized");
    assert!(app.sync.sync_now().is_err());
    assert_eq!(expired().len(), 2, "after a pass that worked, announced again");
}

#[test]
fn inbox_unread_matches_the_snapshot_count() {
    let h = harness(Some(NOW - 600));
    let a = h.account_id;
    let before = h.context.read().unwrap().emails.iter().filter(|e| e.folder == "Inbox" && !e.read && !e.done && !e.is_snoozed(NOW)).count();
    h.context
        .update(|store| {
            let base = store.new_id();
            let mut add = |offset: u64, folder: &str, read: bool, change: &dyn Fn(&mut Email)| {
                let mut email = message(a, 900 + offset as u32, folder, read, NOW);
                email.id = base + offset;
                change(&mut email);
                store.emails.push(email);
            };
            add(1, "Inbox", false, &|_| {});
            add(2, "Inbox", true, &|_| {});
            add(3, "Inbox", false, &|e| e.done = true);
            add(4, "Inbox", false, &|e| e.snoozed_until = Some(NOW + 3600));
            add(5, "Archive", false, &|_| {});
            add(6, "Inbox", false, &|_| {});
            store.next_id = base + 10;
            // A «Fatto» still undoable: the sidebar already counts the message as done.
            store.undo_actions.push(UndoAction {
                id: base + 7,
                operation: UndoOperation::Mail { email_id: base + 6, action: "markAsDone".into(), value: None },
                subject: "Subject".into(),
                created_at_ms: NOW * 1000,
                expires_at_ms: NOW * 1000 + 10_000,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            });
            Ok(())
        })
        .unwrap();
    let workspace = WorkspaceService::new(h.context.clone());
    let snapshot = workspace.snapshot().unwrap().counts["Inbox"];
    assert_eq!(workspace.inbox_unread().unwrap(), snapshot, "the taskbar and the sidebar count the same");
    assert_eq!(snapshot, before + 1, "of the six added, only the first counts");
}
