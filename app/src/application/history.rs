//! The history backfill (W3.1): batches of All Mail metadata, newest first,
//! written with their cursor in one transaction. Never a body, never a delete.
use super::{AppError, AppResult, ServiceContext, events::AppEvent, ports::{HistoryRequest, LabelState}};
use crate::domain::{
    history::{BATCH_UIDS, UNREAD_NOTICE_THRESHOLD},
    model::{AccountKind, Email, Store},
    undo::UndoOperation,
};
use std::collections::{HashMap, HashSet};

/// Whether the account's history still has batches to read.
pub fn pending(context: &ServiceContext, account_id: u32) -> AppResult<bool> {
    let store = context.read_shared()?;
    Ok(store.account(account_id).is_some_and(|account| {
        account.kind == AccountKind::Gmail
            && context.mail.capabilities(account).sync
            && !account.history.complete
            && account.history.stopped_for_space.is_none()
    }))
}

/// Whether a history job is worth queueing after a sync: the walk while it
/// lasts, then the label refresh of the newest messages (W3.2).
pub fn due(context: &ServiceContext, account_id: u32) -> AppResult<bool> {
    let store = context.read_shared()?;
    Ok(store.account(account_id).is_some_and(|account| {
        account.kind == AccountKind::Gmail && context.mail.capabilities(account).sync
    }))
}

/// The folders Gmail's labels decide; any other folder is the person's own.
const SYSTEM_FOLDERS: [&str; 5] = ["Inbox", "Archive", "Sent", "Trash", "Spam"];

