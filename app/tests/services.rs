#[test]
fn email_exports_preserve_original_bytes_deduplicate_and_do_not_mark_read() {
    use qamille::application::mail::ExportFormat;
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let mail = MailService::new(ctx.clone());
    let original = ctx.read().unwrap();
    let id = original.emails[0].id;
    let out = mail.export(&[id, id], ExportFormat::Eml).unwrap();
    assert_eq!(out.count, 1);
    assert_eq!(out.bytes, format!("Subject: original-{id}\r\nX-Original: preserved\r\n\r\nOriginal body\r\n").as_bytes());
    assert_eq!(ctx.read().unwrap().email(id).unwrap().read, original.email(id).unwrap().read);
    let second = original.emails.iter().find(|e| e.account_id == original.emails[0].account_id && e.id != id).unwrap().id;
    let out = mail.export(&[second, id, second], ExportFormat::Mbox).unwrap();
    assert_eq!(out.count, 2);
    assert_eq!(out.bytes.split(|b| *b == b'\n').filter(|line| line.starts_with(b"From ")).count(), 2);
    assert!(mail.export(&[id, second], ExportFormat::Eml).is_err());
    assert!(mail.export(&[], ExportFormat::Mbox).is_err());
    assert!(mail.export(&[id, u64::MAX], ExportFormat::Mbox).is_err());
}

#[test]
fn messages_for_a_draft_are_one_eml_each_within_the_size_budget() {
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let mail = MailService::new(ctx.clone());
    let store = ctx.read().unwrap();
    let id = store.emails[0].id;
    let second = store
        .emails
        .iter()
        .find(|e| e.account_id == store.emails[0].account_id && e.id != id)
        .unwrap()
        .id;
    let out = mail.export_eml_each(&[second, id, second], 25 * 1024 * 1024).unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|e| e.count == 1 && e.filename.ends_with(".eml")));
    let original = format!("Subject: original-{id}\r\nX-Original: preserved\r\n\r\nOriginal body\r\n");
    assert!(out.iter().any(|e| e.bytes == original.as_bytes()));
    let too_large = mail.export_eml_each(&[id, second], 10).unwrap_err();
    assert_eq!(too_large.code, "validation");
    assert!(too_large.message.contains("25 MB"), "{}", too_large.message);
    assert!(mail.export_eml_each(&[], 1024).is_err());
}

#[test]
fn failed_download_attaches_nothing() {
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), gateway);
    let id = ctx.read().unwrap().emails[0].id;
    let error = MailService::new(ctx).export_eml_each(&[id], 25 * 1024 * 1024).unwrap_err();
    assert_eq!(error.code, "external");
}

#[test]
fn failed_download_cannot_produce_a_partial_archive_or_silent_local_thread() {
    use qamille::application::mail::ExportFormat;
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), gateway);
    let id = ctx.read().unwrap().emails[0].id;
    let mail = MailService::new(ctx);
    assert!(mail.export(&[id], ExportFormat::Mbox).is_err());
    assert!(mail.thread(id).is_err());
}

#[test]
fn print_thread_uses_remote_members_even_when_not_in_inbox() {
    let gateway = Arc::new(TestMail::default());
    let store = fixture();
    let anchor = store.emails[0].clone();
    let mut remote_reply = anchor.clone();
    remote_reply.id = 99999;
    remote_reply.folder = "Sent".into();
    *gateway.batch.lock().unwrap() = vec![anchor.clone(), remote_reply];
    let mail = MailService::new(context(store, gateway));
    assert_eq!(mail.thread(anchor.id).unwrap().len(), 2);
}

use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        drafts::DraftService,
        mail::{MailQuery, MailService},
        ports::*,
        scheduler::SchedulerService,
        undo::UndoService,
        workspace::WorkspaceService,
    },
    infrastructure::repository::{JsonStoreRepository, MemoryRepository},
    model::*,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[path = "support/storage.rs"]
mod account_storage;

#[path = "support/indexing.rs"]
mod background_indexing;

#[path = "support/sync_feedback.rs"]
mod sync_feedback;

struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
#[derive(Default)]
struct TestSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for TestSecrets {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
struct TestAttachments;
impl AttachmentGateway for TestAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, a: &Attachment) -> AppResult<String> {
        Ok(a.inline_text.clone().unwrap_or_default())
    }
}
#[derive(Default)]
struct TestMail {
    body_loads: AtomicU64,
    read_denied: AtomicBool,
    downloaded_body: Mutex<Option<String>>,
    during_body: Mutex<Option<Box<dyn Fn() + Send>>>,
    applied: Mutex<Vec<(u32, u32, RemoteAction)>>,
    fail: AtomicBool,
    sent: Mutex<Vec<OutgoingMessage>>,
    batch: Mutex<Vec<Email>>,
    search_requests: Mutex<Vec<RemoteSearchRequest>>,
    sends: AtomicU64,
    during_send: Mutex<Option<Box<dyn Fn() + Send>>>,
    during_sync: Mutex<Option<Box<dyn Fn() + Send>>>,
    sync_failure: Mutex<Option<String>>,
}
impl MailGateway for TestMail {
    fn authorize_read(&self, _: &Account) -> AppResult<()> {
        if self.read_denied.load(Ordering::SeqCst) { return Err(AppError::new("permission_denied", "Read access revoked")); }
        Ok(())
    }
    fn load_body(&self, _: &Account, email: &Email) -> AppResult<Email> {
        self.body_loads.fetch_add(1, Ordering::SeqCst);
        if let Some(work) = self.during_body.lock().unwrap().take() { work(); }
        if self.fail.load(Ordering::SeqCst) { return Err(AppError::external("Connect to Gmail to read this message")); }
        let mut loaded = email.clone();
        loaded.body = self.downloaded_body.lock().unwrap().clone().unwrap_or_else(|| "Downloaded on demand".into());
        for attachment in &mut loaded.attachments { attachment.inline_text = Some("Online attachment".into()); }
        Ok(loaded)
    }
    fn load_body_preview(&self, account: &Account, email: &Email) -> AppResult<Email> {
        let mut loaded = self.load_body(account, email)?;
        for attachment in &mut loaded.attachments { attachment.inline_text = None; }
        Ok(loaded)
    }
    fn search(&self, _: &Account, request: &RemoteSearchRequest, cancel: &AtomicBool) -> AppResult<RemoteSearchPage> {
        if self.fail.load(Ordering::SeqCst) || cancel.load(Ordering::Relaxed) { return Err(AppError::external("Server search failed")); }
        self.search_requests.lock().unwrap().push(request.clone());
        // The fake's cursor is an offset in `batch`.
        let all = self.batch.lock().unwrap().clone();
        let start = request.cursor.as_ref().map_or(0, |c| c.before_uid as usize).min(all.len());
        let end = (start + request.page_size).min(all.len());
        Ok(RemoteSearchPage {
            emails: all[start..end].to_vec(),
            total: all.len() as u32,
            mailbox_total: 48_300,
            next: (end < all.len()).then(|| RemoteCursor { folder: "[Gmail]/All Mail".into(), uid_validity: 1, before_uid: end as u32 }),
        })
    }
    fn originals(&self, _: &Account, emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        if self.fail.load(Ordering::SeqCst) { return Err(AppError::external("Download failed")); }
        Ok(emails.iter().map(|e| format!("Subject: original-{}\r\nX-Original: preserved\r\n\r\nOriginal body\r\n", e.id).into_bytes()).collect())
    }
    fn thread(&self, _: &Account, _: &Email) -> AppResult<Vec<Email>> {
        if self.fail.load(Ordering::SeqCst) { return Err(AppError::external("Thread download failed")); }
        Ok(self.batch.lock().unwrap().clone())
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        if let Some(work) = self.during_sync.lock().unwrap().take() {
            work();
        }
        if let Some(message) = self.sync_failure.lock().unwrap().clone() {
            return Err(AppError::external(message));
        }
        Ok(SyncBatch {
            emails: self.batch.lock().unwrap().clone(),
            archive_folder: Some("All".into()),
            trash_folder: Some("Bin".into()),
        })
    }
    fn apply(&self, account: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        if self.fail.load(Ordering::SeqCst) {
            Err(AppError::external("Server refused"))
        } else {
            self.applied.lock().unwrap().push((account.id, uid, action));
            Ok(())
        }
    }
    fn send(&self, _: &Account, _: &str, message: OutgoingMessage) -> AppResult<()> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::external("Send failed"));
        }
        self.sent.lock().unwrap().push(message);
        if let Some(work) = self.during_send.lock().unwrap().take() {
            work();
        }
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
#[test]
fn reopening_online_mail_reuses_memory_without_restoring_stale_flags_or_writing_bodies() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let id = store.emails[0].id;
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    let ctx = context(store, gateway.clone());
    let service = MailService::new(ctx.clone());
    assert_eq!(service.read(id).unwrap().body, "Downloaded on demand");
    ctx.update(|store| { store.email_mut(id).unwrap().read = true; Ok(()) }).unwrap();
    gateway.fail.store(true, Ordering::SeqCst);
    let reopened = service.clone().read(id).expect("Already opened mail remains immediately available in this session");
    assert_eq!(reopened.body, "Downloaded on demand");
    assert!(reopened.read, "Current local flags must remain authoritative");
    assert_eq!(gateway.body_loads.load(Ordering::SeqCst), 1);
    assert!(ctx.read().unwrap().email(id).unwrap().body.is_empty());
    gateway.read_denied.store(true, Ordering::SeqCst);
    assert_eq!(service.read(id).unwrap_err().code, "permission_denied");
    gateway.read_denied.store(false, Ordering::SeqCst);
    ctx.update(|store| { store.email_mut(id).unwrap().remote_global_id = Some(999); Ok(()) }).unwrap();
    assert!(service.read(id).is_err(), "A changed remote identity cannot reuse the old body");
}
#[test]
fn online_attachment_text_is_loaded_only_when_requested() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let email = &mut store.emails[0];
    email.remote_only = true;
    email.body.clear();
    email.attachments = vec![Attachment { id: 12345, name: "invoice.txt".into(), ..Default::default() }];
    let id = email.id;
    let service = MailService::new(context(store, gateway.clone()));
    let email = service.read(id).unwrap();
    assert_eq!(email.body, "Downloaded on demand");
    assert!(email.attachments[0].inline_text.is_none());
    assert_eq!(gateway.body_loads.load(Ordering::SeqCst), 1);
    assert_eq!(service.attachment_text(id, 12345).unwrap(), "Online attachment");
    assert_eq!(gateway.body_loads.load(Ordering::SeqCst), 2);
}
#[test]
fn document_preview_service_returns_the_kind_and_bytes_of_previewable_attachments() {
    use qamille::application::document_preview::DocumentPreviewService;
    let mut store = fixture();
    let id = store.emails[0].id;
    let attachment = |id: u64, name: &str, text: &str| Attachment {
        id,
        name: name.into(),
        size: text.len() as u64,
        mime: "application/octet-stream".into(),
        path: None,
        inline_text: Some(text.into()),
    };
    store.emails[0].attachments = vec![
        attachment(77, "report.pdf", "%PDF"),
        attachment(78, "empty.pdf", ""),
        attachment(79, "Letter.DOCX", "PK"),
        attachment(80, "notes.txt", "plain"),
    ];
    let mail = MailService::new(context(store, Arc::new(TestMail::default())));
    let previews = DocumentPreviewService::new(mail);
    assert_eq!(previews.document_source(id, 77, &[]).unwrap(), ("pdf", b"%PDF".to_vec()));
    assert_eq!(previews.document_source(id, 79, &[]).unwrap(), ("docx", b"PK".to_vec()));
    assert_eq!(previews.document_source(id, 80, &[]).unwrap_err().code, "preview_unsupported");
    assert_eq!(previews.document_source(id, 78, &[]).unwrap_err().code, "validation");
    assert_eq!(previews.document_source(id, 99, &[]).unwrap_err().code, "not_found");
    // A PDF is not a message: nothing can be nested inside it.
    assert_eq!(previews.document_source(id, 77, &[0]).unwrap_err().code, "validation");
}
#[test]
fn online_read_cache_evicts_old_entries_at_its_count_and_text_budgets() {
    for large in [false, true] {
        let gateway = Arc::new(TestMail::default());
        if large { *gateway.downloaded_body.lock().unwrap() = Some("x".repeat(9 * 1024 * 1024)); }
        let mut store = fixture();
        let mut template = store.emails[0].clone();
        template.remote_only = true;
        template.body.clear();
        template.attachments.clear();
        let count = if large { 2 } else { 21 };
        store.emails = (1..=count).map(|id| Email { id, ..template.clone() }).collect();
        let service = MailService::new(context(store, gateway.clone()));
        for id in 1..=count { service.read(id).unwrap(); }
        gateway.fail.store(true, Ordering::SeqCst);
        assert!(service.read(count).is_ok(), "The newest message remains available");
        assert!(service.read(1).is_err(), "The oldest body was evicted at the cache budget");
    }
}
#[test]
fn prefetch_loads_before_click_and_a_simultaneous_click_shares_the_download() {
    use std::sync::mpsc;
    use std::time::Duration;
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let id = store.emails[0].id;
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    store.emails[0].read = false;
    let ctx = context(store, gateway.clone());
    let service = MailService::new(ctx.clone());
    let (started, entered) = mpsc::channel();
    let (release, proceed) = mpsc::channel();
    *gateway.during_body.lock().unwrap() = Some(Box::new(move || { started.send(()).unwrap(); proceed.recv().unwrap(); }));
    service.prefetch(vec![id]).unwrap();
    entered.recv_timeout(Duration::from_secs(2)).expect("Prefetch must download before the user clicks");
    let reader = service.clone();
    let click = std::thread::spawn(move || reader.read(id));
    release.send(()).unwrap();
    assert_eq!(click.join().unwrap().unwrap().body, "Downloaded on demand");
    assert_eq!(gateway.body_loads.load(Ordering::SeqCst), 1, "A click must share the ongoing prefetch");
    assert!(!ctx.read().unwrap().email(id).unwrap().read);
    assert!(ctx.read().unwrap().email(id).unwrap().body.is_empty());
    assert!(gateway.applied.lock().unwrap().is_empty());
}
fn context(store: Store, mail: Arc<TestMail>) -> Arc<ServiceContext> {
    context_with_clock(store, mail, Arc::new(TestClock))
}
fn context_with_clock(
    store: Store,
    mail: Arc<TestMail>,
    clock: Arc<dyn Clock>,
) -> Arc<ServiceContext> {
    let secrets = Arc::new(TestSecrets::default());
    for a in &store.accounts {
        secrets.set(&account_key(&a.email), "test-only").unwrap();
    }
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail,
        secrets,
        clock,
        attachments: Arc::new(TestAttachments),
    })
}
#[test]
fn search_paginates_after_account_filter_and_uses_original_dates() {
    let mut store = sample_store();
    store.settings.time_zone = "UTC".into();
    let template = store.emails[0].clone();
    let wanted_account = template.account_id;
    store.emails = (1..=75).map(|id| Email {
        id, account_id: if id <= 45 { wanted_account + 999 } else { wanted_account },
        subject: "needle".into(), date: 1_800_000_000 - id as i64,
        returned_at: Some(2_000_000_000), folder: "Inbox".into(), ..template.clone()
    }).collect();
    let service = MailService::new(context(store, Arc::new(TestMail::default())));
    let first = service.list(MailQuery { search: "needle".into(), account_id: Some(wanted_account), limit: 20, ..Default::default() }).unwrap();
    assert_eq!(first.total, 30);
    assert_eq!(first.items.len(), 20);
    assert_eq!(first.next_offset, Some(20));
    assert_eq!(first.items[0].date, 1_800_000_000 - 46);
    let last = service.list(MailQuery { search: "needle".into(), account_id: Some(wanted_account), limit: 20, offset: 20, ..Default::default() }).unwrap();
    assert_eq!(last.items.len(), 10);
    assert_eq!(last.next_offset, None);
    assert!(service.list(MailQuery { search: "around:2026-02-31".into(), ..Default::default() }).is_err());
}

#[test]
fn remote_search_imports_archived_mail_and_reports_failures_without_claiming_complete() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.emails.clear();
    let email = Email { account_id, message_id: "remote-search@example.com".into(), uid: Some(42), remote_folder: Some("[Gmail]/All Mail".into()), remote_uid_validity: Some(7), remote_content_hash: "verified-remote-search-mime".into(), folder: "Archive".into(), subject: "Remote needle".into(), body: "Archived outside Inbox".into(), date: 1_800_000_000, ..Default::default() };
    *gateway.batch.lock().unwrap() = vec![email];
    let ctx = context(store, gateway.clone());
    let service = MailService::new(ctx.clone());
    let query = || MailQuery { search: "needle".into(), account_id: Some(account_id), ..Default::default() };
    assert_eq!(service.list(query()).unwrap().search.unwrap().coverage, "partial");
    let result = service.search_remote("needle", Some(account_id), None).unwrap();
    assert_eq!(result.added, 1);
    assert!(result.errors.is_empty());
    assert_eq!(service.list(query()).unwrap().items.len(), 1);
    assert_eq!(service.list(query()).unwrap().search.unwrap().coverage, "complete");
    assert_eq!(service.search_remote("needle", Some(account_id), None).unwrap().added, 0);
    gateway.fail.store(true, Ordering::SeqCst);
    let result = service.search_remote("different", Some(account_id), None).unwrap();
    assert_eq!(result.errors.len(), 1);
    let page = service.list(MailQuery { search: "different".into(), account_id: Some(account_id), ..Default::default() }).unwrap();
    assert_eq!(page.search.unwrap().coverage, "partial");
    assert_eq!(ctx.read().unwrap().emails.len(), 1);
}

#[test]
fn server_results_are_rows_that_keep_their_local_state() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let mut old = store.emails[0].clone();
    let id = old.id;
    let account_id = old.account_id;
    old.body.clear(); old.attachments.clear(); old.remote_only = true;
    old.subject = "Contract".into();
    old.pinned = true;
    old.read = false;
    old.labels.clear();
    old.remote_global_id = Some(900);
    let fresh = Email { id: 0, pinned: false, read: true, labels: vec!["Clienti".into()], uid: Some(77), remote_folder: Some("[Gmail]/All Mail".into()), remote_uid_validity: Some(3), ..old.clone() };
    store.emails = vec![old];
    *gateway.batch.lock().unwrap() = vec![fresh];
    let ctx = context(store, gateway.clone());
    let service = MailService::new(ctx.clone());
    let report = service.search_remote("needle", Some(account_id), None).unwrap();
    assert!(report.errors.is_empty());
    assert_eq!((report.added, report.ids.clone()), (0, vec![id]), "a known message is not added twice");
    assert_eq!(gateway.search_requests.lock().unwrap()[0].raw, "needle");
    let persisted = ctx.read().unwrap();
    assert_eq!(persisted.emails.len(), 1);
    let row = persisted.email(id).unwrap();
    assert!(row.pinned, "local state survives");
    assert!(!row.read, "local read state is not overwritten by the server");
    assert!(row.labels.contains(&"Clienti".to_string()), "labels the row never had are filled");
    assert_eq!(row.uid, Some(10), "the saved location stays");
    assert!(row.remote_locations.iter().any(|l| l.folder == "[Gmail]/All Mail" && l.uid == 77));
    // The body is not on this computer: only the server could tell it matches.
    let page = service.list(MailQuery { search: "needle".into(), account_id: Some(account_id), ..Default::default() }).unwrap();
    assert_eq!(page.items.iter().map(|i| i.id).collect::<Vec<_>>(), [id]);
    assert_eq!(page.search.unwrap().notice.unwrap(), "Results from the server: 1 of 1.");
}

