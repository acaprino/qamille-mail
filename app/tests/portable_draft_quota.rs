use qamille::{
    application::{ServiceContext, ports::StoreRepository, storage::AccountStorageService},
    domain::{
        outbox::{Delivery, DeliveryStatus},
        storage::{AccountStoragePolicy, StorageLimit},
    },
    infrastructure::{
        gateways::{GmailGateway, KeyringSecrets, LocalAttachments, SystemClock},
        portable_files,
        repository::JsonStoreRepository,
    },
    model::{Account, AccountKind, Attachment, Draft, Email, Store},
};
use std::{fs, path::PathBuf, sync::Arc};

struct Workspace {
    root: PathBuf,
    repository: Arc<JsonStoreRepository>,
}
impl Workspace {
    fn new(limit: StorageLimit) -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../checks/draft-quota-tests");
        fs::create_dir_all(&base).unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Tests run in parallel in one process, and the clock of a Windows runner can give the same nanosecond twice:
        // a counter keeps every workspace's name its own.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = base
            .canonicalize()
            .unwrap()
            .join(format!("workspace-{}-{stamp}-{n}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let repository = Arc::new(JsonStoreRepository::new(root.join("data/store.json")));
        repository
            .transaction(&mut |store| {
                store.accounts.push(Account {
                    id: 1,
                    kind: AccountKind::Gmail,
                    storage_policy: AccountStoragePolicy {
                        limit: limit.clone(),
                        keep_all_offline: false,
                        include_spam_trash: false,
                    },
                    ..Default::default()
                });
                Ok(())
            })
            .unwrap();
        Self { root, repository }
    }
    fn import(&self, name: &str, size: usize) -> String {
        let source = self.root.join(name);
        fs::write(&source, vec![7; size]).unwrap();
        portable_files::import_draft_attachments(&self.root.join("data"), &[source])
            .unwrap()
            .remove(0)
    }
    fn service(&self) -> AccountStorageService {
        AccountStorageService::new(Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
            events: Arc::new(qamille::application::ports::NullSink),
            repository: self.repository.clone(),
            mail: Arc::new(GmailGateway::new(self.root.join("data/attachments"))),
            secrets: Arc::new(KeyringSecrets),
            clock: Arc::new(SystemClock),
            attachments: Arc::new(LocalAttachments),
        }))
    }
    fn add_draft(&self, attachments: Vec<String>) {
        self.repository
            .transaction(&mut |store| {
                store.drafts.push(Draft {
                    id: 1,
                    account_id: 1,
                    attachments: attachments.clone(),
                    ..Default::default()
                });
                Ok(())
            })
            .unwrap();
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let root = self.root.canonicalize().unwrap();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../checks/draft-quota-tests")
            .canonicalize()
            .unwrap();
        assert!(root.starts_with(&base));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("workspace-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}

fn draft_metadata(store: &Store, account: u32) -> u64 {
    store
        .drafts
        .iter()
        .filter(|d| d.account_id == account)
        .map(|draft| serde_json::to_vec_pretty(draft).unwrap().len() as u64)
        .sum()
}

fn path_alias(path: &str) -> String {
    if let Some(path) = path.strip_prefix("\\\\?\\") {
        return path.to_owned();
    }
    let path = PathBuf::from(path);
    path.parent()
        .unwrap()
        .join(".")
        .join(path.file_name().unwrap())
        .to_string_lossy()
        .into_owned()
}

#[test]
fn imported_attachment_growth_over_quota_rolls_back_the_draft() {
    let workspace = Workspace::new(StorageLimit::Limited { max_bytes: 100_000 });
    let attachment = workspace.import("report.bin", 1_000_000);
    let before = fs::read(workspace.root.join("data/store.json")).unwrap();
    let result = workspace.repository.transaction(&mut |store| {
        store.drafts.push(Draft {
            id: 1,
            account_id: 1,
            attachments: vec![attachment.clone()],
            ..Default::default()
        });
        Ok(())
    });
    assert!(
        result.is_err(),
        "An owned 1 MB attachment must exceed the 100 KB account limit"
    );
    assert_eq!(result.unwrap_err().code, "storage_full");
    assert_eq!(
        fs::read(workspace.root.join("data/store.json")).unwrap(),
        before
    );
    assert!(workspace.repository.read().unwrap().drafts.is_empty());
}

#[test]
fn unlimited_drafts_report_their_owned_attachment_bytes_as_protected() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    workspace.add_draft(vec![workspace.import("report.bin", 1_000_000)]);
    let store = workspace.repository.read().unwrap();
    let usage = workspace.service().get(1).unwrap();
    assert_eq!(usage.draft_bytes - draft_metadata(&store, 1), 1_000_000);
    assert_eq!(usage.protected_bytes, usage.used_bytes);
    assert!(!usage.over_limit);
}

#[test]
fn shared_draft_files_count_once_per_account_and_external_files_are_not_owned() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("shared.bin", 200_000);
    let alias = path_alias(&attachment);
    let external = workspace.root.join("external.bin");
    fs::write(&external, vec![4; 300_000]).unwrap();
    workspace
        .repository
        .transaction(&mut |store| {
            let mut second = store.accounts[0].clone();
            second.id = 2;
            store.accounts.push(second);
            store.drafts = vec![
                Draft {
                    id: 1,
                    account_id: 1,
                    attachments: vec![
                        attachment.clone(),
                        alias.clone(),
                        external.to_string_lossy().into_owned(),
                    ],
                    ..Default::default()
                },
                Draft {
                    id: 2,
                    account_id: 1,
                    attachments: vec![alias.clone()],
                    ..Default::default()
                },
                Draft {
                    id: 3,
                    account_id: 2,
                    attachments: vec![attachment.clone()],
                    ..Default::default()
                },
            ];
            Ok(())
        })
        .unwrap();
    let store = workspace.repository.read().unwrap();
    for account in [1, 2] {
        let usage = workspace.service().get(account).unwrap();
        assert_eq!(usage.draft_bytes - draft_metadata(&store, account), 200_000);
    }
}

