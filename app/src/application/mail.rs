use super::{AppError, AppResult, ServiceContext, activity, ports::{RemoteAction, RemoteCursor, RemoteSearchRequest}};
use super::events::AppEvent;
use super::mail_sync_status::{MailSyncStatus, SyncPhase, SyncProgress};
use crate::domain::activity::{ActivityKind, NewActivity, code};
use crate::domain::conversation::{REPLY_CAP, ReplyNode, reply_nodes, reply_tree};
use crate::domain::search;
use crate::domain::{actions, model::Email, rules};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct MailQuery {
    pub folder: String,
    pub account_id: Option<u32>,
    pub search: String,
    pub offset: usize,
    pub limit: usize,
    pub threads: bool,
    pub sort: String,
}

#[derive(Serialize)]
pub struct EmailSummary {
    pub folder: String,
    pub date_missing: bool,
    pub match_reason: String,
    pub match_terms: Vec<String>,
    pub id: u64,
    pub message_ids: Vec<u64>,
    pub account_id: u32,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub snippet: String,
    pub date: i64,
    pub read: bool,
    pub pinned: bool,
    pub done: bool,
    pub category: String,
    pub section: String,
    pub labels: Vec<String>,
    pub attachment_count: usize,
    pub due: Option<i64>,
    /// Root of the conversation set, stable when newer mail arrives; 0 outside the
    /// Conversations mode. Left out of the JSON when empty so a `read_rows` patch, which
    /// carries none of these three, never wipes what `list_mail` gave the row.
    #[serde(skip_serializing_if = "is_zero")]
    pub conversation_id: u64,
    /// The other messages of the conversation in tree order, at most `REPLY_CAP`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<ConversationReply>,
    /// How many more replies `read_replies` can load.
    #[serde(skip_serializing_if = "is_zero_len")]
    pub replies_more: usize,
    /// One of those replies is unread: a closed conversation must not look quiet (ADR-0010
    /// INV-005), and the newest replies, the ones most likely unread, come last in tree order.
    #[serde(skip_serializing_if = "is_false")]
    pub replies_more_unread: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn is_zero(value: &u64) -> bool {
    *value == 0
}
fn is_zero_len(value: &usize) -> bool {
    *value == 0
}
/// A message shown under a conversation's main row (ADR-0010). Display only: the row's
/// `message_ids` stay the messages the open view can act on.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConversationReply {
    pub id: u64,
    pub account_id: u32,
    pub parent_id: Option<u64>,
    pub from_name: String,
    pub from_addr: String,
    pub to: Vec<String>,
    pub snippet: String,
    pub date: i64,
    pub read: bool,
    pub folder: String,
}
impl ConversationReply {
    fn from_node(node: &ReplyNode) -> Self {
        let e = node.email;
        Self {
            id: e.id,
            account_id: e.account_id,
            parent_id: node.parent_id,
            from_name: e.from_name.clone(),
            from_addr: e.from_addr.clone(),
            to: e.to.clone(),
            snippet: list_snippet(e),
            date: e.date,
            read: e.read,
            folder: e.folder.clone(),
        }
    }
}
fn list_snippet(e: &Email) -> String {
    if e.remote_only {
        "Available online · Open to download".into()
    } else {
        e.snippet(140)
    }
}
impl From<&Email> for EmailSummary {
    fn from(e: &Email) -> Self {
        Self {
            folder: e.folder.clone(),
            date_missing: e.date_missing,
            match_reason: String::new(),
            match_terms: vec![],
            id: e.id,
            message_ids: vec![e.id],
            account_id: e.account_id,
            from_name: e.from_name.clone(),
            from_addr: e.from_addr.clone(),
            subject: e.subject.clone(),
            snippet: list_snippet(e),
            date: e.inbox_date(),
            read: e.read,
            pinned: e.pinned,
            done: e.done,
            category: e.category.key().into(),
            section: rules::section_of(e).label().into(),
            labels: e.labels.clone(),
            attachment_count: e.attachments.len(),
            due: e.due,
            conversation_id: 0,
            replies: Vec::new(),
            replies_more: 0,
            replies_more_unread: false,
        }
    }
}
#[derive(Serialize)]
pub struct MailPage {
    pub items: Vec<EmailSummary>,
    pub total: usize,
    pub next_offset: Option<usize>,
    pub search: Option<SearchInfo>,
    /// Unread messages of the whole Inbox view (not of this page), for its headline;
    /// `None` for any other folder and for a search.
    pub unread_total: Option<usize>,
    /// Rows of the Priority group in the whole Inbox view: conversations in the Conversations
    /// mode, messages in the Email mode; `None` where `unread_total` is.
    pub priority_total: Option<usize>,
}
/// One row as `read_rows` answers it (controller contract for the merged UI of
/// Task 2.3): the summary `list_mail` returns plus the three flags that decide
/// Inbox, Tasks and Snoozed membership. They serialize as `null` / `false`
/// when unset — the key is never absent — so patching a row by id can never
/// put a just-snoozed message back in the Inbox (review 2.3, Important 2).
#[derive(Serialize)]
pub struct MailRow {
    #[serde(flatten)]
    pub summary: EmailSummary,
    pub snoozed_until: Option<i64>,
    pub set_aside: bool,
    pub reminder_at: Option<i64>,
}
#[derive(Serialize)]
pub struct SearchInfo {
    pub timezone: String,
    pub locale: String,
    pub coverage: String,
    pub notice: Option<String>,
    pub remote_available: bool,
}
#[derive(Serialize)]
pub struct SearchPerson {
    pub name: String,
    pub address: String,
}
pub struct AttachmentDocument {
    pub name: String,
    pub bytes: Vec<u8>,
}
#[derive(Default, Serialize)]
pub struct SyncReport {
    pub added: usize,
    pub errors: Vec<String>,
}

/// Results per account of a provider search.
pub const REMOTE_PAGE: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountCursor {
    pub account_id: u32,
    pub cursor: Option<RemoteCursor>,
}
/// Continues a provider search: one cursor per account that still has results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoteSearchCursor {
    pub accounts: Vec<AccountCursor>,
}
#[derive(Default, Serialize)]
pub struct RemoteSearchReport {
    pub added: usize,
    pub errors: Vec<String>,
    /// Rows the server found so far for this search, local filter applied.
    pub ids: Vec<u64>,
    /// Matches on the server (before any local filter).
    pub total: u32,
    pub next: Option<RemoteSearchCursor>,
    /// The query exists only on this computer: nothing was asked to the server.
    pub local_only: bool,
    pub notice: Option<String>,
}
/// What the server said about one search, kept in memory for `list`.
#[derive(Clone, Debug, Default)]
struct ServerResult {
    ids: Vec<u64>,
    total: u32,
    complete: bool,
    /// The last request failed for every account: nothing to report as results.
    failed: bool,
    local_filter: bool,
    at: i64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Eml,
    Mbox,
}

#[derive(Debug)]
pub struct MailExport {
    pub filename: String,
    pub bytes: Vec<u8>,
    pub count: usize,
}

fn normalized_person(value: &str) -> (String, String) {
    crate::domain::contact_view::split_person(value)
}


fn matching_snippet(email: &Email, words: &[String], it: bool) -> (String, String) {
    let excerpt = |text: &str| {
        let lower = text.to_lowercase();
        let pos = words
            .iter()
            .filter_map(|w| lower.find(&w.to_lowercase()))
            .min()
            .unwrap_or(0);
        let char_pos = lower
            .chars()
            .scan(0usize, |offset, c| {
                let start = *offset;
                *offset += c.len_utf8();
                Some(start)
            })
            .take_while(|offset| *offset < pos)
            .count();
        let start = char_pos.saturating_sub(45);
        let fragment: String = text.chars().skip(start).take(180).collect();
        format!(
            "{}{}{}",
            if start > 0 { "…" } else { "" },
            fragment.split_whitespace().collect::<Vec<_>>().join(" "),
            if text.chars().count() > start + 180 {
                "…"
            } else {
                ""
            }
        )
    };
    if !words.is_empty()
        && words
            .iter()
            .any(|w| email.body.to_lowercase().contains(&w.to_lowercase()))
    {
        return (
            excerpt(&email.body),
            if it { "Nel testo" } else { "In message text" }.into(),
        );
    }
    for attachment in &email.attachments {
        if words.is_empty() {
            break;
        }
        if let Some(text) = &attachment.inline_text {
            if words
                .iter()
                .any(|w| text.to_lowercase().contains(&w.to_lowercase()))
            {
                return (
                    excerpt(&text),
                    format!(
                        "{}: {}",
                        if it { "Allegato" } else { "Attachment" },
                        attachment.name
                    ),
                );
            }
        }
    }
    (
        email.snippet(180),
        if it {
            "Corrisponde ai filtri"
        } else {
            "Matches the filters"
        }
        .into(),
    )
}

#[derive(Clone)]
pub struct MailService {
    context: Arc<ServiceContext>,
    search_index: Option<Arc<dyn super::ports::SearchGateway>>,
    /// One lock per account (ADR-0007). It serializes the short read and write
    /// phases of mail operations on that account and is never held across a
    /// provider call (INV-001). Entries are created on first use and kept.
    operation: Arc<Mutex<std::collections::HashMap<u32, Arc<Mutex<()>>>>>,
    /// Messages whose mutating action is between plan and commit. The account lock
    /// is free during the provider call, so this keeps two actions on one message apart.
    in_flight: Arc<Mutex<std::collections::HashSet<u64>>>,
    sync_progress: SyncProgress,
    search_cancel: Arc<AtomicBool>,
    search_running: Arc<AtomicBool>,
    /// The rows the provider search found, per search (`search_key`).
    server_results: Arc<Mutex<std::collections::HashMap<String, ServerResult>>>,
    /// Messages in All Mail per account, from the last provider search.
    mailbox_totals: Arc<Mutex<std::collections::HashMap<u32, u32>>>,
    reader_payloads: Arc<Mutex<std::collections::VecDeque<Email>>>,
    reader_locks: Arc<Mutex<std::collections::HashMap<u64, std::sync::Weak<Mutex<()>>>>>,
    reader_active: Arc<AtomicUsize>,
    prefetch_queue: Arc<Mutex<PrefetchQueue>>,
    /// The accounts whose sync merged in this process: from their next sync on, new mail is an arrival
    /// (`AppEvent::MailArrived`). The first one only sets the baseline, so neither a mailbox just added
    /// nor the mail that came while Qamille was closed is announced.
    synced: Arc<Mutex<std::collections::HashSet<u32>>>,
    /// The accounts whose expired sign-in `AppEvent::SignInExpired` announced, until a sync works again.
    sign_in_expired: Arc<Mutex<std::collections::HashSet<u32>>>,
}
/// How much earlier than the account's last sync a new message may have been received (IMAP
/// INTERNALDATE, `Email::received_at`) and still be an arrival: what Gmail received while the previous
/// sync was downloading, or with the clocks apart. Mail received earlier is the sync still bringing in
/// the mailbox (an offline copy being prepared), never announced.
const ARRIVAL_SLACK_SECS: i64 = 15 * 60;
/// One of the account sets of `MailService`; a poisoned lock still holds a usable set.
fn lock_set(set: &Mutex<std::collections::HashSet<u32>>) -> std::sync::MutexGuard<'_, std::collections::HashSet<u32>> {
    set.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