fn remote_mail(account_id: u32, n: u64) -> Email {
    Email { account_id, message_id: format!("remote-{n}@example.com"), remote_global_id: Some(10_000 + n), uid: Some(n as u32), remote_folder: Some("[Gmail]/All Mail".into()), remote_uid_validity: Some(1), remote_only: true, folder: "Archive".into(), from_addr: "anna@example.com".into(), from_name: "Anna".into(), subject: format!("Fattura {n}"), date: 1_500_000_000 - n as i64, ..Default::default() }
}

#[test]
fn server_results_arrive_in_pages_of_fifty_with_a_cursor() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.emails.clear();
    *gateway.batch.lock().unwrap() = (1..=120).map(|n| remote_mail(account_id, n)).collect();
    let ctx = context(store, gateway.clone());
    let service = MailService::new(ctx.clone());
    let first = service.search_remote("enel", Some(account_id), None).unwrap();
    assert_eq!((first.ids.len(), first.total, first.added), (50, 120, 50));
    assert!(first.notice.as_deref().unwrap().contains("50 of 120"));
    let second = service.search_remote("enel", Some(account_id), first.next.clone()).unwrap();
    assert_eq!(second.ids.len(), 100);
    let third = service.search_remote("enel", Some(account_id), second.next.clone()).unwrap();
    assert_eq!((third.ids.len(), third.next.is_none()), (120, true));
    assert_eq!(ctx.read().unwrap().emails.len(), 120);
    let page = service.list(MailQuery { search: "enel".into(), account_id: Some(account_id), limit: 100, ..Default::default() }).unwrap();
    assert_eq!(page.total, 120);
    assert_eq!(page.search.unwrap().coverage, "complete");
    // EX-2: an old message found on the server stays in its folder.
    let archive = service.list(MailQuery { folder: "Archive".into(), account_id: Some(account_id), limit: 100, ..Default::default() }).unwrap();
    assert_eq!(archive.total, 120);
}

#[test]
fn a_partly_local_search_is_filtered_on_this_computer() {
    // EX-3
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.emails.clear();
    let mut task = remote_mail(account_id, 1);
    task.pinned = true;
    store.emails.push(Email { id: 500, ..task.clone() });
    store.next_id = 1000;
    *gateway.batch.lock().unwrap() = vec![task, remote_mail(account_id, 2)];
    let service = MailService::new(context(store, gateway.clone()));
    let report = service.search_remote("is:task from:anna", Some(account_id), None).unwrap();
    assert_eq!(gateway.search_requests.lock().unwrap()[0].raw, "from:anna");
    assert_eq!(report.ids, [500]);
    assert_eq!(report.added, 1, "the other result is still kept as a row");
}

#[test]
fn a_search_only_about_local_data_never_asks_the_server() {
    // EX-4
    let gateway = Arc::new(TestMail::default());
    let store = fixture();
    let account_id = store.accounts[0].id;
    let service = MailService::new(context(store, gateway.clone()));
    let report = service.search_remote("is:snoozed", Some(account_id), None).unwrap();
    assert!(report.local_only);
    assert!(report.notice.unwrap().contains("only on this computer"));
    assert!(gateway.search_requests.lock().unwrap().is_empty());
}

#[test]
fn the_local_notice_says_how_much_of_the_mailbox_is_on_this_computer() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.emails.retain(|e| e.account_id == account_id);
    let local = store.emails.len();
    *gateway.batch.lock().unwrap() = vec![];
    let service = MailService::new(context(store, gateway));
    service.search_remote("unrelated", Some(account_id), None).unwrap();
    let page = service.list(MailQuery { search: "other".into(), account_id: Some(account_id), ..Default::default() }).unwrap();
    assert_eq!(page.search.unwrap().notice.unwrap(), format!("Searching {local} of 48,300 messages in this account."));
}

fn fixture() -> Store {
    let mut s = sample_store();
    s.accounts[0].kind = AccountKind::Gmail;
    s.accounts[0].archive_folder = "[Gmail]/All Mail".into();
    s.accounts[0].trash_folder = "[Gmail]/Trash".into();
    s.emails[0].uid = Some(10);
    s
}

#[test]
fn failed_remote_action_does_not_change_local_mail() {
    let mail = Arc::new(TestMail::default());
    mail.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), mail);
    let id = ctx.read().unwrap().emails[0].id;
    assert!(
        MailService::new(ctx.clone())
            .act(id, "archive", None)
            .is_err()
    );
    let store = ctx.read().unwrap();
    assert_eq!(store.email(id).unwrap().folder, "Inbox");
    assert_eq!(store.email(id).unwrap().uid, Some(10));
}
#[test]
fn archive_commits_and_invalidates_inbox_uid() {
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let id = ctx.read().unwrap().emails[0].id;
    MailService::new(ctx.clone())
        .act(id, "archive", None)
        .unwrap();
    let store = ctx.read().unwrap();
    assert_eq!(store.email(id).unwrap().folder, "Archive");
    assert_eq!(store.email(id).unwrap().uid, None);
}
#[test]
fn mail_actions_on_the_sqlite_store_change_only_their_message() {
    use qamille::infrastructure::sqlite_store::SqliteStoreRepository;
    let root = std::env::temp_dir().join(format!(
        "qamille-actions-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let repository = Arc::new(SqliteStoreRepository::open(root.join("store.sqlite")).unwrap());
    let store = fixture();
    repository.transaction(&mut |s| { *s = store.clone(); Ok(()) }).unwrap();
    let secrets = Arc::new(TestSecrets::default());
    for a in &store.accounts {
        secrets.set(&account_key(&a.email), "test-only").unwrap();
    }
    let ctx = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: repository.clone(),
        mail: Arc::new(TestMail::default()),
        secrets,
        clock: Arc::new(TestClock),
        attachments: Arc::new(TestAttachments),
    });
    let mail = MailService::new(ctx.clone());
    let before = ctx.read().unwrap();
    let id = before.emails[0].id;
    // Each commit names its message (`update_emails`): debug builds check
    // that the action code really changes nothing else.
    for (action, value) in [("markAsUnseen", None), ("pin", None), ("attachLabel", Some("Clienti")), ("archive", None)] {
        mail.act(id, action, value).unwrap();
    }
    assert!(mail.act(id, "moveToFolder", None).is_err(), "a refused action");
    let after = ctx.read().unwrap();
    let email = after.email(id).unwrap();
    assert!(!email.read && email.pinned && email.folder == "Archive");
    assert!(email.labels.iter().any(|label| label == "Clienti"));
    assert_eq!(email.uid, None, "the move made the Inbox UID invalid");
    for other in before.emails.iter().filter(|e| e.id != id) {
        assert_eq!(after.email(other.id), Some(other), "message {} untouched", other.id);
    }
    let reopened = SqliteStoreRepository::open(root.join("store.sqlite")).unwrap().read().unwrap();
    assert_eq!(serde_json::to_value(&after).unwrap(), serde_json::to_value(&reopened).unwrap(), "memory is what was committed");
    drop((ctx, mail, repository));
    let _ = std::fs::remove_dir_all(&root);
}
#[test]
fn stale_draft_save_cannot_overwrite_a_newer_edit() {
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let svc = DraftService::new(ctx.clone());
    let draft = svc.create(Some(1), None, false).unwrap();
    let mut newer = draft.clone();
    newer.body = "new content".into();
    svc.save(newer).unwrap();
    let error = svc.save(draft).unwrap_err();
    assert_eq!(error.code, "conflict");
    assert_eq!(
        ctx.read().unwrap().drafts.last().unwrap().body,
        "new content"
    );
}
#[test]
fn failed_send_keeps_draft_and_does_not_add_sent_mail() {
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), gateway);
    let svc = DraftService::new(ctx.clone());
    let mut d = svc.create(Some(1), None, false).unwrap();
    d.to = "recipient@example.com".into();
    d.body = "keep me".into();
    let d = svc.save(d).unwrap();
    let count = ctx.read().unwrap().emails.len();
    assert!(svc.send(d.id).is_err());
    let s = ctx.read().unwrap();
    assert!(s.drafts.iter().any(|x| x.id == d.id));
    assert_eq!(s.emails.len(), count);
}
#[test]
fn successful_reply_uses_signature_and_keeps_thread() {
    let gateway = Arc::new(TestMail::default());
    let ctx = context(fixture(), gateway.clone());
    let svc = DraftService::new(ctx.clone());
    let email = ctx.read().unwrap().emails[0].clone();
    let mut d = svc.create(None, Some(email.id), false).unwrap();
    d.body = "Approved".into();
    let d = svc.save(d).unwrap();
    svc.send(d.id).unwrap();
    let s = ctx.read().unwrap();
    assert!(!s.drafts.iter().any(|x| x.id == d.id));
    assert!(s.email(email.id).unwrap().replied);
    let sent = s.emails.last().unwrap();
    assert_eq!(sent.folder, "Sent");
    assert_eq!(sent.thread_id, email.thread_id);
    assert_eq!(
        sent.in_reply_to,
        email.message_id.trim_matches(['<', '>']),
        "the reply records the message it answers, not the thread it copies"
    );
    assert!(sent.body.contains("Alex"));
    assert_eq!(
        gateway.sent.lock().unwrap()[0].in_reply_to,
        Some(email.message_id)
    );
}
#[test]
fn sync_deduplicates_and_preserves_local_tasks() {
    let gateway = Arc::new(TestMail::default());
    let mut s = fixture();
    s.emails[0].pinned = true;
    let mut incoming = s.emails[0].clone();
    incoming.pinned = false;
    gateway.batch.lock().unwrap().push(incoming);
    let count = s.emails.len();
    let id = s.emails[0].id;
    let ctx = context(s, gateway);
    let service = MailService::new(ctx.clone());
    assert_eq!(service.sync().unwrap().added, 0);
    let s = ctx.read().unwrap();
    assert_eq!(s.emails.len(), count);
    assert!(s.email(id).unwrap().pinned);
}
#[test]
fn sync_updates_remote_flags_in_both_directions_without_erasing_local_tasks() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    store.emails[0].read = true;
    store.emails[0].starred = true;
    store.emails[0].replied = true;
    store.emails[0].pinned = true;
    let id = store.emails[0].id;
    let mut incoming = store.emails[0].clone();
    incoming.read = false; incoming.starred = false; incoming.replied = false;
    *gateway.batch.lock().unwrap() = vec![incoming];
    let ctx = context(store, gateway);
    MailService::new(ctx.clone()).sync().unwrap();
    let updated = ctx.read().unwrap().email(id).unwrap().clone();
    assert!(!updated.read && !updated.starred && !updated.replied);
    assert!(updated.pinned);
}

