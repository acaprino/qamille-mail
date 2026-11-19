use qamille::{
    application::{
        AppError, AppResult, ServiceContext, drafts::DraftService, mcp_outbox, ports::*,
    },
    domain::outbox::DeliveryStatus,
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Time;
impl Clock for Time {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct Files;
impl AttachmentGateway for Files {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
    fn read_bytes(&self, attachment: &Attachment) -> AppResult<Vec<u8>> {
        if attachment.path.as_deref() == Some("managed-synthetic.txt") {
            Ok(b"synthetic attachment bytes".to_vec())
        } else {
            Err(AppError::validation("Unknown synthetic attachment"))
        }
    }
}
#[derive(Default)]
struct Mail {
    sent: AtomicU64,
    cloud_sent: AtomicU64,
    cloud_ids: Mutex<Vec<String>>,
    fail: AtomicBool,
    messages: Mutex<Vec<OutgoingMessage>>,
}
impl MailGateway for Mail {
    fn send_remote_draft(
        &self,
        _: &Account,
        _: &str,
        message: OutgoingMessage,
        id: &str,
        revision: &str,
    ) -> AppResult<()> {
        assert_eq!(revision, "revision-1");
        let mut ids = self.cloud_ids.lock().unwrap();
        let position = ids
            .iter()
            .position(|item| item == id)
            .ok_or_else(|| AppError::missing("Cloud draft"))?;
        ids.remove(position);
        self.cloud_sent.fetch_add(1, Ordering::SeqCst);
        self.messages.lock().unwrap().push(message);
        Ok(())
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch {
            emails: vec![],
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, message: OutgoingMessage) -> AppResult<()> {
        self.sent.fetch_add(1, Ordering::SeqCst);
        self.messages.lock().unwrap().push(message);
        std::thread::sleep(std::time::Duration::from_millis(20));
        if self.fail.load(Ordering::SeqCst) {
            Err(AppError::external("simulated ambiguous disconnect"))
        } else {
            Ok(())
        }
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
fn setup() -> (Arc<ServiceContext>, Arc<Mail>) {
    let mail = Arc::new(Mail::default());
    let store = Store {
        accounts: vec![Account {
            id: 1,
            email: "sender@example.test".into(),
            ..Default::default()
        }],
        drafts: vec![Draft {
            id: 10,
            account_id: 1,
            to: "to@example.test".into(),
            bcc: "private@example.test".into(),
            subject: "Synthetic".into(),
            body: "Original body".into(),
            updated: 100,
            ..Default::default()
        }],
        next_id: 100,
        settings: Settings {
            mcp_access: Access::Send,
            ..Default::default()
        },
        ..Default::default()
    };
    (
        Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
            events: Arc::new(NullSink),
            repository: Arc::new(MemoryRepository::new(store)),
            mail: mail.clone(),
            secrets: Arc::new(Secrets),
            clock: Arc::new(Time),
            attachments: Arc::new(Files),
        }),
        mail,
    )
}
#[test]
fn concurrent_claim_and_key_replay_only_send_once() {
    let (context, mail) = setup();
    let a = mcp_outbox::enqueue(
        context.clone(),
        10,
        Some(100),
        Some("request-1"),
        None,
        true,
        None,
    )
    .unwrap();
    let b = mcp_outbox::enqueue(
        context.clone(),
        10,
        Some(100),
        Some("request-1"),
        None,
        true,
        None,
    )
    .unwrap();
    assert_eq!(a.id, b.id);
    let c = context.clone();
    let id = a.id;
    let first = std::thread::spawn(move || mcp_outbox::execute(c, id));
    let c = context.clone();
    let second = std::thread::spawn(move || mcp_outbox::execute(c, id));
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
    assert_eq!(
        context.read().unwrap().outbox[0].status,
        DeliveryStatus::Sent
    );
    let replay = mcp_outbox::enqueue(
        context.clone(),
        10,
        Some(100),
        Some("request-1"),
        None,
        true,
        None,
    )
    .unwrap();
    assert_eq!(replay.id, id);
    assert_eq!(replay.status, DeliveryStatus::Sent);
    let sent = mail.messages.lock().unwrap();
    assert_eq!(sent[0].bcc, vec!["private@example.test"]);
    assert!(
        sent[0]
            .message_id
            .as_ref()
            .unwrap()
            .starts_with("<qamille-")
    );
}
#[test]
fn ambiguous_delivery_stays_uncertain_after_worker_restart() {
    let (context, mail) = setup();
    mail.fail.store(true, Ordering::SeqCst);
    let op = mcp_outbox::enqueue(
        context.clone(),
        10,
        None,
        Some("uncertain"),
        None,
        true,
        None,
    )
    .unwrap();
    assert!(mcp_outbox::execute(context.clone(), op.id).is_err());
    assert_eq!(
        context.read().unwrap().outbox[0].status,
        DeliveryStatus::Unknown
    );
    mcp_outbox::tick(context.clone()).unwrap();
    mcp_outbox::tick(context.clone()).unwrap();
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
    assert!(DraftService::new(context.clone()).send(10).is_err());
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
    assert!(DraftService::new(context).discard(10).is_err());
}

#[test]
fn application_send_replay_never_reports_an_uncertain_delivery_as_sent() {
    let (context, mail) = setup();
    let drafts = DraftService::new(context.clone());
    mail.fail.store(true, Ordering::SeqCst);
    assert!(drafts.send(10).is_err());
    mail.fail.store(false, Ordering::SeqCst);
    let error = drafts.send(10).unwrap_err();
    assert_eq!(error.code, "delivery_unknown");
    assert_eq!(context.read().unwrap().outbox[0].status, DeliveryStatus::Unknown);
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
}

struct BlockingCloud {
    drafts: Mutex<Vec<qamille::domain::outbox::RemoteDraft>>,
    created: std::sync::Barrier,
    resume: std::sync::Barrier,
    first_save: AtomicBool,
    fail_first_response: bool,
    saves: AtomicU64,
}

struct RetryCloud {
    attempts: AtomicU64,
    first_error: &'static str,
    accept_first: bool,
    remote: Mutex<Vec<qamille::domain::outbox::RemoteDraft>>,
}
impl MailGateway for RetryCloud {
    fn capabilities(&self, _: &Account) -> qamille::domain::outbox::ProviderCapabilities {
        qamille::domain::outbox::ProviderCapabilities { remote_drafts: true, ..Default::default() }
    }
    fn list_remote_drafts(&self, _: &Account) -> AppResult<Vec<qamille::domain::outbox::RemoteDraft>> {
        Ok(self.remote.lock().unwrap().clone())
    }
    fn save_remote_draft(&self, _: &Account, draft: &qamille::domain::outbox::RemoteDraft) -> AppResult<qamille::domain::outbox::RemoteDraft> {
        let first = self.attempts.fetch_add(1, Ordering::SeqCst) == 0;
        let mut saved = draft.clone();
        saved.id = "retry-cloud".into();
        saved.revision = "revision-1".into();
        if !first || self.accept_first { *self.remote.lock().unwrap() = vec![saved.clone()]; }
        if first { Err(AppError::new(self.first_error, "synthetic cloud failure")) } else { Ok(saved) }
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> { unreachable!() }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> { unreachable!() }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> { unreachable!() }
    fn check_login(&self, _: &Account) -> AppResult<()> { unreachable!() }
}

#[test]
fn definite_cloud_failure_retries_but_uncertain_absence_never_duplicates() {
    for (code, accepted) in [("cloud_save_not_started", false), ("external", false), ("external", true)] {
        let (base, _) = setup();
        let cloud = Arc::new(RetryCloud { attempts: AtomicU64::new(0), first_error: code, accept_first: accepted, remote: Mutex::new(vec![]) });
        let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()), events: Arc::new(NullSink), repository: base.repository.clone(), mail: cloud.clone(),
            secrets: base.secrets.clone(), clock: base.clock.clone(), attachments: base.attachments.clone() });
        let sync = |direction| mcp_outbox::call(context.clone(), "sync_drafts", &serde_json::json!({"account_id":1,"direction":direction}));
        assert_eq!(sync("both").unwrap_err().code, code);
        assert_eq!(context.read().unwrap().drafts[0].cloud_sync_pending, code != "cloud_save_not_started");
        sync("pull").unwrap();
        let retry = sync("both");
        if code == "cloud_save_not_started" {
            assert_eq!(retry.unwrap()["pushed"], 1);
            assert_eq!(cloud.attempts.load(Ordering::SeqCst), 2);
            assert!(!context.read().unwrap().drafts[0].cloud_sync_pending);
        } else if accepted {
            assert_eq!(retry.unwrap()["pushed"], 0);
            assert_eq!(cloud.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(context.read().unwrap().drafts[0].remote_id.as_deref(), Some("retry-cloud"));
        } else {
            assert_eq!(retry.unwrap_err().code, "cloud_sync_unknown");
            assert_eq!(cloud.attempts.load(Ordering::SeqCst), 1);
        }
    }
}
impl BlockingCloud {
    fn new(fail_first_response: bool) -> Self {
        Self {
            drafts: Mutex::new(Vec::new()), created: std::sync::Barrier::new(2),
            resume: std::sync::Barrier::new(2), first_save: AtomicBool::new(true),
            fail_first_response, saves: AtomicU64::new(0),
        }
    }
}
impl MailGateway for BlockingCloud {
    fn capabilities(&self, _: &Account) -> qamille::domain::outbox::ProviderCapabilities {
        qamille::domain::outbox::ProviderCapabilities {
            provider_id: "synthetic-cloud".into(), remote_drafts: true, ..Default::default()
        }
    }
    fn list_remote_drafts(&self, _: &Account) -> AppResult<Vec<qamille::domain::outbox::RemoteDraft>> {
        Ok(self.drafts.lock().unwrap().clone())
    }
    fn save_remote_draft(&self, _: &Account, draft: &qamille::domain::outbox::RemoteDraft) -> AppResult<qamille::domain::outbox::RemoteDraft> {
        let mut saved = draft.clone();
        saved.id = "cloud-1".into();
        saved.revision = format!("revision-{}", self.saves.fetch_add(1, Ordering::SeqCst) + 1);
        *self.drafts.lock().unwrap() = vec![saved.clone()];
        if self.first_save.swap(false, Ordering::SeqCst) {
            self.created.wait();
            self.resume.wait();
            if self.fail_first_response {
                return Err(AppError::external("Response lost after creating the cloud draft"));
            }
        }
        Ok(saved)
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> { unreachable!() }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> { unreachable!() }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> { unreachable!() }
    fn check_login(&self, _: &Account) -> AppResult<()> { unreachable!() }
}
fn cloud_context(cloud: Arc<BlockingCloud>) -> Arc<ServiceContext> {
    let (base, _) = setup();
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: base.repository.clone(), mail: cloud,
        secrets: base.secrets.clone(), clock: base.clock.clone(), attachments: base.attachments.clone(),
    })
}

#[test]
fn first_cloud_save_keeps_concurrent_edits_and_tracks_the_uploaded_revision() {
    let cloud = Arc::new(BlockingCloud::new(false));
    let context = cloud_context(cloud.clone());
    // The editor predates the cloud claim and therefore has none of its metadata.
    let mut editor = context.read().unwrap().drafts[0].clone();
    let worker_context = context.clone();
    let worker = std::thread::spawn(move || mcp_outbox::call(worker_context, "sync_drafts", &serde_json::json!({"account_id":1,"direction":"both"})));
    cloud.created.wait();
    editor.body = "Newer local text".into();
    let edited = DraftService::new(context.clone()).save(editor);
    cloud.resume.wait();
    let synced = worker.join().unwrap().unwrap();
    let mut editor = edited.unwrap();
    assert!(editor.cloud_sync_pending);
    assert!(!editor.cloud_sync_key.is_empty());
    assert!(!editor.cloud_content_hash.is_empty());
    assert_eq!(synced["pushed"], 1);
    let current = context.read().unwrap().drafts[0].clone();
    assert_eq!(current.body, "Newer local text");
    assert_eq!(current.remote_id.as_deref(), Some("cloud-1"));
    assert!(!current.cloud_sync_pending);

    // Saving the editor again also preserves a provider result received since
    // that editor snapshot was read.
    editor.subject = "Newer subject".into();
    let saved = DraftService::new(context.clone()).save(editor).unwrap();
    assert_eq!(saved.remote_id.as_deref(), Some("cloud-1"));
    assert!(!saved.cloud_sync_pending);
    let pulled = mcp_outbox::call(context.clone(), "sync_drafts", &serde_json::json!({"account_id":1,"direction":"pull"})).unwrap();
    assert_eq!(pulled["pulled"], 0);
    assert_eq!(context.read().unwrap().drafts[0].body, "Newer local text");
    let pushed = mcp_outbox::call(context.clone(), "sync_drafts", &serde_json::json!({"account_id":1,"direction":"push"})).unwrap();
    assert_eq!(pushed["pushed"], 1);
    assert_eq!(cloud.drafts.lock().unwrap()[0].draft.body, "Newer local text");
    assert_eq!(cloud.saves.load(Ordering::SeqCst), 2);
}

#[test]
fn lost_first_cloud_response_cannot_overwrite_an_edit_during_reconciliation() {
    let cloud = Arc::new(BlockingCloud::new(true));
    let context = cloud_context(cloud.clone());
    let mut editor = context.read().unwrap().drafts[0].clone();
    let worker_context = context.clone();
    let worker = std::thread::spawn(move || mcp_outbox::call(worker_context, "sync_drafts", &serde_json::json!({"account_id":1,"direction":"both"})));
    cloud.created.wait();
    editor.body = "Keep this unsynchronized edit".into();
    let edited = DraftService::new(context.clone()).save(editor);
    cloud.resume.wait();
    assert!(worker.join().unwrap().is_err());
    edited.unwrap();
    let result = mcp_outbox::call(context.clone(), "sync_drafts", &serde_json::json!({"account_id":1,"direction":"pull"})).unwrap();
    assert_eq!(result["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(result["pulled"], 0);
    assert_eq!(context.read().unwrap().drafts[0].body, "Keep this unsynchronized edit");
    assert_eq!(cloud.saves.load(Ordering::SeqCst), 1);
}

#[test]
fn cloud_draft_account_cannot_change_during_or_after_synchronization() {
    let cloud = Arc::new(BlockingCloud::new(false));
    let context = cloud_context(cloud.clone());
    context.update(|store| {
        store.accounts.push(Account { id: 2, email: "other@example.test".into(), ..Default::default() });
        Ok(())
    }).unwrap();
    // An editor opened before the claim must still respect its live account binding.
    let mut editor = context.read().unwrap().drafts[0].clone();
    let worker_context = context.clone();
    let worker = std::thread::spawn(move || mcp_outbox::call(worker_context, "sync_drafts", &serde_json::json!({"account_id":1,"direction":"push"})));
    cloud.created.wait();
    editor.account_id = 2;
    let changed = DraftService::new(context.clone()).save(editor);
    cloud.resume.wait();
    worker.join().unwrap().unwrap();
    let error = changed.unwrap_err();
    assert_eq!(error.code, "cloud_draft_account_locked");
    let current = context.read().unwrap().drafts[0].clone();
    assert_eq!(current.account_id, 1);
    assert_eq!(current.remote_id.as_deref(), Some("cloud-1"));
    let mut editor = current;
    editor.account_id = 2;
    assert_eq!(DraftService::new(context.clone()).save(editor).unwrap_err().code, "cloud_draft_account_locked");
    assert_eq!(context.read().unwrap().drafts[0].account_id, 1);
}

#[test]
fn cloud_save_cannot_bind_a_response_to_a_changed_account_or_sync_key() {
    for change_account in [true, false] {
        let cloud = Arc::new(BlockingCloud::new(false));
        let context = cloud_context(cloud.clone());
        context.update(|store| {
            store.accounts.push(Account { id: 2, email: "other@example.test".into(), ..Default::default() });
            Ok(())
        }).unwrap();
        let worker_context = context.clone();
        let worker = std::thread::spawn(move || mcp_outbox::call(worker_context, "sync_drafts", &serde_json::json!({"account_id":1,"direction":"push"})));
        cloud.created.wait();
        // Simulate an external repository update rather than bypassing the public
        // draft service's restriction through another application call.
        context.update(|store| {
            if change_account { store.drafts[0].account_id = 2; }
            else { store.drafts[0].cloud_sync_key = "replacement-cloud-key".into(); }
            Ok(())
        }).unwrap();
        cloud.resume.wait();
        assert_eq!(worker.join().unwrap().unwrap_err().code, "revision_conflict");
        let current = context.read().unwrap().drafts[0].clone();
        assert!(current.remote_id.is_none());
        assert!(current.remote_revision.is_none());
        assert!(current.cloud_sync_pending);
    }
}
#[test]
fn queued_snapshot_is_immutable_and_stale_revision_never_enqueues() {
    let (context, mail) = setup();
    assert!(
        mcp_outbox::enqueue(
            context.clone(),
            10,
            Some(99),
            Some("stale"),
            None,
            true,
            None
        )
        .is_err()
    );
    let op = mcp_outbox::enqueue(
        context.clone(),
        10,
        Some(100),
        Some("snapshot"),
        None,
        true,
        None,
    )
    .unwrap();
    context
        .update(|store| {
            store.drafts[0].body = "Newer edit".into();
            store.drafts[0].updated += 1;
            Ok(())
        })
        .unwrap();
    mcp_outbox::execute(context.clone(), op.id).unwrap();
    assert_eq!(mail.messages.lock().unwrap()[0].plain, "Original body");
    assert_eq!(context.read().unwrap().drafts[0].body, "Newer edit");
}
#[test]
fn cancellation_and_revocation_prevent_provider_calls() {
    let (context, mail) = setup();
    let op = mcp_outbox::enqueue(
        context.clone(),
        10,
        None,
        Some("cancel"),
        Some(Time.now() + 60),
        true,
        None,
    )
    .unwrap();
    mcp_outbox::call(
        context.clone(),
        "cancel_operation",
        &serde_json::json!({"id":op.id}),
    )
    .unwrap();
    mcp_outbox::execute(context.clone(), op.id).unwrap();
    assert_eq!(mail.sent.load(Ordering::SeqCst), 0);
    let op =
        mcp_outbox::enqueue(context.clone(), 10, None, Some("revoke"), None, true, None).unwrap();
    context
        .update(|s| {
            s.settings.mcp_access = Access::ReadOnly;
            Ok(())
        })
        .unwrap();
    mcp_outbox::execute(context.clone(), op.id).unwrap();
    assert_eq!(
        context.read().unwrap().outbox.last().unwrap().status,
        DeliveryStatus::NeedsReview
    );
    assert_eq!(mail.sent.load(Ordering::SeqCst), 0);
}
#[test]
fn overdue_scheduled_delivery_requires_review() {
    let (context, mail) = setup();
    let op = mcp_outbox::enqueue(
        context.clone(),
        10,
        None,
        Some("late"),
        Some(Time.now() + 60),
        true,
        None,
    )
    .unwrap();
    context
        .update(|s| {
            s.outbox[0].send_at = Time.now() - 901;
            Ok(())
        })
        .unwrap();
    mcp_outbox::execute(context.clone(), op.id).unwrap();
    assert_eq!(
        context.read().unwrap().outbox[0].status,
        DeliveryStatus::NeedsReview
    );
    assert_eq!(mail.sent.load(Ordering::SeqCst), 0);
}
#[test]
fn mcp_send_returns_queue_and_never_calls_network_inline() {
    let (context, mail) = setup();
    let response = mcp_outbox::call(
        context.clone(),
        "send_draft",
        &serde_json::json!({"id":"10","expected_revision":100,"idempotency_key":"async"}),
    )
    .unwrap();
    assert_eq!(response["status"], "queued");
    assert_eq!(mail.sent.load(Ordering::SeqCst), 0);
    mcp_outbox::tick(context).unwrap();
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
}

#[test]
fn cloud_draft_uses_cloud_sender_removes_remote_draft_and_never_sends_smtp() {
    let (context, mail) = setup();
    mail.cloud_ids.lock().unwrap().push("cloud-1".into());
    context
        .update(|store| {
            store.drafts[0].remote_id = Some("cloud-1".into());
            store.drafts[0].remote_revision = Some("revision-1".into());
            Ok(())
        })
        .unwrap();
    let operation = mcp_outbox::enqueue(
        context.clone(),
        10,
        None,
        Some("cloud-send"),
        None,
        true,
        None,
    )
    .unwrap();
    mcp_outbox::execute(context.clone(), operation.id).unwrap();
    mcp_outbox::execute(context.clone(), operation.id).unwrap();
    assert_eq!(mail.sent.load(Ordering::SeqCst), 0);
    assert_eq!(mail.cloud_sent.load(Ordering::SeqCst), 1);
    assert!(mail.cloud_ids.lock().unwrap().is_empty());
    assert!(context.read().unwrap().drafts.is_empty());
    assert_eq!(
        context.read().unwrap().outbox[0].status,
        DeliveryStatus::Sent
    );
}

#[test]
fn send_email_supports_html_bcc_only_and_managed_attachments() {
    let (context, mail) = setup();
    let args = serde_json::json!({"account_id":"1","bcc":["private@example.test"],"subject":"HTML","body":"Fallback","html":"<b>Hello</b>","format":"html","attachments":["synthetic-handle"],"_attachment_paths":["managed-synthetic.txt"],"idempotency_key":"rich-send"});
    let response = mcp_outbox::call(context.clone(), "send_email", &args).unwrap();
    assert_eq!(response["status"], "queued");
    mcp_outbox::tick(context.clone()).unwrap();
    let stored = context.read().unwrap();
    assert_eq!(stored.outbox.last().unwrap().status, DeliveryStatus::Sent);
    let sent = stored.emails.iter().find(|email| email.folder == "Sent" && email.local_sent).unwrap();
    assert_eq!(sent.attachments.len(), 1);
    let attachment = &sent.attachments[0];
    assert_eq!(attachment.name, "managed-synthetic.txt");
    assert_eq!(attachment.path.as_deref(), Some("managed-synthetic.txt"));
    assert_eq!(attachment.size, b"synthetic attachment bytes".len() as u64);
    assert_eq!(context.attachments.read_bytes(attachment).unwrap(), b"synthetic attachment bytes");
    let messages = mail.messages.lock().unwrap();
    let message = messages.last().unwrap();
    assert_eq!(message.message_id.as_deref(), Some(sent.message_id.as_str()));
    assert!(message.to.is_empty());
    assert_eq!(message.bcc, vec!["private@example.test"]);
    assert!(message.html.as_ref().unwrap().contains("<b>Hello</b>"));
    assert!(message.plain.contains("Hello"));
    assert_eq!(message.attachments, vec!["managed-synthetic.txt"]);
    drop(messages);
    let mut replay = args;
    replay.as_object_mut().unwrap().remove("_attachment_paths");
    let same = mcp_outbox::call(context, "send_email", &replay).unwrap();
    assert_eq!(same["operation_id"], response["operation_id"]);
    assert_eq!(mail.sent.load(Ordering::SeqCst), 1);
}