/// Applies the Gmail state of recent messages to the rows that hold them.
/// A row with an action still pending, or changed since `snapshot` (the view
/// the request was planned on), keeps its local state: it is newer.
fn apply_refresh(store: &mut Store, snapshot: &Store, account_id: u32, states: &[LabelState]) -> Vec<u64> {
    if states.is_empty() {
        return Vec::new();
    }
    let by_global: HashMap<u64, &LabelState> = states.iter().map(|state| (state.global_id, state)).collect();
    let pending: HashSet<u64> = store
        .undo_actions
        .iter()
        .filter_map(|action| match &action.operation {
            UndoOperation::Mail { email_id, .. } => Some(*email_id),
            _ => None,
        })
        .collect();
    let seen: HashMap<u64, &Email> = snapshot
        .emails
        .iter()
        .filter(|e| e.account_id == account_id && e.remote_global_id.is_some_and(|g| by_global.contains_key(&g)))
        .map(|e| (e.id, e))
        .collect();
    let mut touched = Vec::new();
    for email in store.emails.iter_mut().filter(|e| e.account_id == account_id) {
        let Some(state) = email.remote_global_id.and_then(|g| by_global.get(&g)) else { continue };
        let unchanged = seen.get(&email.id).is_some_and(|before| {
            before.folder == email.folder && before.labels == email.labels && before.read == email.read
                && before.starred == email.starred && before.replied == email.replied
        });
        if pending.contains(&email.id) || !unchanged {
            continue;
        }
        let placement = crate::domain::gmail_labels::classify(&state.labels);
        if placement.skip {
            continue;
        }
        // Only the places Gmail decides move: a folder of the person's own ("Projects") stays,
        // and labels are only added, because local labels (auto tags, labels not pushed to
        // Gmail) are not in Gmail's list.
        let folder = if SYSTEM_FOLDERS.contains(&email.folder.as_str()) { placement.folder } else { email.folder.clone() };
        let mut labels = email.labels.clone();
        for label in placement.labels {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        let starred = state.starred || placement.starred;
        if email.folder != folder || email.labels != labels || email.read != state.read
            || email.starred != starred || email.replied != state.replied
        {
            email.folder = folder;
            email.labels = labels;
            email.read = state.read;
            email.starred = starred;
            email.replied = state.replied;
            touched.push(email.id);
        }
    }
    touched
}

/// Reads and writes one batch; `Ok(true)` when another one is due right away.
pub fn run_batch(context: &ServiceContext, account_id: u32) -> AppResult<bool> {
    let store = context.read_shared()?;
    let Some(account) = store.account(account_id).cloned() else { return Ok(false) };
    if account.kind != AccountKind::Gmail || !context.mail.capabilities(&account).sync {
        return Ok(false);
    }
    let authenticated = context.account(&store, account_id)?;
    let snapshot = store;
    let mut roles = account.history.requested.clone();
    let walked = account.history.complete || account.history.stopped_for_space.is_some();
    if account.storage_policy.include_spam_trash && walked {
        for role in ["Spam", "Trash"] {
            if !roles.iter().any(|r| r == role) {
                roles.push(role.into());
            }
        }
    }
    let request = HistoryRequest { progress: account.history.clone(), batch_uids: BATCH_UIDS, roles };
    let batch = match context.mail.history_batch(&authenticated, &request) {
        Ok(batch) => batch,
        Err(error) => {
            let message = error.message.clone();
            context.update_with_events(|store| {
                if let Some(current) = store.accounts.iter_mut().find(|a| a.id == account_id) {
                    current.history.last_error = Some(message.clone());
                }
                // The activity closes; Settings → Search shows the reason.
                Ok(((), vec![AppEvent::HistoryProgress {
                    account_id,
                    fetched: account.history.fetched,
                    total: account.history.total,
                    complete: true,
                }]))
            })?;
            // The same failure again (All Mail hidden from IMAP, say) is not announced after
            // every sync: it is already recorded and shown in Settings → Search.
            if account.history.last_error.as_deref() == Some(message.as_str()) {
                return Ok(false);
            }
            return Err(error);
        }
    };
    let mut next = account.history.clone();
    if next.uid_validity != Some(batch.uid_validity) {
        next.restart(batch.uid_validity);
    }
    next.folder = batch.folder.clone();
    next.total = batch.exists;
    let written = context.update_with_events(|store| {
        let current = store
            .accounts
            .iter()
            .find(|a| a.id == account_id && a.email == account.email)
            .ok_or_else(|| AppError::missing("Account"))?
            .history
            .clone();
        // Another writer moved the cursor meanwhile: this batch is stale. A
        // request added meanwhile (a folder, marking read) does not void it.
        if current.cursor() != account.history.cursor() {
            return Ok((None, Vec::new()));
        }
        let (inserted, enriched) = crate::domain::mail_identity::merge_rows(store, account_id, &batch.emails);
        let unread = inserted
            .iter()
            .filter_map(|id| store.email(*id))
            .filter(|e| e.folder == "Inbox" && !e.read)
            .count() as u32;
        let (extra_inserted, extra_enriched) = crate::domain::mail_identity::merge_rows(store, account_id, &batch.extra);
        let mut progress = next.clone();
        if let Some(range) = batch.range {
            progress.advance(range, inserted.len() as u32);
        } else if progress.before_uid == 0 {
            // An empty All Mail: nothing to walk, the history is complete.
            progress.complete = true;
            progress.last_error = None;
        }
        progress.requested = current.requested.iter().filter(|r| !batch.roles_read.contains(r)).cloned().collect();
        progress.mark_read_before = current.mark_read_before.filter(|_| current.mark_read_before != account.history.mark_read_before);
        progress.unread_notice_shown = current.unread_notice_shown;
        progress.unread_inbox = current.unread_inbox.saturating_add(unread);
        // `complete` closes the activity: a walk that is done or waits for space is not running.
        let mut events = vec![AppEvent::HistoryProgress {
            account_id,
            fetched: progress.fetched,
            total: progress.total,
            complete: progress.complete || progress.stopped_for_space.is_some(),
        }];
        // EX-9: many old unread messages in the Inbox; asked once per account.
        if !progress.unread_notice_shown && progress.unread_inbox > UNREAD_NOTICE_THRESHOLD {
            progress.unread_notice_shown = true;
            events.push(AppEvent::HistoryUnread { account_id, count: progress.unread_inbox });
        }
        if let Some(current) = store.accounts.iter_mut().find(|a| a.id == account_id) {
            current.history = progress.clone();
        }
        let mut ids = inserted;
        ids.extend(enriched);
        ids.extend(extra_inserted);
        ids.extend(extra_enriched);
        ids.extend(apply_refresh(store, &snapshot, account_id, &batch.refresh));
        ids.sort_unstable();
        ids.dedup();
        if !ids.is_empty() {
            events.push(AppEvent::EmailChanged { account_id, ids });
            events.push(AppEvent::CountsChanged { account_id });
        }
        Ok((Some(progress), events))
    });
    let progress = match written {
        Ok(Some(progress)) => progress,
        Ok(None) => return Ok(true),
        Err(error) if error.code == "storage_full" => {
            // EX-8: the quota stops the walk; the list keeps what it has.
            let message = error.message.clone();
            context.update_with_events(|store| {
                let oldest = store.emails.iter().filter(|e| e.account_id == account_id && !e.date_missing).map(|e| e.date).min();
                if let Some(current) = store.accounts.iter_mut().find(|a| a.id == account_id) {
                    current.history.stopped_for_space = Some(oldest.unwrap_or(0));
                    current.history.last_error = Some(message.clone());
                }
                Ok(((), vec![AppEvent::HistoryProgress {
                    account_id,
                    fetched: account.history.fetched,
                    total: account.history.total,
                    complete: true,
                }]))
            })?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    Ok(progress.plan(batch.uid_next, BATCH_UIDS).is_some_and(|range| range.older))
}

fn gmail_account(store: &Store, account_id: u32) -> AppResult<()> {
    match store.account(account_id) {
        Some(account) if account.kind == AccountKind::Gmail => Ok(()),
        Some(_) => Err(AppError::validation("Only Gmail accounts have a server history")),
        None => Err(AppError::missing("Account")),
    }
}

/// Asks for the newest messages of "Spam" or "Trash" with the next history
/// batch: opening the folder in the interface. The caller queues the batch.
pub fn request_folder(context: &ServiceContext, account_id: u32, role: &str) -> AppResult<()> {
    if !matches!(role, "Spam" | "Trash") {
        return Err(AppError::validation("Only Spam and Trash are fetched on request"));
    }
    context.update(|store| {
        gmail_account(store, account_id)?;
        let history = &mut store.accounts.iter_mut().find(|a| a.id == account_id).expect("checked").history;
        if !history.requested.iter().any(|r| r == role) {
            history.requested.push(role.into());
        }
        Ok(())
    })
}

/// Lists Spam and Trash with the history of an account, or stops doing so.
pub fn set_include_spam_trash(context: &ServiceContext, account_id: u32, enabled: bool) -> AppResult<()> {
    context.update(|store| {
        gmail_account(store, account_id)?;
        store.accounts.iter_mut().find(|a| a.id == account_id).expect("checked").storage_policy.include_spam_trash = enabled;
        Ok(())
    })
}

/// The choice of the unread notice: Inbox messages unread and older than
/// `days` are marked read now in the list and on Gmail with the next batch.
/// Returns how many rows changed.
pub fn mark_old_unread_read(context: &ServiceContext, account_id: u32, days: u32) -> AppResult<usize> {
    let before = context.clock.now() - i64::from(days) * 86_400;
    context.update_with_events(|store| {
        gmail_account(store, account_id)?;
        let pending: HashSet<u64> = store
            .undo_actions
            .iter()
            .filter_map(|action| match &action.operation {
                UndoOperation::Mail { email_id, .. } => Some(*email_id),
                _ => None,
            })
            .collect();
        let mut ids = Vec::new();
        for email in store.emails.iter_mut().filter(|e| {
            e.account_id == account_id && e.folder == "Inbox" && !e.read && !e.date_missing && e.date < before
        }) {
            if pending.contains(&email.id) {
                continue;
            }
            email.read = true;
            ids.push(email.id);
        }
        store.accounts.iter_mut().find(|a| a.id == account_id).expect("checked").history.mark_read_before = Some(before);
        let count = ids.len();
        let events = if ids.is_empty() {
            Vec::new()
        } else {
            vec![AppEvent::EmailChanged { account_id, ids }, AppEvent::CountsChanged { account_id }]
        };
        Ok((count, events))
    })
}