#[test]
fn sync_does_not_merge_distinct_uids_with_copied_rfc_headers() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    store.emails[0].message_id = "copied-header@example.com".into();
    store.emails[0].remote_content_hash = "identical-content-distinct-deliveries".into();
    store.emails[0].remote_uid_validity = Some(5);
    let original_id = store.emails[0].id;
    let mut incoming = store.emails[0].clone();
    incoming.uid = Some(11);
    *gateway.batch.lock().unwrap() = vec![incoming];
    let count = store.emails.len();
    let ctx = context(store, gateway);
    let report = MailService::new(ctx.clone()).sync().unwrap();
    assert_eq!(report.added, 1);
    assert!(report.errors.is_empty());
    let saved = ctx.read().unwrap();
    assert_eq!(saved.emails.len(), count + 1);
    assert_eq!(saved.email(original_id).unwrap().uid, Some(10));
    assert!(saved.emails.iter().any(|email| email.uid == Some(11) && email.message_id == "copied-header@example.com"));
}
#[test]
fn retention_evicts_content_but_online_reads_and_reply_quotes_still_work() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let id = store.emails[0].id;
    store.emails[0].date = TestClock.now() - 3 * 86400;
    store.emails[0].pinned = true;
    store.emails[0].body = "Synthetic old mail. ".repeat(2_000);
    store.emails[0].attachments.clear();
    store.emails.retain(|email| email.id == id);
    store.drafts.clear();
    let account_id = store.emails[0].account_id;
    let ctx = context(store, gateway.clone());
    qamille::application::storage::AccountStorageService::new(ctx.clone()).update(account_id,
        qamille::domain::storage::AccountStoragePolicy {
            limit: qamille::domain::storage::StorageLimit::Limited { max_bytes: 8_000 },
            keep_all_offline: false,
            include_spam_trash: false,
        }, true).unwrap();
    let cached = ctx.read().unwrap();
    assert!(cached.email(id).unwrap().remote_only);
    assert!(cached.email(id).unwrap().body.is_empty());
    assert!(cached.email(id).unwrap().pinned);
    let loaded = MailService::new(ctx.clone()).read(id).unwrap();
    assert_eq!(loaded.body, "Downloaded on demand");
    assert!(ctx.read().unwrap().email(id).unwrap().body.is_empty(), "Online reads must not exceed the offline limit");
    let reply = DraftService::new(ctx.clone()).create(None, Some(id), false).unwrap();
    assert!(reply.quote.contains("Downloaded on demand"));
    assert!(gateway.applied.lock().unwrap().is_empty());
    gateway.fail.store(true, Ordering::SeqCst);
    assert!(MailService::new(ctx).read(id).is_err(), "Offline content must not silently appear empty");
}
#[test]
fn incoming_sync_preserves_pending_undo_and_cancel_reveals_the_latest_server_state() {
    let (ctx, gateway, _, undo, _) = undo_fixture();
    let email = ctx.read().unwrap().emails[0].clone();
    let mut remote = email.clone(); remote.read = false;
    *gateway.batch.lock().unwrap() = vec![remote];
    let pending = undo.queue_mail(email.id, "markAsSeen".into(), None).unwrap();
    let mail = MailService::new(ctx.clone());
    mail.sync().unwrap();
    let list = mail.list(MailQuery::default()).unwrap();
    assert!(list.items.iter().find(|e| e.id == email.id).unwrap().read);
    undo.undo(pending.id).unwrap();
    let list = mail.list(MailQuery::default()).unwrap();
    assert!(!list.items.iter().find(|e| e.id == email.id).unwrap().read);
    assert!(gateway.applied.lock().unwrap().is_empty());
}
#[test]
fn scheduled_send_failure_is_disarmed_and_not_retried_every_tick() {
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), gateway.clone());
    let drafts = DraftService::new(ctx.clone());
    let mut d = drafts.create(Some(1), None, false).unwrap();
    d.to = "recipient@example.com".into();
    d.send_at = Some(TestClock.now() - 1);
    let d = drafts.save(d).unwrap();
    let scheduler = SchedulerService::new(ctx.clone(), drafts);
    assert_eq!(scheduler.tick().unwrap().len(), 1);
    assert!(scheduler.tick().unwrap().is_empty());
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        ctx.read()
            .unwrap()
            .drafts
            .iter()
            .find(|x| x.id == d.id)
            .unwrap()
            .send_at,
        None
    );
}
#[test]
fn inbox_transfer_is_paginated_and_does_not_include_message_bodies() {
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let page = MailService::new(ctx)
        .list(MailQuery {
            limit: 2,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_offset, Some(2));
    let json = serde_json::to_value(page).unwrap();
    assert!(json["items"][0].get("body").is_none());
}

#[test]
fn conversation_list_groups_links_before_pagination_and_keeps_accounts_separate() {
    let mut store = fixture();
    let mut root = store.emails[0].clone();
    root.id = 90001;
    root.folder = "Inbox".into();
    root.done = false;
    root.snoozed_until = None;
    root.returned_at = None;
    root.read = false;
    root.message_id = "<root@test>".into();
    root.thread_id = String::new();
    root.date = TestClock.now() - 300;
    let mut reply = root.clone();
    reply.id = 90002;
    reply.message_id = "reply@test".into();
    reply.thread_id = "root@test".into();
    reply.date += 100;
    reply.read = true;
    let mut latest = reply.clone();
    latest.id = 90003;
    latest.thread_id = "<reply@test>".into();
    latest.message_id = "latest@test".into();
    latest.date += 100;
    let mut foreign = latest.clone();
    foreign.id = 90004;
    foreign.account_id = store.accounts.iter().find(|a| a.id != root.account_id).unwrap().id;
    foreign.date -= 400;
    let mut unlinked = root.clone();
    unlinked.id = 90005;
    unlinked.message_id.clear();
    unlinked.date -= 500;
    let mut other_unlinked = unlinked.clone();
    other_unlinked.id = 90006;
    other_unlinked.date -= 100;
    store.emails = vec![latest.clone(), foreign, reply, root.clone(), unlinked, other_unlinked];
    let service = MailService::new(context(store, Arc::new(TestMail::default())));
    let page = service.list(serde_json::from_value(serde_json::json!({
        "folder": "Inbox", "threads": true, "limit": 1
    })).unwrap()).unwrap();
    assert_eq!(page.total, 4, "one linked conversation, another account, and two unlinked messages");
    assert_eq!(page.items[0].id, latest.id);
    assert!(!page.items[0].read, "an older unread member keeps the conversation unread");
    assert_eq!(page.next_offset, Some(1));
    let json = serde_json::to_value(page).unwrap();
    let mut ids: Vec<u64> = serde_json::from_value(json["items"][0]["message_ids"].clone()).unwrap();
    ids.sort();
    assert_eq!(ids, vec![90001, 90002, 90003]);
    let singles = service.list(MailQuery { limit: 100, ..Default::default() }).unwrap();
    assert_eq!(singles.total, 6);
    let filtered = service.list(serde_json::from_value(serde_json::json!({
        "folder": "Inbox", "threads": true, "account_id": root.account_id, "offset": 1
    })).unwrap()).unwrap();
    assert_eq!(filtered.total, 3);
    assert_eq!(filtered.items.len(), 2);
}

#[test]
fn conversation_reader_includes_synced_replies_across_folders_without_network_or_read_side_effects() {
    let mut store = fixture();
    let mut anchor = store.emails[0].clone();
    anchor.message_id = "conversation-root@test".into();
    anchor.thread_id.clear();
    anchor.folder = "Inbox".into();
    anchor.done = false;
    anchor.snoozed_until = None;
    anchor.read = false;
    let mut reply = anchor.clone();
    reply.id = 99001;
    reply.thread_id = "<conversation-root@test>".into();
    reply.message_id = "reply@test".into();
    reply.folder = "Sent".into();
    reply.date += 60;
    store.emails = vec![reply.clone(), anchor.clone()];
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(store, gateway);
    let service = MailService::new(ctx.clone());
    let emails = service.conversation(anchor.id).unwrap();
    assert_eq!(emails.iter().map(|email| email.id).collect::<Vec<_>>(), vec![anchor.id, reply.id]);
    assert!(!ctx.read().unwrap().email(anchor.id).unwrap().read);
    let page = service.list(serde_json::from_value(serde_json::json!({"folder": "Inbox", "threads": true})).unwrap()).unwrap();
    let json = serde_json::to_value(page).unwrap();
    assert_eq!(json["items"][0]["message_ids"], serde_json::json!([anchor.id]));
    assert!(service.conversation(u64::MAX).is_err());
}
#[test]
fn workspace_snapshot_never_serializes_secrets() {
    let mut store = fixture();
    store.accounts[0].password = "never-share-password".into();
    store.settings.ai_key = "never-share-key".into();
    store.settings.google_client_secret = "never-share-secret".into();
    let context = context(store, Arc::new(TestMail::default()));
    let json = serde_json::to_string(&WorkspaceService::new(context).snapshot().unwrap()).unwrap();
    assert!(!json.contains("never-share"));
    assert!(!json.contains("\"password\""));
    assert!(!json.contains("\"ai_key\""));
}

struct LegacyGoogleIdentity {
    calls: Mutex<Vec<(String, String)>>,
    hints: Mutex<Vec<String>>,
    cancels: Mutex<u32>,
}
impl GoogleIdentity for LegacyGoogleIdentity {
    fn sign_in(&self, client_id: &str, client_secret: &str, login_hint: &str) -> AppResult<(String, String)> {
        self.calls.lock().unwrap().push((client_id.into(), client_secret.into()));
        self.hints.lock().unwrap().push(login_hint.into());
        Ok(("new-user@example.com".into(), "google-refresh-token".into()))
    }
    fn cancel_sign_in(&self) {
        *self.cancels.lock().unwrap() += 1;
    }
}

struct AppGoogleIdentity(LegacyGoogleIdentity);
impl GoogleIdentity for AppGoogleIdentity {
    fn client_credentials(&self, client_id: &str, client_secret: &str) -> AppResult<(String, String)> {
        assert!(client_id.is_empty());
        assert!(client_secret.is_empty());
        Ok(("qamille-app-client".into(), "qamille-app-secret".into()))
    }
    fn sign_in(&self, client_id: &str, client_secret: &str, login_hint: &str) -> AppResult<(String, String)> {
        self.0.sign_in(client_id, client_secret, login_hint)
    }
    fn cancel_sign_in(&self) {
        self.0.cancel_sign_in()
    }
}

#[test]
fn google_connect_uses_app_client_without_user_configuration_and_keeps_tokens_private() {
    use qamille::application::accounts::AccountService;
    let mut store = fixture();
    store.settings.google_client_id.clear();
    let ctx = context(store, Arc::new(TestMail::default()));
    let identity = Arc::new(AppGoogleIdentity(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) }));
    AccountService::new(ctx.clone(), identity.clone()).connect_google("").unwrap();
    let saved = ctx.read().unwrap();
    let account = saved.accounts.iter().find(|a| a.email == "new-user@example.com").unwrap();
    assert_eq!(account.oauth_client_id, "qamille-app-client");
    assert!(account.password.is_empty());
    assert!(saved.settings.google_client_id.is_empty());
    assert_eq!(ctx.secrets.get(&account_key(&account.email)).unwrap(), "google-refresh-token");
    assert_eq!(*identity.0.calls.lock().unwrap(), vec![("qamille-app-client".into(), "qamille-app-secret".into())]);
    let json = serde_json::to_string(&WorkspaceService::new(ctx).snapshot().unwrap()).unwrap();
    assert!(!json.contains("google-refresh-token"));
    assert!(!json.contains("qamille-app-secret"));
}

