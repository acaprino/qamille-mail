//! One resumable indexing step at a time. Provider and model work happen outside
//! store transactions; progress advances only after a durable index write.
use super::{
    AppError, AppResult, ServiceContext, activity, events::AppEvent, indexing_ports::*,
    ports::SearchGateway,
};
use crate::domain::{
    activity::{ActivityKind, NewActivity, code},
    indexing::*,
    model::{Account, AccountKind, Email},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

/// Whether an announcement is the moment an account's indexing finished: its
/// percent was watched below 100 and is now 100. Seen first at 100 (a restart
/// on a mailbox that was already done) is not a completion, and neither is a
/// percent that stays at 100.
fn index_completed(previous: Option<u8>, now: u8) -> bool {
    matches!(previous, Some(before) if before < 100) && now >= 100
}

/// An unchanged percent is announced again after this many seconds of work, so
/// a slow account is told from a stuck one. It must stay far below the UI's
/// stale threshold (`activityStaleMs`, desktop/src/lib/activity.ts).
const INDEX_HEARTBEAT_SECS: i64 = 30;

/// A message that keeps failing for its own reasons is set aside after this many
/// attempts, so one bad file can never hold the queue. Pauses and network errors
/// do not count, and an explicit retry starts again from zero.
const MAX_MESSAGE_ATTEMPTS: u32 = 5;
const RETRY_BASE_SECS: i64 = 60;
const RETRY_MAX_SECS: i64 = 3600;

/// Third-party parsers can panic on hostile input. A panic is reported as an
/// ordinary error for the work in progress instead of ending the indexing thread.
fn guarded<T>(work: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown failure".into());
        Err(AppError::new(
            "index_panic",
            format!("Indexing stopped on this message: {}", detail.chars().take(160).collect::<String>()),
        ))
    })
}

/// Live indexing progress between store checkpoints (ADR-0007 decision 5): the
/// last percent announced per account and when, so `IndexProgress` fires when
/// the visible progress moves, and as a heartbeat while it holds still. Memory
/// only — the durable cursors and counters stay in the store, written once per
/// page as before.
#[derive(Default)]
pub struct IndexProgressState {
    announced: std::collections::HashMap<u32, Announced>,
}

/// What was last announced for one account: the percent and the clock second.
#[derive(Debug)]
struct Announced {
    percent: u8,
    at: i64,
}

