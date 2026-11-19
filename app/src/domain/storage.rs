//! Per-account storage decisions. Filesystem measurement belongs to the adapter.
use super::{
    model::{AccountKind, Email, Store},
    undo::{UndoAction, UndoOperation},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const DEFAULT_ACCOUNT_LIMIT_BYTES: u64 = 2_000_000_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum StorageLimit {
    Limited { max_bytes: u64 },
    Unlimited,
}
impl Default for StorageLimit {
    fn default() -> Self {
        Self::Limited {
            max_bytes: DEFAULT_ACCOUNT_LIMIT_BYTES,
        }
    }
}
impl StorageLimit {
    pub fn max_bytes(&self) -> Option<u64> {
        match self {
            Self::Limited { max_bytes } => Some(*max_bytes),
            Self::Unlimited => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountStoragePolicy {
    pub limit: StorageLimit,
    #[serde(default)]
    pub keep_all_offline: bool,
    /// The history also lists Gmail's Spam and Trash (W3.1); off by default.
    #[serde(default)]
    pub include_spam_trash: bool,
}
impl AccountStoragePolicy {
    pub fn legacy() -> Self {
        Self {
            limit: StorageLimit::Unlimited,
            keep_all_offline: false,
            include_spam_trash: false,
        }
    }
    /// Ten percent stays available for commits already admitted and checkpoints.
    pub fn admission_limit_bytes(&self) -> Option<u64> {
        self.limit
            .max_bytes()
            .map(|bytes| bytes.saturating_sub(bytes / 10))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageUsage {
    pub account_id: u32,
    pub policy: AccountStoragePolicy,
    pub metadata_bytes: u64,
    pub content_bytes: u64,
    pub draft_bytes: u64,
    pub extra_bytes: u64,
    pub used_bytes: u64,
    pub protected_bytes: u64,
    pub reclaimable_bytes: u64,
    pub remote_only_messages: usize,
    pub offline_complete: bool,
    pub over_limit: bool,
    pub at_capacity: bool,
    pub cleanup_pending: bool,
    pub index_rebuild_required: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EvictionReport {
    pub account_id: u32,
    pub evicted_ids: Vec<u64>,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    pub blocked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageViolation {
    pub account_id: u32,
    pub message: String,
}

/// Filesystem-owned draft payload not already charged to mail or the outbox.
#[derive(Clone, Debug, Default)]
pub struct DraftStorageBytes {
    pub additional_bytes: u64,
    /// Email path aliases for the same files, which eviction must preserve.
    pub protected_mail_paths: HashSet<String>,
}

/// Apply admission to a transaction's working copy. The repository must discard
/// the working copy on Err, just as for an error from the original operation.
/// Existing over-limit data may shrink; an unrelated account edit never starts
/// cleanup. Changed indexes are supplied by the owning adapter, not read here.
pub fn enforce_changed_accounts(
    before: &Store,
    after: &mut Store,
    message_bytes: &HashMap<u64, u64>,
) -> Result<Vec<EvictionReport>, StorageViolation> {
    let ids: Vec<_> = after.accounts.iter().map(|account| account.id).collect();
    let mut reports = Vec::new();
    for account_id in ids {
        let current = usage_with_message_bytes(after, account_id, message_bytes)
            .expect("listed account exists");
        let previous = usage_with_message_bytes(before, account_id, message_bytes);
        let changed = previous.as_ref().is_none_or(|previous| {
            previous.used_bytes != current.used_bytes || previous.policy != current.policy
        });
        if !changed {
            continue;
        }
        if current.policy.limit.max_bytes() == Some(0) {
            return Err(StorageViolation {
                account_id,
                message: "Choose a positive account storage limit or Unlimited.".into(),
            });
        }
        let previous_bytes = previous.as_ref().map_or(0, |usage| usage.used_bytes);
        let lowered = previous.as_ref().is_some_and(|old| {
            current.policy.limit.max_bytes().is_some_and(|limit| {
                old.policy
                    .limit
                    .max_bytes()
                    .is_none_or(|previous| limit < previous)
            })
        });
        let report = enforce_account_with_message_bytes(after, account_id, message_bytes);
        if current
            .policy
            .limit
            .max_bytes()
            .is_some_and(|limit| report.remaining_bytes > limit)
            && (report.remaining_bytes > previous_bytes || lowered)
        {
            return Err(StorageViolation {
                account_id,
                message: format!(
                    "Account {account_id} has no space for this change without removing protected mail or drafts. Increase its limit or choose Unlimited; this change was not saved."
                ),
            });
        }
        reports.push(report);
    }
    Ok(reports)
}

fn serialized_bytes(value: &impl Serialize) -> u64 {
    serde_json::to_vec_pretty(value).map_or(u64::MAX, |bytes| bytes.len() as u64)
}

fn remote(store: &Store, email: &Email) -> bool {
    email.uid.is_some()
        && store
            .account(email.account_id)
            .is_some_and(|account| account.kind == AccountKind::Gmail)
}

fn protected_ids(store: &Store, account_id: u32) -> HashSet<u64> {
    let mut protected: HashSet<_> = store
        .drafts
        .iter()
        .filter(|draft| draft.account_id == account_id)
        .filter_map(|draft| draft.reply_to)
        .collect();
    protected.extend(
        store
            .outbox
            .iter()
            .filter(|delivery| delivery.draft.account_id == account_id)
            .filter_map(|delivery| delivery.draft.reply_to),
    );
    for pending in &store.undo_actions {
        if let UndoOperation::Mail { email_id, .. } = pending.operation {
            protected.insert(email_id);
        }
    }
    let draft_paths: HashSet<_> = store
        .drafts
        .iter()
        .flat_map(|d| d.attachments.iter())
        .collect();
    for email in store
        .emails
        .iter()
        .filter(|email| email.account_id == account_id)
    {
        if email.attachments.iter().any(|a| {
            a.path
                .as_ref()
                .is_some_and(|path| draft_paths.contains(path))
        }) {
            protected.insert(email.id);
        }
    }
    protected
}

fn eligible(store: &Store, email: &Email, protected: &HashSet<u64>) -> bool {
    remote(store, email)
        && !protected.contains(&email.id)
        && !store
            .account(email.account_id)
            .is_some_and(|a| a.storage_policy.keep_all_offline)
}

fn has_local_state(email: &Email) -> bool {
    email.pinned
        || email.done
        || email.muted
        || email.set_aside
        || email.priority_thread
        || email.snoozed_until.is_some()
        || email.reminder_at.is_some()
        || email.due.is_some()
        || email.assigned_to.is_some()
        || email.returned_at.is_some()
}

fn metadata_bytes(email: &Email) -> u64 {
    let mut metadata = email.clone();
    metadata.body.clear();
    for attachment in &mut metadata.attachments {
        attachment.inline_text = None;
    }
    serialized_bytes(&metadata)
}

/// Serialized account/mail/draft data plus unique referenced attachment sizes.
/// `extra_bytes` accounts for adapter-owned indexes and other measured files.
/// This deliberately does not claim to measure shared JSON indentation, WAL or
/// temporary files: the repository adapter must report those separately.
pub fn usage(store: &Store, account_id: u32) -> Option<StorageUsage> {
    usage_with_extra(store, account_id, 0)
}

pub fn usage_with_extra(store: &Store, account_id: u32, extra_bytes: u64) -> Option<StorageUsage> {
    usage_accounted(store, account_id, extra_bytes, &HashMap::new(), &DraftStorageBytes::default())
}

pub fn usage_with_message_bytes(
    store: &Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
) -> Option<StorageUsage> {
    usage_accounted(store, account_id, 0, message_bytes, &DraftStorageBytes::default())
}

pub fn usage_with_storage_bytes(
    store: &Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
    draft_bytes: &HashMap<u32, DraftStorageBytes>,
) -> Option<StorageUsage> {
    usage_accounted(store, account_id, 0, message_bytes, draft_bytes.get(&account_id).unwrap_or(&DraftStorageBytes::default()))
}

fn usage_accounted(
    store: &Store,
    account_id: u32,
    extra_bytes: u64,
    message_bytes: &HashMap<u64, u64>,
    owned_drafts: &DraftStorageBytes,
) -> Option<StorageUsage> {
    let account = store.account(account_id)?;
    let protected = protected_ids(store, account_id);
    let draft_paths: HashSet<_> = store
        .drafts
        .iter()
        .flat_map(|d| d.attachments.iter())
        .collect();
    let mut metadata = serialized_bytes(account);
    let mut content = 0u64;
    let mut reclaimable = 0u64;
    let mut extra = extra_bytes;
    let mut remote_only_messages = 0usize;
    let mut attachments: HashMap<&str, (u64, bool)> = HashMap::new();
    for email in store
        .emails
        .iter()
        .filter(|email| email.account_id == account_id)
    {
        let base = metadata_bytes(email);
        let payload = serialized_bytes(email).saturating_sub(base);
        metadata = metadata.saturating_add(base);
        content = content.saturating_add(payload);
        let evictable = eligible(store, email, &protected);
        let index_size = message_bytes.get(&email.id).copied().unwrap_or(0);
        extra = extra.saturating_add(index_size);
        if evictable {
            let replaceable = if has_local_state(email) {
                payload
            } else {
                serialized_bytes(email)
            };
            reclaimable = reclaimable
                .saturating_add(replaceable)
                .saturating_add(index_size);
        }
        if email.remote_only {
            remote_only_messages += 1;
        }
        for attachment in &email.attachments {
            if let Some(path) = &attachment.path {
                let entry = attachments.entry(path.as_str()).or_insert((0, true));
                entry.0 = entry.0.max(attachment.size);
                entry.1 &= evictable && !draft_paths.contains(path)
                    && !owned_drafts.protected_mail_paths.contains(path);
            }
        }
    }
    for (size, evictable) in attachments.values() {
        content = content.saturating_add(*size);
        if *evictable {
            reclaimable = reclaimable.saturating_add(*size);
        }
    }
    let draft_bytes = store
        .drafts
        .iter()
        .filter(|d| d.account_id == account_id)
        .fold(owned_drafts.additional_bytes, |size, draft| {
            size.saturating_add(serialized_bytes(draft))
        })
        .saturating_add(
            store
                .outbox
                .iter()
                .filter(|delivery| delivery.draft.account_id == account_id)
                .fold(0u64, |bytes, delivery| {
                    bytes
                        .saturating_add(serialized_bytes(delivery))
                        .saturating_add(delivery.attachment_bytes)
                }),
        );
    for action in &store.undo_actions {
        let owned = match action.operation {
            UndoOperation::Mail { email_id, .. } => store
                .emails
                .iter()
                .any(|e| e.account_id == account_id && e.id == email_id),
            _ => action.operation.draft_id().is_some_and(|id| {
                store
                    .drafts
                    .iter()
                    .any(|d| d.account_id == account_id && d.id == id)
            }),
        };
        if owned {
            metadata = metadata.saturating_add(serialized_bytes(action));
        }
    }
    let used = metadata
        .saturating_add(content)
        .saturating_add(draft_bytes)
        .saturating_add(extra);
    let all_parts_available = store
        .emails
        .iter()
        .filter(|e| e.account_id == account_id)
        .all(|e| {
            !e.remote_only
                && e.attachments
                    .iter()
                    .all(|a| a.path.is_some() || a.inline_text.is_some())
        });
    Some(StorageUsage {
        account_id,
        policy: account.storage_policy.clone(),
        metadata_bytes: metadata,
        content_bytes: content,
        draft_bytes,
        extra_bytes: extra,
        used_bytes: used,
        protected_bytes: used.saturating_sub(reclaimable),
        reclaimable_bytes: reclaimable,
        remote_only_messages,
        offline_complete: account.storage_policy.keep_all_offline
            && account.offline_sync_complete
            && all_parts_available,
        over_limit: account
            .storage_policy
            .limit
            .max_bytes()
            .is_some_and(|limit| used > limit),
        at_capacity: account
            .storage_policy
            .admission_limit_bytes()
            .is_some_and(|limit| used >= limit),
        cleanup_pending: store.storage_maintenance_pending,
        index_rebuild_required: store.storage_index_rebuild_required,
    })
}

pub fn enforce_account(store: &mut Store, account_id: u32) -> EvictionReport {
    enforce_account_with_extra(store, account_id, 0)
}

pub fn enforce_account_with_extra(
    store: &mut Store,
    account_id: u32,
    extra_bytes: u64,
) -> EvictionReport {
    enforce_accounted(store, account_id, extra_bytes, &HashMap::new(), &DraftStorageBytes::default())
}

pub fn enforce_account_with_message_bytes(
    store: &mut Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
) -> EvictionReport {
    enforce_accounted(store, account_id, 0, message_bytes, &DraftStorageBytes::default())
}

pub fn enforce_account_with_storage_bytes(
    store: &mut Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
    draft_bytes: &HashMap<u32, DraftStorageBytes>,
) -> EvictionReport {
    enforce_accounted(store, account_id, 0, message_bytes, draft_bytes.get(&account_id).unwrap_or(&DraftStorageBytes::default()))
}

/// Filesystem adapters remeasure shared files after each victim. A shared
/// SQLite database cannot be credited as physically reclaimed per row.
pub fn enforce_one_with_message_bytes(
    store: &mut Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
) -> EvictionReport {
    enforce_accounted_limit(store, account_id, 0, message_bytes, &DraftStorageBytes::default(), Some(1))
}

pub fn enforce_one_with_storage_bytes(
    store: &mut Store,
    account_id: u32,
    message_bytes: &HashMap<u64, u64>,
    draft_bytes: &HashMap<u32, DraftStorageBytes>,
) -> EvictionReport {
    enforce_accounted_limit(store, account_id, 0, message_bytes, draft_bytes.get(&account_id).unwrap_or(&DraftStorageBytes::default()), Some(1))
}

fn enforce_accounted(
    store: &mut Store,
    account_id: u32,
    extra_bytes: u64,
    message_bytes: &HashMap<u64, u64>,
    owned_drafts: &DraftStorageBytes,
) -> EvictionReport {
    enforce_accounted_limit(store, account_id, extra_bytes, message_bytes, owned_drafts, None)
}

fn enforce_accounted_limit(
    store: &mut Store,
    account_id: u32,
    extra_bytes: u64,
    message_bytes: &HashMap<u64, u64>,
    owned_drafts: &DraftStorageBytes,
    max_evictions: Option<usize>,
) -> EvictionReport {
    let Some(before) = usage_accounted(store, account_id, extra_bytes, message_bytes, owned_drafts) else {
        return EvictionReport {
            account_id,
            ..Default::default()
        };
    };
    let mut report = EvictionReport {
        account_id,
        remaining_bytes: before.used_bytes,
        ..Default::default()
    };
    let Some(limit) = before.policy.admission_limit_bytes() else {
        return report;
    };
    if before.used_bytes <= limit {
        return report;
    }
    let protected = protected_ids(store, account_id);
    let mut candidates: Vec<_> = store
        .emails
        .iter()
        .enumerate()
        .filter(|(_, email)| {
            email.account_id == account_id
                && eligible(store, email, &protected)
                && (max_evictions.is_none() || !email.storage_evicted)
        })
        .map(|(index, email)| (index, email.date_missing, email.date, email.id))
        .collect();
    candidates.sort_unstable_by_key(|&(_, missing, date, id)| {
        (missing, if missing { 0 } else { date }, id)
    });
    let mut path_refs: HashMap<String, (usize, u64)> = HashMap::new();
    for email in store
        .emails
        .iter()
        .filter(|email| email.account_id == account_id)
    {
        for attachment in &email.attachments {
            if let Some(path) = &attachment.path {
                let entry = path_refs.entry(path.clone()).or_default();
                entry.0 += 1;
                entry.1 = entry.1.max(attachment.size);
            }
        }
    }
    let mut draft_paths: HashSet<String> = store
        .drafts
        .iter()
        .flat_map(|d| d.attachments.iter().cloned())
        .collect();
    draft_paths.extend(owned_drafts.protected_mail_paths.iter().cloned());
    let mut used = before.used_bytes;
    let mut removed = HashSet::new();
    let mut remaining_index_bytes = message_bytes.clone();
    for (index, _, _, _) in candidates {
        if used <= limit || max_evictions.is_some_and(|max| report.evicted_ids.len() >= max) {
            break;
        }
        let email = &mut store.emails[index];
        let before_size = serialized_bytes(email);
        let remove_metadata = !has_local_state(email);
        email.body.clear();
        email.remote_only = true;
        email.storage_evicted = true;
        for attachment in &mut email.attachments {
            if let Some(path) = attachment.path.take() {
                if let Some((refs, size)) = path_refs.get_mut(&path) {
                    *refs = refs.saturating_sub(1);
                    if *refs == 0 && !draft_paths.contains(&path) {
                        used = used.saturating_sub(*size);
                    }
                }
                if !store.cache_removals.contains(&path) {
                    store.cache_removals.push(path);
                }
            }
            attachment.inline_text = None;
        }
        let savings = if remove_metadata {
            removed.insert(email.id);
            before_size
        } else {
            before_size.saturating_sub(serialized_bytes(email))
        };
        let index_savings = remaining_index_bytes.remove(&email.id).unwrap_or(0);
        used = used.saturating_sub(savings).saturating_sub(index_savings);
        report.evicted_ids.push(email.id);
    }
    store
        .emails
        .retain(|email| email.account_id != account_id || !removed.contains(&email.id));
    if !report.evicted_ids.is_empty() {
        if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account_id) {
            account.search_index.storage_limited = true;
        }
    }
    let after = usage_accounted(store, account_id, extra_bytes, &remaining_index_bytes, owned_drafts)
        .expect("account retained during cache eviction");
    report.remaining_bytes = after.used_bytes;
    report.freed_bytes = before.used_bytes.saturating_sub(after.used_bytes);
    report.blocked = after.used_bytes > limit;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        model::{Account, Attachment, Draft},
        undo::{UndoAction, UndoStatus},
    };

    fn mailbox() -> Store {
        Store {
            accounts: (1..=2)
                .map(|id| Account {
                    id,
                    kind: AccountKind::Gmail,
                    storage_policy: AccountStoragePolicy::legacy(),
                    ..Default::default()
                })
                .collect(),
            emails: (1..=4)
                .map(|id| Email {
                    id,
                    account_id: if id == 4 { 2 } else { 1 },
                    date: id as i64,
                    message_id: format!("message-{id}"),
                    uid: Some(id as u32),
                    body: "x".repeat(10_000),
                    subject: format!("subject-{id}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn require_reclaim(store: &mut Store, account_id: u32, bytes: u64, index: &HashMap<u64, u64>) {
        let before = usage_with_message_bytes(store, account_id, index)
            .unwrap()
            .used_bytes;
        let target = before.saturating_sub(bytes);
        store
            .accounts
            .iter_mut()
            .find(|a| a.id == account_id)
            .unwrap()
            .storage_policy
            .limit = StorageLimit::Limited {
            max_bytes: target.saturating_mul(10) / 9,
        };
    }

    #[test]
    fn new_accounts_are_limited_but_legacy_deserialization_never_silently_evicts() {
        let new = Account::default();
        assert_eq!(
            new.storage_policy.limit.max_bytes(),
            Some(DEFAULT_ACCOUNT_LIMIT_BYTES)
        );
        let mut old_json = serde_json::to_value(new).unwrap();
        old_json.as_object_mut().unwrap().remove("storage_policy");
        let old: Account = serde_json::from_value(old_json).unwrap();
        assert_eq!(old.storage_policy.limit, StorageLimit::Unlimited);
        let mut store = mailbox();
        let ids: Vec<_> = store.accounts.iter().map(|account| account.id).collect();
        for id in ids {
            enforce_account(&mut store, id);
        }
        assert_eq!(store.emails.len(), 4);
        assert!(store.emails.iter().all(|email| !email.remote_only));
    }

    #[test]
    fn oldest_message_is_removed_only_from_the_account_over_its_limit() {
        let mut store = mailbox();
        store.emails[3].date = -100;
        require_reclaim(&mut store, 1, 1_000, &HashMap::new());
        let report = enforce_account(&mut store, 1);
        assert_eq!(report.evicted_ids, vec![1]);
        assert!(!store.emails.iter().any(|email| email.id == 1));
        assert_eq!(
            store
                .emails
                .iter()
                .find(|email| email.id == 4)
                .unwrap()
                .body
                .len(),
            10_000
        );
        assert!(!report.blocked);
    }

    #[test]
    fn metadata_only_mail_is_a_candidate_and_older_small_mail_is_not_skipped() {
        let mut store = mailbox();
        store.emails[0].body.clear();
        store.emails[0].remote_only = true;
        require_reclaim(&mut store, 1, 3_000, &HashMap::new());
        let report = enforce_account(&mut store, 1);
        assert_eq!(report.evicted_ids, vec![1, 2]);
        assert!(store.emails.iter().any(|email| email.id == 3));
    }

    #[test]
    fn unknown_dates_are_last_and_equal_dates_use_stable_message_id() {
        let mut store = mailbox();
        store.emails[0].date_missing = true;
        store.emails[0].date = -999;
        store.emails[1].date = 10;
        store.emails[2].date = 10;
        store.emails.swap(1, 2);
        require_reclaim(&mut store, 1, 15_000, &HashMap::new());
        assert_eq!(enforce_account(&mut store, 1).evicted_ids, vec![2, 3]);
        assert!(store.emails.iter().any(|email| email.id == 1));
    }

    #[test]
    fn tasks_keep_their_identity_while_payload_and_index_leave_together() {
        let mut store = mailbox();
        store.emails[0].pinned = true;
        store.emails[0].due = Some(200);
        let index = HashMap::from([(1, 7_000), (2, 7_000)]);
        require_reclaim(&mut store, 1, 12_000, &index);
        let report = enforce_account_with_message_bytes(&mut store, 1, &index);
        assert_eq!(report.evicted_ids, vec![1]);
        let task = store.emails.iter().find(|email| email.id == 1).unwrap();
        assert!(task.pinned && task.remote_only && task.storage_evicted);
        assert_eq!(task.due, Some(200));
        assert_eq!(task.subject, "subject-1");
        assert!(task.body.is_empty());
        assert!(report.freed_bytes >= 17_000);
        assert!(!report.blocked);
    }

    #[test]
    fn pending_work_unique_messages_and_offline_archives_are_protected() {
        let mut store = mailbox();
        store.emails[0].uid = None;
        store.drafts.push(Draft {
            id: 50,
            account_id: 1,
            reply_to: Some(2),
            body: "unsent text".into(),
            ..Default::default()
        });
        store.undo_actions.push(UndoAction {
            id: 60,
            operation: UndoOperation::Mail {
                email_id: 3,
                action: "archive".into(),
                value: None,
            },
            subject: "subject-3".into(),
            created_at_ms: 0,
            expires_at_ms: 5_000,
            status: UndoStatus::Pending,
            error: None,
            retry_at_ms: None,
            attempts: 0,
        });
        store.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 1 };
        let report = enforce_account(&mut store, 1);
        assert!(report.blocked && report.evicted_ids.is_empty());
        assert_eq!(store.drafts[0].body, "unsent text");
        store.accounts[1].storage_policy = AccountStoragePolicy {
            limit: StorageLimit::Limited { max_bytes: 1 },
            keep_all_offline: true,
            include_spam_trash: false,
        };
        let report = enforce_account(&mut store, 2);
        assert!(report.blocked && report.evicted_ids.is_empty());
    }

    #[test]
    fn index_usage_and_shared_attachment_bytes_are_counted_once_per_message_or_file() {
        let mut store = mailbox();
        for email in &mut store.emails[..2] {
            email.attachments.push(Attachment {
                name: "file.txt".into(),
                path: Some("cache/same.txt".into()),
                size: 8_000,
                ..Default::default()
            });
        }
        let before = usage(&store, 1).unwrap();
        let accounted =
            usage_with_message_bytes(&store, 1, &HashMap::from([(1, 5_000), (4, 50_000)])).unwrap();
        assert_eq!(accounted.used_bytes, before.used_bytes + 5_000);
        assert_eq!(accounted.extra_bytes, 5_000);
        assert_eq!(before.content_bytes, 38_000);
        store.drafts.push(Draft {
            id: 5,
            account_id: 1,
            attachments: vec!["cache/same.txt".into()],
            ..Default::default()
        });
        require_reclaim(&mut store, 1, 1_000, &HashMap::new());
        assert_eq!(enforce_account(&mut store, 1).evicted_ids, vec![3]);
        assert!(store.cache_removals.is_empty());
    }

    #[test]
    fn offline_complete_requires_provider_proof_and_all_referenced_contents() {
        let mut store = mailbox();
        store.accounts[0].storage_policy.keep_all_offline = true;
        assert!(!usage(&store, 1).unwrap().offline_complete);
        store.accounts[0].offline_sync_complete = true;
        assert!(usage(&store, 1).unwrap().offline_complete);
        store.emails[0].attachments.push(Attachment {
            name: "missing.txt".into(),
            size: 10,
            ..Default::default()
        });
        assert!(!usage(&store, 1).unwrap().offline_complete);
    }

    #[test]
    fn transaction_does_not_clean_an_unchanged_over_limit_account() {
        let mut before = mailbox();
        before.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 1 };
        let mut after = before.clone();
        after.drafts.push(Draft {
            id: 20,
            account_id: 2,
            body: "new work in B".into(),
            ..Default::default()
        });
        let reports = enforce_changed_accounts(&before, &mut after, &HashMap::new()).unwrap();
        assert!(reports.iter().all(|report| report.account_id == 2));
        assert_eq!(
            after
                .emails
                .iter()
                .filter(|email| email.account_id == 1)
                .count(),
            3
        );
    }

    #[test]
    fn transaction_rejects_protected_growth_but_allows_reducing_legacy_over_limit_data() {
        let mut before = mailbox();
        before.accounts[0].storage_policy = AccountStoragePolicy {
            limit: StorageLimit::Limited { max_bytes: 1 },
            keep_all_offline: true,
            include_spam_trash: false,
        };
        before.drafts.push(Draft {
            id: 20,
            account_id: 1,
            body: "draft".repeat(100),
            ..Default::default()
        });
        let mut grows = before.clone();
        grows.drafts[0].body.push_str("more unsaved text");
        let error = enforce_changed_accounts(&before, &mut grows, &HashMap::new()).unwrap_err();
        assert_eq!(error.account_id, 1);
        assert_eq!(grows.emails.len(), before.emails.len());
        let mut shrinks = before.clone();
        shrinks.drafts[0].body.clear();
        assert!(enforce_changed_accounts(&before, &mut shrinks, &HashMap::new()).is_ok());
    }

    #[test]
    fn transaction_makes_room_for_a_draft_without_removing_unique_work() {
        let mut before = mailbox();
        require_reclaim(&mut before, 1, 0, &HashMap::new());
        let mut after = before.clone();
        after.drafts.push(Draft {
            id: 20,
            account_id: 1,
            body: "d".repeat(8_000),
            ..Default::default()
        });
        let reports = enforce_changed_accounts(&before, &mut after, &HashMap::new()).unwrap();
        assert_eq!(reports[0].evicted_ids, vec![1]);
        assert_eq!(after.drafts[0].body.len(), 8_000);
        assert!(!usage(&after, 1).unwrap().over_limit);
        assert_eq!(
            after
                .emails
                .iter()
                .find(|email| email.account_id == 2)
                .unwrap()
                .body
                .len(),
            10_000
        );
    }

    #[test]
    fn the_positional_difference_agrees_with_the_one_by_id() {
        let row = |id: u64| Email { id, subject: format!("Message {id}"), ..Default::default() };
        let shapes = |len: u64| -> Vec<(&'static str, Vec<Email>, Vec<Email>)> {
            let old: Vec<Email> = (1..=len).map(row).collect();
            let edit = |at: usize| { let mut new = old.clone(); new[at].read = true; new };
            let mut appended = edit(1);
            appended.extend([row(len + 10), row(len + 11)]);
            let mut reused = old.clone();
            reused.push(row(2));
            let mut twice = old.clone();
            twice.extend([row(len + 10), row(len + 10)]);
            let mut swapped = old.clone();
            swapped.swap(0, 1);
            let mut middle = old.clone();
            middle.remove(1);
            let mut inserted = old.clone();
            inserted.insert(1, row(len + 10));
            vec![
                ("unchanged", old.clone(), old.clone()),
                ("edited at the end", old.clone(), edit(old.len() - 1)),
                ("edited and appended", old.clone(), appended),
                ("an old id appended again", old.clone(), reused),
                ("a new id appended twice", old.clone(), twice),
                ("dropped from the end", old.clone(), old[..old.len() - 2].to_vec()),
                ("swapped", old.clone(), swapped),
                ("removed in the middle", old.clone(), middle),
                ("inserted in the middle", old.clone(), inserted),
            ]
        };
        // Short lists, and lists long enough to be compared on several threads.
        for len in [5, PARALLEL_DIFF_MIN as u64 + 7] {
            for (shape, old, new) in shapes(len) {
                let by_id = diff_by_id(&old, &new);
                let found = diff_emails(&old, &new);
                assert_eq!(
                    (&found.changed, &found.removed, found.order_kept),
                    (&by_id.changed, &by_id.removed, by_id.order_kept),
                    "{shape}, {len} messages"
                );
            }
        }
        let order_kept = |old: &[Email], new: &[Email]| diff_by_id(old, new).order_kept;
        let old: Vec<Email> = (1..=5).map(row).collect();
        let mut twice = old.clone();
        twice.push(row(9));
        twice.push(row(9));
        assert!(!order_kept(&old, &twice), "a duplicate id keeps nothing");
        let mut moved = old.clone();
        moved.rotate_left(1);
        assert!(!order_kept(&old, &moved), "a new order is rewritten");
        let mut appended = old.clone();
        appended.push(row(6));
        assert!(order_kept(&old, &appended));
    }
}

/// The emails a change touched, found in one pass (W2.3), by position: new
/// or changed rows (index in the new list, index of their previous version)
/// and the positions of the rows that disappeared from the old list.
#[derive(Debug, Default)]
pub struct EmailDiff {
    pub changed: Vec<(usize, Option<usize>)>,
    pub removed: Vec<usize>,
    /// The ids of the new list are unique, and the surviving rows come first,
    /// in their old order, before every new row: a writer that keeps rows in
    /// list order (`seq`) then needs to write the touched rows only.
    pub order_kept: bool,
}

pub fn diff_emails(old: &[Email], new: &[Email]) -> EmailDiff {
    diff_in_place(old, new).unwrap_or_else(|| diff_by_id(old, new))
}

/// `diff_emails` for any shape of change, on one thread.
fn diff_by_id(old: &[Email], new: &[Email]) -> EmailDiff {
    let mut by_id: Option<HashMap<u64, usize>> = None;
    let mut changed = Vec::new();
    let mut seen = 0usize;
    // Order and uniqueness, checked on the way: surviving rows map to strictly
    // increasing old positions (two rows sharing an old id cannot), and no
    // surviving row follows a new one; new rows are checked among themselves
    // (an id of the old list is never new).
    let mut order_kept = true;
    let mut last_old: Option<usize> = None;
    let mut fresh: HashSet<u64> = HashSet::new();
    for (index, email) in new.iter().enumerate() {
        // By position first (the order rarely changes), by id otherwise.
        let previous = match old.get(index) {
            Some(candidate) if candidate.id == email.id => Some(index),
            _ => by_id
                .get_or_insert_with(|| old.iter().enumerate().map(|(i, e)| (e.id, i)).collect())
                .get(&email.id)
                .copied(),
        };
        match previous {
            Some(previous) => {
                order_kept &= fresh.is_empty() && last_old.is_none_or(|last| last < previous);
                last_old = Some(previous);
            }
            None => order_kept &= fresh.insert(email.id),
        }
        seen += usize::from(previous.is_some());
        if previous.is_none_or(|previous| old[previous] != *email) {
            changed.push((index, previous));
        }
    }
    let removed = if seen == old.len() {
        Vec::new()
    } else {
        let present: HashSet<u64> = new.iter().map(|e| e.id).collect();
        old.iter().enumerate().filter(|(_, e)| !present.contains(&e.id)).map(|(i, _)| i).collect()
    };
    EmailDiff { changed, removed, order_kept }
}

/// Lists at least this long are compared on several threads.
const PARALLEL_DIFF_MIN: usize = 20_000;

/// `diff_emails` for the usual shape of a change: every message of the
/// shorter list sits at the same position in both, so rows were only edited,
/// appended with new ids, or dropped from the end. `None` for any other shape.
fn diff_in_place(old: &[Email], new: &[Email]) -> Option<EmailDiff> {
    let common = old.len().min(new.len());
    let appended = &new[common..];
    if !appended.is_empty() {
        // A new row reuses no id, of another new row or of the old list.
        let mut fresh = HashSet::with_capacity(appended.len());
        if !appended.iter().all(|email| fresh.insert(email.id)) {
            return None;
        }
        let newest = old.iter().map(|email| email.id).max();
        if appended.iter().any(|email| newest.is_some_and(|newest| email.id <= newest)) {
            let known: HashSet<u64> = old.iter().map(|email| email.id).collect();
            if appended.iter().any(|email| known.contains(&email.id)) {
                return None;
            }
        }
    }
    let mut changed: Vec<(usize, Option<usize>)> =
        edited(&old[..common], &new[..common])?.into_iter().map(|at| (at, Some(at))).collect();
    changed.extend((common..new.len()).map(|at| (at, None)));
    Some(EmailDiff { changed, removed: (common..old.len()).collect(), order_kept: true })
}

/// Positions where two lists of the same length differ, `None` when one
/// position holds two different ids. Comparing 100,000 messages is, after
/// copying them, the largest cost of a write: a long list is compared in parts
/// on up to four threads.
fn edited(old: &[Email], new: &[Email]) -> Option<Vec<usize>> {
    fn part(offset: usize, old: &[Email], new: &[Email]) -> Option<Vec<usize>> {
        let mut edited = Vec::new();
        for (at, (old, new)) in old.iter().zip(new).enumerate() {
            if old.id != new.id {
                return None;
            }
            if old != new {
                edited.push(offset + at);
            }
        }
        Some(edited)
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4);
    if threads < 2 || old.len() < PARALLEL_DIFF_MIN {
        return part(0, old, new);
    }
    let size = old.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let others: Vec<_> = old
            .chunks(size)
            .zip(new.chunks(size))
            .enumerate()
            .skip(1)
            .map(|(n, (old, new))| scope.spawn(move || part(n * size, old, new)))
            .collect();
        let mut edited = part(0, &old[..size], &new[..size]);
        for other in others {
            match other.join() {
                Ok(found) => edited = edited.zip(found).map(|(mut edited, found)| { edited.extend(found); edited }),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        edited
    })
}

/// `used_bytes` per account of `store`, derived from `previous` (exact for
/// `before`) and the records the change touched, without measuring every
/// message again (W2.3). `None` when the change is not local enough to derive
/// exactly: messages removed, attachment files changed, drafts, outbox or a
/// storage policy changed, accounts added or removed. Callers then measure.
pub fn derived_usage(before: &Store, store: &Store, diff: &EmailDiff, previous: &HashMap<u32, u64>) -> Option<HashMap<u32, u64>> {
    derived_usage_of(before, store, diff, previous, false)
}

/// `derived_usage` where `before.emails` holds only the previous versions of
/// the messages the change touched (`diff` indexes into that list): every other
/// message is the same in `store`, which then tells whose a removed pending
/// action was.
pub fn derived_usage_from_touched(before: &Store, store: &Store, diff: &EmailDiff, previous: &HashMap<u32, u64>) -> Option<HashMap<u32, u64>> {
    derived_usage_of(before, store, diff, previous, true)
}

fn derived_usage_of(before: &Store, store: &Store, diff: &EmailDiff, previous: &HashMap<u32, u64>, touched_only: bool) -> Option<HashMap<u32, u64>> {
    if before.accounts.len() != store.accounts.len() || !diff.removed.is_empty() {
        return None;
    }
    let same = |a: &dyn erased::Json, b: &dyn erased::Json| a.json() == b.json();
    if !same(&before.drafts, &store.drafts) || !same(&before.outbox, &store.outbox) {
        return None;
    }
    let mut delta: HashMap<u32, i128> = HashMap::new();
    for account in &store.accounts {
        let old = before.accounts.iter().find(|old| old.id == account.id)?;
        if old.storage_policy != account.storage_policy || !previous.contains_key(&account.id) {
            return None;
        }
        *delta.entry(account.id).or_default() += i128::from(serialized_bytes(account)) - i128::from(serialized_bytes(old));
    }
    let files = |email: &Email| -> Vec<(String, u64)> {
        email.attachments.iter().filter_map(|a| a.path.clone().map(|path| (path, a.size))).collect()
    };
    for (index, old) in &diff.changed {
        let email = &store.emails[*index];
        let old = old.map(|old| &before.emails[old]);
        match old {
            Some(old) => {
                if old.account_id != email.account_id || files(old) != files(email) {
                    return None;
                }
                *delta.entry(email.account_id).or_default() += i128::from(serialized_bytes(email)) - i128::from(serialized_bytes(old));
            }
            None => {
                if !files(email).is_empty() {
                    return None;
                }
                *delta.entry(email.account_id).or_default() += i128::from(serialized_bytes(email));
            }
        }
    }
    // Pending actions are charged to the account that owns their message or draft.
    let owner = |store: &Store, action: &UndoAction| -> Option<u32> {
        match action.operation {
            UndoOperation::Mail { email_id, .. } => store.emails.iter().find(|e| e.id == email_id).map(|e| e.account_id),
            _ => action.operation.draft_id().and_then(|id| store.drafts.iter().find(|d| d.id == id).map(|d| d.account_id)),
        }
    };
    let key = |action: &UndoAction| serde_json::to_string(action).unwrap_or_default();
    let old_actions: HashSet<String> = before.undo_actions.iter().map(key).collect();
    let new_actions: HashSet<String> = store.undo_actions.iter().map(key).collect();
    for action in &before.undo_actions {
        if !new_actions.contains(&key(action)) {
            let account = owner(before, action).or_else(|| touched_only.then(|| owner(store, action)).flatten());
            if let Some(account) = account {
                *delta.entry(account).or_default() -= i128::from(serialized_bytes(action));
            }
        }
    }
    for action in &store.undo_actions {
        if !old_actions.contains(&key(action)) {
            if let Some(account) = owner(store, action) {
                *delta.entry(account).or_default() += i128::from(serialized_bytes(action));
            }
        }
    }
    Some(
        store
            .accounts
            .iter()
            .map(|account| {
                let used = i128::from(previous[&account.id]) + delta.get(&account.id).copied().unwrap_or(0);
                (account.id, used.clamp(0, i128::from(u64::MAX)) as u64)
            })
            .collect(),
    )
}

mod erased {
    /// A value compared by its JSON form (drafts and deliveries have no `PartialEq`).
    pub trait Json {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}