#[test]
fn google_connect_preserves_custom_client_configuration() {
    use qamille::application::accounts::AccountService;
    let mut store = fixture();
    store.settings.google_client_id = " legacy-client ".into();
    let ctx = context(store, Arc::new(TestMail::default()));
    ctx.secrets.set(GOOGLE_SECRET, "legacy-secret").unwrap();
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    AccountService::new(ctx.clone(), identity.clone()).connect_google("").unwrap();
    assert_eq!(*identity.calls.lock().unwrap(), vec![("legacy-client".into(), "legacy-secret".into())]);
    assert_eq!(ctx.read().unwrap().accounts.iter().find(|a| a.email == "new-user@example.com").unwrap().oauth_client_id, "legacy-client");
}

#[test]
fn google_connect_tells_the_browser_which_account_to_use() {
    use qamille::application::accounts::AccountService;
    let mut store = fixture();
    store.settings.google_client_id = "legacy-client".into();
    let ctx = context(store, Arc::new(TestMail::default()));
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    AccountService::new(ctx, identity.clone()).connect_google("a.caprino@tri-tech.it").unwrap();
    assert_eq!(*identity.hints.lock().unwrap(), vec!["a.caprino@tri-tech.it".to_string()]);
}

#[test]
fn google_reconnect_keeps_the_name_the_person_chose() {
    use qamille::application::accounts::AccountService;
    let mut store = fixture();
    store.settings.google_client_id = "legacy-client".into();
    let ctx = context(store, Arc::new(TestMail::default()));
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    let service = AccountService::new(ctx.clone(), identity);
    service.connect_google("").unwrap();
    assert_eq!(ctx.read().unwrap().accounts.iter().find(|a| a.email == "new-user@example.com").unwrap().name, "new-user@example.com");
    ctx.update(|store| {
        store.accounts.iter_mut().find(|a| a.email == "new-user@example.com").unwrap().name = "Lavoro".into();
        Ok(())
    }).unwrap();
    service.connect_google("").unwrap();
    let saved = ctx.read().unwrap();
    let matching: Vec<_> = saved.accounts.iter().filter(|a| a.email == "new-user@example.com").collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].name, "Lavoro");
}

#[test]
fn password_reconnect_takes_the_name_typed_in_the_form() {
    use qamille::application::accounts::AccountService;
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    let service = AccountService::new(ctx.clone(), identity);
    service.connect_password("Primo", "pw-user@example.com", "app-password").unwrap();
    service.connect_password("Secondo", "pw-user@example.com", "app-password").unwrap();
    let saved = ctx.read().unwrap();
    let matching: Vec<_> = saved.accounts.iter().filter(|a| a.email == "pw-user@example.com").collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].name, "Secondo");
}

#[test]
fn cancelling_google_sign_in_reaches_the_identity_provider() {
    use qamille::application::accounts::AccountService;
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    AccountService::new(ctx, identity.clone()).cancel_google();
    assert_eq!(*identity.cancels.lock().unwrap(), 1);
}