#[derive(Clone)]
pub struct MailIndexService {
    context: Arc<ServiceContext>,
    source: Arc<dyn IndexSource>,
    search: Arc<dyn SearchGateway>,
    running: Arc<Mutex<()>>,
    cancel: Arc<AtomicBool>,
    last_account: Arc<AtomicU32>,
    progress: Arc<Mutex<IndexProgressState>>,
    /// One open provider session per account, reused across batches.
    sessions: Arc<Mutex<std::collections::HashMap<u32, Box<dyn IndexSession>>>>,
    sessions_opened: Arc<std::sync::atomic::AtomicU64>,
    batch: usize,
    /// Accounts whose indexing activity waits for the compute server (INV-6).
    waiting: Arc<Mutex<std::collections::HashSet<u32>>>,
}
impl MailIndexService {
    pub fn new(
        context: Arc<ServiceContext>,
        source: Arc<dyn IndexSource>,
        search: Arc<dyn SearchGateway>,
    ) -> Self {
        Self {
            context,
            source,
            search,
            running: Arc::new(Mutex::new(())),
            cancel: Arc::new(AtomicBool::new(false)),
            last_account: Arc::new(AtomicU32::new(0)),
            progress: Arc::new(Mutex::new(IndexProgressState::default())),
            sessions: Arc::default(),
            sessions_opened: Arc::default(),
            batch: BATCH,
            waiting: Arc::default(),
        }
    }
    /// Messages per batch; `BATCH` unless a test needs single steps.
    pub fn with_batch_size(mut self, batch: usize) -> Self {
        self.batch = batch.max(1);
        self
    }
    /// Storage reconciliation, once at startup: `step` no longer runs it on
    /// every pass (D-2); the maintenance service keeps it up afterwards.
    pub fn reconcile_storage(&self) -> AppResult<()> {
        self.context.repository.reconcile_storage()
    }
    pub fn status(&self) -> AppResult<Vec<AccountIndexStatus>> {
        // A durable completion marker is meaningful only while its optional
        // index artifacts remain available. Recovery can invalidate it here.
        self.context.repository.reconcile_storage()?;
        let store = self.context.read_shared()?;
        Ok(store
            .accounts
            .iter()
            .map(|account| Self::account_status(&store, account))
            .collect())
    }
    /// One account's status, from the store alone. `status()` and the
    /// `IndexProgress` emission share it, so an announced percent can never
    /// disagree with a polled one.
    fn account_status(store: &crate::model::Store, account: &Account) -> AccountIndexStatus {
        let identity = |email: &Email| {
            (
                email.remote_global_id,
                if email.remote_global_id.is_some() {
                    0
                } else {
                    email.id
                },
            )
        };
        let messages: Vec<_> = store
            .emails
            .iter()
            .filter(|email| email.account_id == account.id)
            .collect();
        let known: std::collections::HashSet<_> =
            messages.iter().map(|email| identity(email)).collect();
        let finished: std::collections::HashSet<_> = messages
            .iter()
            .filter(|email| {
                !email.storage_evicted
                    && email.search_index.version == VERSION
                    && !email.search_index.partial
                    && !email.search_index.semantic_pending
                    && email.search_index.content_hash == email.remote_content_hash
            })
            .map(|email| identity(email))
            .collect();
        let mut progress = account.search_index.clone();
        progress.partial = messages
            .iter()
            .filter(|email| {
                (email.search_index.partial || email.storage_evicted)
                    && !finished.contains(&identity(email))
            })
            .map(|email| identity(email))
            .collect::<std::collections::HashSet<_>>()
            .len() as u64;
        // All Mail is a view while the history walks it (W3.1): the folders are not all
        // listed until the walk is complete, whatever their cursors say.
        let history_running = account.kind == AccountKind::Gmail && !account.history.folder.is_empty() && !account.history.complete;
        let scanned_all = !progress.folders.is_empty()
            && progress.folders.iter().all(|folder| folder.complete)
            && !history_running;
        let total = progress
            .inventory_total
            .map(|total| total.max(known.len() as u64))
            .or_else(|| {
                (account.kind == AccountKind::Sample
                    || (scanned_all
                        && progress.inventory_attempted
                        && !progress.storage_limited))
                    .then_some(known.len() as u64)
            });
        let percent = total.map(|total| {
            let percent = if total == 0 {
                100
            } else {
                (finished.len() as u64 * 100 / total).min(100) as u8
            };
            if progress.storage_limited || progress.partial > 0 {
                percent.min(99)
            } else {
                percent
            }
        });
        AccountIndexStatus {
            account_id: account.id,
            account: account.email.clone(),
            progress,
            indexed_present: finished.len(),
            total,
            percent,
            listed: known.len(),
            history: (account.kind == AccountKind::Gmail).then(|| account.history.clone()),
            include_spam_trash: account.storage_policy.include_spam_trash,
        }
    }
    /// Announce one account's indexing progress (`IndexProgress`) when its
    /// visible percent moved, or as a heartbeat when the same percent has held
    /// for `INDEX_HEARTBEAT_SECS` and the pass `worked` (counted the mailbox,
    /// indexed a message or moved a folder cursor): a caught-up rescan or an
    /// idle transition repeats nothing, or an idle account would look busy. The
    /// last percent announced per account lives in memory, never in the store:
    /// the durable checkpoint stays the per-page write, and an account without a
    /// computable total announces nothing — and is forgotten, so a later return
    /// to the same percent moves again instead of being deduped away. Entries
    /// of accounts the store no longer has are pruned on the same pass.
    fn announce_progress(&self, account_id: u32, worked: bool) {
        let Ok(store) = self.context.read() else {
            return;
        };
        let percent = store
            .account(account_id)
            .and_then(|account| Self::account_status(&store, account).percent);
        let now = self.context.clock.now();
        let previous = {
            let mut state = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            // A removed account never announces again. Prune here, where the
            // store read is already in hand: no extra I/O, and every indexed
            // page, census write and idle transition passes through this point.
            state
                .announced
                .retain(|id, _| store.account(*id).is_some());
            let Some(percent) = percent else {
                // The percent became unknowable (census total lost, folder
                // scope changed, account gone): drop the remembered value.
                state.announced.remove(&account_id);
                return;
            };
            let previous = state.announced.get(&account_id).map(|last| last.percent);
            let due = match state.announced.get(&account_id) {
                None => true,
                Some(last) if last.percent != percent => true,
                // The same percent: a heartbeat only after a pass that worked, and
                // a clock moved backwards counts as due, so it cannot mute it.
                Some(last) => {
                    worked && (now < last.at || now - last.at >= INDEX_HEARTBEAT_SECS)
                }
            };
            if !due {
                return;
            }
            state.announced.insert(account_id, Announced { percent, at: now });
            previous
        };
        // Emitted outside the lock: a desktop sink crosses the process boundary.
        if let Some(percent) = percent {
            self.context
                .events
                .emit(AppEvent::IndexProgress { account_id, percent });
            // The activity log (centro attività): the one moment indexing is done. Only the first
            // time per account: after a burst of new mail the percent drops and climbs back to
            // 100, which is the mailbox keeping up, not indexing finishing again.
            if index_completed(previous, percent)
                && self
                    .context
                    .repository
                    .last_activity(&[code::INDEX_COMPLETE], Some(account_id))
                    .is_ok_and(|last| last.is_none())
            {
                activity::record(
                    &self.context,
                    NewActivity {
                        kind: ActivityKind::Event,
                        code: code::INDEX_COMPLETE.into(),
                        account_id: Some(account_id),
                        ..Default::default()
                    },
                );
            }
        }
    }
    pub fn configure(
        &self,
        account_id: Option<u32>,
        enabled: bool,
        retry_partial: bool,
    ) -> AppResult<()> {
        if !enabled {
            self.cancel.store(true, Ordering::Relaxed);
        }
        self.context.update(|store| {
            if account_id.is_some_and(|id| store.account(id).is_none()) {
                return Err(AppError::missing("Account"));
            }
            let retry_accounts: std::collections::HashSet<_> = store
                .accounts
                .iter()
                .filter(|account| {
                    enabled && retry_partial && account_id.is_none_or(|id| account.id == id)
                })
                .map(|account| account.id)
                .collect();
            let readmitted_accounts: std::collections::HashSet<_> = store
                .emails
                .iter()
                .filter(|email| email.storage_evicted && retry_accounts.contains(&email.account_id))
                .map(|email| email.account_id)
                .collect();
            for account in store
                .accounts
                .iter_mut()
                .filter(|account| account_id.is_none_or(|id| account.id == id))
            {
                account.search_index.enabled = enabled;
                account.search_index.state = if enabled { "queued" } else { "paused" }.into();
                account.search_index.last_check = 0;
                account.search_index.notice = None;
                if enabled
                    && retry_partial
                    && (account.search_index.inventory_total.is_some()
                        || account.search_index.storage_limited
                        || readmitted_accounts.contains(&account.id))
                {
                    account.search_index.inventory_attempted = false;
                }
                if enabled
                    && retry_partial
                    && (account.search_index.storage_limited
                        || readmitted_accounts.contains(&account.id))
                {
                    account.search_index.storage_limited = false;
                    for folder in &mut account.search_index.folders {
                        folder.restart();
                    }
                }
            }
            if retry_partial {
                for email in store.emails.iter_mut().filter(|email| {
                    retry_accounts.contains(&email.account_id)
                        && (email.search_index.partial || email.storage_evicted)
                }) {
                    // Readmit only metadata on an explicit retry. Payloads stay
                    // remote and normal repository/index admission still applies.
                    email.storage_evicted = false;
                    email.search_index.version = 0;
                    email.search_index.partial = true;
                    email.search_index.retry_at = 0;
                    email.search_index.attempts = 0;
                }
            }
            Ok(())
        })
    }
    /// Returns true if visible progress changed. A single failing account cannot
    /// starve the others; transient failures have a one-minute retry interval.
    /// Accounts take turns one batch at a time.
    pub fn step(&self) -> AppResult<bool> {
        // A poisoned lock only means an earlier pass panicked; it must not
        // switch indexing off for the rest of the session.
        let _run = match self.running.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
        };
        self.cancel.store(false, Ordering::Relaxed);
        let store = self.context.read_shared()?;
        let now = self.context.clock.now();
        let mut accounts: Vec<_> = store
            .accounts
            .iter()
            .filter(|account| account.search_index.enabled)
            .cloned()
            .collect();
        drop(store);
        let last = self.last_account.load(Ordering::Relaxed);
        accounts.sort_by_key(|account| (account.id <= last, account.id));
        for account in accounts {
            if matches!(account.search_index.state.as_str(), "blocked" | "error" | STATE_UNLINKED | STATE_MODEL_MISMATCH)
                && now.saturating_sub(account.search_index.last_check) < 60
            {
                continue;
            }
            self.last_account.store(account.id, Ordering::Relaxed);
            let result = guarded(|| self.step_account(&account));
            if result.is_err() {
                // A failed connection is not reused.
                self.close_session(account.id);
            }
            match result {
                Ok(changed) if changed => return Ok(true),
                Ok(_) => continue,
                Err(error) if error.code == "index_paused" => return Ok(false),
                // Not a failure of the account: the compute server refuses this device (A8, its
                // notice said once per session by the gateway) or serves another model (D4).
                // Nothing is retried for a minute; no message loses an attempt.
                Err(error) if matches!(error.code, STOPS_ACCOUNT_UNAUTHORIZED | STOPS_ACCOUNT_MISMATCH) => {
                    self.set_waiting(account.id, false);
                    let state = if error.code == STOPS_ACCOUNT_MISMATCH { STATE_MODEL_MISMATCH } else { STATE_UNLINKED };
                    self.context.update(|store| {
                        if let Some(current) = store.accounts.iter_mut().find(|a| a.id == account.id) {
                            current.search_index.state = state.into();
                            current.search_index.notice = Some(error.message.clone());
                            current.search_index.last_check = now;
                        }
                        Ok(())
                    })?;
                    return Ok(true);
                }
                Err(error) => {
                    // The activity log (centro attività): the account's own failure,
                    // one row however often the minute-long retry meets it again.
                    activity::record_failure(
                        &self.context,
                        NewActivity {
                            kind: ActivityKind::Error,
                            code: code::INDEX_FAILED.into(),
                            account_id: Some(account.id),
                            detail: Some(error.message.clone()),
                            ..Default::default()
                        },
                        code::INDEX_COMPLETE,
                    );
                    self.context.update(|store| {
                        let Some(current) = store.accounts.iter_mut().find(|a| a.id == account.id)
                        else {
                            return Ok(());
                        };
                        current.search_index.state = if error.code == "storage_full" {
                            "blocked"
                        } else {
                            "error"
                        }
                        .into();
                        current.search_index.notice = Some(error.message.clone());
                        current.search_index.last_check = now;
                        Ok(())
                    })?;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    /// The open session of an account, or a new one. Taken out of the map while
    /// in use and put back by `keep_session` only after a successful batch.
    fn take_session(&self, account: &Account) -> AppResult<Box<dyn IndexSession>> {
        let kept = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&account.id);
        if let Some(session) = kept {
            if !session.expired() {
                return Ok(session);
            }
            // Closed here, outside the map's lock.
            drop(session);
        }
        if account.kind == AccountKind::Sample {
            return Ok(Box::new(SampleSession));
        }
        self.sessions_opened.fetch_add(1, Ordering::Relaxed);
        Ok(match self.source.open(account)? {
            Some(session) => session,
            None => Box::new(PerMessageSession { source: self.source.clone(), account: account.clone() }),
        })
    }
    /// An account with nothing left to read gives its connection back.
    fn close_session(&self, account_id: u32) {
        let kept = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&account_id);
        drop(kept);
    }
    fn keep_session(&self, account_id: u32, session: Box<dyn IndexSession>) {
        if !session.expired() {
            self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(account_id, session);
        }
    }
    /// The indexing activity of an account waits for the compute server, or no longer (INV-6):
    /// announced only when it changes.
    fn set_waiting(&self, account_id: u32, waiting: bool) {
        let changed = {
            let mut set = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
            if waiting { set.insert(account_id) } else { set.remove(&account_id) }
        };
        if changed {
            self.context.events.emit(AppEvent::IndexWaiting { account_id, waiting });
        }
    }
    /// D4 «reindex»: every vector goes, every message is embedded again by the next passes
    /// (with the model the server serves now). The word index and the counters stay.
    pub fn reindex_vectors(&self) -> AppResult<()> {
        self.cancel.store(true, Ordering::Relaxed);
        let _run = self.running.lock().unwrap_or_else(|e| e.into_inner());
        self.search.reset_vectors()?;
        self.context.update(|store| {
            for email in store.emails.iter_mut().filter(|email| email.search_index.version == VERSION) {
                email.search_index.semantic_pending = true;
                email.search_index.retry_at = 0;
            }
            for account in store.accounts.iter_mut() {
                if account.search_index.enabled {
                    account.search_index.state = "queued".into();
                }
                account.search_index.notice = None;
                account.search_index.last_check = 0;
            }
            Ok(())
        })?;
        self.cancel.store(false, Ordering::Relaxed);
        Ok(())
    }
    /// Sessions opened since the service started (tests and measurements).
    pub fn sessions_opened(&self) -> u64 {
        self.sessions_opened.load(Ordering::Relaxed)
    }
    fn step_account(&self, account: &Account) -> AppResult<bool> {
        let store = self.context.read_shared()?;
        let authenticated = self.context.account(&store, account.id)?;
        let now = self.context.clock.now();
        // At the start of every step (D4, A8): a server that refuses this device or serves
        // another model stops the account here, before any content is read.
        let semantic = self.search.semantic_readiness()?;
        if account.kind != AccountKind::Sample
            && !account.search_index.inventory_attempted
            && !account.search_index.folders.is_empty()
        {
            self.context.update(|store| {
                if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account.id) {
                    account.search_index.state = "counting".into();
                }
                Ok(())
            })?;
            let scanned: Vec<FolderProgress> = account.search_index.folders.iter().filter(|f| !f.view).cloned().collect();
            let total = self.source.inventory_total(&authenticated, &scanned, &self.cancel)?;
            self.context.update(|store| {
                if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account.id) {
                    if !account.search_index.enabled {
                        return Err(AppError::new("index_paused", "Indexing paused"));
                    }
                    account.search_index.inventory_total = total;
                    account.search_index.inventory_attempted = true;
                    account.search_index.state = "scanning".into();
                }
                Ok(())
            })?;
            self.announce_progress(account.id, true);
            return Ok(true);
        }
        // Alternate arrivals and historical batches so neither can starve the other.
        let history_due = account.kind != AccountKind::Sample
            && account.search_index.prefer_history
            && (account.search_index.folders.is_empty()
                || account.search_index.folders.iter().any(|f| !f.complete && !f.view));
        let mut pending: Vec<&Email> = store
            .emails
            .iter()
            .filter(|_| !history_due)
            .filter(|email| {
                email.account_id == account.id
                    && !email.storage_evicted
                    && email.search_index.retry_at <= now
                    && (email.search_index.version != VERSION
                        || email.search_index.content_hash != email.remote_content_hash
                        || (semantic && email.search_index.semantic_pending))
            })
            .filter(|email| {
                account.kind == AccountKind::Sample
                    || (email.uid.is_some() && email.remote_uid_validity.is_some())
            })
            .collect();
        if !pending.is_empty() {
            pending.sort_by_key(|email| std::cmp::Reverse((email.date, email.id)));
            let batch: Vec<Email> = pending.into_iter().take(self.batch).cloned().collect();
            drop(store);
            let mut session = self.take_session(&authenticated)?;
            self.process_batch(&authenticated, session.as_mut(), batch, None)?;
            self.keep_session(account.id, session);
            self.context.update(|store| {
                if let Some(account) = store.accounts.iter_mut().find(|a| a.id == account.id) {
                    account.search_index.prefer_history = true;
                }
                Ok(())
            })?;
            self.announce_progress(account.id, true);
            return Ok(true);
        }
        drop(store);
        if account.kind == AccountKind::Sample {
            return self.set_idle(account.id);
        }
        if account.search_index.folders.is_empty()
            || now.saturating_sub(account.search_index.last_discovery) >= 300
        {
            let folders = self.source.folders(&authenticated)?;
            self.context.update(|store| {
                let current = store
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == account.id)
                    .ok_or_else(|| AppError::missing("Account"))?;
                let previous_names: std::collections::HashSet<_> = current
                    .search_index
                    .folders
                    .iter()
                    .map(|folder| folder.name.as_str())
                    .collect();
                let incoming_names: std::collections::HashSet<_> =
                    folders.iter().map(|folder| folder.name.as_str()).collect();
                let scope_changed = previous_names != incoming_names;
                let had_inventory_scope = !current.search_index.folders.is_empty()
                    || current.search_index.inventory_total.is_some();
                if scope_changed && had_inventory_scope {
                    // A completed snapshot cannot describe an explicitly changed
                    // set of folders. Hide its percentage until the new census.
                    current.search_index.inventory_total = None;
                    current.search_index.inventory_attempted = false;
                }
                current.search_index.folders = folders
                    .into_iter()
                    .map(|folder| {
                        current
                            .search_index
                            .folders
                            .iter()
                            .find(|old| old.name == folder.name)
                            .cloned()
                            .unwrap_or(folder)
                    })
                    .collect();
                plan_folders(&mut current.search_index.folders);
                // With the history backfill running (W3.1) the rows of All Mail
                // come from it: the indexer only reads their contents (the
                // pending batches), never scans the folder again.
                if current.kind == AccountKind::Gmail && !current.history.folder.is_empty() {
                    for folder in current.search_index.folders.iter_mut().filter(|f| f.role == "Archive") {
                        folder.view = true;
                        folder.complete = true;
                    }
                }
                current.search_index.last_discovery = now;
                current.search_index.state = if current.search_index.inventory_attempted {
                    "scanning"
                } else {
                    "counting"
                }
                .into();
                Ok(())
            })?;
            return Ok(true);
        }
        let Some(folder) = next_folder(&account.search_index.folders).cloned() else {
            if now.saturating_sub(account.search_index.last_check) < 60 {
                return self.set_idle(account.id);
            }
            self.context.update(|store| {
                let current = store
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == account.id)
                    .ok_or_else(|| AppError::missing("Account"))?;
                for folder in current.search_index.folders.iter_mut().filter(|f| !f.view) {
                    folder.complete = false;
                }
                current.search_index.last_check = now;
                Ok(())
            })?;
            return Ok(true);
        };
        let mut session = self.take_session(&authenticated)?;
        let batch = session.next_batch(&folder, self.batch, &self.cancel)?;
        let worked = !batch.emails.is_empty()
            || batch.progress.after_uid != folder.after_uid
            || batch.progress.before_uid != folder.before_uid;
        self.process_batch(&authenticated, session.as_mut(), batch.emails, Some(batch.progress))?;
        self.keep_session(account.id, session);
        // The batch checkpoint is durable; the announced percent lives in memory
        // and travels when it visibly moved, or as a heartbeat while it holds.
        self.announce_progress(account.id, worked);
        Ok(true)
    }
    fn set_idle(&self, id: u32) -> AppResult<bool> {
        // A caught-up account keeps its connection for the next rescan until it
        // expires (10 minutes), well before Gmail drops an idle one.
        let expired = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&id).is_some_and(|s| s.expired());
        if expired {
            self.close_session(id);
        }
        let store = self.context.read_shared()?;
        let semantic = self.search.installed();
        let mut waiting = false;
        let partial = store.emails.iter().any(|email| {
            if email.account_id != id {
                return false;
            }
            waiting |= email.search_index.semantic_pending && !semantic;
            email.storage_evicted
                || email.search_index.partial
                || email.search_index.version != VERSION
        });
        let limited = store
            .account(id)
            .is_some_and(|account| account.search_index.storage_limited);
        let target = if partial || limited {
            "partial"
        } else if waiting && self.search.needs_server() {
            "needs_server"
        } else if waiting {
            "waiting_model"
        } else {
            "watching"
        };
        if store
            .account(id)
            .is_none_or(|account| account.search_index.state == target)
        {
            return Ok(false);
        }
        drop(store);
        self.context.update(|store| {
            if let Some(account) = store.accounts.iter_mut().find(|a| a.id == id) {
                account.search_index.state = target.into();
            }
            Ok(())
        })?;
        self.announce_progress(id, false);
        Ok(true)
    }
    /// One batch: the rows and the contents' identities in one write, the
    /// index outside any lock, then the outcomes and the folder cursor in a
    /// second write (INV-4). One failing message never stops the batch.
    fn process_batch(
        &self,
        account: &Account,
        session: &mut dyn IndexSession,
        emails: Vec<Email>,
        progress: Option<FolderProgress>,
    ) -> AppResult<()> {
        let semantic = self.search.installed();
        let scanned = progress.as_ref().map_or(0, |_| emails.len() as u64);
        if emails.is_empty() {
            // Nothing to insert or index: only the cursor moves, in one write.
            return self.finish_batch(account, &[], progress, 0);
        }
        // 1. Which messages need their contents: read only, no write yet.
        let wanted: Vec<bool> = {
            let store = self.context.read_shared()?;
            let mut known: Vec<&Email> = Vec::new();
            emails
                .iter()
                .map(|email| {
                    let row = if email.id != 0 {
                        store.email(email.id).filter(|row| row.account_id == account.id)
                    } else {
                        store.emails.iter().find(|row| row.account_id == account.id && crate::domain::mail_identity::same_message(row, email))
                    };
                    let duplicate = email.id == 0 && known.iter().any(|seen| crate::domain::mail_identity::same_message(seen, email));
                    known.push(email);
                    !duplicate
                        && row.is_none_or(|row| {
                            !row.storage_evicted
                                && (row.search_index.version != VERSION
                                    || row.search_index.content_hash != row.remote_content_hash
                                    || (semantic && row.search_index.semantic_pending))
                        })
                })
                .collect()
        };
        // 2. Contents, outside every lock.
        let fetch: Vec<Email> = emails.iter().zip(&wanted).filter(|(_, wanted)| **wanted).map(|(email, _)| email.clone()).collect();
        let mut fetched = if fetch.is_empty() { Vec::new() } else { session.contents(&fetch, &self.cancel) }.into_iter();
        let mut contents: Vec<Option<AppResult<IndexContent>>> = wanted
            .iter()
            .map(|wanted| if *wanted { fetched.next() } else { None })
            .collect();
        if contents.iter().any(|c| matches!(c, Some(Err(error)) if error.code == "index_paused")) {
            return Err(AppError::new("index_paused", "Indexing paused"));
        }
        // 3. One write: insert or enrich the rows, record the contents' identity.
        let targets: Vec<Option<u64>> = self.context.update(|store| {
            if !store
                .account(account.id)
                .is_some_and(|a| a.search_index.enabled && a.email == account.email)
            {
                return Err(AppError::new("index_paused", "Account changed or paused"));
            }
            let mut targets = Vec::new();
            for (email, content) in emails.iter().zip(&contents) {
                let content = content.as_ref().and_then(|c| c.as_ref().ok());
                let found = if email.id != 0 {
                    store.emails.iter().position(|row| row.id == email.id && row.account_id == account.id)
                } else {
                    store.emails.iter().position(|row| {
                        row.account_id == account.id
                            && (crate::domain::mail_identity::same_message(row, email)
                                || content.is_some_and(|c| crate::domain::mail_identity::same_message(row, &c.email)))
                    })
                };
                let index = match found {
                    Some(index) => {
                        if email.id == 0 {
                            crate::domain::mail_identity::merge_remote(&mut store.emails[index], email);
                        }
                        index
                    }
                    None if email.id != 0 => {
                        targets.push(None);
                        continue;
                    }
                    None => {
                        let mut row = email.clone();
                        row.id = store.new_id();
                        for (part, attachment) in row.attachments.iter_mut().enumerate() {
                            attachment.id = row.id.saturating_mul(1000).saturating_add(part as u64);
                        }
                        store.emails.push(row);
                        store.emails.len() - 1
                    }
                };
                let row = &mut store.emails[index];
                if let Some(content) = content {
                    row.remote_content_hash = content.email.remote_content_hash.clone();
                    if content.email.remote_global_id.is_some() {
                        row.remote_global_id = content.email.remote_global_id;
                    }
                    if row.remote_only {
                        let offset = row.id.saturating_mul(1000);
                        row.attachments = content
                            .email
                            .attachments
                            .iter()
                            .cloned()
                            .map(|mut part| {
                                part.id = offset.saturating_add(part.id % 1000);
                                part.path = None;
                                part.inline_text = None;
                                part
                            })
                            .collect();
                    }
                }
                targets.push(Some(row.id));
            }
            Ok(targets)
        })?;
        // 4. The index, outside every lock.
        let snapshot = self.context.read()?;
        let mut outcomes: Vec<Outcome> = Vec::new();
        // A `401` (A8) or another model (D4) halfway: the batch stops, its messages keep their
        // attempts, the outcomes so far and the cursor are written, the account stops.
        let mut stopped: Option<AppError> = None;
        for (target, content) in targets.iter().zip(contents.iter_mut()) {
            let Some(id) = target else { continue };
            let Some(content) = content.take() else { continue };
            // Only a message this batch is about to index can be blocked by the
            // quota; an evicted row already known is passed over like before.
            let Some(row) = snapshot.email(*id).filter(|row| !row.storage_evicted) else {
                return Err(AppError::new("storage_full", "Local metadata quota reached; indexing paused"));
            };
            let hash = row.remote_content_hash.clone();
            let mut content = match content {
                Ok(content) => content,
                Err(error) if error.code == "index_content_limit" => {
                    outcomes.push(Outcome::Finish { id: *id, hash, partial: true, notice: Some(error.message), semantic_pending: false });
                    continue;
                }
                // The message itself cannot be read: retrying cannot change that.
                Err(error) if matches!(error.code, "index_content_unreadable" | "index_panic") => {
                    outcomes.push(Outcome::Finish { id: *id, hash, partial: true, notice: Some(format!("Message set aside: {}", error.message)), semantic_pending: false });
                    continue;
                }
                // Network and server errors say nothing about the message.
                Err(error) => {
                    outcomes.push(Outcome::Retry { id: *id, error: error.message, counted: false });
                    continue;
                }
            };
            if row.search_index.version == VERSION
                && row.search_index.content_hash == content.email.remote_content_hash
                && !(semantic && row.search_index.semantic_pending)
            {
                continue;
            }
            if content.email.id != *id {
                let offset = id.saturating_mul(1000);
                for attachment in &mut content.email.attachments {
                    attachment.id = offset.saturating_add(attachment.id % 1000);
                }
                for media in &mut content.media {
                    media.attachment_id = offset.saturating_add(media.attachment_id % 1000);
                }
            }
            content.email.id = *id;
            content.email.account_id = account.id;
            let result = if semantic {
                guarded(|| self.search.index_document(&snapshot, &content, &self.cancel))
            } else {
                guarded(|| self.search.index_lexical(&snapshot, &content, &self.cancel))
            };
            let hash = content.email.remote_content_hash.clone();
            outcomes.push(match result {
                Ok(report) if report.indexed => Outcome::Finish {
                    id: *id,
                    hash,
                    partial: !report.warnings.is_empty(),
                    notice: (!report.warnings.is_empty()).then(|| report.warnings.join("; ")),
                    semantic_pending: !semantic,
                },
                Ok(_) => return Err(AppError::external("Indexing did not commit this message")),
                Err(error) if error.code == "index_content_limit" => Outcome::Finish { id: *id, hash, partial: true, notice: Some(error.message), semantic_pending: false },
                Err(error) if matches!(error.code, "storage_full" | "index_paused") => return Err(error),
                Err(error) if matches!(error.code, STOPS_ACCOUNT_UNAUTHORIZED | STOPS_ACCOUNT_MISMATCH) => {
                    outcomes.push(Outcome::Retry { id: *id, error: error.message.clone(), counted: false });
                    stopped = Some(error);
                    break;
                }
                Err(error) if error.code == "index_content_unreadable" => Outcome::Finish { id: *id, hash, partial: true, notice: Some(format!("Message set aside: {}", error.message)), semantic_pending: false },
                // A pause says nothing about the message: wait, never count it.
                Err(error) if error.code == "index_transient" => Outcome::Retry { id: *id, error: error.message, counted: false },
                Err(error) => Outcome::Fail { id: *id, hash, error: error.message },
            });
        }
        drop(snapshot);
        if self.cancel.load(Ordering::Relaxed) {
            return Err(AppError::new("index_paused", "Indexing paused"));
        }
        let waiting = outcomes.iter().any(|outcome| matches!(outcome, Outcome::Retry { counted: false, .. }));
        let worked = outcomes.iter().any(|outcome| matches!(outcome, Outcome::Finish { .. }));
        self.finish_batch(account, &outcomes, progress, scanned)?;
        if let Some(error) = stopped {
            return Err(error);
        }
        // INV-6: a pause of the compute server is the state of the indexing activity.
        if waiting && !worked {
            self.set_waiting(account.id, true);
        } else if worked {
            self.set_waiting(account.id, false);
        }
        Ok(())
    }
    /// The batch's last write: every outcome and the folder cursor together (INV-4).
    fn finish_batch(
        &self,
        account: &Account,
        outcomes: &[Outcome],
        progress: Option<FolderProgress>,
        scanned: u64,
    ) -> AppResult<()> {
        if outcomes.is_empty() && progress.is_none() {
            return Ok(());
        }
        let now = self.context.clock.now();
        self.context.update(|store| {
            for outcome in outcomes {
                apply_outcome(store, outcome, now)?;
            }
            if let Some(progress) = &progress {
                let current = store
                    .accounts
                    .iter_mut()
                    .find(|a| a.id == account.id && a.search_index.enabled)
                    .ok_or_else(|| AppError::new("index_paused", "Indexing paused"))?;
                if let Some(folder) = current
                    .search_index
                    .folders
                    .iter_mut()
                    .find(|folder| folder.name == progress.name)
                {
                    *folder = progress.clone();
                }
                current.search_index.scanned += scanned;
                current.search_index.prefer_history = false;
                current.search_index.state = "scanning".into();
                current.search_index.last_check = now;
            }
            Ok(())
        })?;
        Ok(())
    }
}

