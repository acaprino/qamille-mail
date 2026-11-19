use super::*;
use qamille::application::{indexing::MailIndexService, indexing_ports::*};
use qamille::domain::indexing::{FolderProgress, VERSION};
use std::sync::atomic::AtomicU32;

#[derive(Default)]
struct Source {
    messages: Mutex<Vec<Email>>,
    folders_override: Mutex<Option<Vec<FolderProgress>>>,
    fail_uid: AtomicU32,
    panic_uid: AtomicU32,
    unreadable_uid: AtomicU32,
    panic_next: AtomicBool,
    pages: Mutex<Vec<(u32, u32)>>,
}
impl IndexSource for Source {
    fn inventory_total(
        &self,
        account: &Account,
        _: &[FolderProgress],
        _: &AtomicBool,
    ) -> AppResult<Option<u64>> {
        let ids: std::collections::HashSet<_> = self
            .messages
            .lock()
            .unwrap()
            .iter()
            .filter(|email| email.account_id == account.id)
            .map(|email| {
                (
                    email.remote_global_id,
                    if email.remote_global_id.is_some() {
                        0
                    } else {
                        email.uid.unwrap() as u64
                    },
                )
            })
            .collect();
        Ok(Some(ids.len() as u64))
    }
    fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
        if let Some(folders) = self.folders_override.lock().unwrap().as_ref() {
            return Ok(folders.clone());
        }
        Ok(vec![FolderProgress {
            name: "Archive".into(),
            role: "Archive".into(),
            ..Default::default()
        }])
    }
    fn next(
        &self,
        account: &Account,
        folder: &FolderProgress,
        _: &AtomicBool,
    ) -> AppResult<IndexPage> {
        if self.panic_next.load(Ordering::SeqCst) { panic!("malformed headers"); }
        self.pages
            .lock()
            .unwrap()
            .push((account.id, folder.after_uid));
        // The real cursor logic, so the service tests see the newest-first walk.
        let messages = self.messages.lock().unwrap();
        let uids: Vec<u32> = messages
            .iter()
            .filter(|e| e.account_id == account.id)
            .map(|e| e.uid.unwrap())
            .collect();
        let upper = uids.iter().copied().max().unwrap_or(0);
        let mut progress = folder.clone();
        progress.uid_validity = Some(7);
        let Some(window) = progress.plan(upper, 2048) else {
            return Ok(IndexPage { progress, email: None });
        };
        let uid = progress.advance(window, uids, upper);
        let email = uid.and_then(|uid| {
            messages
                .iter()
                .find(|e| e.account_id == account.id && e.uid == Some(uid))
                .cloned()
        });
        Ok(IndexPage { progress, email })
    }
    fn content(&self, _: &Account, email: &Email, _: &AtomicBool) -> AppResult<IndexContent> {
        if email.uid == Some(self.fail_uid.load(Ordering::SeqCst)) {
            return Err(AppError::external("Temporary download failure"));
        }
        if email.uid == Some(self.panic_uid.load(Ordering::SeqCst)) { panic!("malformed message"); }
        if email.uid == Some(self.unreadable_uid.load(Ordering::SeqCst)) {
            return Err(AppError::new("index_content_unreadable", "Unreadable MIME message"));
        }
        let mut email = email.clone();
        email.body = "Entire temporary body".into();
        email.remote_only = false;
        email.remote_content_hash = format!("verified-{}-{}", email.account_id, email.uid.unwrap());
        email.attachments = vec![Attachment {
            id: email.id * 1000,
            name: "document.txt".into(),
            inline_text: Some("Full attachment contents".into()),
            ..Default::default()
        }];
        Ok(IndexContent {
            email,
            media: vec![IndexMedia {
                attachment_id: 1,
                mime: "image/png".into(),
                bytes: vec![1, 2, 3],
            }],
            warnings: vec![],
        })
    }
}
#[derive(Default)]
struct Search {
    ready: AtomicBool,
    lexical_root: Mutex<Option<std::path::PathBuf>>,
    storage_full: AtomicBool,
    fail: AtomicBool,
    transient: AtomicBool,
    panic: AtomicBool,
    calls: AtomicU32,
    indexed: Mutex<Vec<(u32, u32)>>,
}
impl SearchGateway for Search {
    fn lexical_search(
        &self,
        store: &Store,
        query: &str,
        account: Option<u32>,
        now: i64,
        sort: &str,
    ) -> AppResult<Option<Vec<u64>>> {
        match self.lexical_root.lock().unwrap().as_ref() {
            Some(root) => qamille::infrastructure::lexical_index::search(
                root, store, query, account, now, sort,
            ),
            None => Ok(None),
        }
    }
    fn index_document(
        &self,
        _: &Store,
        content: &IndexContent,
        _: &AtomicBool,
    ) -> AppResult<DocumentIndexReport> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.panic.load(Ordering::SeqCst) { panic!("tokenizer panic"); }
        if self.transient.load(Ordering::SeqCst) {
            return Err(AppError::new("index_transient", "Embedding paused after a worker failure; retry in one minute"));
        }
        if self.storage_full.load(Ordering::SeqCst) {
            return Err(AppError::new(
                "storage_full",
                "Account storage limit reached",
            ));
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::external("Worker memory limit reached"));
        }
        assert_eq!(content.email.body, "Entire temporary body");
        assert_eq!(
            content.email.attachments[0].inline_text.as_deref(),
            Some("Full attachment contents")
        );
        assert_eq!(content.media[0].bytes, [1, 2, 3]);
        self.indexed
            .lock()
            .unwrap()
            .push((content.email.account_id, content.email.uid.unwrap()));
        Ok(DocumentIndexReport {
            indexed: true,
            warnings: vec![],
        })
    }
    fn search(&self, _: &Store, _: &str, _: usize) -> AppResult<Vec<SearchHit>> {
        Ok(vec![])
    }
    fn index(&self, _: &Store) -> AppResult<String> {
        panic!("Full scan must use document indexing")
    }
    fn installed(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}