#[test]
fn google_connect_without_any_client_reports_app_configuration_without_starting_login() {
    use qamille::application::accounts::AccountService;
    let mut store = fixture();
    store.settings.google_client_id.clear();
    let ctx = context(store, Arc::new(TestMail::default()));
    let before = ctx.read().unwrap().accounts.len();
    let identity = Arc::new(LegacyGoogleIdentity { calls: Mutex::new(Vec::new()), hints: Mutex::new(Vec::new()), cancels: Mutex::new(0) });
    let error = AccountService::new(ctx.clone(), identity.clone()).connect_google("").unwrap_err();
    assert_eq!(error.code, "google_not_configured");
    assert_eq!(error.message, GOOGLE_NOT_CONFIGURED);
    assert!(identity.calls.lock().unwrap().is_empty());
    assert_eq!(ctx.read().unwrap().accounts.len(), before);
    assert!(ctx.secrets.get(&account_key("new-user@example.com")).unwrap().is_empty());
}
#[test]
fn memory_repository_rolls_back_a_failed_transaction() {
    let repository = MemoryRepository::new(fixture());
    let before = repository.read().unwrap().next_id;
    assert!(
        repository
            .transaction(&mut |s| {
                s.new_id();
                Err(AppError::validation("stop"))
            })
            .is_err()
    );
    assert_eq!(repository.read().unwrap().next_id, before);
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
fn temp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "qamille-repository-{}-{}.json",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::SeqCst)
    ))
}
#[test]
fn corrupted_repository_preserves_the_original_bytes() {
    let path = temp_path();
    std::fs::write(&path, b"not valid json").unwrap();
    let repo = JsonStoreRepository::new(&path);
    assert!(repo.read().is_err());
    assert!(
        repo.transaction(&mut |s| {
            s.emails.clear();
            Ok(())
        })
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"not valid json");
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(path.with_extension("lock")).unwrap();
}
#[test]
fn concurrent_repository_transactions_do_not_lose_updates() {
    let path = temp_path();
    let repository = Arc::new(JsonStoreRepository::new(&path));
    let before = repository.read().unwrap().next_id;
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let repository = repository.clone();
            std::thread::spawn(move || {
                for _ in 0..10 {
                    repository
                        .transaction(&mut |s| {
                            s.new_id();
                            Ok(())
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(repository.read().unwrap().next_id, before + 40);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(path.with_extension("lock")).unwrap();
}

#[test]
fn sending_preserves_an_external_edit_made_during_delivery() {
    let gateway = Arc::new(TestMail::default());
    let ctx = context(fixture(), gateway.clone());
    let service = DraftService::new(ctx.clone());
    let mut draft = service.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.body = "Version being sent".into();
    let draft = service.save(draft).unwrap();
    let editing = ctx.clone();
    let id = draft.id;
    *gateway.during_send.lock().unwrap() = Some(Box::new(move || {
        editing
            .update(|store| {
                let latest = store.drafts.iter_mut().find(|d| d.id == id).unwrap();
                latest.body = "A newer external edit".into();
                latest.updated += 1;
                Ok(())
            })
            .unwrap();
    }));
    service.send(draft.id).unwrap();
    let store = ctx.read().unwrap();
    assert_eq!(
        store.drafts.iter().find(|d| d.id == id).unwrap().body,
        "A newer external edit"
    );
    assert!(
        store
            .emails
            .last()
            .unwrap()
            .body
            .contains("Version being sent")
    );
}

#[test]
fn local_task_actions_work_without_network_credentials() {
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let store = ctx.read().unwrap();
    let id = store.emails[0].id;
    ctx.secrets
        .set(&account_key(&store.accounts[0].email), "")
        .unwrap();
    MailService::new(ctx.clone())
        .act(id, "setAside", None)
        .unwrap();
    assert!(ctx.read().unwrap().email(id).unwrap().set_aside);
}

#[test]
fn failed_scheduled_send_is_not_retried_after_scheduler_restart() {
    let gateway = Arc::new(TestMail::default());
    gateway.fail.store(true, Ordering::SeqCst);
    let ctx = context(fixture(), gateway.clone());
    let drafts = DraftService::new(ctx.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.send_at = Some(TestClock.now() - 1);
    drafts.save(draft).unwrap();
    SchedulerService::new(ctx.clone(), drafts.clone())
        .tick()
        .unwrap();
    SchedulerService::new(ctx, drafts).tick().unwrap();
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 1);
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
/// The undo fixture with the worker registry wired to the service, the way the
/// desktop runs it (Task 3.2): `commit_due` claims due confirmations and the
/// account workers execute them, then settle the results back through the same
/// service instance (`CommitOutcome`).
fn undo_fixture() -> (
    Arc<ServiceContext>,
    Arc<TestMail>,
    Arc<MovingClock>,
    Arc<UndoService>,
    WorkerRegistry,
) {
    let clock = Arc::new(MovingClock(AtomicU64::new(2_000_000_000_000)));
    let gateway = Arc::new(TestMail::default());
    let context = context_with_clock(fixture(), gateway.clone(), clock.clone());
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
    (context, gateway, clock, undo, workers)
}

#[test]
fn done_can_be_undone_without_touching_the_original_account() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let original = context.read().unwrap().emails[0].clone();
    let pending = undo
        .queue_mail(original.id, "markAsDone".into(), None)
        .unwrap();
    assert_eq!(pending.expires_at_ms - pending.created_at_ms, 5000);
    let view = MailService::new(context.clone());
    assert!(view.read(original.id).unwrap().done);
    assert_eq!(view.read(original.id).unwrap().folder, "Archive");
    assert!(!context.read().unwrap().email(original.id).unwrap().done);
    clock.advance(4999);
    assert!(!undo.commit_due(&workers).unwrap());
    undo.undo(pending.id).unwrap();
    clock.advance(1);
    assert!(!undo.commit_due(&workers).unwrap(), "a cancelled action never commits");
    let restored = view.read(original.id).unwrap();
    assert_eq!(restored.folder, original.folder);
    assert_eq!(restored.read, original.read);
    assert_eq!(restored.pinned, original.pinned);
    assert!(gateway.applied.lock().unwrap().is_empty());
}

#[test]
fn expiry_archives_done_on_the_correct_account_once() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let email = context.read().unwrap().emails[0].clone();
    let pending = undo
        .queue_mail(email.id, "markAsDone".into(), None)
        .unwrap();
    clock.advance(5000);
    assert!(undo.undo(pending.id).is_err());
    assert!(undo.commit_due(&workers).unwrap());
    settled(&context);
    assert!(!undo.commit_due(&workers).unwrap());
    let calls = gateway.applied.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, email.account_id);
    assert_eq!(calls[0].1, email.uid.unwrap());
    assert!(matches!(&calls[0].2, RemoteAction::Archive));
    let store = context.read().unwrap();
    assert!(store.undo_actions.is_empty());
    assert_eq!(store.email(email.id).unwrap().folder, "Archive");
    assert!(store.email(email.id).unwrap().done);
}

#[test]
fn archiving_from_trash_or_spam_moves_the_message_back_to_all_mail() {
    for folder in ["Trash", "Spam"] {
        let (context, gateway, clock, undo, workers) = undo_fixture();
        let id = context.read().unwrap().emails[0].id;
        context.update(|store| { store.email_mut(id).unwrap().folder = folder.into(); Ok(()) }).unwrap();
        undo.queue_mail(id, "archive".into(), None).unwrap();
        clock.advance(5000);
        assert!(undo.commit_due(&workers).unwrap());
        settled(&context);
        let calls = gateway.applied.lock().unwrap();
        assert_eq!(calls.len(), 1, "{folder}");
        assert!(matches!(&calls[0].2, RemoteAction::Move(target) if target == "[Gmail]/All Mail"), "{folder}");
    }
}

#[test]
fn undo_one_action_preserves_other_pending_actions_and_external_edits() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let email = context.read().unwrap().emails[0].clone();
    let pin = undo.queue_mail(email.id, "pin".into(), None).unwrap();
    undo.queue_mail(email.id, "markAsUnseen".into(), None)
        .unwrap();
    context
        .update(|s| {
            s.email_mut(email.id).unwrap().body = "External update".into();
            Ok(())
        })
        .unwrap();
    undo.undo(pin.id).unwrap();
    let view = MailService::new(context.clone()).read(email.id).unwrap();
    assert_eq!(view.pinned, email.pinned);
    assert!(!view.read);
    assert_eq!(view.body, "External update");
    clock.advance(5000);
    assert!(undo.commit_due(&workers).unwrap());
    settled(&context);
    assert!(matches!(
        gateway.applied.lock().unwrap()[0].2,
        RemoteAction::Seen(false)
    ));
}

#[test]
fn consecutive_moves_wait_for_all_timers_then_move_to_the_final_folder() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let id = context.read().unwrap().emails[0].id;
    undo.queue_mail(id, "markAsDone".into(), None).unwrap();
    clock.advance(1000);
    undo.queue_mail(id, "moveToTrash".into(), None).unwrap();
    clock.advance(4000);
    assert!(!undo.commit_due(&workers).unwrap());
    assert!(gateway.applied.lock().unwrap().is_empty());
    clock.advance(1000);
    assert!(undo.commit_due(&workers).unwrap());
    settled(&context);
    let calls = gateway.applied.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(matches!(&calls[0].2, RemoteAction::Move(folder) if folder == "[Gmail]/Trash"));
    assert_eq!(context.read().unwrap().email(id).unwrap().folder, "Trash");
}

#[test]
fn failure_restores_the_visible_mail_and_is_not_retried() {
    use qamille::domain::undo::UndoStatus;
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let id = context.read().unwrap().emails[0].id;
    let pending = undo.queue_mail(id, "archive".into(), None).unwrap();
    gateway.fail.store(true, Ordering::SeqCst);
    clock.advance(5000);
    assert!(undo.commit_due(&workers).unwrap());
    settled(&context);
    assert_eq!(
        MailService::new(context.clone()).read(id).unwrap().folder,
        "Inbox"
    );
    assert_eq!(
        context.read().unwrap().undo_actions[0].status,
        UndoStatus::Failed
    );
    assert!(!undo.commit_due(&workers).unwrap());
    undo.dismiss(pending.id).unwrap();
    assert!(context.read().unwrap().undo_actions.is_empty());
}

#[test]
fn undo_send_keeps_the_full_draft_and_never_contacts_smtp() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let drafts = DraftService::new(context.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.body = "Keep this message".into();
    let draft = drafts.save(draft).unwrap();
    let pending = undo.queue_draft(draft.id, "send", None).unwrap();
    assert!(
        WorkspaceService::new(context.clone())
            .snapshot()
            .unwrap()
            .drafts
            .iter()
            .all(|d| d.id != draft.id)
    );
    assert!(drafts.save(draft.clone()).is_err());
    clock.advance(4999);
    undo.undo(pending.id).unwrap();
    clock.advance(1);
    assert!(!undo.commit_due(&workers).unwrap(), "a cancelled send never leaves");
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        context
            .read()
            .unwrap()
            .drafts
            .iter()
            .find(|d| d.id == draft.id)
            .unwrap(),
        &draft
    );
}

#[test]
fn queued_send_survives_reload_and_sends_only_once_after_expiry() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let drafts = DraftService::new(context.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    let draft = drafts.save(draft).unwrap();
    undo.queue_draft(draft.id, "send", None).unwrap();
    let serialized = serde_json::to_string(&context.read().unwrap()).unwrap();
    let reloaded = context_with_clock(
        serde_json::from_str(&serialized).unwrap(),
        gateway.clone(),
        clock.clone(),
    );
    let resumed_mail = MailService::new(reloaded.clone());
    let resumed_drafts = DraftService::new(reloaded.clone());
    let resumed = Arc::new(UndoService::new(
        reloaded.clone(),
        resumed_mail.clone(),
        resumed_drafts.clone(),
    ));
    let resumed_workers = WorkerRegistry::new(
        resumed_mail,
        resumed_drafts,
        reloaded.clone(),
        Arc::new(NullSink),
        resumed.clone() as Arc<dyn CommitOutcome>,
    );
    resumed.recover_interrupted().unwrap();
    assert!(!resumed.commit_due(&resumed_workers).unwrap());
    clock.advance(5000);
    assert!(resumed.commit_due(&resumed_workers).unwrap());
    settled(&reloaded);
    assert!(!resumed.commit_due(&resumed_workers).unwrap(), "the send is never replayed");
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 1);
    assert!(
        reloaded
            .read()
            .unwrap()
            .drafts
            .iter()
            .all(|d| d.id != draft.id)
    );
}

#[test]
fn interrupted_send_is_not_replayed_on_restart() {
    use qamille::domain::undo::UndoStatus;
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let drafts = DraftService::new(context.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    let draft = drafts.save(draft).unwrap();
    undo.queue_draft(draft.id, "send", None).unwrap();
    context
        .update(|s| {
            s.undo_actions[0].status = UndoStatus::Committing;
            Ok(())
        })
        .unwrap();
    undo.recover_interrupted().unwrap();
    clock.advance(6000);
    assert!(!undo.commit_due(&workers).unwrap(), "a recovered interruption is never replayed");
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        context.read().unwrap().undo_actions[0].status,
        UndoStatus::Failed
    );
}