#[test]
fn shared_mail_file_only_adds_bytes_missing_from_its_declared_size() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("shared.bin", 200_000);
    let alias = path_alias(&attachment);
    workspace
        .repository
        .transaction(&mut |store| {
            store.emails.push(Email {
                id: 2,
                account_id: 1,
                uid: Some(2),
                attachments: vec![Attachment {
                    path: Some(alias.clone()),
                    size: 50_000,
                    ..Default::default()
                }],
                ..Default::default()
            });
            store.drafts.push(Draft {
                id: 1,
                account_id: 1,
                attachments: vec![attachment.clone()],
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    let store = workspace.repository.read().unwrap();
    let usage = workspace.service().get(1).unwrap();
    assert_eq!(usage.draft_bytes - draft_metadata(&store, 1), 150_000);
    assert!(usage.protected_bytes >= 200_000);
}

#[test]
fn outbox_snapshot_already_accounted_does_not_add_a_second_draft_payload() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("shared.bin", 200_000);
    workspace
        .repository
        .transaction(&mut |store| {
            let draft = Draft {
                id: 1,
                account_id: 1,
                attachments: vec![attachment.clone()],
                ..Default::default()
            };
            store.drafts.push(draft.clone());
            store.outbox.push(Delivery {
                id: 2,
                key: "key".into(),
                request_hash: "hash".into(),
                draft,
                plain: String::new(),
                html: None,
                sender_name: String::new(),
                attachment_bytes: 200_000,
                status: DeliveryStatus::Queued,
                created_at: 0,
                updated_at: 0,
                send_at: 0,
                requested_send_at: None,
                error: None,
                owner_pid: None,
                requires_mcp_send: false,
                source_profile: None,
            });
            Ok(())
        })
        .unwrap();
    let store = workspace.repository.read().unwrap();
    let metadata = draft_metadata(&store, 1)
        + serde_json::to_vec_pretty(&store.outbox[0]).unwrap().len() as u64;
    assert_eq!(
        workspace.service().get(1).unwrap().draft_bytes - metadata,
        200_000
    );
}

#[test]
fn lowering_limit_cannot_evict_a_referenced_draft_attachment() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("report.bin", 1_000_000);
    workspace.add_draft(vec![attachment.clone()]);
    let error = workspace
        .service()
        .update(
            1,
            AccountStoragePolicy {
                limit: StorageLimit::Limited { max_bytes: 100_000 },
                keep_all_offline: false,
                include_spam_trash: false,
            },
            true,
        )
        .expect_err("The protected attachment prevents lowering this limit");
    assert_eq!(error.code, "storage_protected");
    assert_eq!(
        workspace.repository.read().unwrap().accounts[0]
            .storage_policy
            .limit,
        StorageLimit::Unlimited
    );
    assert_eq!(fs::metadata(&attachment).unwrap().len(), 1_000_000);
}

#[test]
fn evicting_mail_keeps_shared_draft_bytes_in_the_measured_total() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("shared.bin", 200_000);
    let alias = path_alias(&attachment);
    workspace
        .repository
        .transaction(&mut |store| {
            store.emails.push(Email {
                id: 2,
                account_id: 1,
                uid: Some(2),
                body: "x".repeat(100_000),
                attachments: vec![Attachment {
                    path: Some(alias.clone()),
                    size: 50_000,
                    ..Default::default()
                }],
                ..Default::default()
            });
            store.drafts.push(Draft {
                id: 1,
                account_id: 1,
                attachments: vec![attachment.clone()],
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    let usage = workspace
        .service()
        .update(
            1,
            AccountStoragePolicy {
                limit: StorageLimit::Limited { max_bytes: 250_000 },
                keep_all_offline: false,
                include_spam_trash: false,
            },
            true,
        )
        .unwrap();
    let store = workspace.repository.read().unwrap();
    assert!(store.emails.is_empty());
    assert_eq!(usage.draft_bytes - draft_metadata(&store, 1), 200_000);
    assert_eq!(fs::metadata(&attachment).unwrap().len(), 200_000);
    assert!(!usage.over_limit);
}

#[test]
fn missing_owned_attachment_does_not_block_draft_removal() {
    let workspace = Workspace::new(StorageLimit::Unlimited);
    let attachment = workspace.import("report.bin", 200_000);
    workspace.add_draft(vec![attachment.clone()]);
    fs::remove_file(&attachment).unwrap();
    workspace
        .repository
        .transaction(&mut |store| {
            store.drafts.clear();
            Ok(())
        })
        .unwrap();
    assert!(workspace.repository.read().unwrap().drafts.is_empty());
}