/// Messages per batch (W1.2).
const BATCH: usize = 25;

/// Error codes that stop an account's indexing without being its failure (A8, D4).
const STOPS_ACCOUNT_UNAUTHORIZED: &str = "compute_unauthorized";
const STOPS_ACCOUNT_MISMATCH: &str = super::compute::codes::MODEL_MISMATCH;
/// The account waits for the person to sign in again (A8, EX-15).
pub const STATE_UNLINKED: &str = "unlinked";
/// The account waits for the person to reindex (D4, EX-7).
pub const STATE_MODEL_MISMATCH: &str = "model_mismatch";

/// What a batch decided for one message, written in its final transaction.
enum Outcome {
    Finish { id: u64, hash: String, partial: bool, notice: Option<String>, semantic_pending: bool },
    /// Retry later; `counted` failures use up the message's attempts.
    Retry { id: u64, error: String, counted: bool },
    /// A failure caused by the message: bounded retries, then set aside.
    Fail { id: u64, hash: String, error: String },
}

fn apply_outcome(store: &mut crate::model::Store, outcome: &Outcome, now: i64) -> AppResult<()> {
    let finish = |store: &mut crate::model::Store, id: u64, hash: &str, partial: bool, notice: Option<String>, semantic_pending: bool| -> AppResult<()> {
        let Some(email) = store.emails.iter_mut().find(|email| email.id == id) else {
            return Err(AppError::new("storage_full", "Message was evicted before its index checkpoint"));
        };
        // The vectors added after a words-only pass complete the same message.
        let counted = email.search_index.semantic_pending
            && email.search_index.version == VERSION
            && email.search_index.content_hash == hash
            && !email.search_index.partial;
        email.search_index = MessageIndexState {
            version: VERSION,
            content_hash: hash.into(),
            partial,
            notice: notice.clone(),
            retry_at: 0,
            attempts: 0,
            semantic_pending,
        };
        let account_id = email.account_id;
        if let Some(account) = store.accounts.iter_mut().find(|account| account.id == account_id) {
            if !counted {
                account.search_index.indexed += u64::from(!partial);
                account.search_index.partial += u64::from(partial);
            }
            account.search_index.state = if account.search_index.enabled { "scanning" } else { "paused" }.into();
            account.search_index.notice = notice;
        }
        Ok(())
    };
    let retry = |store: &mut crate::model::Store, id: u64, error: &str, counted: bool| {
        if let Some(email) = store.emails.iter_mut().find(|email| email.id == id) {
            email.search_index.partial = true;
            email.search_index.notice = Some(error.into());
            let delay = if counted {
                email.search_index.attempts = email.search_index.attempts.saturating_add(1);
                (RETRY_BASE_SECS << (email.search_index.attempts - 1).min(10)).min(RETRY_MAX_SECS)
            } else {
                RETRY_BASE_SECS
            };
            email.search_index.retry_at = now.saturating_add(delay);
            let account_id = email.account_id;
            if let Some(account) = store.accounts.iter_mut().find(|account| account.id == account_id) {
                account.search_index.notice = Some(error.into());
            }
        }
    };
    match outcome {
        Outcome::Finish { id, hash, partial, notice, semantic_pending } => finish(store, *id, hash, *partial, notice.clone(), *semantic_pending),
        Outcome::Retry { id, error, counted } => {
            retry(store, *id, error, *counted);
            Ok(())
        }
        Outcome::Fail { id, hash, error } => {
            let attempts = store.email(*id).map_or(0, |email| email.search_index.attempts).saturating_add(1);
            if attempts >= MAX_MESSAGE_ATTEMPTS {
                finish(store, *id, hash, true, Some(format!("Message skipped after {MAX_MESSAGE_ATTEMPTS} attempts: {error}")), false)
            } else {
                retry(store, *id, error, true);
                Ok(())
            }
        }
    }
}

