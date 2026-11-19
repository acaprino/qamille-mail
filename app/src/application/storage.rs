//! Account storage settings are independent from provider credentials and delivery.
use super::{AppError, AppResult, ServiceContext};
use crate::domain::storage::{self, AccountStoragePolicy, StorageLimit, StorageUsage};
use std::sync::Arc;

#[derive(Clone)]
pub struct AccountStorageService {
    context: Arc<ServiceContext>,
}

impl AccountStorageService {
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self { context }
    }

    pub fn get(&self, account_id: u32) -> AppResult<StorageUsage> {
        let _ = self.context.repository.reconcile_storage();
        let store = self.context.read_shared()?;
        let measured = self.context.repository.message_storage_bytes(&store)?;
        let drafts = self.context.repository.draft_storage_bytes(&store)?;
        storage::usage_with_storage_bytes(&store, account_id, &measured, &drafts)
            .ok_or_else(|| AppError::missing("Account"))
    }

    pub fn update(
        &self,
        account_id: u32,
        policy: AccountStoragePolicy,
        confirm_cleanup: bool,
    ) -> AppResult<StorageUsage> {
        if let StorageLimit::Limited { max_bytes } = policy.limit {
            if max_bytes == 0 || max_bytes > 9_007_199_254_740_991 {
                return Err(AppError::validation(
                    "Enter a positive storage limit, or choose Unlimited",
                ));
            }
        }
        self.context.update(|store| {
            let measured = self.context.repository.message_storage_bytes(store)?;
            let drafts = self.context.repository.draft_storage_bytes(store)?;
            let account = store.accounts.iter_mut().find(|a| a.id == account_id)
                .ok_or_else(|| AppError::missing("Account"))?;
            // «Include Spam and Trash» belongs to Settings → Search: a storage page loaded
            // before that switch moved must not take it back.
            let mut policy = policy.clone();
            policy.include_spam_trash = account.storage_policy.include_spam_trash;
            if account.storage_policy != policy {
                account.offline_sync_complete = false;
                account.offline_sync_error = None;
                // A new limit may leave room: a history stopped for space tries again.
                if account.history.stopped_for_space.take().is_some() {
                    account.history.last_error = None;
                }
            }
            account.storage_policy = policy.clone();
            let proposed = storage::usage_with_storage_bytes(store, account_id, &measured, &drafts)
                .ok_or_else(|| AppError::missing("Account"))?;
            let unique_minimum=storage::usage_with_storage_bytes(store,account_id,&std::collections::HashMap::new(),&drafts).ok_or_else(||AppError::missing("Account"))?;
            if policy.admission_limit_bytes().is_some_and(|limit| unique_minimum.protected_bytes > limit) {
                return Err(AppError::new("storage_protected", format!(
                    "Protected mail and drafts need {:.2} GB before the safety reserve. Choose a larger limit or Unlimited. No data was removed.",
                    unique_minimum.protected_bytes as f64 / 1_000_000_000.0
                )));
            }
            let mut preview = store.clone();
            let eviction = storage::enforce_account_with_storage_bytes(&mut preview, account_id, &measured, &drafts);
            let remaining = measured.into_iter().filter(|(id, _)| !eviction.evicted_ids.contains(id)).collect();
            let remaining_drafts = self.context.repository.draft_storage_bytes(&preview)?;
            let after = storage::usage_with_storage_bytes(&preview, account_id, &remaining, &remaining_drafts)
                .ok_or_else(|| AppError::missing("Account"))?;
            let removed = proposed.used_bytes.saturating_sub(after.used_bytes);
            if removed > 0 && !confirm_cleanup {
                return Err(AppError::new("cleanup_confirmation_required", format!(
                    "This limit removes approximately {:.2} MB of the oldest replaceable local mail data from this account. Mail on Gmail and protected drafts stay unchanged. Apply this limit?",
                    removed as f64 / 1_000_000.0
                )));
            }
            // Shared SQLite shares cannot establish protected physical space.
            // The repository compacts and remeasures before accepting the policy.
            // The preview only obtains confirmation. The repository chooses
            // actual victims one at a time and remeasures shared index files.
            Ok(after)
        })?;
        // Context performs retriable maintenance after commit. Report the latest
        // measured state, including a pending-cleanup marker if maintenance failed.
        self.get(account_id)
    }
}
