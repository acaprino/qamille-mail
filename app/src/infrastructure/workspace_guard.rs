//! Keep the snapshot-based legacy UI separate from transactional MCP writers.
use crate::application::{AppError, AppResult, ports::StoreRepository};
use crate::model::Store;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::Arc,
};
pub fn legacy(root: &Path) -> AppResult<File> {
    std::fs::create_dir_all(root)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("legacy-writer.lock"))?;
    file.try_lock().map_err(|_|AppError::new("workspace_busy","Close MCP clients/background worker before opening the legacy interface, or use the current desktop app"))?;
    Ok(file)
}
pub fn modern(base: Arc<dyn StoreRepository>, root: &Path) -> AppResult<Arc<dyn StoreRepository>> {
    std::fs::create_dir_all(root)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("legacy-writer.lock"))?;
    file.try_lock_shared().map_err(|_| {
        AppError::new(
            "workspace_busy",
            "Close the legacy interface before connecting MCP",
        )
    })?;
    Ok(Arc::new(Modern { base, _guard: file }))
}
struct Modern {
    base: Arc<dyn StoreRepository>,
    _guard: File,
}
impl StoreRepository for Modern {
    fn read(&self) -> AppResult<Store> {
        self.base.read()
    }
    fn transaction(&self, f: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.base.transaction(f)
    }
    fn transaction_on_emails(&self, emails: &[u64], f: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.base.transaction_on_emails(emails, f)
    }
    fn reconcile_storage(&self) -> AppResult<()> {
        self.base.reconcile_storage()
    }
    fn message_storage_bytes(&self, s: &Store) -> AppResult<std::collections::HashMap<u64, u64>> {
        self.base.message_storage_bytes(s)
    }
    fn draft_storage_bytes(&self, s: &Store) -> AppResult<std::collections::HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
        self.base.draft_storage_bytes(s)
    }
    fn poll_change_feed(&self, since: i64) -> AppResult<Vec<crate::application::ports::ChangeFeedRow>> {
        self.base.poll_change_feed(since)
    }
    fn change_feed_head(&self) -> AppResult<i64> {
        self.base.change_feed_head()
    }
    fn storage_checkpoint(&self) -> AppResult<()> {
        self.base.storage_checkpoint()
    }
}