fn message(account: u32, uid: u32) -> Email {
    Email {
        account_id: account,
        uid: Some(uid),
        remote_uid_validity: Some(7),
        remote_folder: Some("Archive".into()),
        folder: "Archive".into(),
        remote_only: true,
        message_id: format!("{account}-{uid}@example.com"),
        subject: format!("Historic mail {uid}"),
        date: uid as i64,
        ..Default::default()
    }
}
fn setup() -> (
    Arc<ServiceContext>,
    Arc<Source>,
    Arc<Search>,
    MailIndexService,
) {
    let mut store = fixture();
    store.emails.clear();
    store.accounts.truncate(1);
    store.accounts[0].id = 1;
    store.accounts[0].search_index.inventory_attempted = true;
    let context = context(store, Arc::new(TestMail::default()));
    let source = Arc::new(Source::default());
    let search = Arc::new(Search::default());
    search.ready.store(true, Ordering::SeqCst);
    // One message per step: these tests follow the walk message by message.
    let service = MailIndexService::new(context.clone(), source.clone(), search.clone()).with_batch_size(1);
    (context, source, search, service)
}

#[test]
fn full_scan_resumes_old_mail_and_discards_temporary_bodies_and_images() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(1, 2), message(1, 3)];
    service.step().unwrap(); // Discover folders.
    service.step().unwrap(); // Newest message first, no date cutoff.
    {
        let store = context.read().unwrap();
        let folder = &store.accounts[0].search_index.folders[0];
        assert_eq!((folder.after_uid, folder.before_uid), (3, 3));
    }
    let resumed = MailIndexService::new(context.clone(), source.clone(), search.clone()).with_batch_size(1);
    for _ in 0..6 {
        resumed.step().unwrap();
    }
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 3), (1, 2), (1, 1)]);
    let store = context.read().unwrap();
    assert_eq!(store.accounts[0].search_index.state, "watching");
    assert!(
        store
            .emails
            .iter()
            .all(|e| e.body.is_empty() && e.remote_only && e.search_index.version == VERSION)
    );
    assert!(
        store
            .emails
            .iter()
            .flat_map(|e| &e.attachments)
            .all(|a| a.path.is_none() && a.inline_text.is_none())
    );
    assert_eq!(resumed.status().unwrap()[0].indexed_present, 3);
}

