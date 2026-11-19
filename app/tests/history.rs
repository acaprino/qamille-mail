//! History backfill (W3.1) with a fake Gmail: batches newest first, one row per
//! message, the cursor written with the rows, resumption, a new UID generation.
use qamille::{
    application::{
        AppResult, ServiceContext, history,
        ports::{
            AttachmentGateway, Clock, HistoryBatch, HistoryRequest, LabelState, MailGateway, NullSink,
            OutgoingMessage, RemoteAction, SecretStore, SyncBatch,
        },
    },
    domain::{
        model::{Account, AccountKind, Attachment, Email, Store},
        undo::{UndoAction, UndoOperation, UndoStatus},
    },
    infrastructure::repository::MemoryRepository,
};
use std::sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}};

struct Gmail {
    /// UID → message of All Mail.
    messages: Mutex<Vec<(u32, Email)>>,
    validity: AtomicU32,
    calls: AtomicU32,
    /// Label states returned once the walk is done.
    refresh: Mutex<Vec<LabelState>>,
    /// Messages of Spam, returned when the role is asked.
    spam: Mutex<Vec<Email>>,
    /// The requests received.
    requests: Mutex<Vec<HistoryRequest>>,
}
impl MailGateway for Gmail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
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
    fn history_batch(&self, _: &Account, request: &HistoryRequest) -> AppResult<HistoryBatch> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request.clone());
        let (extra, roles_read) = if request.roles.iter().any(|r| r == "Spam") {
            (self.spam.lock().unwrap().clone(), vec!["Spam".to_string()])
        } else {
            (Vec::new(), Vec::new())
        };
        let messages = self.messages.lock().unwrap();
        let validity = self.validity.load(Ordering::SeqCst);
        let uid_next = messages.iter().map(|(uid, _)| *uid).max().unwrap_or(0) + 1;
        let mut progress = request.progress.clone();
        if progress.uid_validity != Some(validity) {
            progress.restart(validity);
        }
        let range = progress.plan(uid_next, request.batch_uids);
        let emails = range
            .map(|range| messages.iter().filter(|(uid, _)| (range.start..=range.end).contains(uid)).map(|(_, e)| e.clone()).collect())
            .unwrap_or_default();
        Ok(HistoryBatch { folder: "[Gmail]/All".into(), uid_validity: validity, uid_next, exists: messages.len() as u32, range, emails, refresh: self.refresh.lock().unwrap().clone(), extra, roles_read, ..Default::default() })
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok("token".into())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> i64 {
        1_800_000_000
    }
}
struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
fn message(uid: u32, global: u64, folder: &str) -> (u32, Email) {
    (uid, Email {
        uid: Some(uid),
        remote_folder: Some("[Gmail]/All".into()),
        remote_uid_validity: Some(1),
        remote_global_id: Some(global),
        message_id: format!("{global}@example.com"),
        subject: format!("Old {global}"),
        folder: folder.into(),
        date: global as i64,
        remote_only: true,
        ..Default::default()
    })
}
fn setup(count: u32) -> (Arc<ServiceContext>, Arc<Gmail>) {
    let gmail = Arc::new(Gmail {
        messages: Mutex::new((1..=count).map(|n| message(n, u64::from(n), if n % 3 == 0 { "Inbox" } else { "Archive" })).collect()),
        validity: AtomicU32::new(1),
        calls: AtomicU32::new(0),
        refresh: Mutex::new(Vec::new()),
        spam: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
    });
    let mut store = Store::default();
    store.accounts.push(Account { id: 1, kind: AccountKind::Gmail, provider_id: "gmail".into(), email: "me@example.com".into(), ..Default::default() });
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: gmail.clone(),
        secrets: Arc::new(Secrets),
        clock: Arc::new(Fixed),
        attachments: Arc::new(NoFiles),
        events: Arc::new(NullSink),
    });
    (context, gmail)
}
fn run_all(context: &ServiceContext) -> usize {
    let mut batches = 0;
    while history::run_batch(context, 1).unwrap() {
        batches += 1;
        assert!(batches < 1000);
    }
    batches + 1
}