/// Sample accounts have no provider: their stored text is their content.
struct SampleSession;
impl IndexSession for SampleSession {
    fn next_batch(&mut self, folder: &FolderProgress, _: usize, _: &AtomicBool) -> AppResult<IndexBatch> {
        Ok(IndexBatch { progress: folder.clone(), emails: vec![] })
    }
    fn contents(&mut self, emails: &[Email], _: &AtomicBool) -> Vec<AppResult<IndexContent>> {
        emails.iter().map(|email| Ok(IndexContent { email: email.clone(), media: vec![], warnings: vec![] })).collect()
    }
    fn expired(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    //! Review 2.2b, Minor 3: the in-memory percent state must not outlive what
    //! it describes. A percent that becomes unknowable clears the remembered
    //! value, and entries of removed accounts are pruned.
    use super::*;
    use crate::{
        application::ports::{
            AttachmentGateway, Clock, EventSink, MailGateway, OutgoingMessage, RemoteAction,
            SearchHit, SecretStore, SyncBatch,
        },
        domain::model::{Attachment, Store},
        infrastructure::repository::MemoryRepository,
    };
    use std::sync::atomic::AtomicI64;

    /// Records every emission, in order.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<AppEvent>>);
    impl EventSink for Recorder {
        fn emit(&self, event: AppEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    impl Recorder {
        fn events(&self) -> Vec<AppEvent> {
            self.0.lock().unwrap().clone()
        }
    }
    struct NoMail;
    impl MailGateway for NoMail {
        fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
            Ok(SyncBatch {
                emails: vec![],
                archive_folder: None,
                trash_folder: None,
            })
        }
        fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
            Ok(())
        }
        fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
            Ok(())
        }
        fn check_login(&self, _: &Account) -> AppResult<()> {
            Ok(())
        }
    }
    struct NoSecrets;
    impl SecretStore for NoSecrets {
        fn get(&self, _: &str) -> AppResult<String> {
            Ok("test-only".into())
        }
        fn set(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }
    }
    /// A clock the tests move by hand, in whole seconds.
    struct TickClock(AtomicI64);
    impl Clock for TickClock {
        fn now(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    impl TickClock {
        fn advance(&self, seconds: i64) {
            self.0.fetch_add(seconds, Ordering::SeqCst);
        }
    }
    struct NoFiles;
    impl AttachmentGateway for NoFiles {
        fn total_size(&self, paths: &[String]) -> AppResult<u64> {
            Ok(paths.len() as u64)
        }
        fn read_text(&self, _: &Attachment) -> AppResult<String> {
            Ok(String::new())
        }
    }
    struct NoSource;
    impl IndexSource for NoSource {
        fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
            Ok(vec![])
        }
        fn next(
            &self,
            _: &Account,
            folder: &FolderProgress,
            _: &AtomicBool,
        ) -> AppResult<IndexPage> {
            Ok(IndexPage {
                progress: folder.clone(),
                email: None,
            })
        }
        fn content(
            &self,
            _: &Account,
            email: &Email,
            _: &AtomicBool,
        ) -> AppResult<IndexContent> {
            Ok(IndexContent {
                email: email.clone(),
                media: vec![],
                warnings: vec![],
            })
        }
    }
    struct ReadySearch;
    impl SearchGateway for ReadySearch {
        fn search(&self, _: &Store, _: &str, _: usize) -> AppResult<Vec<SearchHit>> {
            Ok(vec![])
        }
        fn index(&self, _: &Store) -> AppResult<String> {
            Ok(String::new())
        }
        fn installed(&self) -> bool {
            true
        }
    }