#[test]
fn configurable_timeout_and_legacy_store_defaults() {
    let (context, _, clock, undo, _) = undo_fixture();
    context
        .update(|s| {
            s.settings.undo_seconds = 15;
            Ok(())
        })
        .unwrap();
    let id = context.read().unwrap().emails[0].id;
    let pending = undo.queue_mail(id, "archive".into(), None).unwrap();
    clock.advance(14999);
    undo.undo(pending.id).unwrap();
    let mut json = serde_json::to_value(context.read().unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("undo_actions");
    json["settings"]
        .as_object_mut()
        .unwrap()
        .remove("undo_seconds");
    let old: Store = serde_json::from_value(json).unwrap();
    assert_eq!(old.settings.undo_seconds, 5);
    assert!(old.undo_actions.is_empty());
}

#[test]
fn meeting_settings_default_for_old_files() {
    let mut json = serde_json::to_value(fixture()).unwrap();
    let settings = json["settings"].as_object_mut().unwrap();
    for key in [
        "meeting_language",
        "meeting_engine",
        "meeting_model",
        "meeting_remote_model",
        "meeting_microphone",
        "meeting_system_audio",
    ] {
        settings.remove(key);
    }
    let old: Store = serde_json::from_value(json).unwrap();
    assert_eq!(old.settings.meeting_language, "it");
    assert_eq!(old.settings.meeting_engine, "compute");
    assert_eq!(old.settings.meeting_model, "base");
    assert_eq!(old.settings.meeting_remote_model, "");
    assert_eq!(old.settings.meeting_microphone, "");
    assert_eq!(old.settings.meeting_system_audio, "");
    let options = old.settings.meeting_options();
    assert_eq!(options.provider, "compute");
    assert_eq!(options.model, "base");
    assert_eq!(options.language, "it");
}

#[test]
fn meeting_options_follow_the_chosen_engine() {
    let mut settings = Settings::default();
    settings.meeting_model = "small".into();
    settings.meeting_remote_model = "whisper-1".into();
    settings.meeting_language = "en".into();
    let local = settings.meeting_options();
    assert_eq!((local.provider.as_str(), local.model.as_str(), local.language.as_str()), ("compute", "small", "en"));
    settings.meeting_engine = "external".into();
    let remote = settings.meeting_options();
    assert_eq!((remote.provider.as_str(), remote.model.as_str(), remote.language.as_str()), ("external", "whisper-1", "en"));
    // A value set before the compute server (still in memory) means the same engine.
    settings.meeting_engine = "remote".into();
    assert_eq!(settings.meeting_options().provider, "external");
}

#[test]
fn meeting_engines_saved_before_the_compute_server_read_as_their_new_names() {
    for (old, new) in [("local", "compute"), ("remote", "external"), ("compute", "compute"), ("external", "external")] {
        let mut json = serde_json::to_value(fixture()).unwrap();
        json["settings"]["meeting_engine"] = serde_json::json!(old);
        let store: Store = serde_json::from_value(json).unwrap();
        assert_eq!(store.settings.meeting_engine, new, "{old}");
        assert_eq!(serde_json::to_value(&store.settings).unwrap()["meeting_engine"], new, "written with the new name");
        // The options of a job saved then read the same way.
        let options: qamille::domain::meeting::TranscribeOptions =
            serde_json::from_value(serde_json::json!({"provider": old, "model": "base", "language": "it"})).unwrap();
        assert_eq!(options.provider, new);
    }
}

#[test]
fn save_settings_rejects_bad_meeting_values() {
    use qamille::application::workspace::SettingsUpdate;
    let ctx = context(fixture(), Arc::new(TestMail::default()));
    let service = WorkspaceService::new(ctx.clone());
    let attempt = |change: &dyn Fn(&mut Settings)| {
        let mut settings = ctx.read().unwrap().settings.clone();
        change(&mut settings);
        service.save_settings(SettingsUpdate { settings, ai_key: None, google_client_secret: None })
    };
    let engine = attempt(&|s| s.meeting_engine = "cloud".into()).unwrap_err();
    assert_eq!(engine.code, "validation");
    let language = attempt(&|s| s.meeting_language = "it_IT!".into()).unwrap_err();
    assert_eq!(language.code, "validation");
    assert_eq!(attempt(&|s| s.meeting_language = String::new()).unwrap_err().code, "validation");
    let model = attempt(&|s| s.meeting_model = "huge".into()).unwrap_err();
    assert_eq!(model.code, "validation");
    // The remote model name is sent to the endpoint: the engine refuses what is refused here.
    for bad in ["whisper 1", "whisper;rm", "è-whisper", "a\\b", "a\nb", &"m".repeat(201)] {
        let refused = attempt(&|s| s.meeting_remote_model = bad.into()).unwrap_err();
        assert_eq!(refused.code, "validation", "{bad:?}");
    }
    assert_eq!(ctx.read().unwrap().settings.meeting_engine, "compute", "a refused save changes nothing");
    assert_eq!(ctx.read().unwrap().settings.meeting_remote_model, "", "a refused name is not kept");
    for good in ["", "whisper-1", "org/whisper-large_v3.1:turbo", &"m".repeat(200)] {
        attempt(&|s| s.meeting_remote_model = good.into()).unwrap();
        assert_eq!(ctx.read().unwrap().settings.meeting_remote_model, good);
    }
    // An older window may still send the former name: it is saved as the new one.
    attempt(&|s| {
        s.meeting_engine = "remote".into();
        s.meeting_remote_model = "any-free-model-name".into();
        s.meeting_language = "pt-BR".into();
        s.meeting_microphone = "{0.0.1.00000000}.{abc}".into();
    })
    .unwrap();
    let saved = ctx.read().unwrap().settings;
    assert_eq!(saved.meeting_engine, "external");
    assert_eq!(saved.meeting_remote_model, "any-free-model-name");
    assert_eq!(saved.meeting_options().model, "any-free-model-name");
}

#[test]
fn discard_and_schedule_are_undoable_and_pending_drafts_do_not_send_early() {
    let (context, gateway, clock, undo, _workers) = undo_fixture();
    let drafts = DraftService::new(context.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.send_at = Some(clock.now() - 1);
    let draft = drafts.save(draft).unwrap();
    let discard = undo.queue_draft(draft.id, "discard", None).unwrap();
    SchedulerService::new(context.clone(), drafts.clone())
        .tick()
        .unwrap();
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 0);
    undo.undo(discard.id).unwrap();
    let schedule = undo
        .queue_draft(draft.id, "schedule", Some(clock.now() + 3600))
        .unwrap();
    undo.undo(schedule.id).unwrap();
    assert_eq!(
        context
            .read()
            .unwrap()
            .drafts
            .iter()
            .find(|d| d.id == draft.id)
            .unwrap()
            .send_at,
        draft.send_at
    );
}

// --- Deadline-driven commits dispatched per account (Task 3.2) --------------

use qamille::application::account_worker::{CommitOutcome, WorkerRegistry};
use qamille::domain::undo::{UndoOperation, UndoStatus};
use std::time::{Duration, Instant};

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

/// `commit_due` hands claimed rows to worker threads: wait until every claim
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

#[test]
fn commit_waits_for_the_deadline_without_polling() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let email = context.read().unwrap().emails[0].clone();
    let pending = undo.queue_mail(email.id, "archive".into(), None).unwrap();
    // The schedule is the action itself: the deadline thread sleeps until
    // this instant (INV-005), no interval polling of the store.
    assert_eq!(undo.next_deadline().unwrap(), Some(pending.expires_at_ms));
    clock.advance(4999);
    assert!(!undo.commit_due(&workers).unwrap(), "nothing is due before the deadline");
    assert!(gateway.applied.lock().unwrap().is_empty(), "no job ran before the deadline");
    assert_eq!(context.read().unwrap().undo_actions[0].status, UndoStatus::Pending);
    clock.advance(1);
    assert!(undo.commit_due(&workers).unwrap(), "the deadline dispatches the confirmation");
    settled(&context);
    let calls = gateway.applied.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0].2, RemoteAction::Archive));
    assert!(context.read().unwrap().undo_actions.is_empty());
    assert_eq!(undo.next_deadline().unwrap(), None, "an empty queue has no deadline");
}

#[test]
fn slow_send_does_not_delay_mail_commit() {
    let (context, gateway, clock, undo, workers) = undo_fixture();
    let email = context.read().unwrap().emails[0].clone();
    let drafts = DraftService::new(context.clone());
    let mut draft = drafts.create(Some(1), None, false).unwrap();
    draft.to = "recipient@example.com".into();
    draft.body = "Held up at the provider".into();
    let draft = drafts.save(draft).unwrap();
    // The drafts worker blocks inside the SMTP call until the test releases it.
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let released = barrier.clone();
    *gateway.during_send.lock().unwrap() = Some(Box::new(move || {
        released.wait();
    }));
    undo.queue_draft(draft.id, "send", None).unwrap();
    undo.queue_mail(email.id, "archive".into(), None).unwrap();
    clock.advance(5000);
    assert!(undo.commit_due(&workers).unwrap());
    // The mail confirmation commits on account 1's worker while the send is
    // still blocked on the drafts worker (decision 4: a slow confirmation
    // never delays another one).
    assert!(
        wait_until(|| !gateway.applied.lock().unwrap().is_empty()
            && context
                .read()
                .unwrap()
                .undo_actions
                .iter()
                .all(|a| matches!(a.operation, UndoOperation::SendDraft { .. }))),
        "the blocked send delayed the mail confirmation"
    );
    barrier.wait(); // release the slow SMTP call
    settled(&context);
    assert_eq!(gateway.sends.load(Ordering::SeqCst), 1);
    assert!(context.read().unwrap().undo_actions.is_empty());
}