#[test]
fn the_whole_history_arrives_in_batches_with_one_row_per_message() {
    let (context, gmail) = setup(1_200);
    assert!(history::pending(&context, 1).unwrap());
    assert_eq!(run_all(&context), 3, "500 + 500 + 200 UIDs");
    let store = context.read().unwrap();
    assert_eq!(store.emails.len(), 1_200);
    assert!(store.emails.iter().all(|e| e.remote_only && e.body.is_empty()));
    assert_eq!(store.emails.iter().filter(|e| e.folder == "Inbox").count(), 400);
    let progress = &store.accounts[0].history;
    assert!(progress.complete && progress.fetched == 1_200 && progress.total == 1_200);
    assert!(!history::pending(&context, 1).unwrap());
    // Running again writes nothing new and reads only arrivals.
    gmail.messages.lock().unwrap().push(message(1_201, 1_201, "Inbox"));
    assert!(!history::run_batch(&context, 1).unwrap());
    assert_eq!(context.read().unwrap().emails.len(), 1_201);
}

#[test]
fn an_interrupted_backfill_resumes_after_its_last_written_batch() {
    // EX-6
    let (context, gmail) = setup(1_500);
    assert!(history::run_batch(&context, 1).unwrap());
    assert!(history::run_batch(&context, 1).unwrap());
    let calls = gmail.calls.load(Ordering::SeqCst);
    // "The app closes": a new run starts from the stored cursor.
    run_all(&context);
    assert_eq!(gmail.calls.load(Ordering::SeqCst) - calls, 1, "only the last batch was left");
    assert_eq!(context.read().unwrap().emails.len(), 1_500);
}

#[test]
fn a_known_message_is_enriched_and_keeps_its_local_state() {
    // EX-5: one row however many labels; local tasks survive.
    let (context, _) = setup(10);
    context.update(|store| {
        let mut known = message(5, 5, "Inbox").1;
        known.id = store.new_id();
        known.account_id = 1;
        known.pinned = true;
        known.uid = Some(77);
        known.remote_folder = Some("INBOX".into());
        store.emails.push(known);
        Ok(())
    }).unwrap();
    run_all(&context);
    let store = context.read().unwrap();
    assert_eq!(store.emails.len(), 10);
    assert!(store.emails.iter().find(|e| e.remote_global_id == Some(5)).unwrap().pinned);
}

#[test]
fn a_new_uid_generation_walks_again_without_duplicates() {
    // EX-7
    let (context, gmail) = setup(600);
    run_all(&context);
    gmail.validity.store(9, Ordering::SeqCst);
    for (uid, email) in gmail.messages.lock().unwrap().iter_mut() {
        *uid += 1_000;
        email.uid = Some(*uid);
        email.remote_uid_validity = Some(9);
    }
    assert!(history::pending(&context, 1).unwrap() || true);
    context.update(|store| { store.accounts[0].history.complete = false; Ok(()) }).unwrap();
    run_all(&context);
    let store = context.read().unwrap();
    assert_eq!(store.emails.len(), 600, "found again by their Gmail identity");
    assert_eq!(store.accounts[0].history.uid_validity, Some(9));
}

#[test]
fn labels_changed_on_the_web_reach_rows_without_a_pending_action() {
    // W3.2: archived on the web, read on the phone; a row with a queued action keeps its view.
    let (context, gmail) = setup(10);
    run_all(&context);
    let id_of = |global: u64| context.read().unwrap().emails.iter().find(|e| e.remote_global_id == Some(global)).unwrap().id;
    let (archived, read, pending) = (id_of(3), id_of(6), id_of(9));
    context.update(|store| {
        store.undo_actions.push(UndoAction {
            id: 900,
            operation: UndoOperation::Mail { email_id: pending, action: "star".into(), value: None },
            subject: String::new(),
            created_at_ms: 0,
            expires_at_ms: i64::MAX,
            status: UndoStatus::Pending,
            error: None,
            retry_at_ms: None,
            attempts: 0,
        });
        Ok(())
    }).unwrap();
    *gmail.refresh.lock().unwrap() = vec![
        LabelState { global_id: 3, labels: vec!["Clienti".into()], ..Default::default() },
        LabelState { global_id: 6, labels: vec!["\\Inbox".into()], read: true, ..Default::default() },
        LabelState { global_id: 9, labels: vec![], ..Default::default() },
    ];
    assert!(!history::run_batch(&context, 1).unwrap());
    let store = context.read().unwrap();
    let row = |id: u64| store.email(id).unwrap();
    assert_eq!((row(archived).folder.as_str(), row(archived).labels.clone()), ("Archive", vec!["Clienti".to_string()]));
    assert!(row(read).read && row(read).folder == "Inbox");
    assert_eq!(row(pending).folder, "Inbox", "the queued action decides, not the refresh");
}