#[test]
fn arrivals_and_history_both_advance_and_paused_accounts_do_no_work() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(1, 2)];
    service.step().unwrap();
    context
        .update(|store| {
            let mut e = message(1, 99);
            e.id = store.new_id();
            store.emails.push(e);
            Ok(())
        })
        .unwrap();
    service.step().unwrap();
    service.step().unwrap();
    assert_eq!(
        *search.indexed.lock().unwrap(),
        [(1, 99), (1, 2)],
        "the arrival first, then the newest of the history"
    );
    service.configure(Some(1), false, false).unwrap();
    assert!(!service.step().unwrap());
    assert_eq!(search.indexed.lock().unwrap().len(), 2);
    service.configure(Some(1), true, false).unwrap();
    service.step().unwrap();
    assert_eq!(search.indexed.lock().unwrap().last(), Some(&(1, 1)));
}

#[test]
fn failed_content_keeps_retry_state_without_stalling_the_scan() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(1, 2)];
    source.fail_uid.store(1, Ordering::SeqCst);
    for _ in 0..5 {
        service.step().unwrap();
    }
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 2)]);
    let store = context.read().unwrap();
    let failed = store.emails.iter().find(|e| e.uid == Some(1)).unwrap();
    assert_eq!(failed.search_index.version, 0);
    assert!(failed.search_index.partial && failed.search_index.retry_at > 2_000_000_000);
    assert_eq!(store.accounts[0].search_index.state, "partial");
    source.fail_uid.store(0, Ordering::SeqCst);
    service.configure(Some(1), true, true).unwrap();
    service.step().unwrap();
    assert_eq!(search.indexed.lock().unwrap().last(), Some(&(1, 1)));
}

fn retry_now(context: &ServiceContext) {
    context.update(|store| { for e in &mut store.emails { e.search_index.retry_at = 0; } Ok(()) }).unwrap();
}

#[test]
fn a_message_that_panics_or_is_unreadable_is_set_aside_and_the_scan_continues() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(1, 2), message(1, 3)];
    source.panic_uid.store(3, Ordering::SeqCst);
    source.unreadable_uid.store(2, Ordering::SeqCst);
    for _ in 0..8 {
        service.step().unwrap();
    }
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 1)]);
    let store = context.read().unwrap();
    for uid in [2, 3] {
        let skipped = store.emails.iter().find(|e| e.uid == Some(uid)).unwrap();
        assert_eq!(skipped.search_index.version, VERSION, "uid {uid} must not stay in the queue");
        assert!(skipped.search_index.partial && skipped.search_index.notice.is_some());
    }
}

#[test]
fn a_panic_while_indexing_a_message_is_a_bounded_retry_not_a_dead_indexer() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1)];
    search.panic.store(true, Ordering::SeqCst);
    for _ in 0..12 {
        service.step().unwrap();
        retry_now(&context);
    }
    assert_eq!(search.calls.load(Ordering::SeqCst), 5, "five attempts, then the message is given up");
    search.panic.store(false, Ordering::SeqCst);
    service.step().unwrap();
    assert_eq!(search.calls.load(Ordering::SeqCst), 5);
}

