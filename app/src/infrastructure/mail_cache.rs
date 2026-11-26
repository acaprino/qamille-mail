//! Removes unreferenced managed attachments after retention commits successfully.
use crate::application::AppResult;
use crate::model::Store;
use std::{collections::HashSet, path::Path};

pub fn cleanup(root: &Path, store: &Store) -> AppResult<()> {
    if !root.exists() {
        return Ok(());
    }
    let root = root.canonicalize()?;
    let keep: HashSet<_> = store
        .emails
        .iter()
        .flat_map(|e| e.attachments.iter().filter_map(|a| a.path.as_ref()))
        .chain(store.drafts.iter().flat_map(|d| d.attachments.iter()))
        .chain(store.outbox.iter().flat_map(|delivery|delivery.draft.attachments.iter()))
        .filter_map(|path| Path::new(path).canonicalize().ok())
        .collect();
    for pending in &store.cache_removals {
        let pending = Path::new(pending);
        if !pending.exists() {
            continue;
        }
        if !pending.symlink_metadata()?.is_file() {
            continue;
        }
        let Some(name) = pending.file_name() else {
            continue;
        };
        let name = name.to_string_lossy();
        let mut parts = name.split('_');
        let old_cache_name = parts
            .next()
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            && parts
                .next()
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
        if !name.starts_with("gmail-") && !old_cache_name {
            continue;
        }
        let path = pending.canonicalize()?;
        if path.parent() == Some(root.as_path()) && !keep.contains(&path) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Remove only Qamille's private outgoing snapshots, after the repository writer has
/// collected the current references. The grace period protects an enqueue preparing
/// files; enqueue also checks them under that same writer before publishing its row.
pub(super) fn cleanup_outbox(root: &Path, store: &Store) -> AppResult<()> {
    let hex_digest = |name: &str| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit());
    if !root.try_exists()? || !root.symlink_metadata()?.is_dir() { return Ok(()) }
    let root = root.canonicalize()?;
    let keep: HashSet<_> = store.emails.iter()
        .flat_map(|e| e.attachments.iter().filter_map(|a| a.path.as_ref()))
        .chain(store.drafts.iter().flat_map(|d| &d.attachments))
        .chain(store.outbox.iter().flat_map(|o| &o.draft.attachments))
        .filter_map(|p| Path::new(p).canonicalize().ok())
        .collect();
    for envelope in std::fs::read_dir(&root)? {
        let envelope = envelope?;
        if !envelope.file_type()?.is_dir() || !hex_digest(&envelope.file_name().to_string_lossy()) { continue }
        if !envelope.path().canonicalize()?.starts_with(&root) { continue }
        for item in std::fs::read_dir(envelope.path())? {
            let item = item?;
            let name = item.file_name();
            let name = name.to_string_lossy();
            let owned = name.split_once('-').is_some_and(|(index, hash)| {
                !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) && hex_digest(hash)
            });
            if !owned || !item.file_type()?.is_dir() || !item.path().canonicalize()?.starts_with(&root) { continue }
            for file in std::fs::read_dir(item.path())? {
                let file = file?;
                if !file.file_type()?.is_file() { continue }
                let path = file.path().canonicalize()?;
                let age = file.metadata()?.modified()?.elapsed().unwrap_or_default();
                if !keep.contains(&path) && path.starts_with(&root) && age >= std::time::Duration::from_secs(300) {
                    std::fs::remove_file(path)?;
                }
            }
            if std::fs::read_dir(item.path())?.next().is_none() { let _ = std::fs::remove_dir(item.path()); }
        }
        if std::fs::read_dir(envelope.path())?.next().is_none() { let _ = std::fs::remove_dir(envelope.path()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attachment, Draft, Email};
    #[test]
    fn outgoing_cleanup_protects_sent_attachments_and_frees_unreferenced_snapshots() {
        let root = std::env::temp_dir().join(format!("qamille-outgoing-cleanup-{}", std::process::id()));
        let envelope = root.join("a".repeat(64));
        let item = envelope.join(format!("0-{}", "b".repeat(64)));
        std::fs::create_dir_all(&item).unwrap();
        let path = item.join("invoice.txt");
        let file = std::fs::File::create(&path).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86400)).unwrap();
        drop(file);
        let mut store = Store::default();
        store.emails.push(Email {
            attachments: vec![Attachment { path: Some(path.to_string_lossy().into_owned()), ..Default::default() }],
            ..Default::default()
        });
        cleanup_outbox(&root, &store).unwrap();
        assert!(path.exists());
        store.emails.clear();
        cleanup_outbox(&root, &store).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn cleanup_removes_only_unused_cache_files_and_preserves_drafts_and_unmanaged_files() {
        let root = std::env::temp_dir().join(format!(
            "qamille-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let paths: Vec<_> = [
            "gmail-old.txt",
            "1_0_legacy.txt",
            "gmail-current.txt",
            "gmail-draft.txt",
            "personal.txt",
        ]
        .into_iter()
        .map(|name| root.join(name))
        .collect();
        for path in &paths {
            std::fs::write(path, "synthetic").unwrap();
        }
        let mut store = Store::default();
        store.cache_removals = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        store.emails.push(Email {
            attachments: vec![Attachment {
                path: Some(paths[2].to_string_lossy().into_owned()),
                ..Default::default()
            }],
            ..Default::default()
        });
        store.drafts.push(Draft {
            attachments: vec![paths[3].to_string_lossy().into_owned()],
            ..Default::default()
        });
        cleanup(&root, &store).unwrap();
        assert!(!paths[0].exists() && !paths[1].exists());
        assert!(paths[2].exists() && paths[3].exists() && paths[4].exists());
        for path in &paths[2..] {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
