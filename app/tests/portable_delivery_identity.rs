use qamille::{
    application::{AppError, AppResult, ServiceContext, mcp_outbox, ports::*},
    domain::outbox::{ProviderCapabilities, RemoteDraft},
    infrastructure::{portable_files, repository::JsonStoreRepository},
    model::{Account, Attachment, Draft, Store},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "qamille-portable-deliveries-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn initialize(&self, root: &Path, folder: &str, bytes: &[u8]) {
        let directory = root.join("draft-attachments").join(folder);
        fs::create_dir_all(&directory).unwrap();
        let attachment = directory.join("report.txt");
        fs::write(&attachment, bytes).unwrap();
        let mut store = Store::default();
        store.accounts.push(Account {
            id: 1,
            email: "sender@example.test".into(),
            ..Default::default()
        });
        store.drafts.push(Draft {
            id: 12,
            account_id: 1,
            updated: 123,
            to: "recipient@example.test".into(),
            subject: "Portable report".into(),
            body: "The attached report is unchanged.".into(),
            attachments: vec![attachment.to_string_lossy().into_owned()],
            ..Default::default()
        });
        store.next_id = 20;
        JsonStoreRepository::new(root.join("store.json"))
            .transaction(&mut |current| {
                *current = store.clone();
                Ok(())
            })
            .unwrap();
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qamille-portable-deliveries-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}

struct Files(PathBuf);
impl AttachmentGateway for Files {
    fn persistent_paths(&self, paths: &[String]) -> AppResult<Vec<String>> {
        paths
            .iter()
            .map(|path| portable_files::persistent_path(path, &self.0))
            .collect()
    }
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        paths
            .iter()
            .try_fold(0, |total, path| Ok(total + fs::metadata(path)?.len()))
    }
    fn read_text(&self, attachment: &Attachment) -> AppResult<String> {
        String::from_utf8(self.read_bytes(attachment)?)
            .map_err(|_| AppError::validation("Invalid test text"))
    }
    fn read_bytes(&self, attachment: &Attachment) -> AppResult<Vec<u8>> {
        fs::read(
            attachment
                .path
                .as_ref()
                .ok_or_else(|| AppError::missing("Attachment"))?,
        )
        .map_err(Into::into)
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
struct Time;
impl Clock for Time {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
#[derive(Default)]
struct Cloud {
    remote: Mutex<Vec<RemoteDraft>>,
    saves: AtomicUsize,
    sent_ids: Mutex<Vec<String>>,
}
impl MailGateway for Cloud {
    fn capabilities(&self, _: &Account) -> ProviderCapabilities {
        ProviderCapabilities {
            remote_drafts: true,
            ..Default::default()
        }
    }
    fn list_remote_drafts(&self, _: &Account) -> AppResult<Vec<RemoteDraft>> {
        Ok(self.remote.lock().unwrap().clone())
    }
    fn save_remote_draft(&self, _: &Account, draft: &RemoteDraft) -> AppResult<RemoteDraft> {
        let mut saved = draft.clone();
        saved.id = "portable-remote-draft".into();
        saved.revision = format!("revision-{}", self.saves.fetch_add(1, Ordering::SeqCst) + 1);
        *self.remote.lock().unwrap() = vec![saved.clone()];
        Ok(saved)
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        unreachable!()
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        unreachable!()
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, message: OutgoingMessage) -> AppResult<()> {
        for path in &message.attachments {
            assert!(!fs::read(path)?.is_empty());
        }
        self.sent_ids
            .lock()
            .unwrap()
            .push(message.message_id.unwrap());
        Ok(())
    }
}
fn context(root: &Path, cloud: Arc<Cloud>) -> Arc<ServiceContext> {
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(JsonStoreRepository::new(root.join("store.json"))),
        mail: cloud,
        secrets: Arc::new(Secrets),
        clock: Arc::new(Time),
        attachments: Arc::new(Files(root.to_path_buf())),
    })
}
fn sync(context: Arc<ServiceContext>) -> serde_json::Value {
    mcp_outbox::call(
        context,
        "sync_drafts",
        &serde_json::json!({"account_id":1,"direction":"both"}),
    )
    .unwrap()
}
fn operation(context: Arc<ServiceContext>, id: u64) -> serde_json::Value {
    mcp_outbox::call(context, "get_operation", &serde_json::json!({"id":id})).unwrap()
}

#[test]
fn moved_workspace_does_not_reupload_an_unchanged_cloud_draft() {
    let workspace = Workspace::new();
    let first = workspace.0.join("first");
    let moved = workspace.0.join("moved");
    workspace.initialize(
        &first,
        "unique-content-hash",
        b"unique portable report bytes",
    );
    let cloud = Arc::new(Cloud::default());
    let original = context(&first, cloud.clone());
    assert_eq!(sync(original.clone())["pushed"], 1);
    let hash = original.read().unwrap().drafts[0]
        .cloud_content_hash
        .clone();
    drop(original);
    fs::rename(&first, &moved).unwrap();
    let reopened = context(&moved, cloud.clone());
    let draft = reopened.read().unwrap().drafts.remove(0);
    assert_eq!(
        fs::read(&draft.attachments[0]).unwrap(),
        b"unique portable report bytes"
    );
    let result = sync(reopened.clone());
    assert_eq!(
        result["pushed"], 0,
        "Moving the app must not change draft contents"
    );
    assert!(result["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(cloud.saves.load(Ordering::SeqCst), 1);
    assert_eq!(reopened.read().unwrap().drafts[0].cloud_content_hash, hash);
}

#[test]
fn moved_workspace_keeps_the_message_id_that_was_sent() {
    let workspace = Workspace::new();
    let first = workspace.0.join("first");
    let moved = workspace.0.join("moved");
    workspace.initialize(
        &first,
        "unique-content-hash",
        b"unique portable report bytes",
    );
    let cloud = Arc::new(Cloud::default());
    let original = context(&first, cloud.clone());
    let queued = mcp_outbox::enqueue(
        original.clone(),
        12,
        Some(123),
        Some("portable-send"),
        None,
        false,
        None,
    )
    .unwrap();
    mcp_outbox::execute(original.clone(), queued.id).unwrap();
    let sent_id = cloud.sent_ids.lock().unwrap()[0].clone();
    assert_eq!(
        operation(original.clone(), queued.id)["message_id"],
        sent_id
    );
    drop(original);
    fs::rename(&first, &moved).unwrap();
    assert_eq!(
        operation(context(&moved, cloud), queued.id)["message_id"],
        sent_id
    );
}

#[test]
fn matching_attachment_names_in_distinct_managed_folders_keep_distinct_message_ids() {
    let workspace = Workspace::new();
    let first = workspace.0.join("first");
    let second = workspace.0.join("second");
    workspace.initialize(&first, "first-content-hash", b"first unique report bytes");
    workspace.initialize(
        &second,
        "second-content-hash",
        b"second unique report bytes",
    );
    let cloud = Arc::new(Cloud::default());
    let mut ids = Vec::new();
    for root in [&first, &second] {
        let ctx = context(root, cloud.clone());
        let queued = mcp_outbox::enqueue(
            ctx.clone(),
            12,
            Some(123),
            Some("portable-send"),
            None,
            false,
            None,
        )
        .unwrap();
        ids.push(operation(ctx, queued.id)["message_id"].clone());
    }
    assert_ne!(
        ids[0], ids[1],
        "The content folder is part of attachment identity"
    );
}