#[test]
fn a_local_failure_backs_off_then_gives_up_but_a_pause_never_counts() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1)];
    search.fail.store(true, Ordering::SeqCst);
    service.step().unwrap(); // Folder discovery.
    let mut delays = Vec::new();
    for _ in 0..4 {
        service.step().unwrap();
        let store = context.read().unwrap();
        let email = store.emails.iter().find(|e| e.uid == Some(1)).unwrap();
        delays.push(email.search_index.retry_at - 2_000_000_000);
        drop(store);
        retry_now(&context);
    }
    assert_eq!(delays, [60, 120, 240, 480]);
    service.step().unwrap();
    {
        let store = context.read().unwrap();
        let email = store.emails.iter().find(|e| e.uid == Some(1)).unwrap();
        assert_eq!(email.search_index.version, VERSION);
        assert!(email.search_index.partial);
        assert!(email.search_index.notice.as_deref().unwrap().contains("skipped after 5 attempts"));
    }
    // The user's explicit retry starts again from a clean slate.
    service.configure(Some(1), true, true).unwrap();
    search.fail.store(false, Ordering::SeqCst);
    search.transient.store(true, Ordering::SeqCst);
    for _ in 0..10 {
        service.step().unwrap();
        retry_now(&context);
    }
    let store = context.read().unwrap();
    let email = store.emails.iter().find(|e| e.uid == Some(1)).unwrap();
    assert_eq!((email.search_index.version, email.search_index.attempts), (0, 0));
}

#[test]
fn a_panic_while_scanning_a_folder_is_an_error_state_and_indexing_recovers() {
    let (context, source, search, service) = setup();
    *source.messages.lock().unwrap() = vec![message(1, 1)];
    source.panic_next.store(true, Ordering::SeqCst);
    service.step().unwrap(); // Folder discovery, which does not read headers.
    assert!(service.step().unwrap(), "the panic is reported, it does not escape the step");
    assert_eq!(context.read().unwrap().accounts[0].search_index.state, "error");
    source.panic_next.store(false, Ordering::SeqCst);
    context.update(|store| { store.accounts[0].search_index.last_check = 0; Ok(()) }).unwrap();
    for _ in 0..3 {
        service.step().unwrap();
    }
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 1)]);
}
#[test]
fn accounts_are_fairly_scheduled_and_model_failure_preserves_checkpoints() {
    let (context, source, search, service) = setup();
    context
        .update(|store| {
            let mut account = store.accounts[0].clone();
            account.id = 2;
            store.accounts.push(account);
            Ok(())
        })
        .unwrap();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(2, 1)];
    search.ready.store(false, Ordering::SeqCst);
    service.step().unwrap();
    assert!(source.pages.lock().unwrap().is_empty());
    search.ready.store(true, Ordering::SeqCst);
    for _ in 0..4 {
        service.step().unwrap();
    }
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 1), (2, 1)]);
    source.messages.lock().unwrap().push(message(1, 2));
    // The periodic rescan reopens the folder, whose read range ends at UID 1.
    context
        .update(|store| {
            store.accounts[0].search_index.folders[0].complete = false;
            Ok(())
        })
        .unwrap();
    search.fail.store(true, Ordering::SeqCst);
    // Accounts take turns: within two steps account 1 has had its batch.
    service.step().unwrap();
    service.step().unwrap();
    let store = context.read().unwrap();
    // The new message is above the read range: the checkpoint moved up to it.
    assert_eq!(store.accounts[0].search_index.folders[0].after_uid, 2);
    assert_eq!(
        store
            .emails
            .iter()
            .find(|e| e.account_id == 1 && e.uid == Some(1))
            .unwrap()
            .search_index
            .version,
        VERSION
    );
    assert!(
        store
            .emails
            .iter()
            .find(|e| e.uid == Some(2))
            .unwrap()
            .search_index
            .partial
    );
}

#[test]
fn semantic_concepts_allow_remote_only_mail_but_exact_and_negative_text_need_content() {
    use qamille::domain::search::ParsedQuery;
    let mut store = fixture();
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    store.emails[0].from_addr = "alice@example.com".into();
    let parse = |query| ParsedQuery::parse(&store, query, 2_000_000_000).unwrap();
    assert!(
        parse("travel budget from:alice@example.com").semantic_matches(&store.emails[0], &store)
    );
    assert!(
        !parse("travel budget from:bob@example.com").semantic_matches(&store.emails[0], &store)
    );
    assert!(!parse("\"travel budget\"").semantic_matches(&store.emails[0], &store));
    assert!(!parse("travel -cancelled").semantic_matches(&store.emails[0], &store));
    assert!(!parse("body:budget").semantic_matches(&store.emails[0], &store));
}

