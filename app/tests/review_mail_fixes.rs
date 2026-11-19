use qamille::{
    application::{
        AppResult, ServiceContext,
        drafts::DraftService,
        mail::{ExportFormat, MailService},
        ports::*,
    },
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{Arc, Mutex};

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
        Ok("invoice".into())
    }
    fn read_bytes(&self, _: &Attachment) -> AppResult<Vec<u8>> {
        Ok(b"invoice".to_vec())
    }
}
#[derive(Default)]
struct Mail {
    batch: Mutex<Vec<Email>>,
    sent: Mutex<Vec<OutgoingMessage>>,
    moved: Mutex<Vec<Option<u64>>>,
}
impl MailGateway for Mail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch {
            emails: self.batch.lock().unwrap().clone(),
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn resolve_identity(&self, _: &Account, email: &Email) -> AppResult<Email> {
        Ok(Email {
            remote_global_id: Some(700),
            ..email.clone()
        })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn apply_message(&self, _: &Account, email: &Email, _: RemoteAction) -> AppResult<()> {
        self.moved.lock().unwrap().push(email.remote_global_id);
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, message: OutgoingMessage) -> AppResult<()> {
        self.sent.lock().unwrap().push(message);
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
fn setup(mut store: Store) -> (Arc<ServiceContext>, Arc<Mail>) {
    store.accounts.push(Account {
        id: 1,
        email: "sender@example.com".into(),
        kind: AccountKind::Gmail,
        provider_id: "gmail".into(),
        ..Default::default()
    });
    let mail = Arc::new(Mail::default());
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: mail.clone(),
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(Files),
    });
    (context, mail)
}
fn incoming() -> Email {
    Email {
        id: 30,
        account_id: 1,
        uid: Some(10),
        remote_uid_validity: Some(99),
        remote_folder: Some("INBOX".into()),
        message_id: "original@example.com".into(),
        folder: "Inbox".into(),
        subject: "Original".into(),
        from_addr: "other@example.com".into(),
        remote_only: true,
        ..Default::default()
    }
}

#[test]
fn sent_record_and_local_eml_keep_exact_outbound_identity_and_attachment_bytes() {
    let mut store = Store::default();
    store.drafts.push(Draft {
        id: 10,
        account_id: 1,
        to: "receiver@example.com".into(),
        subject: "Invoice".into(),
        body: "Hello".into(),
        updated: 1,
        attachments: vec!["invoice.txt".into()],
        ..Default::default()
    });
    let (context, gateway) = setup(store);
    DraftService::new(context.clone()).send(10).unwrap();
    let stored = context.read().unwrap();
    let email = stored
        .emails
        .iter()
        .find(|email| email.folder == "Sent")
        .unwrap();
    let outbound = gateway.sent.lock().unwrap();
    assert_eq!(Some(&email.message_id), outbound[0].message_id.as_ref());
    assert!(email.local_sent);
    assert!(!email.outgoing_fingerprint.is_empty());
    assert_eq!(email.attachments[0].name, "invoice.txt");
    assert_eq!(email.attachments[0].size, 7);
    assert_eq!(
        email.attachments[0].path,
        outbound[0].attachments.first().cloned()
    );
    let exported = MailService::new(context.clone())
        .export(&[email.id], ExportFormat::Eml)
        .unwrap();
    let parsed = mail_parser::MessageParser::default()
        .parse(&exported.bytes)
        .unwrap();
    assert_eq!(
        parsed.message_id(),
        Some(email.message_id.trim_matches(['<', '>']))
    );
    assert_eq!(parsed.attachment(0).unwrap().contents(), b"invoice");
}

#[test]
fn archive_inbox_roundtrip_retains_provider_identity_and_local_tasks() {
    let mut store = Store::default();
    let old = Email {
        pinned: true,
        due: Some(2_000_100_000),
        ..incoming()
    };
    store.emails.push(old.clone());
    let (context, gateway) = setup(store);
    let service = MailService::new(context.clone());
    service.act(30, "archive", None).unwrap();
    service.act(30, "moveToInbox", None).unwrap();
    assert_eq!(*gateway.moved.lock().unwrap(), vec![Some(700), Some(700)]);
    *gateway.batch.lock().unwrap() = vec![Email {
        uid: Some(90),
        remote_global_id: Some(700),
        ..old
    }];
    assert!(service.sync().unwrap().errors.is_empty());
    let stored = context.read().unwrap();
    assert_eq!(stored.emails.len(), 1);
    assert_eq!(stored.emails[0].id, 30);
    assert_eq!(stored.emails[0].uid, Some(90));
    assert!(stored.emails[0].pinned);
    assert_eq!(stored.emails[0].due, Some(2_000_100_000));
}

#[test]
fn external_move_updates_saved_membership_without_losing_tasks_or_pending_undo() {
    use qamille::domain::undo::{UndoAction, UndoOperation, UndoStatus};
    let mut store = Store::default();
    let old = Email {
        remote_global_id: Some(700),
        pinned: true,
        labels: vec!["local-task".into()],
        ..incoming()
    };
    store.emails.push(old.clone());
    store.undo_actions.push(UndoAction {
        id: 100,
        operation: UndoOperation::Mail {
            email_id: 30,
            action: "moveToInbox".into(),
            value: None,
        },
        subject: "Original".into(),
        created_at_ms: 2_000_000_000_000,
        expires_at_ms: 2_000_000_100_000,
        status: UndoStatus::Pending,
        error: None,
        retry_at_ms: None,
        attempts: 0,
    });
    let (context, gateway) = setup(store);
    *gateway.batch.lock().unwrap() = vec![Email {
        uid: Some(4),
        folder: "Trash".into(),
        remote_folder: Some("Trash".into()),
        pinned: false,
        labels: vec![],
        ..old
    }];
    MailService::new(context.clone()).sync().unwrap();
    let stored = context.read().unwrap();
    assert_eq!(stored.email(30).unwrap().folder, "Trash");
    assert!(stored.email(30).unwrap().pinned);
    assert_eq!(stored.email(30).unwrap().labels, vec!["local-task"]);
    assert_eq!(stored.undo_actions.len(), 1);
    assert_eq!(
        qamille::domain::undo::project(stored)
            .email(30)
            .unwrap()
            .folder,
        "Inbox"
    );
}

#[test]
fn matching_headers_do_not_collapse_distinct_provider_deliveries() {
    let mut store = Store::default();
    let old = incoming();
    store.emails.push(old.clone());
    let (context, gateway) = setup(store);
    *gateway.batch.lock().unwrap() = vec![Email {
        uid: Some(11),
        ..old
    }];
    MailService::new(context.clone()).sync().unwrap();
    assert_eq!(context.read().unwrap().emails.len(), 2);
}

#[test]
fn missing_from_bounded_batch_is_not_deletion_evidence() {
    let mut store = Store::default();
    store.emails.push(incoming());
    let (context, _) = setup(store);
    MailService::new(context.clone()).sync().unwrap();
    assert_eq!(context.read().unwrap().email(30).unwrap().folder, "Inbox");
}

#[test]
fn authoritative_tombstone_retains_cached_data_and_blocks_server_restore() {
    let mut store = Store::default();
    let old = Email {
        remote_global_id: Some(700),
        body: "cached".into(),
        pinned: true,
        remote_only: false,
        ..incoming()
    };
    store.emails.push(old.clone());
    let (context, gateway) = setup(store);
    *gateway.batch.lock().unwrap() = vec![Email {
        folder: "Trash".into(),
        remote_deleted: true,
        uid: None,
        remote_folder: None,
        remote_uid_validity: None,
        ..old
    }];
    let service = MailService::new(context.clone());
    service.sync().unwrap();
    let stored = context.read().unwrap();
    let saved = stored.email(30).unwrap();
    assert_eq!(saved.folder, "Trash");
    assert!(saved.pinned && saved.remote_deleted);
    assert_eq!(saved.body, "cached");
    assert_eq!(
        service.act(30, "moveToInbox", None).unwrap_err().code,
        "remote_message_deleted"
    );
}

#[test]
fn historical_index_content_reconciles_pending_sent_before_normal_sync() {
    use qamille::application::{indexing::MailIndexService, indexing_ports::*};
    use qamille::domain::indexing::FolderProgress;
    use std::sync::atomic::AtomicBool;
    struct Source {
        raw: Vec<u8>,
        metadata: Email,
    }
    impl IndexSource for Source {
        fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
            unreachable!()
        }
        fn next(
            &self,
            _: &Account,
            folder: &FolderProgress,
            _: &AtomicBool,
        ) -> AppResult<IndexPage> {
            let mut progress = folder.clone();
            progress.after_uid = 9;
            progress.complete = true;
            Ok(IndexPage {
                progress,
                email: Some(self.metadata.clone()),
            })
        }
        fn content(&self, _: &Account, email: &Email, _: &AtomicBool) -> AppResult<IndexContent> {
            qamille::infrastructure::mail_index_source::parse_content(&self.raw, email)
        }
    }
    struct Search;
    impl SearchGateway for Search {
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
    let mut store = Store::default();
    store.drafts.push(Draft {
        id: 10,
        account_id: 1,
        to: "receiver@example.com".into(),
        subject: "Index echo".into(),
        body: "Hello".into(),
        updated: 1,
        ..Default::default()
    });
    let (context, _) = setup(store);
    DraftService::new(context.clone()).send(10).unwrap();
    let original = context.read().unwrap().emails[0].clone();
    let raw = qamille::domain::mail_export::eml(&original, vec![]).unwrap();
    context
        .update(|store| {
            store.email_mut(original.id).unwrap().pinned = true;
            let progress = &mut store.accounts[0].search_index;
            progress.inventory_attempted = true;
            progress.last_discovery = 2_000_000_000;
            progress.prefer_history = true;
            progress.folders = vec![FolderProgress {
                name: "Sent".into(),
                role: "Sent".into(),
                ..Default::default()
            }];
            Ok(())
        })
        .unwrap();
    let metadata = Email {
        id: 0,
        account_id: 1,
        uid: Some(9),
        remote_uid_validity: Some(7),
        remote_global_id: Some(700),
        remote_folder: Some("Sent".into()),
        folder: "Sent".into(),
        message_id: original.message_id.clone(),
        subject: original.subject.clone(),
        from_addr: original.from_addr.clone(),
        remote_only: true,
        ..Default::default()
    };
    MailIndexService::new(
        context.clone(),
        Arc::new(Source { raw, metadata }),
        Arc::new(Search),
    )
    .step()
    .unwrap();
    let stored = context.read().unwrap();
    assert_eq!(
        stored.emails.len(),
        1,
        "historical shadow must be reconciled after full payload verification"
    );
    assert_eq!(stored.emails[0].id, original.id);
    assert!(stored.emails[0].pinned);
    assert_eq!(stored.emails[0].remote_global_id, Some(700));
}