// --- Attachment previews of messages whose body is not downloaded -----------

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0xFF, 0xD9];
const PDF: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n1 0 obj<<>>endobj\n%%EOF\n";
const FIRST_SCAN: &[u8] = &[1, 2, 3, 0xFF];
const SECOND_SCAN: &[u8] = &[9, 8, 7, 0xFE];

fn attachment_part(boundary: &str, name: &str, mime: &str, bytes: &[u8]) -> String {
    format!(
        "--{boundary}\r\nContent-Type: {mime}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
        base64(bytes)
    )
}

/// image001.jpg, report.pdf, two different files both named scan.bin, and a
/// forwarded message that carries inner.pdf: the original of an online message.
fn online_original() -> Vec<u8> {
    let inner = format!(
        "From: b@example.com\r\nSubject: Inner\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=in\r\n\r\n--in\r\nContent-Type: text/plain\r\n\r\nhi\r\n{}--in--\r\n",
        attachment_part("in", "inner.pdf", "application/pdf", PDF)
    );
    format!(
        "From: a@example.com\r\nSubject: Documents\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n{}{}{}{}--b\r\nContent-Type: message/rfc822; name=\"forward.eml\"\r\nContent-Disposition: attachment; filename=\"forward.eml\"\r\n\r\n{inner}\r\n--b--\r\n",
        attachment_part("b", "image001.jpg", "image/jpeg", JPEG),
        attachment_part("b", "report.pdf", "application/pdf", PDF),
        attachment_part("b", "scan.bin", "application/octet-stream", FIRST_SCAN),
        attachment_part("b", "scan.bin", "application/octet-stream", SECOND_SCAN),
    )
    .into_bytes()
}

fn listed_attachment(id: u64, name: &str, mime: &str, size: u64) -> Attachment {
    Attachment { id, name: name.into(), mime: mime.into(), size, path: None, inline_text: None }
}

/// The message list of `online_original`, in the order of its parts.
fn online_attachments() -> Vec<Attachment> {
    vec![
        listed_attachment(501, "image001.jpg", "image/jpeg", JPEG.len() as u64),
        listed_attachment(502, "report.pdf", "application/pdf", PDF.len() as u64),
        listed_attachment(503, "scan.bin", "application/octet-stream", FIRST_SCAN.len() as u64),
        listed_attachment(504, "scan.bin", "application/octet-stream", SECOND_SCAN.len() as u64),
        listed_attachment(505, "forward.eml", "message/rfc822", 400),
    ]
}

/// A provider that serves the original message and, like the real one,
/// extracts text only from the attachments it can read as text.
struct OnlineMail {
    original: Vec<u8>,
    original_downloads: AtomicU64,
}
impl OnlineMail {
    fn new() -> Arc<Self> {
        Self::with_original(online_original())
    }
    fn with_original(original: Vec<u8>) -> Arc<Self> {
        Arc::new(Self { original, original_downloads: AtomicU64::new(0) })
    }
}
impl MailGateway for OnlineMail {
    fn load_body(&self, _: &Account, email: &Email) -> AppResult<Email> {
        let mut loaded = email.clone();
        loaded.body = "Downloaded on demand".into();
        for attachment in &mut loaded.attachments {
            attachment.inline_text = (attachment.mime != "image/jpeg").then(|| "extracted text".into());
        }
        Ok(loaded)
    }
    fn load_body_preview(&self, account: &Account, email: &Email) -> AppResult<Email> {
        let mut loaded = self.load_body(account, email)?;
        for attachment in &mut loaded.attachments {
            attachment.inline_text = None;
        }
        Ok(loaded)
    }
    fn originals(&self, _: &Account, emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        self.original_downloads.fetch_add(emails.len() as u64, Ordering::SeqCst);
        Ok(emails.iter().map(|_| self.original.clone()).collect())
    }
    /// Like the real adapter: the ids of `email.attachments` are given in the
    /// order of the original's parts, so the position of the id is the part.
    fn attachment_bytes(&self, account: &Account, email: &Email, attachment_id: u64) -> AppResult<Vec<u8>> {
        let index = self
            .load_body(account, email)?
            .attachments
            .iter()
            .position(|a| a.id == attachment_id)
            .ok_or_else(|| AppError::missing("Attachment"))?;
        let raw = self.originals(account, std::slice::from_ref(email))?.remove(0);
        qamille::domain::attached_message::nested(&raw, &[index as u32])
            .map(|(_, bytes)| bytes)
            .map_err(AppError::validation)
    }
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
}

/// A store whose first message has `attachments` listed and, when
/// `remote_only`, no downloaded content.
fn online_only_store(remote_only: bool, attachments: Vec<Attachment>) -> (Store, u64) {
    let mut store = fixture();
    let email = &mut store.emails[0];
    email.remote_only = remote_only;
    email.body.clear();
    email.attachments = attachments;
    let id = email.id;
    (store, id)
}

/// The real attachment gateway, so that what is read from disk is read as in the app.
fn online_context(store: Store, mail: Arc<OnlineMail>) -> Arc<ServiceContext> {
    let secrets = Arc::new(TestSecrets::default());
    for a in &store.accounts {
        secrets.set(&account_key(&a.email), "test-only").unwrap();
    }
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail,
        secrets,
        clock: Arc::new(TestClock),
        attachments: Arc::new(qamille::infrastructure::gateways::LocalAttachments),
    })
}

#[test]
fn an_online_message_attachment_opens_with_its_original_bytes() {
    let mail = OnlineMail::new();
    let (store, id) = online_only_store(true, online_attachments());
    let ctx = online_context(store, mail.clone());
    let service = MailService::new(ctx.clone());

    // An image has no extracted text: before the fix this was "Attachment unavailable".
    let image = service.attachment_document(id, 501).unwrap();
    assert_eq!(image.name, "image001.jpg");
    assert_eq!(image.bytes, JPEG);
    // A PDF has extracted text, which is not the file the preview engine needs.
    let pdf = service.attachment_document(id, 502).unwrap();
    assert_eq!(pdf.name, "report.pdf");
    assert_eq!(pdf.bytes, PDF);

    // Online reads never enlarge the offline cache.
    let stored = ctx.read().unwrap();
    let stored = stored.email(id).unwrap();
    assert!(stored.remote_only && stored.body.is_empty());
    assert!(stored.attachments.iter().all(|a| a.path.is_none() && a.inline_text.is_none()));
}

#[test]
fn same_named_online_attachments_are_not_swapped() {
    let mail = OnlineMail::new();
    let (store, id) = online_only_store(true, online_attachments());
    let service = MailService::new(online_context(store, mail));
    assert_eq!(service.attachment_document(id, 503).unwrap().bytes, FIRST_SCAN);
    assert_eq!(service.attachment_document(id, 504).unwrap().bytes, SECOND_SCAN);
    assert_eq!(service.attachment_document(id, 503).unwrap().bytes, FIRST_SCAN);
}

#[test]
fn a_file_inside_an_online_forwarded_message_inherits_the_original_bytes() {
    let mail = OnlineMail::new();
    let (store, id) = online_only_store(true, online_attachments());
    let service = MailService::new(online_context(store, mail));
    let inner = service.attached_document(id, 505, &[0]).unwrap();
    assert_eq!(inner.name, "inner.pdf");
    assert_eq!(inner.bytes, PDF);
}

#[test]
fn an_oversize_online_attachment_is_refused_before_the_original_is_downloaded() {
    let mail = OnlineMail::new();
    let mut attachments = online_attachments();
    attachments[0].size = 25 * 1024 * 1024 + 1;
    let (store, id) = online_only_store(true, attachments);
    let service = MailService::new(online_context(store, mail.clone()));
    let error = service.attachment_document(id, 501).err().expect("An oversize file is refused");
    assert_eq!(error.code, "validation");
    assert!(error.message.contains("25 MB"), "{}", error.message);
    assert_eq!(mail.original_downloads.load(Ordering::SeqCst), 0);
}

#[test]
fn an_attachment_with_a_local_file_still_reads_from_disk() {
    let mail = OnlineMail::new();
    let file = std::env::temp_dir().join(format!("qamille-local-attachment-{}.jpg", std::process::id()));
    std::fs::write(&file, b"bytes on disk").unwrap();
    let mut on_disk = online_attachments();
    on_disk[0].path = Some(file.to_string_lossy().into_owned());
    for remote_only in [false, true] {
        let (store, id) = online_only_store(remote_only, on_disk.clone());
        let service = MailService::new(online_context(store, mail.clone()));
        assert_eq!(service.attachment_document(id, 501).unwrap().bytes, b"bytes on disk");
    }
    std::fs::remove_file(&file).unwrap();
    assert_eq!(mail.original_downloads.load(Ordering::SeqCst), 0);
}


#[test]
fn default_gateway_attachment_bytes_errors() {
    struct Bare;
    impl MailGateway for Bare {
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
    }
    let store = fixture();
    let (account, email) = (&store.accounts[0], &store.emails[0]);
    let expected = "Attachment bytes unavailable for this provider";
    let error = Bare.attachment_bytes(account, email, 1).unwrap_err();
    assert_eq!((error.code, error.message.as_str()), ("external", expected));
    // A provider that downloads bodies and originals but not single parts gets
    // no guessed part either: ids are not positions it could map.
    let mut listed = email.clone();
    listed.attachments = vec![listed_attachment(1, "a.bin", "application/octet-stream", 1)];
    let error = TestMail::default().attachment_bytes(account, &listed, 1).unwrap_err();
    assert_eq!((error.code, error.message.as_str()), ("external", expected));
}