#[test]
fn account_percentage_counts_unique_messages_and_never_hides_incomplete_content() {
    let (context, source, _, service) = setup();
    context
        .update(|store| {
            store.accounts[0].search_index.inventory_attempted = false;
            Ok(())
        })
        .unwrap();
    // Newest first: the unique message, its duplicate, then a second unique one.
    let mut first = message(1, 3);
    first.remote_global_id = Some(100);
    let mut duplicate = message(1, 2);
    duplicate.remote_global_id = Some(100);
    let mut last = message(1, 1);
    last.remote_global_id = Some(200);
    *source.messages.lock().unwrap() = vec![first, duplicate, last];
    assert_eq!(service.status().unwrap()[0].percent, None);
    service.step().unwrap();
    service.step().unwrap();
    assert_eq!(service.status().unwrap()[0].total, Some(2));
    assert_eq!(service.status().unwrap()[0].percent, Some(0));
    service.step().unwrap();
    assert_eq!(service.status().unwrap()[0].percent, Some(50));
    service.step().unwrap();
    assert_eq!(service.status().unwrap()[0].percent, Some(50));
    source.fail_uid.store(1, Ordering::SeqCst);
    service.step().unwrap();
    assert_eq!(service.status().unwrap()[0].percent, Some(50));
    assert_eq!(service.status().unwrap()[0].progress.partial, 1);
    source.fail_uid.store(0, Ordering::SeqCst);
    service.configure(Some(1), true, true).unwrap();
    for _ in 0..5 {
        service.step().unwrap();
    }
    assert_eq!(service.status().unwrap()[0].percent, Some(100));
    assert_eq!(service.status().unwrap()[0].indexed_present, 2);
    context
        .update(|store| {
            store.accounts[0].search_index.storage_limited = true;
            Ok(())
        })
        .unwrap();
    assert_eq!(service.status().unwrap()[0].percent, Some(99));
}

#[test]
fn discovering_a_new_folder_invalidates_the_completed_inventory_percentage() {
    let (context, source, _, service) = setup();
    let archived = FolderProgress {
        name: "Archive".into(),
        role: "Archive".into(),
        after_uid: 2,
        complete: true,
        ..Default::default()
    };
    context
        .update(|store| {
            let account = &mut store.accounts[0];
            account.search_index.folders = vec![archived.clone()];
            account.search_index.inventory_total = Some(2);
            account.search_index.inventory_attempted = true;
            account.search_index.last_discovery = 0;
            account.search_index.state = "watching".into();
            for uid in 1..=2 {
                let mut email = message(1, uid);
                email.id = store.new_id();
                email.remote_global_id = Some(uid as u64 * 100);
                email.remote_content_hash = format!("verified-1-{uid}");
                email.search_index.version = VERSION;
                email.search_index.content_hash = email.remote_content_hash.clone();
                store.emails.push(email);
            }
            Ok(())
        })
        .unwrap();
    let mut messages: Vec<_> = (1..=3)
        .map(|uid| {
            let mut email = message(1, uid);
            email.remote_global_id = Some(uid as u64 * 100);
            email
        })
        .collect();
    messages[2].remote_folder = Some("New folder".into());
    *source.messages.lock().unwrap() = messages;
    *source.folders_override.lock().unwrap() = Some(vec![
        archived,
        // Next to All Mail a label folder is only a view; a scanned role is new scope.
        FolderProgress {
            name: "New folder".into(),
            role: "Trash".into(),
            ..Default::default()
        },
    ]);
    assert_eq!(service.status().unwrap()[0].percent, Some(100));

    service.step().unwrap(); // The periodic discovery sees the new scope.
    let status = service.status().unwrap().remove(0);
    assert_eq!(status.total, None);
    assert_eq!(status.percent, None);
    assert_eq!(status.progress.state, "counting");
    assert!(!status.progress.inventory_attempted);
    assert_eq!(status.progress.folders[0].after_uid, 2);
    assert!(!status.progress.folders[1].complete);

    service.step().unwrap(); // Census includes the previously unknown message.
    let status = service.status().unwrap().remove(0);
    assert_eq!(status.total, Some(3));
    assert_eq!(status.indexed_present, 2);
    assert_eq!(status.percent, Some(66));
}