#[derive(Default)]
struct PrefetchQueue { ids: std::collections::VecDeque<u64>, running: bool }
/// What `plan_actions` decided under the account lock. The provider phase and the
/// commit work from this alone, so the store is not re-read between them.
struct ActionPlan {
    id: u64,
    account_id: u32,
    commands: Vec<(String, Option<String>)>,
    pending_ids: Vec<u64>,
    now: i64,
    time_zone: String,
    locale: String,
    /// The message as it was when planned.
    email: Email,
    /// Provider operations in the order to run them; empty for a local-only action.
    remote: Vec<RemoteAction>,
    /// The account with its credentials. `Some` exactly when `remote` is not empty.
    authenticated: Option<crate::domain::model::Account>,
    /// `remote` ends in a move, so the old folder UID stops being valid.
    moved: bool,
}
/// What a sync needs from the store, read under the account lock so the download
/// itself can run without it.
struct SyncSnapshot {
    /// The account as stored; the merge checks its storage settings did not change.
    account: crate::domain::model::Account,
    /// The account with its credentials. A sign-in problem is a sync error of this
    /// account, reported like a failed download, so it travels as a result.
    authenticated: AppResult<crate::domain::model::Account>,
    max_bytes: Option<u64>,
    /// Every message the store knew when the sync started.
    known: Vec<Email>,
}
/// Marks a message as having an action in flight; released on drop, so an error or a
/// panic cannot leave the message blocked.
struct InFlight {
    set: Arc<Mutex<std::collections::HashSet<u64>>>,
    id: u64,
}
impl Drop for InFlight {
    fn drop(&mut self) {
        self.set.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}
/// What the provider phase did, for `commit_actions`.
struct RemoteOutcome {
    moved: bool,
}
impl MailService {
    pub fn sync_status(&self) -> MailSyncStatus {
        self.sync_progress.snapshot()
    }
    /// The mail lock of one account. Hold it only for a read or a write of the
    /// store, never across a call to the provider.
    fn account_lock(&self, account_id: u32) -> Arc<Mutex<()>> {
        // The map only holds `Arc`s, so a panic elsewhere cannot leave it half-updated.
        let mut locks = self.operation.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(account_id).or_default().clone()
    }
    /// Test probe for ADR-0007 INV-001 (no network under a lock): is the mail lock
    /// of `account_id` held right now, by any thread? Not `cfg(test)`: integration
    /// tests link the library without that flag.
    #[doc(hidden)]
    pub fn lock_held(&self, account_id: u32) -> bool {
        let lock = self
            .operation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&account_id)
            .cloned();
        lock.is_some_and(|lock| {
            matches!(lock.try_lock(), Err(std::sync::TryLockError::WouldBlock))
        })
    }
    pub fn with_search_index(mut self, search:Arc<dyn super::ports::SearchGateway>)->Self {self.search_index=Some(search);self}
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self {
            context,
            search_index: None,
            operation: Arc::new(Mutex::new(std::collections::HashMap::new())),
            in_flight: Arc::default(),
            sync_progress: SyncProgress::default(),
            search_cancel: Arc::new(AtomicBool::new(false)),
            search_running: Arc::new(AtomicBool::new(false)),
            server_results: Arc::default(),
            mailbox_totals: Arc::default(),
            reader_payloads: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            reader_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reader_active: Arc::new(AtomicUsize::new(0)),
            prefetch_queue: Arc::new(Mutex::new(PrefetchQueue::default())),
            synced: Arc::default(),
            sign_in_expired: Arc::default(),
        }
    }
    pub fn list(&self, q: MailQuery) -> AppResult<MailPage> {
        // The shared snapshot, copied only when something must change in it:
        // pending actions to project, regional fallbacks, attachment text.
        let shared = self.context.read_shared()?;
        let now = self.context.clock.now();
        // The regional values resolved by the clock (the system ones for empty settings) are
        // passed to the parser and the snippets: no copy of the store just to hold them.
        let regional = self.context.clock.regional(&shared.settings);
        let parse = |store: &crate::model::Store| {
            search::ParsedQuery::parse_regional(store, &q.search, now, q.account_id, &regional.time_zone, &regional.locale)
        };
        let wants_text = !q.search.trim().is_empty() && parse(&shared).is_ok_and(|p| p.needs_attachment_content());
        let mut owned = (crate::domain::undo::has_pending(&shared) || wants_text).then(|| {
            let mut store = crate::domain::undo::project((*shared).clone());
            store.settings.time_zone = regional.time_zone.clone();
            store.settings.locale = regional.locale.clone();
            store
        });
        if let Some(store) = owned.as_mut().filter(|_| wants_text) {
            for email in store
                .emails
                .iter_mut()
                .filter(|e| q.account_id.is_none_or(|id| id == e.account_id))
            {
                for attachment in &mut email.attachments {
                    if attachment.inline_text.is_none() {
                        attachment.inline_text =
                            self.context.attachments.read_text(attachment).ok();
                    }
                }
            }
        }
        let store: &crate::model::Store = owned.as_ref().unwrap_or(&shared);
        let parsed = if q.search.trim().is_empty() {
            None
        } else {
            Some(
                parse(store)
                    .map_err(AppError::validation)?,
            )
        };
        let search_ids = if parsed.is_some() {
            let indexed=match &self.search_index {Some(index)=>index.lexical_search(&store,&q.search,q.account_id,now,&q.sort)?,None=>None};
            let mut ids = match indexed {Some(ids)=>ids,None=>search::execute(&store,&q.search,q.account_id,now,&q.sort).map_err(AppError::validation)?};
            // Rows the provider search found for this very search: their body is
            // not on this computer, so only the server could tell they match.
            let found = self.server_results.lock().ok().and_then(|results| results.get(&self.search_key(&store, &q.search, q.account_id)).map(|r| r.ids.clone()));
            if let Some(found) = found {
                let known: std::collections::HashSet<u64> = ids.iter().copied().collect();
                // Local state still decides: a result moved to the Trash here, or no
                // longer a task, leaves the results like any other row.
                let mut extra: Vec<&Email> = found.iter().filter(|id| !known.contains(id)).filter_map(|id| store.email(*id))
                    .filter(|e| q.account_id.is_none_or(|a| a == e.account_id))
                    .filter(|e| parsed.as_ref().is_none_or(|p| p.server_result_matches(e))).collect();
                extra.sort_by_key(|e| (std::cmp::Reverse(e.date), std::cmp::Reverse(e.id)));
                ids.extend(extra.into_iter().map(|e| e.id));
                if matches!(q.sort.as_str(), "date" | "newest" | "oldest") {
                    let dates: std::collections::HashMap<u64, i64> = ids.iter().filter_map(|id| store.email(*id).map(|e| (*id, e.date))).collect();
                    ids.sort_by_key(|id| (std::cmp::Reverse(dates.get(id).copied().unwrap_or(0)), std::cmp::Reverse(*id)));
                    if q.sort == "oldest" { ids.reverse(); }
                }
            }
            Some(ids)
        } else {
            None
        };
        let search_ranks = search_ids.as_ref().map(|ids| {
            ids.iter()
                .enumerate()
                .map(|(rank, id)| (*id, rank))
                .collect::<std::collections::HashMap<_, _>>()
        });
        let search_info = parsed.as_ref().map(|parsed| {
            let mut info = self.search_info(&store, &q.search, q.account_id, now, parsed);
            if regional.warning.is_some() {
                info.notice = regional.warning.clone();
            }
            info
        });
        let summarize = |email: &Email| {
            let mut summary = EmailSummary::from(email);
            if let Some(parsed) = &parsed {
                summary.date = email.date;
                summary.match_terms = parsed.words();
                let (snippet, reason) = matching_snippet(
                    email,
                    &summary.match_terms,
                    regional.locale.starts_with("it"),
                );
                summary.snippet = snippet;
                summary.match_reason = reason;
            }
            summary
        };
        let mut mails: Vec<&Email> = store
            .emails
            .iter()
            .filter(|e| {
                if q.account_id.is_some_and(|a| a != e.account_id) {
                    return false;
                }
                if let Some(ranks) = &search_ranks {
                    return ranks.contains_key(&e.id);
                }
                match q.folder.as_str() {
                    "" | "Inbox" => e.folder == "Inbox" && !e.done && !e.is_snoozed(now),
                    "Tasks" => {
                        e.is_task() && !e.done && !matches!(e.folder.as_str(), "Trash" | "Spam")
                    }
                    "Snoozed" => {
                        e.is_snoozed(now) && !matches!(e.folder.as_str(), "Trash" | "Spam")
                    }
                    f if f.starts_with("label:") => {
                        e.labels.iter().any(|l| l == &f[6..])
                            && !matches!(e.folder.as_str(), "Trash" | "Spam")
                    }
                    f => e.folder.eq_ignore_ascii_case(f),
                }
            })
            .collect();
        if let Some(ranks) = &search_ranks {
            mails.sort_by_key(|e| ranks.get(&e.id));
        } else {
            mails.sort_by_key(|e| (std::cmp::Reverse(e.inbox_date()), std::cmp::Reverse(e.id)));
        }
        let limit = if q.limit == 0 {
            60
        } else {
            q.limit.clamp(1, 100)
        };
        let end = q.offset.saturating_add(limit);
        let inbox_counts = search_ranks.is_none() && matches!(q.folder.as_str(), "" | "Inbox");
        let unread_total = inbox_counts.then(|| mails.iter().filter(|e| !e.read).count());
        if !q.threads {
            let total = mails.len();
            let priority_total = inbox_counts.then(|| {
                mails
                    .iter()
                    .filter(|e| rules::section_of(e) == rules::Section::Priority)
                    .count()
            });
            return Ok(MailPage {
                items: mails
                    .into_iter()
                    .skip(q.offset)
                    .take(limit)
                    .map(summarize)
                    .collect(),
                total,
                next_offset: (end < total).then_some(end),
                search: search_info,
                unread_total,
                priority_total,
            });
        }
        let conversations = crate::domain::mail_export::conversation_ids(&store.emails);
        let mut summaries: Vec<EmailSummary> = Vec::new();
        if q.threads {
            let mut positions = std::collections::HashMap::new();
            for email in mails {
                let group = conversations[&email.id];
                if let Some(&index) = positions.get(&group) {
                    let summary: &mut EmailSummary = &mut summaries[index];
                    summary.message_ids.push(email.id);
                    summary.read &= email.read;
                    summary.pinned |= email.pinned;
                    summary.done &= email.done;
                    summary.attachment_count += email.attachments.len();
                    for label in &email.labels {
                        if !summary.labels.contains(label) {
                            summary.labels.push(label.clone());
                        }
                    }
                    let section = rules::section_of(email).label();
                    let rank = |s: &str| match s {
                        "Pins" => 0,
                        "Priority" => 1,
                        "New" => 2,
                        "Notifications" => 3,
                        "Newsletters" => 4,
                        _ => 5,
                    };
                    if rank(section) < rank(&summary.section) {
                        summary.section = section.into();
                    }
                } else {
                    positions.insert(group, summaries.len());
                    summaries.push(summarize(email));
                }
            }
        }
        let total = summaries.len();
        let priority_total = inbox_counts.then(|| {
            summaries
                .iter()
                .filter(|s| s.section == rules::Section::Priority.label())
                .count()
        });
        let mut items: Vec<EmailSummary> =
            summaries.into_iter().skip(q.offset).take(limit).collect();
        for item in &mut items {
            item.conversation_id = conversations[&item.id];
        }
        // The other messages of each conversation hang under the rows of this page only, and
        // a search stays a flat list of matches (ADR-0010).
        if search_ranks.is_none() {
            let mut groups: std::collections::HashMap<u64, Vec<&Email>> =
                std::collections::HashMap::new();
            for email in &store.emails {
                groups.entry(conversations[&email.id]).or_default().push(email);
            }
            for item in &mut items {
                let group = &groups[&item.conversation_id];
                let Some(main) = group.iter().find(|e| e.id == item.id) else {
                    continue;
                };
                let nodes = reply_tree(main, group, &q.folder);
                item.replies_more = nodes.len().saturating_sub(REPLY_CAP);
                item.replies_more_unread = nodes.iter().skip(REPLY_CAP).any(|n| !n.email.read);
                item.replies = nodes
                    .iter()
                    .take(REPLY_CAP)
                    .map(ConversationReply::from_node)
                    .collect();
            }
        }
        Ok(MailPage {
            items,
            total,
            next_offset: (end < total).then_some(end),
            search: search_info,
            unread_total,
            priority_total,
        })
    }

    /// The current rows for the ids an `email-changed` event named, so the UI
    /// patches exactly them instead of reloading the list (ADR-0007 decision 6).
    /// Answered in the requested order, one row per id, at most the 100 ids of
    /// a list page; ids the projected store no longer holds — deleted or never
    /// existing — are omitted, never answered as ghost rows.
    pub fn read_rows(&self, ids: Vec<u64>) -> AppResult<Vec<MailRow>> {
        let store = crate::domain::undo::project(self.context.read()?);
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            if seen.len() > 100 {
                break;
            }
            if let Some(email) = store.email(id) {
                rows.push(MailRow {
                    summary: EmailSummary::from(email),
                    snoozed_until: email.snoozed_until,
                    set_aside: email.set_aside,
                    reminder_at: email.reminder_at,
                });
            }
        }
        Ok(rows)
    }

    fn search_key(
        &self,
        store: &crate::model::Store,
        text: &str,
        account_id: Option<u32>,
    ) -> String {
        let mut accounts: Vec<_> = store
            .accounts
            .iter()
            .filter(|a| account_id.is_none_or(|id| id == a.id))
            .map(|a| (a.id, a.email.as_str(), format!("{:?}", a.kind)))
            .collect();
        accounts.sort();
        let regional = self.context.clock.regional(&store.settings);
        format!(
            "{accounts:?}:{}:{}:{text}",
            regional.time_zone, regional.locale
        )
    }
    fn search_info(
        &self,
        store: &crate::model::Store,
        text: &str,
        account_id: Option<u32>,
        _now: i64,
        parsed: &search::ParsedQuery,
    ) -> SearchInfo {
        let regional = self.context.clock.regional(&store.settings);
        let locale = regional.locale;
        let it = locale.starts_with("it");
        let remote = store.accounts.iter().any(|a| {
            self.context.mail.capabilities(a).remote_search
                && account_id.is_none_or(|id| id == a.id)
        });
        let result = self
            .server_results
            .lock()
            .ok()
            .and_then(|results| results.get(&self.search_key(store, text, account_id)).cloned());
        let checked = result.as_ref().is_some_and(|r| r.complete);
        let has_unreadable = parsed.needs_content()
            && store
                .emails
                .iter()
                .filter(|e| account_id.is_none_or(|id| id == e.account_id))
                .any(|e| {
                    e.remote_only
                        || (parsed.needs_attachment_content()
                            && e.attachments
                                .iter()
                                .any(|a| a.inline_text.as_ref().is_none_or(|t| t.is_empty())))
                });
        let scope: Vec<u32> = store
            .accounts
            .iter()
            .filter(|a| account_id.is_none_or(|id| id == a.id))
            .map(|a| a.id)
            .collect();
        let on_server = self.mailbox_totals.lock().ok().and_then(|totals| {
            scope.iter().map(|id| totals.get(id).copied()).sum::<Option<u32>>()
        });
        let local = store.emails.iter().filter(|e| scope.contains(&e.account_id)).count();
        // W3.3: every Gmail account in scope has listed its whole history, so
        // sender, subject and dates cover all the mail; text only what is indexed.
        let history_complete = remote
            && store
                .accounts
                .iter()
                .filter(|a| scope.contains(&a.id) && self.context.mail.capabilities(a).remote_search)
                .all(|a| a.history.complete);
        let indexed = || {
            store
                .emails
                .iter()
                .filter(|e| {
                    scope.contains(&e.account_id)
                        && e.search_index.version == crate::domain::indexing::VERSION
                        && !e.search_index.partial
                })
                .count()
        };
        let text_missing = history_complete && parsed.needs_content() && indexed() < local;
        let notice = if let Some(warning) = regional.warning {
            Some(warning)
        } else if let Some(result) = result.as_ref().filter(|r| remote && !r.failed) {
            Some(Self::server_notice(it, result))
        } else if history_complete {
            text_missing.then(|| {
                let (indexed, local) = (crate::domain::regional::group_digits(indexed() as u64, it), crate::domain::regional::group_digits(local as u64, it));
                if it { format!("Mittente, oggetto e date: tutta la posta. Testo: {indexed} messaggi su {local}.") } else { format!("Sender, subject and dates: all mail. Text: {indexed} of {local} messages.") }
            })
        } else if let (true, Some(total)) = (remote, on_server) {
            let (local, total) = (crate::domain::regional::group_digits(local as u64, it), crate::domain::regional::group_digits(u64::from(total), it));
            Some(if it { format!("Cerchi in {local} messaggi su {total} di questo account.") } else { format!("Searching {local} of {total} messages in this account.") })
        } else if remote {
            Some(if it { "Risultati locali: parte della posta sul server potrebbe mancare. Cerca sul server per completare questa ricerca." } else { "Local results: some mail on the server may be missing. Search the server to complete this search." }.into())
        } else if has_unreadable {
            Some(if it { "Risultati parziali: alcuni messaggi o allegati non hanno testo ricercabile disponibile." } else { "Partial results: some message or attachment text is unavailable." }.into())
        } else {
            None
        };
        SearchInfo {
            timezone: regional.time_zone,
            locale,
            coverage: if !checked && ((remote && (!history_complete || text_missing)) || (!history_complete && has_unreadable)) {
                "partial"
            } else {
                "complete"
            }
            .into(),
            notice,
            remote_available: remote,
        }
    }
    pub fn search_suggestions(&self, account_id: Option<u32>) -> AppResult<Vec<SearchPerson>> {
        let store = self.context.read_shared()?;
        let mut people: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        let seen = crate::domain::contact_view::observed_people(&store.emails);
        for person in seen.into_iter().filter(|p| account_id.is_none_or(|id| id == p.account_id)) {
            let name = people.entry(person.address).or_default();
            if name.is_empty() {
                *name = person.name;
            }
        }
        Ok(people
            .into_iter()
            .map(|(address, name)| SearchPerson { name, address })
            .collect())
    }
    pub fn cancel_remote_search(&self) {
        self.search_cancel.store(true, Ordering::Relaxed);
    }
    /// Searches the provider with the translated query, one page of
    /// `REMOTE_PAGE` per account, and keeps every result as an ordinary row
    /// (metadata only, `remote_only`). `cursor` continues a previous call.
    pub fn search_remote(
        &self,
        text: &str,
        account_id: Option<u32>,
        cursor: Option<RemoteSearchCursor>,
    ) -> AppResult<RemoteSearchReport> {
        if self
            .search_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AppError::validation("A server search is already running"));
        }
        struct Running<'a>(&'a AtomicBool);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _running = Running(&self.search_running);
        self.search_cancel.store(false, Ordering::Relaxed);
        let mut store = self.context.read()?;
        let regional = self.context.clock.regional(&store.settings);
        store.settings.time_zone = regional.time_zone.clone();
        store.settings.locale = regional.locale.clone();
        let it = regional.locale.starts_with("it");
        let now = self.context.clock.now();
        let parsed = search::ParsedQuery::parse_scoped(&store, text, now, account_id)
            .map_err(AppError::validation)?;
        let key = self.search_key(&store, text, account_id);
        let translated = crate::domain::search_remote::to_gmail_query(&parsed);
        let mut report = RemoteSearchReport::default();
        let Some(raw) = translated.raw.clone().filter(|_| !translated.local_only) else {
            // EX-4: nothing of this query exists on the server.
            report.local_only = true;
            report.notice = Some(if it {
                "Questa ricerca riguarda dati che esistono solo su questo computer.".into()
            } else {
                "This search is about data that exists only on this computer.".into()
            });
            return Ok(report);
        };
        let allowed = parsed.accounts();
        let continuing = cursor.is_some();
        let mut cursors: std::collections::HashMap<u32, Option<RemoteCursor>> = match cursor {
            Some(cursor) => cursor.accounts.into_iter().map(|a| (a.account_id, a.cursor)).collect(),
            None => store
                .accounts
                .iter()
                .filter(|a| {
                    self.context.mail.capabilities(a).remote_search
                        && account_id.is_none_or(|id| id == a.id)
                        && allowed.as_ref().is_none_or(|ids| ids.contains(&a.id))
                })
                .map(|a| (a.id, None))
                .collect(),
        };
        let mut previous = if continuing {
            self.server_results
                .lock()
                .map_err(|_| AppError::storage("Search state unavailable"))?
                .get(&key)
                .cloned()
                .unwrap_or_default()
        } else {
            ServerResult::default()
        };
        let mut next = Vec::new();
        let mut ids = Vec::new();
        let mut total = 0u32;
        let mut failed = false;
        let mut accounts: Vec<_> = cursors.keys().copied().collect();
        accounts.sort_unstable();
        for id in accounts {
            let start = cursors.remove(&id).flatten();
            if self.search_cancel.load(Ordering::Relaxed) {
                report.errors.push("Server search cancelled".into());
                failed = true;
                break;
            }
            let Some(account) = store.account(id).cloned() else { continue };
            let request = RemoteSearchRequest {
                raw: raw.clone(),
                scopes: translated.scopes.clone(),
                cursor: start,
                page_size: REMOTE_PAGE,
            };
            let page = self
                .context
                .account(&store, id)
                .and_then(|a| self.context.mail.search(&a, &request, &self.search_cancel));
            let page = match page {
                Ok(page) => page,
                Err(error) => {
                    failed = true;
                    report.errors.push(format!("{}: {}", account.email, error.message));
                    // «Altri risultati» retries this account from where it stood.
                    if continuing {
                        next.push(AccountCursor { account_id: id, cursor: request.cursor.clone() });
                    }
                    continue;
                }
            };
            total = total.saturating_add(page.total);
            if let Ok(mut totals) = self.mailbox_totals.lock() {
                totals.insert(id, page.mailbox_total);
            }
            if page.next.is_some() {
                next.push(AccountCursor { account_id: id, cursor: page.next.clone() });
            }
            let (added, rows) = self.context.update(|current| {
                if current.account(id).is_none_or(|a| a.email != account.email) {
                    return Ok((0, Vec::new()));
                }
                let mut added = 0;
                let mut rows = Vec::new();
                for mut email in page.emails.clone() {
                    email.account_id = id;
                    if let Some(old) = current
                        .emails
                        .iter_mut()
                        .find(|old| old.account_id == id && crate::domain::mail_identity::same_message(old, &email))
                    {
                        crate::domain::mail_identity::merge_remote(old, &email);
                        rows.push(old.id);
                        continue;
                    }
                    email.id = current.new_id();
                    for (index, attachment) in email.attachments.iter_mut().enumerate() {
                        attachment.id = email.id.saturating_mul(1000).saturating_add(index as u64);
                        attachment.path = None;
                        attachment.inline_text = None;
                    }
                    email.to = email.to.iter().map(|s| normalized_person(s).1).collect();
                    email.cc = email.cc.iter().map(|s| normalized_person(s).1).collect();
                    if current.settings.auto_classify {
                        email.category = rules::classify_offline(&email);
                    }
                    email.body.clear();
                    email.body_html.clear();
                    email.remote_only = true;
                    rows.push(email.id);
                    current.emails.push(email);
                    added += 1;
                }
                Ok((added, rows))
            })?;
            report.added += added;
            ids.extend(rows);
        }
        if translated.needs_local_filter {
            // EX-3: what Gmail could not check is checked on the stored rows.
            let current = crate::domain::undo::project(self.context.read()?);
            ids.retain(|id| current.email(*id).is_some_and(|email| parsed.server_result_matches(email)));
        }
        for id in ids {
            if !previous.ids.contains(&id) {
                previous.ids.push(id);
            }
        }
        if !continuing {
            previous.total = total;
        }
        previous.complete = !failed && next.is_empty();
        previous.failed = failed && previous.ids.is_empty();
        previous.at = now;
        previous.local_filter = translated.needs_local_filter;
        report.ids = previous.ids.clone();
        report.total = previous.total;
        report.next = (!next.is_empty()).then_some(RemoteSearchCursor { accounts: next });
        report.notice = Some(Self::server_notice(it, &previous));
        {
            let mut results = self
                .server_results
                .lock()
                .map_err(|_| AppError::storage("Search state unavailable"))?;
            results.insert(key, previous);
            // Only recent searches are kept: the rows themselves stay in the store.
            while results.len() > 20 {
                let Some(oldest) = results.iter().min_by_key(|(_, r)| r.at).map(|(k, _)| k.clone()) else { break };
                results.remove(&oldest);
            }
        }
        self.context.cleanup_mail_cache()?;
        Ok(report)
    }
    fn server_notice(it: bool, result: &ServerResult) -> String {
        let shown = crate::domain::regional::group_digits(result.ids.len() as u64, it);
        let total = crate::domain::regional::group_digits(u64::from(result.total), it);
        match (it, result.local_filter) {
            // The server's total counts matches before this computer's filter.
            (true, true) => format!("Risultati dal server: {shown}, filtrati su questo computer tra {total} corrispondenze."),
            (false, true) => format!("Results from the server: {shown}, filtered on this computer among {total} matches."),
            (true, false) => format!("Risultati dal server: {shown} di {total}."),
            (false, false) => format!("Results from the server: {shown} of {total}."),
        }
    }
    pub fn read(&self, id: u64) -> AppResult<Email> {
        self.reader_active.fetch_add(1, Ordering::SeqCst);
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
        let _active = Active(&self.reader_active);
        self.read_inner(id)
    }
    fn read_inner(&self, id: u64) -> AppResult<Email> {
        let lock = {
            let mut locks = self.reader_locks.lock().map_err(|_| AppError::storage("Mail reader lock failed"))?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            let lock = locks.get(&id).and_then(std::sync::Weak::upgrade).unwrap_or_else(|| Arc::new(Mutex::new(())));
            locks.insert(id, Arc::downgrade(&lock));
            lock
        };
        let _guard = lock.lock().map_err(|_| AppError::storage("Mail reader lock failed"))?;
        let store = crate::domain::undo::project(self.context.read()?);
        let email = store
            .email(id)
            .cloned()
            .ok_or_else(|| AppError::missing("Message"))?;
        if !email.remote_only {
            return Ok(email);
        }
        // Check account access on every read, even when the body is in memory.
        let account = self.context.account(&store, email.account_id)?;
        self.context.mail.authorize_read(&account)?;
        let cached = self.reader_payloads.lock().ok().and_then(|mut payloads| {
            let index = payloads.iter().position(|cached| {
                cached.id == email.id && cached.account_id == email.account_id
                    && cached.remote_global_id == email.remote_global_id
                    && cached.remote_content_hash == email.remote_content_hash
                    && cached.message_id == email.message_id
                    && cached.uid == email.uid && cached.remote_folder == email.remote_folder
                    && cached.remote_uid_validity == email.remote_uid_validity
            })?;
            let cached = payloads.remove(index)?;
            payloads.push_back(cached.clone());
            Some(cached)
        });
        let mut email = email;
        if let Some(cached) = cached {
            email.body = cached.body;
            email.body_html = cached.body_html;
            email.attachments = cached.attachments;
        } else {
            let loaded = self.context.mail.load_body_preview(&account, &email)?;
            email.body = loaded.body;
            email.body_html = loaded.body_html;
            email.attachments = loaded.attachments;
            // Temporary, bounded session cache; the offline repository stays unchanged.
            if let Ok(mut payloads) = self.reader_payloads.lock() {
                let bytes = |email: &Email| email.body.len() + email.body_html.len() + email.attachments.iter()
                    .map(|a| a.name.len() + a.inline_text.as_ref().map_or(0, String::len))
                    .sum::<usize>();
                const MAX_BYTES: usize = 16 * 1024 * 1024;
                let size = bytes(&email);
                if size <= MAX_BYTES {
                    payloads.retain(|cached| cached.id != email.id);
                    while payloads.len() >= 20 || payloads.iter().map(bytes).sum::<usize>() + size > MAX_BYTES {
                        payloads.pop_front();
                    }
                    payloads.push_back(email.clone());
                }
            }
        }
        email.remote_only = false;
        Ok(email)
    }
    pub fn prefetch(&self, ids: Vec<u64>) -> AppResult<()> {
        let mut queue = self.prefetch_queue.lock().map_err(|_| AppError::storage("Mail prefetch lock failed"))?;
        queue.ids.clear();
        for id in ids.into_iter().take(6) {
            if !queue.ids.contains(&id) { queue.ids.push_back(id); }
        }
        if queue.running || queue.ids.is_empty() { return Ok(()); }
        queue.running = true;
        let service = self.clone();
        if let Err(error) = std::thread::Builder::new().name("qamille-mail-prefetch".into()).spawn(move || {
            loop {
                // Finish at most the current text fetch before yielding to a click.
                if service.reader_active.load(Ordering::SeqCst) > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
                let id = {
                    let Ok(mut queue) = service.prefetch_queue.lock() else { return };
                    match queue.ids.pop_front() {
                        Some(id) => id,
                        None => { queue.running = false; return; }
                    }
                };
                // Prefetch failures stay quiet; an explicit click can retry normally.
                let _ = service.read_inner(id);
            }
        }) {
            queue.running = false;
            return Err(AppError::external(error.to_string()));
        }
        Ok(())
    }
    fn load_content(&self, store: &crate::model::Store, mut email: Email) -> AppResult<Email> {
        if email.remote_only {
            let account = self.context.account(store, email.account_id)?;
            let loaded = self.context.mail.load_body(&account, &email)?;
            email.body = loaded.body;
            email.body_html = loaded.body_html;
            email.attachments = loaded.attachments;
            email.remote_only = false;
        }
        Ok(email)
    }
    pub fn conversation(&self, id: u64) -> AppResult<Vec<Email>> {
        let store = crate::domain::undo::project(self.context.read()?);
        let anchor = store
            .email(id)
            .ok_or_else(|| AppError::missing("Message"))?;
        Ok(crate::domain::mail_export::thread(&store, anchor))
    }
    /// The other messages of the conversation a list row stands for, in the same tree order as
    /// the row's own `replies`; this is how the ones beyond `REPLY_CAP` are read. `limit` is
    /// 1 to 100, and 0 means `REPLY_CAP`. `folder` is the open view, which decides what is listed
    /// (the Trash and Spam views hold their own members): it must be the one the row came from.
    pub fn replies(
        &self,
        main_id: u64,
        folder: &str,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<ConversationReply>> {
        let store = crate::domain::undo::project(self.context.read()?);
        store.email(main_id).ok_or_else(|| AppError::missing("Message"))?;
        let groups = crate::domain::mail_export::conversation_ids(&store.emails);
        let limit = if limit == 0 { REPLY_CAP } else { limit.clamp(1, 100) };
        Ok(reply_nodes(&store, &groups, main_id, folder)
            .iter()
            .skip(offset)
            .take(limit)
            .map(ConversationReply::from_node)
            .collect())
    }
    pub fn selection(&self, ids: &[u64]) -> AppResult<Vec<Email>> {
        if ids.is_empty() {
            return Err(AppError::validation("Select at least one message"));
        }
        let store = crate::domain::undo::project(self.context.read()?);
        let mut selected = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if seen.insert(*id) {
                let email = store
                    .email(*id)
                    .cloned()
                    .ok_or_else(|| AppError::missing("Selected message"))?;
                selected.push(self.load_content(&store, email)?);
            }
        }
        selected.sort_by_key(|e| (e.date, e.id));
        Ok(selected)
    }
    pub fn thread(&self, id: u64) -> AppResult<Vec<Email>> {
        let store = crate::domain::undo::project(self.context.read()?);
        let anchor = store
            .email(id)
            .ok_or_else(|| AppError::missing("Message"))?;
        let local = crate::domain::mail_export::thread(&store, anchor);
        let account = store
            .account(anchor.account_id)
            .ok_or_else(|| AppError::missing("Account"))?;
        if account.resolved_provider_id() == "sample" {
            return Ok(local);
        }
        let remote_anchor = local
            .iter()
            .find(|e| !e.message_id.is_empty() || e.uid.is_some())
            .ok_or_else(|| {
                AppError::validation("Sync this conversation before printing the complete thread")
            })?;
        let authenticated = self.context.account(&store, account.id)?;
        self.context.mail.thread(&authenticated, remote_anchor)
    }
    /// The provider's original bytes for each selected message that has one.
    /// `failure` is the error shown when any of them cannot be downloaded.
    fn originals(
        &self,
        emails: &[Email],
        failure: &'static str,
    ) -> AppResult<std::collections::HashMap<u64, Vec<u8>>> {
        let store = self.context.read_shared()?;
        let mut originals = std::collections::HashMap::new();
        let account_ids: std::collections::BTreeSet<_> =
            emails.iter().map(|e| e.account_id).collect();
        for account_id in account_ids {
            let account = store
                .account(account_id)
                .ok_or_else(|| AppError::missing("Account"))?;
            if account.resolved_provider_id() == "sample" {
                continue;
            }
            let remote: Vec<_> = emails
                .iter()
                .filter(|e| {
                    e.account_id == account_id && !e.local_sent && !e.remote_deleted && (!e.message_id.is_empty() || e.uid.is_some())
                })
                .cloned()
                .collect();
            if remote.is_empty() {
                continue;
            }
            let account = self.context.account(&store, account_id)?;
            let raw = self.context.mail.originals(&account, &remote)?;
            if raw.len() != remote.len() || raw.iter().any(Vec::is_empty) {
                return Err(AppError::external(failure));
            }
            originals.extend(remote.into_iter().zip(raw).map(|(e, raw)| (e.id, raw)));
        }
        Ok(originals)
    }
    /// A message rebuilt from local content when the provider has no original.
    fn rebuilt_eml(&self, email: &Email) -> AppResult<Vec<u8>> {
        let attachments = email
            .attachments
            .iter()
            .map(|a| self.context.attachments.read_bytes(a))
            .collect::<AppResult<_>>()?;
        crate::domain::mail_export::eml(email, attachments).map_err(AppError::validation)
    }
    pub fn export(&self, ids: &[u64], format: ExportFormat) -> AppResult<MailExport> {
        use crate::domain::mail_export;
        let emails = self.selection(ids)?;
        if matches!(format, ExportFormat::Eml) && emails.len() != 1 {
            return Err(AppError::validation(
                "Use MBOX to download more than one message",
            ));
        }
        let mut originals = self.originals(
            &emails,
            "Some original messages could not be downloaded. No export was saved.",
        )?;
        let mut bytes = Vec::new();
        for email in &emails {
            let raw = match originals.remove(&email.id) {
                Some(raw) => raw,
                None => self.rebuilt_eml(email)?,
            };
            match format {
                ExportFormat::Eml => bytes = raw,
                ExportFormat::Mbox => mail_export::append_mbox(&mut bytes, email, &raw),
            }
        }
        Ok(MailExport {
            filename: match format {
                ExportFormat::Eml => mail_export::filename(&emails[0].subject, emails[0].id, "eml"),
                ExportFormat::Mbox => format!("qamille-{}-messages.mbox", emails.len()),
            },
            bytes,
            count: emails.len(),
        })
    }
    /// Each message as its own `.eml`, to attach to a draft. All of them
    /// together must fit `budget` bytes: an estimate is checked before any
    /// download, and the real size after each message.
    pub fn export_eml_each(&self, ids: &[u64], budget: u64) -> AppResult<Vec<MailExport>> {
        const TOO_LARGE: &str = "The messages are too large to attach: attachments are limited to 25 MB";
        let emails = self.selection(ids)?;
        let estimate = emails.iter().fold(0u64, |total, email| {
            let attachments = email.attachments.iter().map(|a| a.size).sum::<u64>();
            total
                .saturating_add(attachments)
                .saturating_add(email.body.len() as u64)
                .saturating_add(email.body_html.len() as u64)
        });
        if estimate > budget {
            return Err(AppError::validation(TOO_LARGE));
        }
        let mut originals = self.originals(
            &emails,
            "Some messages could not be downloaded. Nothing was attached.",
        )?;
        let mut used = 0u64;
        let mut exports = Vec::with_capacity(emails.len());
        for email in &emails {
            let bytes = match originals.remove(&email.id) {
                Some(raw) => raw,
                None => self.rebuilt_eml(email)?,
            };
            used = used.saturating_add(bytes.len() as u64);
            if used > budget {
                return Err(AppError::validation(TOO_LARGE));
            }
            exports.push(MailExport {
                filename: crate::domain::mail_export::filename(&email.subject, email.id, "eml"),
                bytes,
                count: 1,
            });
        }
        Ok(exports)
    }
    fn resolved_attachment(
        &self,
        id: u64,
        attachment_id: u64,
    ) -> AppResult<crate::model::Attachment> {
        let mut email = self.read(id)?;
        if email
            .attachments
            .iter()
            .any(|a| a.id == attachment_id && a.path.is_none() && a.inline_text.is_none())
        {
            let store = crate::domain::undo::project(self.context.read()?);
            if let Some(original) = store.email(id).filter(|e| e.remote_only) {
                email = self.load_content(&store, original.clone())?;
            }
        }
        email
            .attachments
            .into_iter()
            .find(|a| a.id == attachment_id)
            .ok_or_else(|| AppError::missing("Attachment"))
    }
    /// The account and stored message to download `attachment` from, when the
    /// message is not downloaded and the attachment has no file of its own.
    fn online_attachment_source(
        &self,
        id: u64,
        attachment: &crate::model::Attachment,
    ) -> AppResult<Option<(crate::model::Account, Email)>> {
        if attachment.path.is_some() {
            return Ok(None);
        }
        let store = crate::domain::undo::project(self.context.read()?);
        let Some(stored) = store.email(id).filter(|e| e.remote_only).cloned() else {
            return Ok(None);
        };
        let account = self.context.account(&store, stored.account_id)?;
        Ok(Some((account, stored)))
    }
    pub fn attachment_text(&self, id: u64, attachment_id: u64) -> AppResult<String> {
        let attachment = self.resolved_attachment(id, attachment_id)?;
        self.context.attachments.read_text(&attachment)
    }
    pub fn attachment_document(
        &self,
        id: u64,
        attachment_id: u64,
    ) -> AppResult<AttachmentDocument> {
        let shown = self.read(id)?.attachments;
        let attachment = shown
            .iter()
            .find(|a| a.id == attachment_id)
            .cloned()
            .ok_or_else(|| AppError::missing("Attachment"))?;
        let bytes = match self.online_attachment_source(id, &attachment)? {
            // Nothing of this message is kept on disk, and the text extracted
            // for search is not the file: take the part from the original.
            Some((account, mut locator)) => {
                // The declared size is enough to refuse before downloading.
                if attachment.size > crate::rules::ATTACHMENT_CAP_BYTES {
                    return Err(AppError::validation("File is larger than 25 MB."));
                }
                // The id belongs to the list the reader returned, which after a
                // server search is not the stored one: locate the part in it.
                locator.attachments = shown;
                self.context.mail.attachment_bytes(&account, &locator, attachment.id)?
            }
            None => self.context.attachments.read_bytes(&attachment)?,
        };
        if bytes.is_empty() {
            return Err(AppError::validation("Attachment has no readable content."));
        }
        if bytes.len() as u64 > crate::rules::ATTACHMENT_CAP_BYTES {
            return Err(AppError::validation("File is larger than 25 MB."));
        }
        Ok(AttachmentDocument {
            name: attachment.name,
            bytes,
        })
    }
    /// The attachment itself (`path` empty) or a part nested inside it when the
    /// attachment is a message: see `domain::attached_message` for addressing.
    pub fn attached_document(
        &self,
        id: u64,
        attachment_id: u64,
        path: &[u32],
    ) -> AppResult<AttachmentDocument> {
        let outer = self.attachment_document(id, attachment_id)?;
        if path.is_empty() {
            return Ok(outer);
        }
        let (name, bytes) = crate::domain::attached_message::nested(&outer.bytes, path)
            .map_err(AppError::validation)?;
        if bytes.is_empty() {
            return Err(AppError::validation("Attachment has no readable content."));
        }
        Ok(AttachmentDocument {
            name: name.unwrap_or(outer.name),
            bytes,
        })
    }
    pub fn attached_message(
        &self,
        id: u64,
        attachment_id: u64,
        path: &[u32],
    ) -> AppResult<crate::domain::attached_message::AttachedMessage> {
        let document = self.attached_document(id, attachment_id, path)?;
        crate::domain::attached_message::parse(&document.bytes).map_err(AppError::validation)
    }
    pub fn attached_text(&self, id: u64, attachment_id: u64, path: &[u32]) -> AppResult<String> {
        if path.is_empty() {
            return self.attachment_text(id, attachment_id);
        }
        let document = self.attached_document(id, attachment_id, path)?;
        self.context
            .attachments
            .text_from_bytes(&document.name, &document.bytes)
    }
    pub fn act(&self, id: u64, action: &str, value: Option<&str>) -> AppResult<()> {
        self.apply_actions(id, &[(action, value)], &[])
    }
    pub fn act_queued(&self, pending: &[crate::domain::undo::UndoAction]) -> AppResult<()> {
        use crate::domain::undo::UndoOperation;
        let Some(UndoOperation::Mail { email_id, .. }) = pending.first().map(|a| &a.operation)
        else {
            return Err(AppError::validation("No mail action to apply"));
        };
        let actions: Vec<_> = pending
            .iter()
            .filter_map(|a| match &a.operation {
                UndoOperation::Mail { action, value, .. } => {
                    Some((action.as_str(), value.as_deref()))
                }
                _ => None,
            })
            .collect();
        let ids: Vec<_> = pending.iter().map(|a| a.id).collect();
        self.apply_actions(*email_id, &actions, &ids)
    }
    /// A user action in three phases (ADR-0007 INV-001): `plan_actions` reads the
    /// store and validates under the account lock, the provider calls run with no
    /// lock held, `commit_actions` writes the result under the account lock again.
    /// A slow provider therefore never keeps a sync, or another action, waiting.
    pub(crate) fn apply_actions( // pub(crate) for AccountWorker (Task 3.1); controller ruling 2026-11-04
        &self,
        id: u64,
        commands: &[(&str, Option<&str>)],
        pending_ids: &[u64],
    ) -> AppResult<()> {
        let _in_flight = self.claim(id)?;
        let plan = self.plan_actions(id, commands, pending_ids)?;
        let outcome = self.run_remote_actions(&plan)?;
        self.commit_actions(plan, outcome)
    }
    fn claim(&self, id: u64) -> AppResult<InFlight> {
        let mut set = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(id) {
            return Err(AppError::new("mail_in_flight", "Another action on this message is in progress"));
        }
        Ok(InFlight { set: self.in_flight.clone(), id })
    }
    /// Phase 1: read the store, then under the account lock validate the commands
    /// on a scratch copy and work out which provider operations they need.
    fn plan_actions(
        &self,
        id: u64,
        commands: &[(&str, Option<&str>)],
        pending_ids: &[u64],
    ) -> AppResult<ActionPlan> {
        // The lock is keyed by the message's account, which only the store can tell:
        // read once for that, then read again inside the lock so the plan never works
        // from a locator another action has just changed.
        // Both reads share the committed snapshot: no copy of the mailbox.
        let account_id = self
            .context
            .read_shared()?
            .email(id)
            .map(|email| email.account_id)
            .ok_or_else(|| AppError::missing("Message"))?;
        let lock = self.account_lock(account_id);
        let _guard = lock
            .lock()
            .map_err(|_| AppError::storage("Mail operation lock failed"))?;
        let store = self.context.read_shared()?;
        let now = self.context.clock.now();
        let regional = self.context.clock.regional(&store.settings);
        let email = store
            .email(id)
            .cloned()
            .filter(|email| email.account_id == account_id)
            .ok_or_else(|| AppError::missing("Message"))?;
        if email.remote_deleted && commands.iter().any(|(action, _)| matches!(*action, "moveToInbox" | "moveToFolder" | "markAsSpam")) {
            return Err(AppError::new("remote_message_deleted", "This message was deleted on the provider. Its cached local copy cannot be moved back to the server."));
        }
        // The commands touch this message alone: they are validated on a
        // scratch store that holds only a copy of it.
        let mut scratch = crate::model::Store { emails: vec![email.clone()], ..Default::default() };
        for (action, value) in commands {
            actions::apply_at(&mut scratch, id, action, *value, now, &regional.time_zone, &regional.locale).map_err(AppError::validation)?;
        }
        drop(scratch);
        let mut moved = false;
        let mut remote = Vec::new();
        let mut authenticated = None;
        if email.uid.is_some() || email.remote_folder.is_some() {
            let account = store
                .account(email.account_id)
                .ok_or_else(|| AppError::missing("Account"))?;
            let requires_remote = commands.iter().any(|(action, _)| {
                matches!(
                    *action,
                    "markAsSeen"
                        | "markAsUnseen"
                        | "pin"
                        | "unpin"
                        | "archive"
                        | "markAsDone"
                        | "moveToTrash"
                        | "moveToInbox"
                        | "moveToFolder"
                        | "markAsSpam"
                ) || (store.settings.push_labels
                    && matches!(*action, "attachLabel" | "detachLabel"))
            });
            if requires_remote && !self.context.mail.capabilities(account).remote_actions {
                return Err(AppError::new(
                    "unsupported_capability",
                    "This provider does not support the requested remote mail action",
                ));
            }
            if self.context.mail.capabilities(account).remote_actions {
                for (action, value) in commands {
                    match *action {
                        "markAsSeen" => remote.push(RemoteAction::Seen(true)),
                        "markAsUnseen" => remote.push(RemoteAction::Seen(false)),
                        "pin" => remote.push(RemoteAction::Flag(true)),
                        "unpin" => remote.push(RemoteAction::Flag(false)),
                        "attachLabel" if store.settings.push_labels => {
                            remote.push(RemoteAction::Label(value.unwrap_or_default().into()))
                        }
                        "detachLabel" if store.settings.push_labels => {
                            remote.push(RemoteAction::RemoveLabel(value.unwrap_or_default().into()))
                        }
                        _ => {}
                    }
                }
                // Several quick moves share the same Inbox UID. Apply flags first,
                // then only the final move, before that UID becomes invalid.
                if let Some((action, _)) = commands.iter().rev().find(|(a, _)| {
                    matches!(
                        *a,
                        "archive"
                            | "markAsDone"
                            | "moveToTrash"
                            | "moveToInbox"
                            | "moveToFolder"
                            | "markAsSpam"
                    )
                }) {
                    let destination = match *action {
                        // Dropping the Inbox label cannot take a message out of Trash or Spam.
                        "archive" | "markAsDone" if matches!(email.folder.as_str(), "Trash" | "Spam") => {
                            Some(RemoteAction::Move(account.archive_folder.clone()))
                        }
                        "archive" | "markAsDone" => Some(RemoteAction::Archive),
                        "moveToTrash" => Some(RemoteAction::Move(account.trash_folder.clone())),
                        "moveToInbox" => Some(RemoteAction::Move("INBOX".into())),
                        "moveToFolder" => commands
                            .iter()
                            .rev()
                            .find(|(a, _)| *a == "moveToFolder")
                            .and_then(|(_, value)| value.map(|folder| RemoteAction::Move(folder.to_owned()))),
                        _ => None,
                    };
                    if let Some(remote_action) = destination {
                        remote.push(remote_action);
                        moved = true;
                    }
                    if *action == "markAsSpam" {
                        remote.push(RemoteAction::Spam);
                        moved = true;
                    }
                }
                if !remote.is_empty() {
                    authenticated = Some(self.context.account(&store, email.account_id)?);
                }
            }
        }
        Ok(ActionPlan {
            id,
            account_id: email.account_id,
            commands: commands
                .iter()
                .map(|(action, value)| (action.to_string(), value.map(str::to_string)))
                .collect(),
            pending_ids: pending_ids.to_vec(),
            now,
            time_zone: regional.time_zone,
            locale: regional.locale,
            email,
            remote,
            authenticated,
            moved,
        })
    }
    /// Phase 2, no lock: everything that talks to the provider. A move first
    /// resolves the verified provider identity and saves it, in its own short
    /// transaction, so a later network or storage failure cannot lose the locator
    /// of a message the provider has already moved.
    fn run_remote_actions(&self, plan: &ActionPlan) -> AppResult<RemoteOutcome> {
        let Some(account) = &plan.authenticated else {
            return Ok(RemoteOutcome { moved: false });
        };
        let mut email = plan.email.clone();
        if plan.moved {
            email = self.context.mail.resolve_identity(account, &email)?;
            // Commit identity before the destructive provider move, so a
            // later network/storage failure cannot discard our locator.
            let lock = self.account_lock(plan.account_id);
            let _guard = lock
                .lock()
                .map_err(|_| AppError::storage("Mail operation lock failed"))?;
            self.context.update_emails(&[plan.id], |current| {
                let target = current.email_mut(plan.id).ok_or_else(|| AppError::missing("Message"))?;
                if target.remote_deleted {
                    return Err(AppError::new("remote_message_deleted", "This message was deleted on the provider. Its cached local copy cannot be moved back to the server."));
                }
                target.remote_global_id = email.remote_global_id.or(target.remote_global_id);
                if !email.remote_content_hash.is_empty() { target.remote_content_hash = email.remote_content_hash.clone(); }
                for location in &email.remote_locations {
                    if !target.remote_locations.contains(location) { target.remote_locations.push(location.clone()); }
                }
                Ok(())
            })?;
        }
        // Batch invariant (INV-010): flags first, the move LAST, and flags are idempotent.
        // A failure on the move can follow flags already sent; retrying the whole claim
        // re-sends them harmlessly. Do not add a non-idempotent action before the move
        // without revisiting the retry policy.
        for action in &plan.remote {
            self.context
                .mail
                .apply_message(account, &email, action.clone())?;
        }
        Ok(RemoteOutcome { moved: plan.moved })
    }
    /// Phase 3, under the account lock: apply the commands to the current store.
    fn commit_actions(&self, plan: ActionPlan, outcome: RemoteOutcome) -> AppResult<()> {
        let lock = self.account_lock(plan.account_id);
        let _guard = lock
            .lock()
            .map_err(|_| AppError::storage("Mail operation lock failed"))?;
        // Only the message acted on changes (and the queue): no copy of the mailbox.
        let committed = self.context.update_emails_with_events(&[plan.id], |current| {
            // The queue rows this commit consumes leave the UI first, then the
            // message it changed and the counters follow (ADR-0007 decision 5).
            let mut events: Vec<AppEvent> = current
                .undo_actions
                .iter()
                .filter(|a| plan.pending_ids.contains(&a.id))
                .map(|a| AppEvent::UndoRemoved { id: a.id })
                .collect();
            for (action, value) in &plan.commands {
                actions::apply_at(current, plan.id, action, value.as_deref(), plan.now, &plan.time_zone, &plan.locale).map_err(AppError::validation)?;
            }
            current
                .undo_actions
                .retain(|a| !plan.pending_ids.contains(&a.id));
            if outcome.moved {
                if let Some(e) = current.email_mut(plan.id) {
                    e.uid = None;
                }
            }
            events.push(AppEvent::EmailChanged {
                account_id: plan.account_id,
                ids: vec![plan.id],
            });
            events.push(AppEvent::CountsChanged {
                account_id: plan.account_id,
            });
            Ok(((), events))
        });
        let Err(failure) = committed else {
            // A committed snooze or reminder is a new or moved scheduler
            // deadline (INV-005): the scheduler thread recomputes its wake
            // instead of polling for it. `due` dates are not scheduler
            // deadlines, so setDue/clearDue do not note.
            if plan.commands.iter().any(|(action, _)| {
                matches!(
                    action.as_str(),
                    "snooze" | "unsnooze" | "changeReminder" | "clearReminder"
                )
            }) {
                super::scheduler::note_schedule_change();
            }
            return Ok(());
        };
        if plan.authenticated.is_none() {
            return Err(failure);
        }
        // The provider already applied the action. Drop the queued action so nothing
        // replays it (INV-007), and forget the Inbox UID a move made invalid; the next
        // sync restores the real state.
        let _ = self.context.update_emails_with_events(&[plan.id], |current| {
            let events: Vec<AppEvent> = current
                .undo_actions
                .iter()
                .filter(|a| plan.pending_ids.contains(&a.id))
                .map(|a| AppEvent::UndoRemoved { id: a.id })
                .collect();
            current.undo_actions.retain(|a| !plan.pending_ids.contains(&a.id));
            if outcome.moved {
                if let Some(e) = current.email_mut(plan.id) {
                    e.uid = None;
                }
            }
            Ok(((), events))
        });
        Err(AppError::new("remote_applied_local_failed", format!("The provider applied the action, but the local copy could not be updated: {}. Sync to reconcile.", failure.message)))
    }
    pub fn sync(&self) -> AppResult<SyncReport> {
        self.sync_selected(None)
    }
    pub fn sync_account(&self, account_id: u32) -> AppResult<SyncReport> {
        self.sync_selected(Some(account_id))
    }
    /// Phase (a) of a sync: what the download needs from the store, read under
    /// the account lock. `None` when the account was removed in the meantime.
    fn sync_snapshot(&self, account_id: u32) -> AppResult<Option<SyncSnapshot>> {
        let lock = self.account_lock(account_id);
        let _guard = lock
            .lock()
            .map_err(|_| AppError::storage("Mail operation lock failed"))?;
        let store = self.context.read_shared()?;
        let Some(account) = store.account(account_id).cloned() else {
            return Ok(None);
        };
        let measured = self.context.repository.message_storage_bytes(&store)?;
        let measured_drafts = self.context.repository.draft_storage_bytes(&store)?;
        // This is the allowance for NEW data. Existing cached bodies
        // are reused by the provider and are already included in usage.
        let max_bytes = account.storage_policy.admission_limit_bytes().map(|limit| {
            limit.saturating_sub(
                crate::domain::storage::usage_with_storage_bytes(
                    &store, account.id, &measured, &measured_drafts,
                )
                .map_or(0, |usage| usage.used_bytes),
            )
        });
        let authenticated = self.context.account(&store, account_id);
        Ok(Some(SyncSnapshot { account, authenticated, max_bytes, known: store.emails.clone() }))
    }
    fn sync_selected(&self, account_id: Option<u32>) -> AppResult<SyncReport> {
        let listed = self.context.read()?;
        let mut report = SyncReport::default();
        if let Some(id) = account_id {
            let account = listed
                .account(id)
                .ok_or_else(|| AppError::missing("Account"))?;
            if !self.context.mail.capabilities(account).sync {
                return Err(AppError::new(
                    "unsupported_capability",
                    "Synchronization is unavailable for this account's provider",
                ));
            }
        }
        let targets: Vec<u32> = listed
            .accounts
            .iter()
            .filter(|a| {
                self.context.mail.capabilities(a).sync && account_id.is_none_or(|id| a.id == id)
            })
            .map(|a| a.id)
            .collect();
        // No lock is held between one account and the next.
        for id in targets {
            self.sync_progress.update(id, SyncPhase::Syncing, 0, vec![]);
            self.context.events.emit(AppEvent::SyncProgress {
                account_id: id,
                phase: SyncPhase::Syncing,
                added: 0,
            });
            let previous_added = report.added;
            let previous_errors = report.errors.len();
            // The informational lines this pass pushed into `report.errors`, to tell them from failures.
            let mut notices: Vec<String> = Vec::new();
            // Always finish the status, including failures while saving downloaded mail.
            let account_result = (|| -> AppResult<()> {
                // (a) A short look at the store, under the account lock.
                let Some(snapshot) = self.sync_snapshot(id)? else {
                    return Ok(());
                };
                let account = &snapshot.account;
                // (b) The download: no lock held, however long the provider takes.
                let result = snapshot.authenticated.and_then(|a| {
                    self.context.mail.sync_with_policy(
                        &a,
                        &super::ports::SyncRequest {
                            max_bytes: snapshot.max_bytes,
                            known: &snapshot.known,
                        },
                    )
                });
                // (c) The merge is the only write: one short transaction under the
                // account lock. Mail that was saved meanwhile is matched by provider
                // identity (`same_message`), not overwritten wholesale.
                let lock = self.account_lock(id);
                let _guard = lock
                    .lock()
                    .map_err(|_| AppError::storage("Mail operation lock failed"))?;
                match result {
                    Err(e) => {
                        self.context.update(|current| {
                            if let Some(current_account) =
                                current.accounts.iter_mut().find(|a| a.id == account.id)
                            {
                                current_account.offline_sync_complete = false;
                                current_account.offline_sync_error = Some(e.message.clone());
                            }
                            Ok(())
                        })?;
                        report
                            .errors
                            .push(format!("{}: {}", account.email, e.message));
                        // The credential is gone from the vault, or Google refuses it: once per
                        // account, until a sync works again. A network error is no such thing.
                        if matches!(e.code, "sign_in_required" | "sign_in_expired")
                            && lock_set(&self.sign_in_expired).insert(account.id)
                        {
                            self.context.events.emit(AppEvent::SignInExpired {
                                account_id: Some(account.id),
                            });
                        }
                    }
                    Ok(outcome) => {
                        lock_set(&self.sign_in_expired).remove(&account.id);
                        // Only a sync after this process's first merge of the account announces
                        // arrivals (`synced`).
                        let baseline_set = lock_set(&self.synced).contains(&account.id);
                        let notice = outcome.notice.clone();
                        let batch = outcome.batch;
                        let downloaded_paths: Vec<_> = batch
                            .emails
                            .iter()
                            .flat_map(|email| {
                                email
                                    .attachments
                                    .iter()
                                    .filter_map(|attachment| attachment.path.clone())
                            })
                            .collect();
                        // The download describes the mailbox as of the snapshot. A
                        // user action may have committed since; what it changed is
                        // newer than this view and must not be put back.
                        let before: std::collections::HashMap<u64, &Email> =
                            snapshot.known.iter().map(|email| (email.id, email)).collect();
                        let result = self.context.update_with_events(|current| {
                            if current.account(account.id).is_none_or(|a| a.storage_policy != account.storage_policy) {
                                return Err(AppError::validation("Account storage settings changed during sync. Sync again to apply the new settings."));
                            }
                            // Messages the user moved since the snapshot no longer sit where
                            // the download saw them, so they may not match it by locator.
                            // They are found by the locator they had at the snapshot.
                            let moved_since: Vec<&Email> = snapshot
                                .known
                                .iter()
                                .filter(|b| current.email(b.id).is_some_and(|c| c.folder != b.folder || c.uid != b.uid))
                                .collect();
                            let mut added = 0;
                            // Every message the merge actually changed, new or
                            // updated: one event names them all, per account
                            // (INV-004). A routine re-sync that writes back the
                            // same values names none of them, so the UI keeps
                            // patching by ids instead of reloading (decision 6).
                            let mut touched: Vec<u64> = Vec::new();
                            // The new unread messages of the Inbox that are an arrival: received by
                            // the provider since the account last synced (`ARRIVAL_SLACK_SECS`), with
                            // this process's baseline set. Never the first sync of an account.
                            let arrival_since = current
                                .account(account.id)
                                .and_then(|a| a.last_sync)
                                .filter(|_| baseline_set)
                                .map(|last| last - ARRIVAL_SLACK_SECS);
                            let mut arrived: Vec<u64> = Vec::new();
                            for mut email in batch.emails {
                                let position = current
                                    .emails
                                    .iter()
                                    .position(|old| crate::domain::mail_identity::same_message(old, &email))
                                    .or_else(|| {
                                        let b = moved_since.iter().find(|b| crate::domain::mail_identity::same_message(b, &email))?;
                                        current.emails.iter().position(|old| old.id == b.id)
                                    });
                                if let Some(old) = position.map(|index| &mut current.emails[index]) {
                                    let seen = before.get(&old.id);
                                    let moved_since = seen.is_some_and(|b| b.folder != old.folder || b.uid != old.uid);
                                    let mut changed = false;
                                    if seen.is_none_or(|b| b.read == old.read) {
                                        changed |= old.read != email.read;
                                        old.read = email.read;
                                    }
                                    if seen.is_none_or(|b| b.starred == old.starred) {
                                        changed |= old.starred != email.starred;
                                        old.starred = email.starred;
                                    }
                                    if seen.is_none_or(|b| b.replied == old.replied) {
                                        changed |= old.replied != email.replied;
                                        old.replied = email.replied;
                                    }
                                    // `old` stays the stored copy, so the parent it recorded
                                    // when the reply was sent (in_reply_to) is kept as it is.
                                    changed |= old.remote_deleted != email.remote_deleted;
                                    old.remote_deleted = email.remote_deleted;
                                    changed |= email.remote_global_id.is_some()
                                        && email.remote_global_id != old.remote_global_id;
                                    old.remote_global_id = email.remote_global_id.or(old.remote_global_id);
                                    if email.remote_global_id.is_some() {
                                        changed |= old.local_sent;
                                        old.local_sent = false;
                                    }
                                    if !moved_since {
                                        changed |= old.folder != email.folder
                                            || old.uid != email.uid
                                            || old.remote_folder != email.remote_folder
                                            || old.remote_uid_validity != email.remote_uid_validity;
                                        old.folder = email.folder;
                                        old.uid = email.uid;
                                        old.remote_folder = email.remote_folder;
                                        old.remote_uid_validity = email.remote_uid_validity;
                                    }
                                    if old.remote_deleted {
                                        changed |= !old.remote_locations.is_empty();
                                        old.remote_locations.clear();
                                    }
                                    for location in email.remote_locations {
                                        if !old.remote_locations.contains(&location) {
                                            old.remote_locations.push(location);
                                            changed = true;
                                        }
                                    }
                                    if !email.remote_only {
                                        // Content takeover: a change when the payload,
                                        // its hash or the presence flags really move.
                                        // Attachment ids are reassigned below, so the
                                        // comparison ignores them.
                                        changed |= old.remote_content_hash != email.remote_content_hash
                                            || old.body != email.body
                                            || old.remote_only
                                            || old.storage_evicted
                                            || old.attachments.len() != email.attachments.len()
                                            || old
                                                .attachments
                                                .iter()
                                                .zip(&email.attachments)
                                                .any(|(stored, incoming)| {
                                                    stored.name != incoming.name
                                                        || stored.size != incoming.size
                                                        || stored.mime != incoming.mime
                                                        || stored.path != incoming.path
                                                        || stored.inline_text != incoming.inline_text
                                                });
                                        old.remote_content_hash = email.remote_content_hash;
                                        for (index, attachment) in email.attachments.iter_mut().enumerate() {
                                            attachment.id = old.id.saturating_mul(1000).saturating_add(index as u64);
                                        }
                                        old.body = email.body;
                                        old.attachments = email.attachments;
                                        old.remote_only = false;
                                        old.storage_evicted = false;
                                    }
                                    if changed {
                                        touched.push(old.id);
                                    }
                                    continue;
                                }
                                email.id = current.new_id();
                                for (index, attachment) in email.attachments.iter_mut().enumerate() {
                                    attachment.id = email.id.saturating_mul(1000).saturating_add(index as u64);
                                }
                                if current.settings.auto_classify {
                                    email.category = rules::classify_offline(&email);
                                }
                                if current.settings.auto_tag {
                                    email.labels = rules::labels_offline(&email);
                                }
                                email.classified = true;
                                touched.push(email.id);
                                // The moment of receipt decides, not the Date header; a provider
                                // without one has only the date. The store never keeps it.
                                let received = email.received_at.take().unwrap_or(email.date);
                                if email.folder == "Inbox"
                                    && !email.read
                                    && arrival_since.is_some_and(|since| received >= since)
                                {
                                    arrived.push(email.id);
                                }
                                current.emails.push(email);
                                added += 1;
                            }
                            if let Some(a) = current.accounts.iter_mut().find(|a| a.id == account.id) {
                                a.last_sync = Some(self.context.clock.now());
                                a.offline_sync_complete = account.storage_policy.keep_all_offline && outcome.offline_complete;
                                a.offline_sync_error = notice.clone();
                                if let Some(f) = batch.archive_folder {
                                    a.archive_folder = f;
                                }
                                if let Some(f) = batch.trash_folder {
                                    a.trash_folder = f;
                                }
                            }
                            let mut events = Vec::new();
                            if !touched.is_empty() {
                                events.push(AppEvent::EmailChanged {
                                    account_id: account.id,
                                    ids: touched,
                                });
                                events.push(AppEvent::CountsChanged {
                                    account_id: account.id,
                                });
                            }
                            // Never forwarded to the page: a Windows notification may say it
                            // (desktop `notifications.rs`). Dropped when empty.
                            events.push(AppEvent::MailArrived {
                                account_id: account.id,
                                ids: arrived,
                            });
                            Ok((added, events))
                        });
                        match result {
                            Ok(added) => {
                                report.added += added;
                                lock_set(&self.synced).insert(account.id);
                            }
                            Err(error) => {
                                // The provider may already have written managed files.
                                // Queue them for cleanup against the latest committed
                                // references rather than deleting from a stale snapshot.
                                self.context.update(|current| {
                                    for path in downloaded_paths {
                                        if !current.cache_removals.contains(&path) {
                                            current.cache_removals.push(path);
                                        }
                                    }
                                    Ok(())
                                })?;
                                report
                                    .errors
                                    .push(format!("{}: {}", account.email, error.message));
                            }
                        }
                        if let Some(notice) = notice {
                            let line = format!("{}: {notice}", account.email);
                            notices.push(line.clone());
                            report.errors.push(line);
                        }
                    }
                }
                Ok(())
            })();
            let mut errors = report.errors[previous_errors..].to_vec();
            if let Err(error) = &account_result {
                errors.push(error.message.clone());
            }
            let phase = if errors.is_empty() {
                SyncPhase::Complete
            } else {
                SyncPhase::Error
            };
            let added = report.added - previous_added;
            let logged = sync_activity(id, added, &real_failures(&errors, &notices));
            self.sync_progress.update(id, phase, added, errors);
            self.context.events.emit(AppEvent::SyncProgress {
                account_id: id,
                phase,
                added,
            });
            // The activity log (centro attività), outside any transaction: new
            // mail is a fact, a failure is one row until its message changes.
            for (entry, failure) in logged {
                if failure {
                    activity::record_failure(&self.context, entry, code::SYNC_COMPLETE);
                } else {
                    activity::record(&self.context, entry);
                }
            }
            account_result?;
        }
        self.context.cleanup_mail_cache()?;
        Ok(report)
    }
}

