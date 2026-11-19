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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attachment, Draft, Email};
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