/// Status and the startup reconciliation verify the index artifacts; `step`
/// no longer does it on every pass (D-2: it rewrote the whole store each time).
#[test]
fn idle_index_status_and_startup_reconcile_artifacts_and_surface_recovery_failures() {
    struct MaintenanceRepository {
        inner: MemoryRepository,
        fail: AtomicBool,
        calls: AtomicU64,
    }
    impl StoreRepository for MaintenanceRepository {
        fn read(&self) -> AppResult<Store> {
            self.inner.read()
        }
        fn transaction(
            &self,
            operation: &mut dyn FnMut(&mut Store) -> AppResult<()>,
        ) -> AppResult<()> {
            self.inner.transaction(operation)
        }
        fn reconcile_storage(&self) -> AppResult<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                Err(AppError::storage("Cannot verify persisted indexes"))
            } else {
                Ok(())
            }
        }
    }
    let (base, source, search, _) = setup();
    let mut store = base.read().unwrap();
    store.accounts[0].kind = AccountKind::Sample;
    store.accounts[0].search_index.state = "watching".into();
    let mut email = message(1, 1);
    email.id = store.new_id();
    email.remote_content_hash = "current".into();
    email.search_index.version = VERSION;
    email.search_index.content_hash = "current".into();
    store.emails.push(email);
    let repository = Arc::new(MaintenanceRepository {
        inner: MemoryRepository::new(store),
        fail: AtomicBool::new(false),
        calls: AtomicU64::new(0),
    });
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(qamille::application::ports::NullSink),
        repository: repository.clone(),
        mail: base.mail.clone(),
        secrets: base.secrets.clone(),
        clock: base.clock.clone(),
        attachments: base.attachments.clone(),
    });
    let service = MailIndexService::new(context, source.clone(), search.clone());
    assert_eq!(service.status().unwrap()[0].percent, Some(100));
    assert!(!service.step().unwrap());
    assert_eq!(repository.calls.load(Ordering::SeqCst), 1, "a pass does not reconcile");
    service.reconcile_storage().unwrap();
    assert_eq!(repository.calls.load(Ordering::SeqCst), 2);
    repository.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        service.status().err().unwrap().message,
        "Cannot verify persisted indexes"
    );
    assert_eq!(
        service.reconcile_storage().unwrap_err().message,
        "Cannot verify persisted indexes"
    );
    assert!(source.pages.lock().unwrap().is_empty());
    assert!(search.indexed.lock().unwrap().is_empty());
}

