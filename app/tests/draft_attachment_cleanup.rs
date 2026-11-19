use sha2::{Digest, Sha256};
use qamille::{
    application::ports::StoreRepository,
    infrastructure::{portable_files, repository::JsonStoreRepository},
    model::{Attachment, Draft, Email},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qamille-draft-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn owned(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let folder = self
            .0
            .join("data/draft-attachments")
            .join(format!("{:x}", Sha256::digest(bytes)));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
    fn repository(&self) -> JsonStoreRepository {
        JsonStoreRepository::new(self.0.join("data/store.json"))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qamille-draft-cleanup-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn deleting_last_committed_reference_removes_only_the_owned_copy() {
    let scratch = Scratch::new();
    let owned = scratch.owned("report.txt", b"owned report");
    let unknown = scratch
        .0
        .join("data/draft-attachments/cloud-draft/report.txt");
    fs::create_dir_all(unknown.parent().unwrap()).unwrap();
    fs::write(&unknown, b"unknown cloud draft").unwrap();
    let external = scratch.0.join("external.txt");
    fs::write(&external, b"external report").unwrap();
    let repository = scratch.repository();
    repository
        .transaction(&mut |store| {
            store.drafts.push(Draft {
                id: 1,
                attachments: vec![text(&owned), text(&external)],
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    repository
        .transaction(&mut |store| {
            store.drafts.clear();
            Ok(())
        })
        .unwrap();
    repository.reconcile_storage().unwrap();
    assert!(!owned.exists(), "The final committed reference was removed");
    assert!(unknown.exists());
    assert_eq!(fs::read(external).unwrap(), b"external report");
}

#[test]
fn shared_mail_reference_survives_until_its_last_owner_is_removed() {
    let scratch = Scratch::new();
    let owned = scratch.owned("shared.txt", b"shared report");
    let repository = scratch.repository();
    repository
        .transaction(&mut |store| {
            store.drafts.push(Draft {
                id: 1,
                attachments: vec![text(&owned)],
                ..Default::default()
            });
            store.emails.push(Email {
                id: 2,
                attachments: vec![Attachment {
                    path: Some(text(&owned)),
                    ..Default::default()
                }],
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    repository
        .transaction(&mut |store| {
            store.drafts.clear();
            Ok(())
        })
        .unwrap();
    repository.reconcile_storage().unwrap();
    assert_eq!(fs::read(&owned).unwrap(), b"shared report");
    repository
        .transaction(&mut |store| {
            store.emails.clear();
            Ok(())
        })
        .unwrap();
    repository.reconcile_storage().unwrap();
    assert!(!owned.exists());
}

#[test]
fn failed_import_batch_rolls_back_copies_created_before_the_failure() {
    let scratch = Scratch::new();
    let source = scratch.0.join("first.txt");
    fs::write(&source, b"first report").unwrap();
    let data = scratch.0.join("data");
    assert!(
        portable_files::import_draft_attachments(
            &data,
            &[source.clone(), scratch.0.join("missing.txt")]
        )
        .is_err()
    );
    let target = data
        .join("draft-attachments")
        .join(format!("{:x}", Sha256::digest(b"first report")))
        .join("first.txt");
    assert!(
        !target.exists(),
        "A failed batch must not leave an unused owned copy"
    );
    assert_eq!(fs::read(source).unwrap(), b"first report");
}

#[test]
fn interrupted_import_payload_is_collected_without_removing_unknown_files() {
    let scratch = Scratch::new();
    let committed = scratch.owned("committed.txt", b"complete attachment payload");
    let folder = committed.parent().unwrap();
    let interrupted = folder.join(format!(".{}.import", "a".repeat(32)));
    let referenced = folder.join(format!(".{}.import", "b".repeat(32)));
    let unknown = folder.join(".unfinished.import");
    let external = scratch.0.join(format!(".{}.import", "c".repeat(32)));
    for path in [&interrupted, &referenced, &unknown, &external] {
        fs::write(path, b"partial payload").unwrap();
    }
    let repository = scratch.repository();
    repository
        .transaction(&mut |store| {
            store.drafts.push(Draft {
                id: 1,
                attachments: vec![text(&committed), text(&referenced)],
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    repository.reconcile_storage().unwrap();
    assert!(
        !interrupted.exists(),
        "A crashed import's partial bytes need no completed digest"
    );
    assert!(committed.exists());
    assert!(
        referenced.exists(),
        "A committed reference always takes precedence"
    );
    assert!(
        unknown.exists(),
        "Unknown temporary names do not establish ownership"
    );
    assert!(
        external.exists(),
        "Temporary names outside managed hash folders stay untouched"
    );
}

#[test]
fn interrupted_staging_metadata_is_collected_after_its_lock_is_released() {
    let scratch = Scratch::new();
    let staging = scratch.0.join("data/draft-attachments/.staging");
    fs::create_dir_all(&staging).unwrap();
    let temporary = |token: char| staging.join(format!(".{}.tmp", token.to_string().repeat(32)));
    let lock = |token: char| staging.join(format!("{}.lock", token.to_string().repeat(32)));
    for token in ['a', 'c', 'd', 'e'] {
        fs::write(temporary(token), b"interrupted JSON").unwrap();
    }
    for token in ['a', 'b', 'c', 'e'] {
        fs::write(lock(token), b"").unwrap();
    }
    let live = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock('c'))
        .unwrap();
    live.lock().unwrap();
    let published = staging.join(format!("{}.json", "e".repeat(32)));
    fs::write(
        &published,
        serde_json::to_vec(&serde_json::json!({
            "version": 1, "paths": [], "expires_at": qamille::model::now_ts() + 600,
        }))
        .unwrap(),
    )
    .unwrap();
    let unknown = staging.join(".unfinished.tmp");
    let unknown_lock = staging.join("notes.lock");
    fs::write(&unknown, b"unknown temporary data").unwrap();
    fs::write(&unknown_lock, b"unknown lock data").unwrap();
    let repository = scratch.repository();
    repository.reconcile_storage().unwrap();
    assert!(!temporary('a').exists() && !lock('a').exists());
    assert!(
        !lock('b').exists(),
        "A crash can leave a lock without its temporary record"
    );
    assert!(
        !temporary('d').exists(),
        "An orphan temporary record needs no lock"
    );
    assert!(
        temporary('c').exists() && lock('c').exists(),
        "A live import stays protected"
    );
    assert!(
        published.exists() && temporary('e').exists() && lock('e').exists(),
        "Published records take precedence"
    );
    assert!(unknown.exists() && unknown_lock.exists());
    drop(live);
    repository.reconcile_storage().unwrap();
    assert!(!temporary('c').exists() && !lock('c').exists());
}

#[test]
fn pending_imports_survive_cleanup_and_only_the_captured_import_is_released() {
    let scratch = Scratch::new();
    let source = scratch.0.join("pending.txt");
    fs::write(&source, b"pending editor report").unwrap();
    let data = scratch.0.join("data");
    let first = portable_files::stage_draft_attachments(&data, &[source.clone()]).unwrap();
    // A second picker may finish before an older autosave returns. It reuses the
    // same content-owned file but must retain an independent staging lease.
    let second = portable_files::stage_draft_attachments(&data, &[source]).unwrap();
    let repository = scratch.repository();
    repository.reconcile_storage().unwrap();
    assert!(Path::new(&first.paths[0]).exists());
    portable_files::release_draft_attachment_imports(&data, &[first.lease_id.unwrap()]).unwrap();
    repository.reconcile_storage().unwrap();
    assert!(
        Path::new(&second.paths[0]).exists(),
        "A later picker result is still being edited"
    );
    portable_files::release_draft_attachment_imports(&data, &[second.lease_id.unwrap()]).unwrap();
    repository.reconcile_storage().unwrap();
    assert!(
        !Path::new(&second.paths[0]).exists(),
        "A removed precommit import has no owner"
    );
}

#[test]
fn rejected_save_keeps_staged_copies_available_for_retry() {
    let scratch = Scratch::new();
    let source = scratch.0.join("retry.txt");
    fs::write(&source, b"retry editor report").unwrap();
    let data = scratch.0.join("data");
    let staged = portable_files::stage_draft_attachments(&data, &[source]).unwrap();
    let repository = scratch.repository();
    assert!(
        repository
            .transaction(&mut |_| Err(qamille::application::AppError::validation(
                "save rejected"
            )))
            .is_err()
    );
    repository.reconcile_storage().unwrap();
    assert_eq!(fs::read(&staged.paths[0]).unwrap(), b"retry editor report");
    repository
        .transaction(&mut |store| {
            store.drafts.push(Draft {
                id: 1,
                attachments: staged.paths.clone(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    portable_files::release_draft_attachment_imports(&data, &[staged.lease_id.unwrap()]).unwrap();
    repository.reconcile_storage().unwrap();
    assert!(Path::new(&staged.paths[0]).exists());
    repository
        .transaction(&mut |store| {
            store.drafts.clear();
            Ok(())
        })
        .unwrap();
    repository.reconcile_storage().unwrap();
    assert!(!Path::new(&staged.paths[0]).exists());
}