    fn indexer_with(
        store: Store,
    ) -> (
        MailIndexService,
        Arc<Recorder>,
        Arc<ServiceContext>,
    ) {
        let (indexer, recorder, context, _) = indexer_with_clock(store);
        (indexer, recorder, context)
    }
    fn indexer_with_clock(
        store: Store,
    ) -> (
        MailIndexService,
        Arc<Recorder>,
        Arc<ServiceContext>,
        Arc<TickClock>,
    ) {
        let recorder = Arc::new(Recorder::default());
        let clock = Arc::new(TickClock(AtomicI64::new(1_000)));
        let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(crate::application::compute::NoCompute),
            repository: Arc::new(MemoryRepository::new(store)),
            mail: Arc::new(NoMail),
            secrets: Arc::new(NoSecrets),
            clock: clock.clone(),
            attachments: Arc::new(NoFiles),
            events: recorder.clone(),
        });
        let indexer = MailIndexService::new(
            context.clone(),
            Arc::new(NoSource),
            Arc::new(ReadySearch),
        );
        (indexer, recorder, context, clock)
    }

    /// One account with a known census: two messages, one indexed — percent 50.
    fn half_indexed_store() -> Store {
        let mut store = Store::default();
        store.accounts.push(Account {
            id: 5,
            kind: AccountKind::Gmail,
            email: "half@example.com".into(),
            search_index: IndexProgress {
                inventory_total: Some(2),
                inventory_attempted: true,
                ..Default::default()
            },
            ..Default::default()
        });
        store.emails.push(Email {
            id: 91,
            account_id: 5,
            subject: "Indexed".into(),
            search_index: MessageIndexState {
                version: VERSION,
                ..Default::default()
            },
            ..Default::default()
        });
        store.emails.push(Email {
            id: 92,
            account_id: 5,
            subject: "Pending".into(),
            ..Default::default()
        });
        store
    }
    fn set_census(context: &ServiceContext, total: Option<u64>) {
        context
            .update(|store| {
                let account = store
                    .accounts
                    .iter_mut()
                    .find(|account| account.id == 5)
                    .expect("account 5");
                account.search_index.inventory_total = total;
                account.search_index.inventory_attempted = total.is_some();
                Ok(())
            })
            .unwrap();
    }

    /// P -> None -> P announces P twice: an unknowable percent forgets the last
    /// announced value instead of leaving it to suppress the return.
    #[test]
    fn an_unknowable_percent_is_forgotten_and_announced_again() {
        let (indexer, recorder, context) = indexer_with(half_indexed_store());
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events(),
            vec![AppEvent::IndexProgress {
                account_id: 5,
                percent: 50
            }]
        );
        // The folder scope changed: the census total is unknowable again.
        set_census(&context, None);
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events().len(),
            1,
            "an unknowable percent announces nothing: {:?}",
            recorder.events()
        );
        assert!(
            indexer.progress.lock().unwrap().announced.is_empty(),
            "and forgets the last announced percent: {:?}",
            indexer.progress.lock().unwrap().announced
        );
        // The new census lands on the very same numbers as before.
        set_census(&context, Some(2));
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events(),
            vec![
                AppEvent::IndexProgress {
                    account_id: 5,
                    percent: 50
                },
                AppEvent::IndexProgress {
                    account_id: 5,
                    percent: 50
                },
            ],
            "the return to a known percent is announced again"
        );
    }

    fn progress_50() -> AppEvent {
        AppEvent::IndexProgress {
            account_id: 5,
            percent: 50,
        }
    }

    /// On a big mailbox the visible percent can stay put for minutes while the
    /// indexer works. The same percent travels again once the heartbeat interval
    /// has passed, so the UI can tell a slow account from a stuck one.
    #[test]
    fn an_unchanged_percent_is_announced_again_after_the_heartbeat_interval() {
        let (indexer, recorder, _context, clock) = indexer_with_clock(half_indexed_store());
        indexer.announce_progress(5, true);
        assert_eq!(recorder.events(), vec![progress_50()]);
        // Same percent well inside the interval: a repeat would be noise.
        clock.advance(INDEX_HEARTBEAT_SECS - 1);
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events(),
            vec![progress_50()],
            "no repeat before the interval"
        );
        // The interval elapsed: the unchanged percent is announced as a heartbeat.
        clock.advance(1);
        indexer.announce_progress(5, true);
        assert_eq!(recorder.events(), vec![progress_50(), progress_50()]);
        // The interval restarts from the heartbeat, not from the first announce.
        clock.advance(INDEX_HEARTBEAT_SECS - 1);
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events().len(),
            2,
            "a heartbeat restarts the interval: {:?}",
            recorder.events()
        );
    }

    /// The heartbeat means "the indexer worked", not "the indexer was called": a
    /// caught-up account rescanned every minute must not look busy for ever.
    #[test]
    fn an_idle_pass_never_sends_a_heartbeat() {
        let (indexer, recorder, _context, clock) = indexer_with_clock(half_indexed_store());
        indexer.announce_progress(5, true);
        clock.advance(INDEX_HEARTBEAT_SECS * 4);
        indexer.announce_progress(5, false);
        assert_eq!(
            recorder.events(),
            vec![progress_50()],
            "an idle pass repeats nothing, however late it comes"
        );
        // The next pass that did work is due at once: the interval has long run.
        indexer.announce_progress(5, true);
        assert_eq!(recorder.events(), vec![progress_50(), progress_50()]);
    }

    /// An idle pass still announces a percent that moved: the end of the work
    /// is noticed by a pass that did none.
    #[test]
    fn a_moved_percent_is_announced_by_an_idle_pass_too() {
        let (indexer, recorder, context, _clock) = indexer_with_clock(half_indexed_store());
        indexer.announce_progress(5, true);
        context
            .update(|store| {
                let email = store.emails.iter_mut().find(|email| email.id == 92);
                email.expect("pending message").search_index.version = VERSION;
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, false);
        assert_eq!(
            recorder.events(),
            vec![
                progress_50(),
                AppEvent::IndexProgress {
                    account_id: 5,
                    percent: 100
                },
                // Reaching 100% from a watched percent is also the one moment
                // the activity log records "indexing finished" (row 1).
                AppEvent::ActivityChanged { ids: vec![1] },
            ]
        );
    }

    /// A wall clock moved backwards must not mute the heartbeat until it catches
    /// up: the account would be shown as stuck while it works.
    #[test]
    fn a_clock_moved_backwards_does_not_mute_the_heartbeat() {
        let (indexer, recorder, _context, clock) = indexer_with_clock(half_indexed_store());
        indexer.announce_progress(5, true);
        clock.advance(-3_600);
        indexer.announce_progress(5, true);
        assert_eq!(
            recorder.events(),
            vec![progress_50(), progress_50()],
            "the unchanged percent is announced again after a backward jump"
        );
    }

    /// An account removed from the store leaves no percent entry behind: the
    /// next announce pass prunes it.
    #[test]
    fn a_removed_accounts_entry_is_pruned() {
        let (indexer, recorder, context) = indexer_with(half_indexed_store());
        indexer.announce_progress(5, true);
        assert_eq!(
            indexer.progress.lock().unwrap().announced.get(&5).map(|last| last.percent),
            Some(50),
            "the announced percent is remembered"
        );
        context
            .update(|store| {
                store.accounts.retain(|account| account.id != 5);
                store.emails.retain(|email| email.account_id != 5);
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, true);
        assert!(
            !indexer.progress.lock().unwrap().announced.contains_key(&5),
            "a removed account leaves no percent behind: {:?}",
            indexer.progress.lock().unwrap().announced
        );
        assert_eq!(
            recorder.events().len(),
            1,
            "the removed account announces nothing new: {:?}",
            recorder.events()
        );
    }

    // ---- the activity log (centro attività, Task 5) ----

    #[test]
    fn index_completion_needs_a_previous_lower_percent() {
        assert!(index_completed(Some(99), 100));
        assert!(index_completed(Some(0), 100));
        assert!(!index_completed(None, 100), "first seen at 100: nothing was watched finishing");
        assert!(!index_completed(Some(100), 100));
        assert!(!index_completed(Some(40), 60));
        assert!(!index_completed(Some(100), 40), "a census that grew is not a completion");
    }

    fn logged(context: &ServiceContext) -> Vec<crate::domain::activity::ActivityEntry> {
        context
            .repository
            .activity_page(None, 100, crate::domain::activity::ActivityFilter::All)
            .unwrap()
    }

    #[test]
    fn index_completion_is_logged_once_per_account() {
        let (indexer, _recorder, context) = indexer_with(half_indexed_store());
        indexer.announce_progress(5, true);
        assert!(logged(&context).is_empty(), "50% is progress, not a fact");
        // The pending message gets indexed too: 100%.
        context
            .update(|store| {
                let pending = store.emails.iter_mut().find(|email| email.id == 92).expect("email 92");
                pending.search_index.version = VERSION;
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, true);
        let rows = logged(&context);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "indexComplete");
        assert_eq!(rows[0].account_id, Some(5));
        assert_eq!(rows[0].kind, crate::domain::activity::ActivityKind::Event);
        indexer.announce_progress(5, true);
        assert_eq!(logged(&context).len(), 1, "an unchanged 100% is not a new completion");
    }

    #[test]
    fn a_burst_of_new_mail_on_a_finished_account_is_not_a_new_completion() {
        let (indexer, _recorder, context) = indexer_with(half_indexed_store());
        indexer.announce_progress(5, true);
        // The pending message gets indexed: the one moment indexing is done.
        context
            .update(|store| {
                let pending = store.emails.iter_mut().find(|email| email.id == 92).expect("email 92");
                pending.search_index.version = VERSION;
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, true);
        assert_eq!(logged(&context).len(), 1);

        // A sync brings two new messages: the percent drops below 100 and climbs back as they are
        // indexed. That is the mailbox keeping up, not indexing finishing again.
        context
            .update(|store| {
                for id in [93, 94] {
                    store.emails.push(Email {
                        id,
                        account_id: 5,
                        subject: format!("New {id}"),
                        ..Default::default()
                    });
                }
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, false);
        context
            .update(|store| {
                for id in [93, 94] {
                    let email = store.emails.iter_mut().find(|email| email.id == id).expect("new email");
                    email.search_index.version = VERSION;
                }
                Ok(())
            })
            .unwrap();
        indexer.announce_progress(5, true);
        assert_eq!(
            logged(&context).len(),
            1,
            "a mailbox that already finished indexing does not log it again after every burst of mail"
        );
    }

    /// A source whose census always fails, the way an unreachable server does.
    struct FailingSource;
    impl IndexSource for FailingSource {
        fn inventory_total(
            &self,
            _: &Account,
            _: &[FolderProgress],
            _: &AtomicBool,
        ) -> AppResult<Option<u64>> {
            Err(AppError::external("IMAP connect: timed out"))
        }
        fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
            Ok(vec![])
        }
        fn next(&self, _: &Account, folder: &FolderProgress, _: &AtomicBool) -> AppResult<IndexPage> {
            Ok(IndexPage { progress: folder.clone(), email: None })
        }
        fn content(&self, _: &Account, email: &Email, _: &AtomicBool) -> AppResult<IndexContent> {
            Ok(IndexContent { email: email.clone(), media: vec![], warnings: vec![] })
        }
    }

    fn unreachable_account_store() -> Store {
        let mut store = Store::default();
        store.accounts.push(Account {
            id: 5,
            kind: AccountKind::Gmail,
            email: "down@example.com".into(),
            search_index: IndexProgress {
                folders: vec![FolderProgress { name: "INBOX".into(), ..Default::default() }],
                inventory_attempted: false,
                ..Default::default()
            },
            ..Default::default()
        });
        store
    }

    #[test]
    fn a_failing_account_is_logged_once_however_often_it_retries() {
        let clock = Arc::new(TickClock(AtomicI64::new(1_000)));
        let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(crate::application::compute::NoCompute),
            repository: Arc::new(MemoryRepository::new(unreachable_account_store())),
            mail: Arc::new(NoMail),
            secrets: Arc::new(NoSecrets),
            clock: clock.clone(),
            attachments: Arc::new(NoFiles),
            events: Arc::new(Recorder::default()),
        });
        let indexer = MailIndexService::new(context.clone(), Arc::new(FailingSource), Arc::new(ReadySearch));

        assert!(indexer.step().unwrap(), "the failure is a visible change");
        let rows = logged(&context);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "indexFailed");
        assert_eq!(rows[0].account_id, Some(5));
        assert_eq!(rows[0].kind, crate::domain::activity::ActivityKind::Error);
        assert_eq!(rows[0].detail.as_deref(), Some("IMAP connect: timed out"));

        // The pass retries a minute later and fails the same way, every minute.
        for _ in 0..3 {
            clock.advance(61);
            indexer.step().unwrap();
        }
        assert_eq!(logged(&context).len(), 1, "the same failure is not logged again");
    }

    // ---------------------------------------------------------------------
    // Batches (W1.2): one session for many batches, two writes per batch,
    // Gmail history read once in All Mail, words without the model.

    /// A provider mailbox: messages per folder, read with the real cursor logic.
    #[derive(Default)]
    struct MailboxData {
        folders: Vec<FolderProgress>,
        messages: Mutex<std::collections::HashMap<String, Vec<Email>>>,
        /// A session expires after this many batches (0: never).
        batches_per_session: usize,
        huge: Mutex<std::collections::HashSet<u32>>,
        reads: Mutex<Vec<String>>,
        content_calls: AtomicU32,
    }
    struct Mailbox(Arc<MailboxData>);
    impl IndexSource for Mailbox {
        fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
            Ok(self.0.folders.clone())
        }
        fn open(&self, _: &Account) -> AppResult<Option<Box<dyn IndexSession>>> {
            Ok(Some(Box::new(MailboxSession { data: self.0.clone(), batches: 0 })))
        }
    }
    struct MailboxSession {
        data: Arc<MailboxData>,
        batches: usize,
    }
    impl IndexSession for MailboxSession {
        fn next_batch(&mut self, folder: &FolderProgress, max: usize, _: &AtomicBool) -> AppResult<IndexBatch> {
            self.batches += 1;
            self.data.reads.lock().unwrap().push(folder.name.clone());
            let messages = self.data.messages.lock().unwrap().get(&folder.name).cloned().unwrap_or_default();
            let uids: Vec<u32> = messages.iter().filter_map(|e| e.uid).collect();
            let upper = uids.iter().copied().max().unwrap_or(0);
            let mut progress = folder.clone();
            progress.uid_validity = Some(7);
            let mut taken = Vec::new();
            while taken.len() < max {
                let Some(window) = progress.plan(upper, 2048) else { break };
                taken.extend(progress.advance_many(window, uids.iter().copied(), upper, max - taken.len()));
            }
            let emails = taken.iter().filter_map(|uid| messages.iter().find(|e| e.uid == Some(*uid)).cloned()).collect();
            Ok(IndexBatch { progress, emails })
        }
        fn contents(&mut self, emails: &[Email], _: &AtomicBool) -> Vec<AppResult<IndexContent>> {
            emails
                .iter()
                .map(|email| {
                    self.data.content_calls.fetch_add(1, Ordering::SeqCst);
                    if self.data.huge.lock().unwrap().contains(&email.uid.unwrap()) {
                        return Err(AppError::new("index_content_limit", "Message exceeds the 32 MiB temporary download budget; contents not indexed"));
                    }
                    let mut email = email.clone();
                    email.body = format!("body {}", email.uid.unwrap());
                    email.remote_only = false;
                    email.remote_content_hash = format!("mime-{}", email.remote_global_id.unwrap_or(u64::from(email.uid.unwrap())));
                    Ok(IndexContent { email, media: vec![], warnings: vec![] })
                })
                .collect()
        }
        fn expired(&self) -> bool {
            self.data.batches_per_session != 0 && self.batches >= self.data.batches_per_session
        }
    }
    /// Counts the store writes.
    struct CountingRepository {
        inner: MemoryRepository,
        writes: AtomicU32,
    }
    impl crate::application::ports::StoreRepository for CountingRepository {
        fn read(&self) -> AppResult<Store> {
            self.inner.read()
        }
        fn transaction(&self, operation: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.inner.transaction(operation)
        }
    }
    #[derive(Default)]
    struct WordsSearch {
        installed: AtomicBool,
        needs_server: AtomicBool,
        documents: Mutex<Vec<u64>>,
        words: Mutex<Vec<u64>>,
    }
    impl SearchGateway for WordsSearch {
        fn index_document(&self, _: &Store, content: &IndexContent, _: &AtomicBool) -> AppResult<DocumentIndexReport> {
            assert!(content.email.body.starts_with("body "), "the temporary body reaches the index");
            self.documents.lock().unwrap().push(content.email.id);
            Ok(DocumentIndexReport { indexed: true, warnings: vec![] })
        }
        fn index_lexical(&self, _: &Store, content: &IndexContent, _: &AtomicBool) -> AppResult<DocumentIndexReport> {
            self.words.lock().unwrap().push(content.email.id);
            Ok(DocumentIndexReport { indexed: true, warnings: vec![] })
        }
        fn search(&self, _: &Store, _: &str, _: usize) -> AppResult<Vec<SearchHit>> {
            Ok(vec![])
        }
        fn index(&self, _: &Store) -> AppResult<String> {
            Ok(String::new())
        }
        fn installed(&self) -> bool {
            self.installed.load(Ordering::SeqCst)
        }
        fn needs_server(&self) -> bool {
            self.needs_server.load(Ordering::SeqCst)
        }
    }
    fn folder(name: &str, role: &str) -> FolderProgress {
        FolderProgress { name: name.into(), role: role.into(), ..Default::default() }
    }
    fn mail(folder: &str, uid: u32, global: u64) -> Email {
        Email {
            account_id: 4,
            uid: Some(uid),
            remote_uid_validity: Some(7),
            remote_folder: Some(folder.into()),
            remote_global_id: Some(global),
            folder: "Archive".into(),
            remote_only: true,
            message_id: format!("{global}@example.com"),
            subject: format!("Mail {global}"),
            date: global as i64,
            ..Default::default()
        }
    }
    struct Harness {
        indexer: MailIndexService,
        context: Arc<ServiceContext>,
        repository: Arc<CountingRepository>,
        search: Arc<WordsSearch>,
        data: Arc<MailboxData>,
    }
    fn harness(data: MailboxData, known_folders: bool) -> Harness {
        let mut store = Store::default();
        store.accounts.push(Account {
            id: 4,
            kind: AccountKind::Gmail,
            provider_id: "gmail".into(),
            email: "batch@example.com".into(),
            search_index: IndexProgress {
                inventory_attempted: true,
                folders: if known_folders { data.folders.clone() } else { vec![] },
                last_discovery: if known_folders { 1_000 } else { 0 },
                ..Default::default()
            },
            ..Default::default()
        });
        let repository = Arc::new(CountingRepository { inner: MemoryRepository::new(store), writes: AtomicU32::new(0) });
        let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(crate::application::compute::NoCompute),
            repository: repository.clone(),
            mail: Arc::new(NoMail),
            secrets: Arc::new(NoSecrets),
            clock: Arc::new(TickClock(AtomicI64::new(1_000))),
            attachments: Arc::new(NoFiles),
            events: Arc::new(Recorder::default()),
        });
        let data = Arc::new(data);
        let search = Arc::new(WordsSearch::default());
        search.installed.store(true, Ordering::SeqCst);
        let indexer = MailIndexService::new(context.clone(), Arc::new(Mailbox(data.clone())), search.clone());
        Harness { indexer, context, repository, search, data }
    }
    fn run_until_idle(indexer: &MailIndexService) -> usize {
        (0..1_000).take_while(|_| indexer.step().unwrap()).count()
    }
    fn archive_of(count: u32) -> MailboxData {
        let data = MailboxData { folders: vec![folder("[Gmail]/All", "Archive")], ..Default::default() };
        data.messages.lock().unwrap().insert("[Gmail]/All".into(), (1..=count).map(|n| mail("[Gmail]/All", n, u64::from(n))).collect());
        data
    }

    #[test]
    fn one_session_serves_every_batch_with_two_writes_each() {
        let h = harness(archive_of(60), true);
        for _ in 0..3 {
            let before = h.repository.writes.load(Ordering::SeqCst);
            assert!(h.indexer.step().unwrap());
            assert_eq!(h.repository.writes.load(Ordering::SeqCst) - before, 2, "rows, then outcomes and cursor");
        }
        run_until_idle(&h.indexer);
        let store = h.context.read().unwrap();
        assert_eq!(store.emails.len(), 60);
        assert!(store.emails.iter().all(|e| e.search_index.version == VERSION && !e.search_index.semantic_pending));
        assert_eq!(h.search.documents.lock().unwrap().len(), 60);
        assert_eq!(h.indexer.sessions_opened(), 1);
        assert_eq!(h.data.content_calls.load(Ordering::SeqCst), 60, "every body is read once");
    }

    #[test]
    fn an_expired_session_is_closed_and_a_new_one_opened() {
        let mut data = archive_of(60);
        data.batches_per_session = 1;
        let h = harness(data, true);
        run_until_idle(&h.indexer);
        assert_eq!(h.context.read().unwrap().emails.len(), 60);
        assert!(h.indexer.sessions_opened() >= 3, "one per batch: {}", h.indexer.sessions_opened());
    }

    #[test]
    fn copies_of_one_gmail_message_in_a_batch_are_one_row() {
        let data = archive_of(3);
        data.messages.lock().unwrap().get_mut("[Gmail]/All").unwrap().push(mail("[Gmail]/All", 4, 2));
        let h = harness(data, true);
        run_until_idle(&h.indexer);
        let store = h.context.read().unwrap();
        assert_eq!(store.emails.len(), 3);
        assert_eq!(h.search.documents.lock().unwrap().len(), 3);
    }

    #[test]
    fn gmail_history_is_read_once_in_all_mail_after_new_inbox_mail() {
        let data = MailboxData {
            folders: vec![folder("INBOX", "Inbox"), folder("[Gmail]/Sent", "Sent"), folder("[Gmail]/All", "Archive"), folder("[Gmail]/Spam", "Spam")],
            ..Default::default()
        };
        {
            let mut messages = data.messages.lock().unwrap();
            messages.insert("[Gmail]/All".into(), (1..=30).map(|n| mail("[Gmail]/All", n, u64::from(n))).collect());
            messages.insert("INBOX".into(), (25..=30).map(|n| mail("INBOX", n - 20, u64::from(n))).collect());
            messages.insert("[Gmail]/Sent".into(), (1..=5).map(|n| mail("[Gmail]/Sent", n, u64::from(n))).collect());
        }
        let h = harness(data, false);
        run_until_idle(&h.indexer);
        let reads = h.data.reads.lock().unwrap().clone();
        assert_eq!(reads.first().map(String::as_str), Some("INBOX"), "{reads:?}");
        assert!(!reads.iter().any(|r| r == "[Gmail]/Sent"), "label folders are views: {reads:?}");
        let first_all = reads.iter().position(|r| r == "[Gmail]/All").unwrap();
        let spam = reads.iter().position(|r| r == "[Gmail]/Spam").unwrap();
        assert!(first_all < spam, "{reads:?}");
        let store = h.context.read().unwrap();
        assert_eq!(store.emails.len(), 30, "one row per Gmail message");
        let folders = &store.accounts[0].search_index.folders;
        assert!(folders.iter().find(|f| f.name == "[Gmail]/Sent").unwrap().view);
        assert!(folders.iter().find(|f| f.name == "INBOX").unwrap().arrivals_only);
        drop(store);
        // New Inbox mail is read from the Inbox, without waiting for All Mail.
        h.data.messages.lock().unwrap().get_mut("INBOX").unwrap().push(mail("INBOX", 11, 31));
        h.data.messages.lock().unwrap().get_mut("[Gmail]/All").unwrap().push(mail("[Gmail]/All", 31, 31));
        h.context.update(|store| { for f in &mut store.accounts[0].search_index.folders { if !f.view { f.complete = false; } } Ok(()) }).unwrap();
        run_until_idle(&h.indexer);
        assert_eq!(h.context.read().unwrap().emails.len(), 31);
    }

    #[test]
    fn without_the_model_words_are_indexed_and_vectors_follow_its_installation() {
        // EX-9
        let h = harness(archive_of(30), true);
        h.search.installed.store(false, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        {
            let store = h.context.read().unwrap();
            assert_eq!(store.emails.len(), 30, "history arrives without the model");
            assert!(store.emails.iter().all(|e| e.search_index.semantic_pending));
            assert_eq!(store.accounts[0].search_index.state, "waiting_model");
        }
        assert_eq!(h.search.words.lock().unwrap().len(), 30);
        assert!(h.search.documents.lock().unwrap().is_empty());
        h.search.installed.store(true, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        let store = h.context.read().unwrap();
        assert_eq!(h.search.documents.lock().unwrap().len(), 30, "the semantic pass reads only those bodies");
        assert!(store.emails.iter().all(|e| !e.search_index.semantic_pending));
        assert_eq!(store.accounts[0].search_index.state, "watching");
    }

    #[test]
    fn without_a_server_the_account_says_needs_server_not_waiting_model() {
        let h = harness(archive_of(5), true);
        h.search.installed.store(false, Ordering::SeqCst);
        h.search.needs_server.store(true, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        assert_eq!(h.context.read().unwrap().accounts[0].search_index.state, "needs_server");
        assert_eq!(h.search.words.lock().unwrap().len(), 5, "the words part is done without a server");
        h.search.needs_server.store(false, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        assert_eq!(h.context.read().unwrap().accounts[0].search_index.state, "waiting_model");
    }

    #[test]
    fn an_oversized_message_is_set_aside_without_stopping_its_batch() {
        let h = harness(archive_of(10), true);
        h.data.huge.lock().unwrap().insert(5);
        assert!(h.indexer.step().unwrap());
        let store = h.context.read().unwrap();
        assert_eq!(store.emails.len(), 10);
        let huge = store.emails.iter().find(|e| e.uid == Some(5)).unwrap();
        assert!(huge.search_index.partial && huge.search_index.notice.as_deref().unwrap().contains("32 MiB"));
        assert_eq!(h.search.documents.lock().unwrap().len(), 9);
    }

    #[test]
    fn an_evicted_known_row_does_not_block_the_scan() {
        let h = harness(archive_of(10), true);
        h.context.update(|store| {
            let mut evicted = mail("[Gmail]/All", 5, 5);
            evicted.id = store.new_id();
            evicted.storage_evicted = true;
            store.emails.push(evicted);
            Ok(())
        }).unwrap();
        run_until_idle(&h.indexer);
        let store = h.context.read().unwrap();
        assert_eq!(store.emails.len(), 10);
        assert_ne!(store.accounts[0].search_index.state, "blocked");
        assert!(store.accounts[0].search_index.folders[0].complete);
        assert_eq!(h.search.documents.lock().unwrap().len(), 9);
    }

    #[test]
    fn the_vectors_after_a_words_only_pass_are_not_counted_twice() {
        let h = harness(archive_of(5), true);
        h.search.installed.store(false, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        h.search.installed.store(true, Ordering::SeqCst);
        run_until_idle(&h.indexer);
        assert_eq!(h.context.read().unwrap().accounts[0].search_index.indexed, 5);
    }

    #[test]
    fn measure_five_hundred_messages() {
        let h = harness(archive_of(500), true);
        let started = std::time::Instant::now();
        let before = h.repository.writes.load(Ordering::SeqCst);
        let mut batches = 0;
        while h.context.read().unwrap().emails.iter().filter(|e| e.search_index.version == VERSION).count() < 500 {
            assert!(h.indexer.step().unwrap());
            batches += 1;
        }
        let writes = h.repository.writes.load(Ordering::SeqCst) - before;
        eprintln!(
            "measure: 500 messages in {} ms, {batches} batches, {} sessions, {writes} writes",
            started.elapsed().as_millis(),
            h.indexer.sessions_opened()
        );
        assert_eq!((batches, writes), (20, 40));
        assert_eq!(h.indexer.sessions_opened(), 1);
    }
}

