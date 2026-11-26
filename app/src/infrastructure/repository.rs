use crate::application::{AppError, AppResult, ports::StoreRepository};
use crate::domain::activity::{ActivityEntry, ActivityFilter, ActivityOutcome, NewActivity};
use crate::domain::model::{Store, sample_store};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

fn activity_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// JSON compatibility adapter with an inter-process lock and atomic replacement.
/// Invalid existing data is an error, never silently replaced by sample mail.
pub struct JsonStoreRepository {
    path: PathBuf,
    demo: bool,
    /// The history of the data directory, in its own file (centro attività); none in a demo.
    activity: Option<super::activity_db::ActivityDb>,
}

pub(crate) fn draft_storage_bytes(
    root: &Path,
    store: &Store,
) -> AppResult<std::collections::HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
    use crate::domain::storage::DraftStorageBytes;
    use std::collections::HashMap;

    fn file(path: &str) -> AppResult<Option<(PathBuf, u64)>> {
        let canonical = match Path::new(path).canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = std::fs::metadata(&canonical)?;
        Ok(metadata.is_file().then_some((canonical, metadata.len())))
    }

    let root = root.canonicalize()?;
    let mut owned: HashMap<u32, HashMap<PathBuf, u64>> = HashMap::new();
    for draft in &store.drafts {
        for path in &draft.attachments {
            if let Some((path, bytes)) = file(path)? {
                if path.starts_with(&root) {
                    owned.entry(draft.account_id).or_default().insert(path, bytes);
                }
            }
        }
    }
    let mut measurements = HashMap::new();
    for (account, files) in owned {
        let mut measured = DraftStorageBytes::default();
        let mut accounted: HashMap<PathBuf, u64> = HashMap::new();
        // Mail usage deduplicates by its stored path and charges the largest
        // declared size for that path. Preserve that exact existing credit.
        let mut mail_paths: HashMap<&str, u64> = HashMap::new();
        for attachment in store.emails.iter().filter(|email| email.account_id == account)
            .flat_map(|email| &email.attachments)
        {
            if let Some(path) = attachment.path.as_deref() {
                let size = mail_paths.entry(path).or_default();
                *size = (*size).max(attachment.size);
            }
        }
        for (path, bytes) in mail_paths {
            if let Some((canonical, _)) = file(path)? {
                if files.contains_key(&canonical) {
                    measured.protected_mail_paths.insert(path.to_owned());
                    let credit = accounted.entry(canonical).or_default();
                    *credit = credit.saturating_add(bytes);
                }
            }
        }
        for delivery in store.outbox.iter().filter(|delivery| delivery.draft.account_id == account) {
            let mut snapshots = Vec::new();
            let mut total = 0u64;
            let mut complete = true;
            for path in &delivery.draft.attachments {
                match file(path)? {
                    Some((path, bytes)) => {
                        total = total.saturating_add(bytes);
                        snapshots.push((path, bytes));
                    }
                    None => complete = false,
                }
            }
            // An immutable snapshot's recorded total establishes each file's
            // existing charge. If altered or incomplete, do not invent credits.
            if complete && total == delivery.attachment_bytes {
                for (path, bytes) in snapshots {
                    if files.contains_key(&path) {
                        let credit = accounted.entry(path).or_default();
                        *credit = credit.saturating_add(bytes);
                    }
                }
            }
        }
        measured.additional_bytes = files.iter().fold(0u64, |total, (path, bytes)| {
            total.saturating_add(bytes.saturating_sub(accounted.get(path).copied().unwrap_or(0)))
        });
        measurements.insert(account, measured);
    }
    Ok(measurements)
}
impl JsonStoreRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // The history lives in its own file in the data directory (nothing is created until the
        // first row is written).
        let activity = path
            .parent()
            .map(|dir| super::activity_db::ActivityDb::in_dir(dir));
        Self { path, demo: false, activity }
    }
    /// Only explicit previews seed a new workspace with synthetic mail.
    /// Existing files are always loaded unchanged, regardless of this setting. A demo keeps
    /// no activity history: it must never write into the real data directory.
    pub fn demo(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), demo: true, activity: None }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn lock(&self) -> AppResult<File> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))?;
        file.lock()?;
        Ok(file)
    }
    fn load(&self) -> AppResult<Store> {
        let mut store = match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                AppError::storage(format!(
                    "Mail data cannot be read. The original file is unchanged: {e}"
                ))
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.demo { sample_store() } else { Store::default() }
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(root) = self.path.parent() {
            super::portable_files::resolve_store_paths(&mut store, root)?;
        }
        store.adopt_old_signature();
        Ok(store)
    }
    fn save(&self, store: &Store) -> AppResult<()> {
        let mut portable = store.clone();
        // Received HTML is reader-only memory; never write it to the offline store.
        for email in &mut portable.emails {
            email.body_html.clear();
        }
        if let Some(root) = self.path.parent() {
            super::portable_files::persist_store_paths(&mut portable, root)?;
        }
        let bytes =
            serde_json::to_vec_pretty(&portable).map_err(|e| AppError::storage(e.to_string()))?;
        let _space = super::storage_fs::reserve_commit(&self.path, bytes.len() as u64)?;
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
    fn quarantine_index(&self, root: &Path) -> std::io::Result<()> {
        use std::{
            sync::atomic::{AtomicU64, Ordering},
            time::{SystemTime, UNIX_EPOCH},
        };
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Both names are fixed children of the repository directory. Keep the
        // original bytes; never replace an earlier recovery file.
        loop {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let target = root.join(format!(
                "rag_index.corrupt-{}-{timestamp}-{sequence}.bin",
                std::process::id()
            ));
            match std::fs::symlink_metadata(&target) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            return std::fs::rename(root.join("rag_index.bin"), target);
        }
    }
    fn cleanup_intent_path(&self) -> PathBuf {
        self.path.with_file_name("index-cleanup.pending")
    }
    pub(super) fn read_cleanup_intent(&self, store: &Store) -> AppResult<std::collections::HashSet<u64>> {
        let path = self.cleanup_intent_path();
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(std::collections::HashSet::new());
            }
            Err(e) => return Err(e.into()),
        };
        let fallback = || {
            store
                .emails
                .iter()
                .filter(|email| !email.storage_evicted)
                .map(|email| email.id)
                .collect()
        };
        if metadata.len() > 32 * 1024 * 1024 {
            return Ok(fallback());
        }
        let bytes = std::fs::read(path)?;
        Ok(serde_json::from_slice::<Vec<u64>>(&bytes)
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_else(|_| fallback()))
    }
    fn write_cleanup_intent(&self, ids: &std::collections::HashSet<u64>) -> AppResult<()> {
        let path = self.cleanup_intent_path();
        let mut ids: Vec<_> = ids.iter().copied().collect();
        ids.sort_unstable();
        let bytes = serde_json::to_vec(&ids)?;
        let _space = super::storage_fs::reserve_commit(&path, bytes.len() as u64)?;
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    pub(super) fn clear_cleanup_intent(&self) -> AppResult<()> {
        match std::fs::remove_file(self.cleanup_intent_path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
    pub(super) fn invalidate_indexes(store: &mut Store, ids: &std::collections::HashSet<u64>) -> bool {
        let mut accounts = std::collections::HashSet::new();
        let mut changed = false;
        for email in store
            .emails
            .iter_mut()
            .filter(|email| ids.contains(&email.id) && !email.storage_evicted)
        {
            email.search_index.version = 0;
            email.search_index.partial = true;
            email.search_index.retry_at = 0;
            email.search_index.notice =
                Some("Local search index needs rebuilding; the original mail is unchanged".into());
            accounts.insert(email.account_id);
            changed = true;
        }
        for account in store
            .accounts
            .iter_mut()
            .filter(|account| accounts.contains(&account.id))
        {
            account.search_index.state = if account.search_index.enabled {
                "queued"
            } else {
                "paused"
            }
            .into();
            account.search_index.notice = Some("Local search index is being rebuilt".into());
        }
        changed
    }
    pub(super) fn recover_lexical(&self, store: &mut Store, changed: &mut bool) -> AppResult<()> {
        let Some(root) = self.path.parent() else {
            return Ok(());
        };
        let accounts: Vec<_> = store.accounts.iter().map(|account| account.id).collect();
        for id in accounts {
            let emails: Vec<_> = store
                .emails
                .iter()
                .filter(|email| email.account_id == id)
                .collect();
            let inspection = super::lexical_index::inspect(root, id, &emails)?;
            let ids = inspection.rebuild_ids.into_iter().collect();
            *changed |= Self::invalidate_indexes(store, &ids) || inspection.quarantined;
        }
        Ok(())
    }
    fn persist_recovery(
        &self,
        store: &mut Store,
        affected: &std::collections::HashSet<u64>,
    ) -> AppResult<()> {
        let pending = self.read_cleanup_intent(store)?;
        let mut changed = Self::invalidate_indexes(store, affected);
        changed |= Self::invalidate_indexes(store, &pending);
        let result = self.recover_lexical(store, &mut changed);
        if changed {
            self.save(store)?;
        }
        self.clear_cleanup_intent()?;
        if result.is_ok() {
            if let Some(root) = self.path.parent() {
                super::lexical_index::discard_quarantines(root)?;
            }
        }
        result
    }
    pub(super) fn enforce_physical(
        &self,
        before: &Store,
        store: &mut Store,
        affected: &mut std::collections::HashSet<u64>,
    ) -> AppResult<Vec<crate::domain::storage::EvictionReport>> {
        use crate::domain::storage;
        let initial = self.message_storage_bytes(before)?;
        let initial_drafts = self.draft_storage_bytes(before)?;
        let mut reports = Vec::new();
        let accounts: Vec<_> = store.accounts.iter().map(|account| account.id).collect();
        for id in accounts {
            let mut measured = self.message_storage_bytes(store)?;
            let mut measured_drafts = self.draft_storage_bytes(store)?;
            let mut current = storage::usage_with_storage_bytes(store, id, &measured, &measured_drafts)
                .ok_or_else(|| AppError::missing("Account"))?;
            let previous = storage::usage_with_storage_bytes(before, id, &initial, &initial_drafts);
            if previous.as_ref().is_some_and(|old| {
                old.used_bytes == current.used_bytes && old.policy == current.policy
            }) {
                continue;
            }
            if current.policy.limit.max_bytes() == Some(0) {
                return Err(AppError::new(
                    "storage_full",
                    "Choose a positive account storage limit or Unlimited.",
                ));
            }
            let mut report = storage::EvictionReport {
                account_id: id,
                ..Default::default()
            };
            let original = current.used_bytes;
            while current
                .policy
                .admission_limit_bytes()
                .is_some_and(|limit| current.used_bytes > limit)
            {
                let step = storage::enforce_one_with_storage_bytes(store, id, &measured, &measured_drafts);
                if step.evicted_ids.is_empty() {
                    break;
                }
                affected.extend(step.evicted_ids.iter().copied());
                report.evicted_ids.extend(step.evicted_ids);
                // Row-count shares are usage attribution, never proof of freed
                // SQLite pages. Physically clean and remeasure after ONE victim.
                self.write_cleanup_intent(affected)?;
                if let Some(root) = self.path.parent() {
                    super::rag::reconcile_index(root, store)?;
                }
                measured = self.message_storage_bytes(store)?;
                measured_drafts = self.draft_storage_bytes(store)?;
                current = storage::usage_with_storage_bytes(store, id, &measured, &measured_drafts)
                    .ok_or_else(|| AppError::missing("Account"))?;
            }
            let previous_bytes = previous.as_ref().map_or(0, |old| old.used_bytes);
            let lowered = previous.as_ref().is_some_and(|old| {
                current.policy.limit.max_bytes().is_some_and(|limit| {
                    old.policy
                        .limit
                        .max_bytes()
                        .is_none_or(|previous| limit < previous)
                })
            });
            if current
                .policy
                .limit
                .max_bytes()
                .is_some_and(|limit| current.used_bytes > limit)
                && (current.used_bytes > previous_bytes || lowered)
            {
                return Err(AppError::new(
                    "storage_full",
                    format!(
                        "Account {id} has no space for this change without removing protected mail or drafts. Increase its limit or choose Unlimited; this change was not saved."
                    ),
                ));
            }
            report.remaining_bytes = current.used_bytes;
            report.freed_bytes = original.saturating_sub(current.used_bytes);
            report.blocked = current
                .policy
                .admission_limit_bytes()
                .is_some_and(|limit| current.used_bytes > limit);
            reports.push(report);
        }
        Ok(reports)
    }
    /// `used_bytes` of every account of `store`, measured.
    pub(super) fn usage_by_account(&self, store: &Store) -> AppResult<std::collections::HashMap<u32, u64>> {
        let measured = self.message_storage_bytes(store)?;
        let drafts = self.draft_storage_bytes(store)?;
        Ok(store
            .accounts
            .iter()
            .filter_map(|account| {
                crate::domain::storage::usage_with_storage_bytes(store, account.id, &measured, &drafts)
                    .map(|usage| (account.id, usage.used_bytes))
            })
            .collect())
    }
    /// Whether `usage` (derived for `store`) needs no eviction and breaks no
    /// limit: then the measured quota check would change nothing (W2.3).
    pub(super) fn within_limits(store: &Store, usage: &std::collections::HashMap<u32, u64>) -> bool {
        store.accounts.iter().all(|account| {
            account.storage_policy.limit.max_bytes() != Some(0)
                && account
                    .storage_policy
                    .admission_limit_bytes()
                    .is_none_or(|limit| usage.get(&account.id).is_some_and(|used| *used <= limit))
        })
    }
    pub(super) fn compact_delivery_receipts(&self, store: &mut Store) -> AppResult<bool> {
        use crate::domain::outbox::{self, DeliveryStatus};
        let Some(root) = self.path.parent() else { return Ok(false) };
        let mut changed = false;
        for operation in &mut store.outbox {
            if matches!(operation.status, DeliveryStatus::Sent | DeliveryStatus::Cancelled)
                && operation.receipt_message_id.is_none()
            {
                let mut portable = operation.draft.clone();
                for path in &mut portable.attachments {
                    *path = super::portable_files::persistent_path(path, root)?;
                }
                operation.compact(outbox::message_id(&portable));
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) fn reconcile_locked(&self, store: &mut Store) -> AppResult<()> {
        if let Some(root) = self.path.parent() {
            let index_result = match super::rag::reconcile_index(root, store) {
                Ok(()) => {
                    if root.join("rag_index.bin").is_file() {
                        // Reaching this branch proves the active index decoded.
                        // Preserved recovery copies remain available separately.
                        store.storage_index_rebuild_required = false;
                        Ok(())
                    } else if !store.storage_index_rebuild_required {
                        // Recover the warning if the process stopped after the
                        // quarantine rename but before saving its state marker.
                        (|| -> std::io::Result<bool> {
                            for entry in std::fs::read_dir(root)? {
                                if entry?
                                    .file_name()
                                    .to_string_lossy()
                                    .starts_with("rag_index.corrupt-")
                                {
                                    return Ok(true);
                                }
                            }
                            Ok(false)
                        })()
                        .map(|exists| store.storage_index_rebuild_required = exists)
                        .map_err(AppError::from)
                    } else {
                        Ok(())
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => self
                    .quarantine_index(root)
                    .map(|()| {
                        store.storage_index_rebuild_required = true;
                    })
                    .map_err(AppError::from),
                Err(error) => Err(AppError::from(error)),
            };
            // A failed index operation must not block independent cleanup of
            // unreferenced, managed attachments under the same fresh store lock.
            let attachments_result = super::mail_cache::cleanup(&root.join("attachments"), store);
            let drafts_result = super::portable_files::cleanup_draft_attachments(root, store);
            let outbox_result = super::mail_cache::cleanup_outbox(&root.join("outbox-files"), store);
            let failures: Vec<_> = [index_result, attachments_result, drafts_result, outbox_result]
                .into_iter().filter_map(Result::err).map(|error| error.message).collect();
            if !failures.is_empty() {
                return Err(AppError::storage(failures.join(" ")));
            }
        }
        Ok(())
    }
}
impl StoreRepository for JsonStoreRepository {
    fn activity_log(&self) -> Option<&dyn crate::application::ports::ActivityStore> {
        self.activity.as_ref().map(|log| log as &dyn crate::application::ports::ActivityStore)
    }
    fn storage_checkpoint(&self) -> AppResult<()> {
        // The JSON store has nothing to fold or vacuum; the maintenance pass still prunes the history.
        self.prune_activity(activity_now_ms())
    }
    fn read(&self) -> AppResult<Store> {
        let _lock = self.lock()?;
        let missing = !self.path.exists();
        let store = self.load()?;
        if missing {
            self.save(&store)?;
        }
        Ok(store)
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        let _lock = self.lock()?;
        let mut before = self.load()?;
        self.persist_recovery(&mut before, &std::collections::HashSet::new())?;
        let mut store = before.clone();
        op(&mut store)?;
        let mut affected: std::collections::HashSet<_> = before
            .emails
            .iter()
            .filter(|email| {
                store
                    .email(email.id)
                    .is_none_or(|current| current.storage_evicted && !email.storage_evicted)
            })
            .map(|email| email.id)
            .collect();
        let reports = match self.enforce_physical(&before, &mut store, &mut affected) {
            Ok(reports) => reports,
            Err(error) => {
                let _ = self.persist_recovery(&mut before, &affected);
                return Err(error);
            }
        };
        let remaining: std::collections::HashMap<_, _> = store
            .emails
            .iter()
            .map(|email| (email.id, email.storage_evicted))
            .collect();
        let removed_sources = before
            .emails
            .iter()
            .any(|email| match remaining.get(&email.id) {
                None => true,
                Some(evicted) => *evicted && !email.storage_evicted,
            });
        if removed_sources
            || reports.iter().any(|report| !report.evicted_ids.is_empty())
            || !store.cache_removals.is_empty()
        {
            store.storage_maintenance_pending = true;
        }
        if let Err(error) = self.save(&store) {
            let _ = self.persist_recovery(&mut before, &affected);
            return Err(error);
        }
        // The operation committed. Failure to remove a derived maintenance
        // marker is retried; it must not report a rollback to the caller.
        let _ = self.clear_cleanup_intent();
        Ok(())
    }
    fn message_storage_bytes(
        &self,
        store: &Store,
    ) -> AppResult<std::collections::HashMap<u64, u64>> {
        let Some(root) = self.path.parent() else {
            return Ok(std::collections::HashMap::new());
        };
        // ADR-0007 decision 2: measurement runs inside every write's admission
        // check, so a damaged optional index must never block a user write —
        // it contributes nothing until the next maintenance pass quarantines
        // and rebuilds it. Genuine I/O errors stay visible.
        match super::rag::message_index_bytes(root, store) {
            Ok(sizes) => Ok(sizes),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                Ok(std::collections::HashMap::new())
            }
            Err(error) => Err(AppError::from(error)),
        }
    }
    fn draft_storage_bytes(
        &self,
        store: &Store,
    ) -> AppResult<std::collections::HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
        let Some(root) = self.path.parent() else {
            return Ok(std::collections::HashMap::new());
        };
        draft_storage_bytes(root, store)
    }
    fn reconcile_storage(&self) -> AppResult<()> {
        let _lock = self.lock()?;
        let mut store = self.load()?;
        let before_pending = store.storage_maintenance_pending;
        let before_rebuild = store.storage_index_rebuild_required;
        let had_removals = !store.cache_removals.is_empty();
        let mut recovered = self.compact_delivery_receipts(&mut store)?;
        let mut intent_loaded = false;
        let result = (|| {
            let pending = self.read_cleanup_intent(&store)?;
            intent_loaded = true;
            recovered |= Self::invalidate_indexes(&mut store, &pending);
            self.recover_lexical(&mut store, &mut recovered)?;
            self.reconcile_locked(&mut store)
        })();
        store.storage_maintenance_pending = result.is_err();
        if result.is_ok() {
            store.cache_removals.clear();
        }
        if recovered
            || before_pending != store.storage_maintenance_pending
            || before_rebuild != store.storage_index_rebuild_required
            || (had_removals && result.is_ok())
        {
            self.save(&store)?;
        }
        if intent_loaded {
            self.clear_cleanup_intent()?;
        }
        if result.is_ok() {
            if let Some(root) = self.path.parent() {
                super::lexical_index::discard_quarantines(root)?;
            }
        }
        result
    }
}

/// The in-memory activity log: rows oldest first, and the seen marker.
#[derive(Default)]
struct MemoryActivity {
    rows: Vec<ActivityEntry>,
    seen_ms: i64,
}

/// In-memory adapter used for isolated previews and service contract tests.
pub struct MemoryRepository(Mutex<Store>, Mutex<MemoryActivity>);
impl MemoryRepository {
    pub fn new(store: Store) -> Self {
        Self(Mutex::new(store), Mutex::new(MemoryActivity::default()))
    }
    fn activity(&self) -> AppResult<std::sync::MutexGuard<'_, MemoryActivity>> {
        self.1.lock().map_err(|_| AppError::storage("Repository lock failed"))
    }
}
impl StoreRepository for MemoryRepository {
    fn read(&self) -> AppResult<Store> {
        Ok(self
            .0
            .lock()
            .map_err(|_| AppError::storage("Repository lock failed"))?
            .clone())
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| AppError::storage("Repository lock failed"))?;
        let mut working = guard.clone();
        op(&mut working)?;
        crate::domain::storage::enforce_changed_accounts(
            &guard,
            &mut working,
            &std::collections::HashMap::new(),
        )
        .map_err(|error| AppError::new("storage_full", error.message))?;
        *guard = working;
        Ok(())
    }
    fn activity_supported(&self) -> bool {
        true
    }
    fn append_activity(&self, entry: &NewActivity, at_ms: i64) -> AppResult<Option<i64>> {
        let mut log = self.activity()?;
        let seq = log.rows.last().map_or(1, |row| row.seq + 1);
        log.rows.push(ActivityEntry::new(seq, at_ms, entry));
        Ok(Some(seq))
    }
    fn set_activity_outcome(
        &self,
        undo_id: u64,
        outcome: ActivityOutcome,
        detail: Option<&str>,
    ) -> AppResult<Option<i64>> {
        let mut log = self.activity()?;
        let Some(row) = log.rows.iter_mut().rev().find(|row| row.undo_id == Some(undo_id)) else {
            return Ok(None);
        };
        row.outcome = outcome;
        if let Some(detail) = detail {
            row.detail = Some(detail.to_string());
        }
        Ok(Some(row.seq))
    }
    fn activity_page(
        &self,
        before_seq: Option<i64>,
        limit: u32,
        filter: ActivityFilter,
    ) -> AppResult<Vec<ActivityEntry>> {
        let log = self.activity()?;
        Ok(log
            .rows
            .iter()
            .rev()
            .filter(|row| before_seq.is_none_or(|before| row.seq < before) && filter.matches(row))
            .take(limit as usize)
            .cloned()
            .collect())
    }
    fn last_activity(&self, codes: &[&str], account_id: Option<u32>) -> AppResult<Option<ActivityEntry>> {
        let log = self.activity()?;
        Ok(log
            .rows
            .iter()
            .rev()
            .find(|row| row.account_id == account_id && codes.contains(&row.code.as_str()))
            .cloned())
    }
    fn activity_seen_ms(&self) -> AppResult<i64> {
        Ok(self.activity()?.seen_ms)
    }
    fn set_activity_seen_ms(&self, at_ms: i64) -> AppResult<()> {
        self.activity()?.seen_ms = at_ms;
        Ok(())
    }
    fn unseen_activity_errors(&self) -> AppResult<u32> {
        let log = self.activity()?;
        Ok(log.rows.iter().filter(|row| row.is_error() && row.at_ms > log.seen_ms).count() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::ServiceContext,
        infrastructure::gateways::{GmailGateway, KeyringSecrets, LocalAttachments, SystemClock},
        model::{Account, Attachment, Draft, Email},
    };
    use std::sync::Arc;

    struct Fixture {
        root: PathBuf,
        repository: Arc<JsonStoreRepository>,
    }
    impl Fixture {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "qamille-index-recovery-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir(&root).unwrap();
            let repository = Arc::new(JsonStoreRepository::new(root.join("store.json")));
            let store = Store {
                accounts: vec![Account {
                    id: 1,
                    ..Default::default()
                }],
                ..Default::default()
            };
            repository.save(&store).unwrap();
            Self { root, repository }
        }
        fn context(&self) -> ServiceContext {
            ServiceContext { compute: std::sync::Arc::new(crate::infrastructure::fake_compute::FakeCompute::default()),
                events: Arc::new(crate::application::ports::NullSink),
                repository: self.repository.clone(),
                mail: Arc::new(GmailGateway::new(self.root.join("attachments"))),
                secrets: Arc::new(KeyringSecrets),
                clock: Arc::new(SystemClock),
                attachments: Arc::new(LocalAttachments),
            }
        }
        fn quarantine_files(&self) -> Vec<PathBuf> {
            std::fs::read_dir(&self.root)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("rag_index.corrupt-")
                })
                .map(|entry| entry.path())
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(root), Ok(temp)) = (
                self.root.canonicalize(),
                std::env::temp_dir().canonicalize(),
            ) {
                if root.starts_with(temp)
                    && root.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with("qamille-index-recovery-")
                    })
                {
                    let _ = std::fs::remove_dir_all(root);
                }
            }
        }
    }

    #[test]
    fn received_html_is_never_written_to_the_offline_store() {
        let fixture = Fixture::new();
        let store = Store {
            emails: vec![Email {
                id: 7,
                body: "Plain".into(),
                body_html: "<p>Tracked <img src=\"https://pixel.example/x\"></p>".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        fixture.repository.save(&store).unwrap();
        let bytes = std::fs::read_to_string(fixture.root.join("store.json")).unwrap();
        assert!(!bytes.contains("pixel.example"), "HTML stays in memory only");
        let loaded = fixture.repository.load().unwrap();
        assert_eq!(loaded.emails[0].body, "Plain");
        assert!(loaded.emails[0].body_html.is_empty());
        assert!(store.emails[0].body_html.contains("pixel"), "The caller's copy is untouched");
    }

    #[test]
    fn corrupt_optional_index_is_preserved_and_first_draft_save_succeeds() {
        let fixture = Fixture::new();
        let corrupt = b"damaged-index-with-original-bytes";
        std::fs::write(fixture.root.join("rag_index.bin"), corrupt).unwrap();
        fixture
            .context()
            .update(|store| {
                store.drafts.push(Draft {
                    id: 1,
                    account_id: 1,
                    body: "unique unsent text".into(),
                    ..Default::default()
                });
                Ok(())
            })
            .unwrap();
        // ADR-0007 decision 2: the draft save itself no longer runs maintenance
        // in line; the next background pass quarantines the damaged index.
        fixture.repository.reconcile_storage().unwrap();
        let saved = fixture.repository.read().unwrap();
        assert_eq!(saved.drafts[0].body, "unique unsent text");
        assert!(saved.storage_index_rebuild_required);
        assert!(!saved.storage_maintenance_pending);
        assert!(!fixture.root.join("rag_index.bin").exists());
        let recovery = fixture.quarantine_files();
        assert_eq!(recovery.len(), 1);
        assert_eq!(std::fs::read(&recovery[0]).unwrap(), corrupt);

        // The valid empty v04 index is sufficient proof of a completed rebuild.
        let mut rebuilt = b"SLRAG04".to_vec();
        rebuilt.extend_from_slice(&(super::super::rag::DIM as u32).to_le_bytes());
        rebuilt.extend_from_slice(&0u64.to_le_bytes());
        std::fs::write(fixture.root.join("rag_index.bin"), rebuilt).unwrap();
        fixture.repository.reconcile_storage().unwrap();
        assert!(
            !fixture
                .repository
                .read()
                .unwrap()
                .storage_index_rebuild_required
        );
        assert_eq!(std::fs::read(&recovery[0]).unwrap(), corrupt);
    }

    #[test]
    fn index_io_failure_stays_visible_but_does_not_block_managed_attachment_cleanup() {
        let fixture = Fixture::new();
        let attachments = fixture.root.join("attachments");
        std::fs::create_dir(&attachments).unwrap();
        let unused = attachments.join("gmail-unused.txt");
        let referenced = attachments.join("gmail-referenced.txt");
        std::fs::write(&unused, "replaceable").unwrap();
        std::fs::write(&referenced, "referenced").unwrap();
        let mut store = fixture.repository.read().unwrap();
        store.storage_maintenance_pending = true;
        store.cache_removals = vec![
            unused.to_string_lossy().into_owned(),
            referenced.to_string_lossy().into_owned(),
        ];
        store.emails.push(Email {
            id: 1,
            account_id: 1,
            attachments: vec![Attachment {
                name: "referenced.txt".into(),
                path: Some(referenced.to_string_lossy().into_owned()),
                size: 10,
                ..Default::default()
            }],
            ..Default::default()
        });
        fixture.repository.save(&store).unwrap();
        std::fs::create_dir(fixture.root.join("rag_index.bin")).unwrap();
        assert!(fixture.repository.reconcile_storage().is_err());
        assert!(!unused.exists());
        assert!(referenced.exists());
        assert!(fixture.quarantine_files().is_empty());
        let saved = fixture.repository.read().unwrap();
        assert!(saved.storage_maintenance_pending);
        assert!(!saved.storage_index_rebuild_required);
        assert!(!saved.cache_removals.is_empty());

        std::fs::remove_dir(fixture.root.join("rag_index.bin")).unwrap();
        fixture.repository.reconcile_storage().unwrap();
        let saved = fixture.repository.read().unwrap();
        assert!(!saved.storage_maintenance_pending);
        assert!(saved.cache_removals.is_empty());
        assert!(referenced.exists());
    }

    #[test]
    fn recovered_index_does_not_commit_a_rejected_user_operation() {
        let fixture = Fixture::new();
        std::fs::write(fixture.root.join("rag_index.bin"), b"invalid").unwrap();
        let result: AppResult<()> = fixture.context().update(|store| {
            store.drafts.push(Draft {
                id: 1,
                account_id: 1,
                body: "rejected edit".into(),
                ..Default::default()
            });
            Err(AppError::validation("simulated operation failure"))
        });
        assert!(result.is_err());
        // ADR-0007 decision 2: index recovery is a maintenance pass of its own,
        // not a side effect of the write; the rejected operation stays uncommitted
        // and the recovery still persists on its own.
        fixture.repository.reconcile_storage().unwrap();
        let saved = fixture.repository.read().unwrap();
        assert!(saved.drafts.is_empty());
        assert!(saved.storage_index_rebuild_required);
        assert_eq!(fixture.quarantine_files().len(), 1);
    }
    fn remote(id: u64) -> Email {
        Email {
            id,
            account_id: 1,
            uid: Some(id as u32),
            date: id as i64,
            folder: "Inbox".into(),
            remote_only: true,
            remote_content_hash: format!("hash-{id}"),
            ..Default::default()
        }
    }
    fn seed_index(fixture: &Fixture, store: &mut Store, id: u64, text: &str) {
        let email = store
            .emails
            .iter_mut()
            .find(|email| email.id == id)
            .unwrap();
        email.search_index.version = 3;
        email.search_index.content_hash = email.remote_content_hash.clone();
        let mut content = email.clone();
        content.body = text.into();
        super::super::lexical_index::write(&fixture.root, &content, None).unwrap();
    }
    fn limited_fixture(small_indexed: bool, protected: bool) -> Fixture {
        use crate::domain::storage::{AccountStoragePolicy, StorageLimit};
        let fixture = Fixture::new();
        let mut store = fixture.repository.read().unwrap();
        store.accounts[0].kind = crate::model::AccountKind::Gmail;
        store.accounts[0].storage_policy = AccountStoragePolicy::legacy();
        store.emails = vec![remote(1), remote(2)];
        if small_indexed {
            seed_index(&fixture, &mut store, 1, "tiny old document");
        }
        let huge = (0..30000).map(|n| format!("term{n} ")).collect::<String>();
        seed_index(
            &fixture,
            &mut store,
            2,
            if small_indexed {
                &huge
            } else {
                "only the new mail is indexed"
            },
        );
        if protected {
            store.emails[1].pinned = true;
            store.drafts.push(Draft {
                id: 90,
                account_id: 1,
                reply_to: Some(2),
                body: "unique saved draft".into(),
                ..Default::default()
            });
        }
        store.accounts[0].storage_policy.limit = StorageLimit::Limited {
            max_bytes: 1_000_000,
        };
        let measured = fixture.repository.message_storage_bytes(&store).unwrap();
        let used = crate::domain::storage::usage_with_message_bytes(&store, 1, &measured)
            .unwrap()
            .used_bytes;
        store.accounts[0].storage_policy.limit = StorageLimit::Limited {
            max_bytes: (used + 100) * 10 / 9,
        };
        fixture.repository.save(&store).unwrap();
        fixture
    }
    #[test]
    fn quota_remeasures_after_unindexed_and_small_indexed_victims() {
        for small_indexed in [false, true] {
            let fixture = limited_fixture(small_indexed, false);
            fixture
                .repository
                .transaction(&mut |store| {
                    store.drafts.push(Draft {
                        id: 100,
                        account_id: 1,
                        body: "x".repeat(10000),
                        ..Default::default()
                    });
                    Ok(())
                })
                .unwrap();
            let saved = fixture.repository.read().unwrap();
            let measured = fixture.repository.message_storage_bytes(&saved).unwrap();
            let usage =
                crate::domain::storage::usage_with_message_bytes(&saved, 1, &measured).unwrap();
            assert!(
                usage.used_bytes <= usage.policy.admission_limit_bytes().unwrap(),
                "usage={}, hard_limit={:?}",
                usage.used_bytes,
                usage.policy.limit.max_bytes()
            );
            assert!(saved.email(1).is_none());
            assert_eq!(saved.drafts[0].body.len(), 10000);
        }
    }
    #[test]
    fn rejected_growth_after_index_cleanup_preserves_original_drafts_and_task_state() {
        let fixture = limited_fixture(true, true);
        let result = fixture.repository.transaction(&mut |store| {
            store.drafts.push(Draft {
                id: 100,
                account_id: 1,
                body: "x".repeat(1_000_000),
                ..Default::default()
            });
            Ok(())
        });
        assert!(result.is_err());
        let saved = fixture.repository.read().unwrap();
        assert_eq!(saved.drafts.len(), 1);
        assert_eq!(saved.drafts[0].body, "unique saved draft");
        assert!(saved.email(2).unwrap().pinned);
        assert!(!saved.email(2).unwrap().storage_evicted);
        assert!(saved.email(1).is_some());
        assert_eq!(saved.email(1).unwrap().search_index.version, 0);
        assert!(saved.email(1).unwrap().search_index.partial);
    }
    #[test]
    fn missing_and_corrupt_lexical_indexes_queue_rebuild_without_touching_other_accounts() {
        for corrupt in [false, true] {
            let fixture = Fixture::new();
            let mut store = fixture.repository.read().unwrap();
            store.accounts[0].search_index.enabled = false;
            store.accounts.push(Account {
                id: 2,
                ..Default::default()
            });
            store.emails = vec![
                remote(1),
                Email {
                    account_id: 2,
                    ..remote(2)
                },
            ];
            seed_index(&fixture, &mut store, 1, "first account body");
            seed_index(&fixture, &mut store, 2, "second account body");
            store.drafts.push(Draft {
                id: 90,
                account_id: 1,
                body: "unique saved draft".into(),
                ..Default::default()
            });
            fixture.repository.save(&store).unwrap();
            let file = fixture.root.join("search-lexical/1.sqlite");
            if corrupt {
                std::fs::write(&file, b"corrupt sqlite bytes").unwrap();
            } else {
                std::fs::remove_file(&file).unwrap();
            }
            fixture.repository.reconcile_storage().unwrap();
            let saved = fixture.repository.read().unwrap();
            assert_eq!(saved.email(1).unwrap().search_index.version, 0);
            assert!(saved.email(1).unwrap().search_index.partial);
            assert_eq!(saved.accounts[0].search_index.state, "paused");
            assert_eq!(saved.email(2).unwrap().search_index.version, 3);
            assert_eq!(saved.drafts[0].body, "unique saved draft");
            assert!(
                super::super::lexical_index::Reader::open(&fixture.root, 2)
                    .unwrap()
                    .unwrap()
                    .current(saved.email(2).unwrap())
                    .unwrap()
            );
            assert!(
                !std::fs::read_dir(fixture.root.join("search-lexical"))
                    .unwrap()
                    .any(|entry| entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains(".corrupt-"))
            );
        }
    }
    #[test]
    fn cleanup_intent_recovers_a_crash_after_vector_deletion_before_lexical_deletion() {
        let fixture = limited_fixture(true, false);
        let mut before = fixture.repository.read().unwrap();
        before.drafts.push(Draft {
            id: 90,
            account_id: 1,
            body: "unique saved draft".into(),
            ..Default::default()
        });
        fixture.repository.save(&before).unwrap();
        let vector = fixture.root.join("search-index/1/1.bin");
        std::fs::create_dir_all(vector.parent().unwrap()).unwrap();
        std::fs::write(&vector, b"replaceable vector fixture").unwrap();
        fixture
            .repository
            .write_cleanup_intent(&std::collections::HashSet::from([1]))
            .unwrap();
        std::fs::remove_file(vector).unwrap();
        assert!(
            super::super::lexical_index::Reader::open(&fixture.root, 1)
                .unwrap()
                .unwrap()
                .current(before.email(1).unwrap())
                .unwrap()
        );
        let restarted = JsonStoreRepository::new(fixture.root.join("store.json"));
        restarted.reconcile_storage().unwrap();
        let saved = restarted.read().unwrap();
        assert_eq!(saved.email(1).unwrap().search_index.version, 0);
        assert!(saved.email(1).unwrap().search_index.partial);
        assert_eq!(saved.email(2).unwrap().search_index.version, 3);
        assert_eq!(saved.drafts[0].body, "unique saved draft");
        assert!(!restarted.cleanup_intent_path().exists());
    }
    #[test]
    fn malformed_cleanup_intent_conservatively_requeues_indexes_without_losing_drafts() {
        let fixture = limited_fixture(true, false);
        let mut before = fixture.repository.read().unwrap();
        before.drafts.push(Draft {
            id: 90,
            account_id: 1,
            body: "unique saved draft".into(),
            ..Default::default()
        });
        fixture.repository.save(&before).unwrap();
        std::fs::write(fixture.repository.cleanup_intent_path(), b"[1, interrupted").unwrap();
        fixture.repository.reconcile_storage().unwrap();
        let saved = fixture.repository.read().unwrap();
        assert!(
            saved
                .emails
                .iter()
                .all(|email| email.search_index.version == 0 && email.search_index.partial)
        );
        assert_eq!(saved.drafts[0].body, "unique saved draft");
        assert!(!fixture.repository.cleanup_intent_path().exists());
    }
    #[test]
    fn actual_compaction_accepts_a_valid_limit_despite_a_protected_sqlite_share() {
        use crate::domain::storage::{AccountStoragePolicy, StorageLimit};
        let fixture = Fixture::new();
        let mut store = fixture.repository.read().unwrap();
        store.accounts[0].kind = crate::model::AccountKind::Gmail;
        store.accounts[0].storage_policy = AccountStoragePolicy::legacy();
        store.emails = vec![remote(1), remote(2)];
        store.emails[1].pinned = true;
        let huge = (0..30000).map(|n| format!("term{n} ")).collect::<String>();
        seed_index(&fixture, &mut store, 1, &huge);
        seed_index(&fixture, &mut store, 2, "tiny protected body");
        store.drafts.push(Draft {
            id: 90,
            account_id: 1,
            reply_to: Some(2),
            body: "unique saved draft".into(),
            ..Default::default()
        });
        fixture.repository.save(&store).unwrap();
        let service =
            crate::application::storage::AccountStorageService::new(Arc::new(fixture.context()));
        let policy = AccountStoragePolicy {
            limit: StorageLimit::Limited { max_bytes: 128000 },
            keep_all_offline: false,
            include_spam_trash: false,
        };
        let usage = service.update(1, policy, true).unwrap();
        assert!(usage.used_bytes <= usage.policy.admission_limit_bytes().unwrap());
        let saved = fixture.repository.read().unwrap();
        assert!(saved.email(1).is_none());
        assert!(saved.email(2).unwrap().pinned && !saved.email(2).unwrap().storage_evicted);
        assert_eq!(saved.drafts[0].body, "unique saved draft");
    }
    #[test]
    fn lowering_an_impossible_limit_is_rejected_but_existing_overfull_data_may_shrink() {
        use crate::domain::storage::{AccountStoragePolicy, StorageLimit};
        let fixture = Fixture::new();
        let mut store = fixture.repository.read().unwrap();
        store.accounts[0].kind = crate::model::AccountKind::Gmail;
        store.accounts[0].storage_policy = AccountStoragePolicy {
            limit: StorageLimit::Limited {
                max_bytes: 1_000_000,
            },
            keep_all_offline: false,
            include_spam_trash: false,
        };
        store.emails = vec![remote(1)];
        store.emails[0].pinned = true;
        store.drafts.push(Draft {
            id: 90,
            account_id: 1,
            reply_to: Some(1),
            body: "x".repeat(5000),
            ..Default::default()
        });
        fixture.repository.save(&store).unwrap();
        assert!(
            fixture
                .repository
                .transaction(&mut |store| {
                    store.accounts[0].storage_policy.limit =
                        StorageLimit::Limited { max_bytes: 1000 };
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(
            fixture.repository.read().unwrap().accounts[0]
                .storage_policy
                .limit
                .max_bytes(),
            Some(1_000_000)
        );
        store.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 1000 };
        fixture.repository.save(&store).unwrap();
        fixture
            .repository
            .transaction(&mut |store| {
                store.drafts[0].body.truncate(4000);
                Ok(())
            })
            .unwrap();
        let saved = fixture.repository.read().unwrap();
        assert_eq!(saved.drafts[0].body.len(), 4000);
        assert!(saved.email(1).unwrap().pinned);
    }
}