/// What a finished sync of one account adds to the activity log, in this order: the new mail
/// (a fact, even when the same sync also failed), then the failure (an error), each flagged with
/// whether it is one. An uneventful sync is nothing: it runs every few minutes and would bury the
/// rest. `failures` are the real errors, not the notices of `real_failures`.
fn sync_activity(account_id: u32, added: usize, failures: &[String]) -> Vec<(NewActivity, bool)> {
    let mut rows = Vec::new();
    if added > 0 {
        rows.push((
            NewActivity {
                kind: ActivityKind::Event,
                code: code::SYNC_COMPLETE.into(),
                account_id: Some(account_id),
                count: Some(u32::try_from(added).unwrap_or(u32::MAX)),
                ..Default::default()
            },
            false,
        ));
    }
    if !failures.is_empty() {
        rows.push((
            NewActivity {
                kind: ActivityKind::Error,
                code: code::SYNC_FAILED.into(),
                account_id: Some(account_id),
                detail: Some(failures.join("\n")),
                ..Default::default()
            },
            true,
        ));
    }
    rows
}

/// The lines of a sync that are failures. The sync status shows notices ("the offline copy is still
/// being prepared") next to errors and turns red for both; the history keeps only the errors.
fn real_failures(lines: &[String], notices: &[String]) -> Vec<String> {
    lines.iter().filter(|line| !notices.contains(line)).cloned().collect()
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::domain::activity::{ActivityKind, code};

    fn failures(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn sync_without_new_mail_is_not_logged() {
        assert!(sync_activity(1, 0, &[]).is_empty());
    }

    #[test]
    fn sync_with_new_mail_logs_the_count() {
        let rows = sync_activity(1, 42, &[]);
        assert_eq!(rows.len(), 1);
        let (entry, failure) = &rows[0];
        assert!(!failure);
        assert_eq!((entry.code.as_str(), entry.kind), (code::SYNC_COMPLETE, ActivityKind::Event));
        assert_eq!((entry.count, entry.account_id), (Some(42), Some(1)));
    }

    #[test]
    fn sync_error_is_a_failure() {
        let rows = sync_activity(2, 0, &failures(&["timeout", "auth"]));
        assert_eq!(rows.len(), 1);
        let (entry, failure) = &rows[0];
        assert!(failure);
        assert_eq!((entry.code.as_str(), entry.kind), (code::SYNC_FAILED, ActivityKind::Error));
        assert_eq!(entry.detail.as_deref(), Some("timeout\nauth"));
        assert_eq!(entry.account_id, Some(2));
    }

    #[test]
    fn new_mail_is_logged_even_when_the_same_sync_also_failed() {
        // 40 messages arrived and then something else went wrong: both are facts, the mail first.
        let rows = sync_activity(1, 40, &failures(&["a@example.com: disk is full"]));
        let codes: Vec<_> = rows.iter().map(|(entry, failure)| (entry.code.as_str(), *failure)).collect();
        assert_eq!(codes, vec![(code::SYNC_COMPLETE, false), (code::SYNC_FAILED, true)]);
    }

    #[test]
    fn notices_are_told_from_failures() {
        let notice = "a@example.com: Offline copy is still being prepared. The next sync continues downloading missing content.";
        let lines = failures(&["a@example.com: IMAP login: timed out", notice]);
        let notices = failures(&[notice]);
        assert_eq!(
            real_failures(&lines, &notices),
            failures(&["a@example.com: IMAP login: timed out"]),
            "a notice is shown by the sync status but is no failure of the history"
        );
        assert!(real_failures(&failures(&[notice]), &notices).is_empty());
        assert!(sync_activity(1, 0, &real_failures(&failures(&[notice]), &notices)).is_empty());
    }
}