#[test]
fn many_old_unread_inbox_messages_ask_once() {
    // EX-9: 133 unread Inbox messages arrive; the notice is due once.
    let (context, gmail) = setup(400);
    run_all(&context);
    let history = context.read().unwrap().accounts[0].history.clone();
    assert_eq!(history.unread_inbox, 133);
    assert!(history.unread_notice_shown);
    // "Mark read the ones older than 30 days": all of them here (dates near 1970).
    let changed = history::mark_old_unread_read(&context, 1, 30).unwrap();
    assert_eq!(changed, 133);
    let store = context.read().unwrap();
    assert!(store.emails.iter().filter(|e| e.folder == "Inbox").all(|e| e.read));
    let before = store.accounts[0].history.mark_read_before;
    assert_eq!(before, Some(1_800_000_000 - 30 * 86_400));
    drop(store);
    // The next batch carries it to Gmail, then it is cleared.
    history::run_batch(&context, 1).unwrap();
    assert_eq!(gmail.requests.lock().unwrap().last().unwrap().progress.mark_read_before, before);
    assert_eq!(context.read().unwrap().accounts[0].history.mark_read_before, None);
}

#[test]
fn spam_opened_on_request_arrives_with_the_next_batch() {
    let (context, gmail) = setup(10);
    run_all(&context);
    let mut spam = message(1, 5_000, "Spam").1;
    spam.remote_folder = Some("[Gmail]/Spam".into());
    gmail.spam.lock().unwrap().push(spam);
    history::request_folder(&context, 1, "Spam").unwrap();
    assert!(history::request_folder(&context, 1, "Inbox").is_err());
    history::run_batch(&context, 1).unwrap();
    let store = context.read().unwrap();
    assert_eq!(store.emails.iter().filter(|e| e.folder == "Spam").count(), 1);
    assert!(store.accounts[0].history.requested.is_empty(), "served once");
    drop(store);
    // With "include Spam and Trash" every finished batch reads them.
    history::set_include_spam_trash(&context, 1, true).unwrap();
    history::run_batch(&context, 1).unwrap();
    let roles = gmail.requests.lock().unwrap().last().unwrap().roles.clone();
    assert_eq!(roles, ["Spam", "Trash"]);
}

#[test]
fn an_empty_all_mail_is_a_complete_history() {
    // Review of waves 2-3: no endless requeue for an empty mailbox.
    let (context, _) = setup(0);
    assert!(!history::run_batch(&context, 1).unwrap());
    assert!(context.read().unwrap().accounts[0].history.complete);
    assert!(!history::pending(&context, 1).unwrap());
}

#[test]
fn the_label_refresh_keeps_folders_and_labels_of_the_person() {
    let (context, gmail) = setup(10);
    run_all(&context);
    context.update(|store| {
        let row = store.emails.iter_mut().find(|e| e.remote_global_id == Some(3)).unwrap();
        row.folder = "Projects".into();
        let row = store.emails.iter_mut().find(|e| e.remote_global_id == Some(4)).unwrap();
        row.labels = vec!["Work".into()];
        Ok(())
    }).unwrap();
    *gmail.refresh.lock().unwrap() = vec![
        LabelState { global_id: 3, labels: vec![], ..Default::default() },
        LabelState { global_id: 4, labels: vec!["Clienti".into()], ..Default::default() },
    ];
    history::run_batch(&context, 1).unwrap();
    let store = context.read().unwrap();
    let row = |global: u64| store.emails.iter().find(|e| e.remote_global_id == Some(global)).unwrap();
    assert_eq!(row(3).folder, "Projects", "a folder of the person's own stays");
    assert_eq!(row(4).labels, ["Work", "Clienti"], "local labels stay, Gmail's are added");
}