#[test]
fn explicit_retry_readmits_evicted_task_metadata_and_respects_index_admission() {
    let (context, source, search, service) = setup();
    context
        .update(|store| {
            let account = &mut store.accounts[0];
            // A previous failed retry may already have cleared this marker.
            account.search_index.storage_limited = false;
            account.search_index.inventory_total = Some(1);
            account.search_index.folders = vec![FolderProgress {
                name: "Archive".into(),
                after_uid: 1,
                complete: true,
                ..Default::default()
            }];
            let mut other = account.clone();
            other.id = 2;
            other.search_index.enabled = false;
            other.search_index.state = "paused".into();
            store.accounts.push(other);
            for account in [1, 2] {
                let mut email = message(account, 1);
                email.id = store.new_id();
                email.pinned = true;
                email.storage_evicted = true;
                email.remote_content_hash = "old-content".into();
                email.search_index.version = VERSION;
                email.search_index.content_hash = "old-content".into();
                store.emails.push(email);
            }
            Ok(())
        })
        .unwrap();
    *source.messages.lock().unwrap() = vec![message(1, 1), message(2, 1)];
    assert_eq!(service.status().unwrap()[0].progress.partial, 1);
    service.configure(Some(1), true, true).unwrap();
    let store = context.read().unwrap();
    let admitted = store
        .emails
        .iter()
        .find(|email| email.account_id == 1)
        .unwrap();
    assert!(admitted.pinned && admitted.remote_only && admitted.body.is_empty());
    assert!(!admitted.storage_evicted);
    assert_eq!(admitted.search_index.version, 0);
    assert!(admitted.search_index.partial);
    assert_eq!(admitted.search_index.retry_at, 0);
    assert_eq!(store.accounts[0].search_index.folders[0].after_uid, 0);
    assert!(!store.accounts[0].search_index.folders[0].complete);
    assert!(!store.accounts[0].search_index.inventory_attempted);
    let untouched = store
        .emails
        .iter()
        .find(|email| email.account_id == 2)
        .unwrap();
    assert!(untouched.storage_evicted);
    assert_eq!(untouched.search_index.version, VERSION);
    assert!(!untouched.search_index.partial);
    assert!(!store.accounts[1].search_index.enabled);
    drop(store);

    search.storage_full.store(true, Ordering::SeqCst);
    service.step().unwrap(); // Refresh census after readmission.
    service.step().unwrap(); // Fresh index admission still rejects a full account.
    let status = service.status().unwrap().remove(0);
    assert_eq!(status.progress.state, "blocked");
    assert_eq!(status.indexed_present, 0);
    assert_eq!(status.progress.partial, 1);
    assert_ne!(status.percent, Some(100));
    assert!(search.indexed.lock().unwrap().is_empty());

    // Increasing space and retrying must recover the retained task instead of
    // skipping its stale completion marker or requesting offline copies.
    context
        .update(|store| {
            store.accounts[0].storage_policy.limit =
                qamille::domain::storage::StorageLimit::Unlimited;
            Ok(())
        })
        .unwrap();
    search.storage_full.store(false, Ordering::SeqCst);
    service.configure(Some(1), true, true).unwrap();
    service.step().unwrap();
    service.step().unwrap();
    assert_eq!(*search.indexed.lock().unwrap(), [(1, 1)]);
    let store = context.read().unwrap();
    let recovered = store
        .emails
        .iter()
        .find(|email| email.account_id == 1)
        .unwrap();
    assert!(recovered.pinned && recovered.remote_only && recovered.body.is_empty());
    assert!(!recovered.storage_evicted && !recovered.search_index.partial);
    assert_eq!(recovered.search_index.version, VERSION);
    assert_eq!(service.status().unwrap()[0].percent, Some(100));
}

#[test]
fn mailbox_search_uses_bm25_for_remote_bodies_before_pagination() {
    let (context, _, search, _) = setup();
    let root = std::env::temp_dir().join(format!(
        "qamille-mailbox-bm25-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    context
        .update(|store| {
            for uid in 1..=3 {
                let mut email = message(1, uid);
                email.id = store.new_id();
                email.remote_content_hash = format!("hash-{uid}");
                email.search_index.version = VERSION;
                email.search_index.content_hash = email.remote_content_hash.clone();
                let mut temporary = email.clone();
                temporary.body = if uid < 3 {
                    "needle only in online body"
                } else {
                    "unrelated"
                }
                .into();
                qamille::infrastructure::lexical_index::write(&root, &temporary, None)?;
                store.emails.push(email);
            }
            Ok(())
        })
        .unwrap();
    *search.lexical_root.lock().unwrap() = Some(root.clone());
    let mail = MailService::new(context).with_search_index(search);
    let first = mail
        .list(MailQuery {
            search: "needle".into(),
            account_id: Some(1),
            limit: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(first.total, 2);
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.next_offset, Some(1));
    let last = mail
        .list(MailQuery {
            search: "needle".into(),
            account_id: Some(1),
            limit: 1,
            offset: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(last.items.len(), 1);
    assert_ne!(first.items[0].id, last.items[0].id);
    assert!(
        mail.list(MailQuery {
            search: "needle".into(),
            account_id: Some(999),
            ..Default::default()
        })
        .unwrap()
        .items
        .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}
