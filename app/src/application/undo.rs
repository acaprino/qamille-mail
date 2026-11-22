use super::{
    AppError, AppResult, ServiceContext,
    account_worker::{CommitOutcome, DRAFTS_ACCOUNT, Job, Priority, WorkerRegistry},
    activity, connectivity,
    drafts::DraftService,
    events::AppEvent,
    mail::MailService,
};
use crate::domain::{
    actions,
    activity::ActivityOutcome,
    model::Store,
    rules,
    undo::{UndoAction, UndoOperation, UndoStatus, project},
};
use chrono::DateTime;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
use std::time::Duration;

/// Incremental backoff for a confirmation that never reached the provider:
/// 5 s, 15 s, 60 s, 300 s, then 600 s as a cap (ADR-0007 v3, decision 8).
const BACKOFF_SECONDS: [i64; 5] = [5, 15, 60, 300, 600];
fn backoff_ms(attempts: u32) -> i64 {
    let last = BACKOFF_SECONDS.len() - 1;
    BACKOFF_SECONDS[attempts.saturating_sub(1).min(last as u32) as usize] * 1000
}

/// Deferrals, not failures: no attempt is counted, no notice is shown, and the
/// undo window (`expires_at_ms`) is left alone — the rows go back to `Pending`
/// with a short retry time. `mail_in_flight` is another action on the same
/// message; the queue codes are a worker that cannot accept the job right now
/// (full, throttled, or stopping). A dispatch refusal never became a provider
/// contact, so it must never look like a provider failure (INV-010).
const DEFERRAL_CODES: [&str; 4] = [
    "mail_in_flight",
    "queue_full",
    "queue_throttled",
    "worker_stopped",
];

/// How far ahead a due row waits while a `Committing` same-target sibling holds
/// it. The sibling's settlement signals the deadline thread itself, so this is
/// only the safety bound for a provider call that stalls: without it the loop
/// would re-arm immediately forever (two store reads per pass, 100% CPU).
const COMMITTING_RECHECK_MS: i64 = 250;

/// When a due `Pending` row stops being held back by a same-target sibling the
/// claim would refuse it for, or `None` when nothing holds it: the claim refuses
/// a row while any non-failed sibling is `Committing` (its settlement is the
/// only way out, rechecked [`COMMITTING_RECHECK_MS`] ahead) or still inside its
/// undo window (the wait ends at that window's expiry, a known instant). Read
/// off a snapshot, so a held row costs no claim transaction and a store rewrite.
fn blocked_until(store: &Store, row: &UndoAction, now: i64) -> Option<i64> {
    store
        .undo_actions
        .iter()
        .filter(|a| a.id != row.id && a.operation.same_target(&row.operation))
        .filter_map(|a| match a.status {
            UndoStatus::Committing => Some(now + COMMITTING_RECHECK_MS),
            UndoStatus::Pending if a.expires_at_ms > now => Some(a.expires_at_ms),
            _ => None,
        })
        .min()
}

/// The account an operation belongs to, while its mail or draft still exists.
fn account_of(store: &Store, operation: &UndoOperation) -> Option<u32> {
    match operation {
        UndoOperation::Mail { email_id, .. } => {
            store.email(*email_id).map(|email| email.account_id)
        }
        operation => operation
            .draft_id()
            .and_then(|id| store.drafts.iter().find(|draft| draft.id == id))
            .map(|draft| draft.account_id),
    }
}

/// The earliest instant the queue has something to do: the effective deadline
/// (retry backoff, else undo-window expiry) of every `Pending` row, where a row
/// that is already due but held back by a same-target sibling counts from the
/// instant its blocker lets go ([`blocked_until`]) instead of its own, already
/// past, deadline — otherwise the deadline thread would re-arm into the past
/// and spin. `i64::MAX` when the queue holds no `Pending` row.
fn earliest_pending(store: &Store, now: i64) -> i64 {
    store
        .undo_actions
        .iter()
        .filter(|a| a.status == UndoStatus::Pending)
        .fold(i64::MAX, |earliest, action| {
            let due = action.retry_at_ms.unwrap_or(action.expires_at_ms);
            let wake = if due <= now {
                blocked_until(store, action, now).unwrap_or(due)
            } else {
                due
            };
            earliest.min(wake)
        })
}

/// Events that make a row visible again after its action stops projecting
/// (definitive failure, startup recovery, giving up on a retry): the message or
/// draft the action hid comes back and the counters follow — the contract the
/// merged UI was built against (preview `commitPreview`: the `undo-changed`
/// plus the rows it touched). No event when the target itself is gone: an id
/// the store does not hold is never announced (INV-004).
fn restore_events(store: &Store, operation: &UndoOperation) -> Vec<AppEvent> {
    let Some(account_id) = account_of(store, operation) else {
        return Vec::new();
    };
    match operation {
        UndoOperation::Mail { email_id, .. } => vec![
            AppEvent::EmailChanged {
                account_id,
                ids: vec![*email_id],
            },
            AppEvent::CountsChanged { account_id },
        ],
        operation => vec![
            AppEvent::DraftsChanged {
                ids: operation.draft_id().map_or_else(Vec::new, |id| vec![id]),
            },
            AppEvent::CountsChanged { account_id },
        ],
    }
}

/// Whether a failed confirmation may be attempted again. Only a mail action can
/// be replayed: a send is reconciled by the outbox, which records an uncertain
/// result after any delivery attempt, so a second `send` answers
/// `delivery_unknown` without contacting the provider again. The text decides
/// the rest, and anything ambiguous stays definitive (INV-008, RA-007).
/// `remote_applied_local_failed` is the provider having already executed the
/// action: its message quotes a local failure that may read like a network
/// one, and replaying it would apply the action twice.
fn is_retryable(operation: &UndoOperation, error: &AppError) -> bool {
    matches!(operation, UndoOperation::Mail { .. })
        && !matches!(
            error.code,
            "delivery_unknown"
                | "sent_not_saved"
                | "delivery_pending"
                | "delivery_record_failed"
                | "remote_applied_local_failed"
        )
        && connectivity::is_retryable_connect_error(&error.message)
}

/// What is left of the queue when Qamille is asked to quit ([`UndoService::unsettled`]), by
/// what quitting does to it: a `Pending` row stays in the queue and the next start carries it out;
/// a `Committing` one dies with the process, and startup recovery marks it failed without
/// retrying it ([`UndoService::recover_interrupted`], INV-008). Only `SendDraft` sends a message:
/// an archive, a move, a schedule or a discarded draft is an action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Unsettled {
    /// Sends still in the queue (no connection, credentials the vault cannot open): the next start sends them.
    pub sends: usize,
    /// Sends on their way to the provider: quitting may stop them, and nothing sends them again.
    pub sending: usize,
    /// Other actions still in the queue: the next start carries them out.
    pub actions: usize,
    /// Other actions being confirmed: quitting may interrupt them, and the next start reports them failed.
    pub confirming: usize,
}
impl Unsettled {
    pub fn total(&self) -> usize {
        self.sends + self.sending + self.actions + self.confirming
    }
}

#[derive(Clone)]
pub struct UndoService {
    context: Arc<ServiceContext>,
    mail: MailService,
    drafts: DraftService,
    has_work: Arc<AtomicBool>,
    /// Earliest instant the queue has something to do ([`earliest_pending`]:
    /// retry backoff, else undo-window expiry, and for a due row held back by
    /// a same-target sibling the instant that sibling lets go), cached so an
    /// idle queue costs no store reads: offline, a backing-off action would
    /// otherwise keep full-store reads alive for hours, and a held row would
    /// re-arm into the past. Serves both the pass fast path and
    /// [`next_deadline`](Self::next_deadline). `i64::MIN` forces a full pass;
    /// recomputed after every pass and every invalidation.
    earliest_due: Arc<AtomicI64>,
    /// Set whenever the queued-action set changes outside a pass, making the
    /// cached deadline untrusted. Cleared immediately before the snapshot that
    /// recomputes it, so a concurrent enqueue either lands in that snapshot or
    /// leaves the flag set — the cache can then only be stale-early (one wasted
    /// pass), never stale-late (a stranded action).
    schedule_dirty: Arc<AtomicBool>,
    /// Wake source of the deadline thread (ADR-0007 decision 4, INV-005): the
    /// flag means "the schedule changed since the last wait began". Writers
    /// set it under the mutex they also notify, and the waiter holds that mutex
    /// from its predicate check into the wait, so no wake is lost. Queue
    /// writers raise it through [`notify`](Self::notify); a provider connection
    /// that succeeds raises it through `connectivity::note_success`, to which
    /// the service subscribes at construction (EX-009).
    waker: Arc<connectivity::WakeSignal>,
}
impl UndoService {
    pub fn new(context: Arc<ServiceContext>, mail: MailService, drafts: DraftService) -> Self {
        let waker: Arc<connectivity::WakeSignal> = Arc::new((Mutex::new(false), Condvar::new()));
        connectivity::subscribe(&waker);
        Self {
            context,
            mail,
            drafts,
            has_work: Arc::new(AtomicBool::new(true)),
            // Unknown schedule (e.g. a restart with actions already in retry):
            // the first pass reads the store and seeds the cache.
            earliest_due: Arc::new(AtomicI64::new(i64::MIN)),
            schedule_dirty: Arc::new(AtomicBool::new(true)),
            waker,
        }
    }
    /// Tell the deadline thread the schedule changed: a queued, removed, or
    /// rescheduled action. Cheap and loss-free (the flag survives until a wait
    /// consumes it), so every writer calls it unconditionally.
    fn notify(&self) {
        let (flag, condvar) = &*self.waker;
        let mut signaled = flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *signaled = true;
        condvar.notify_all();
    }
    pub fn queue_mail(
        &self,
        email_id: u64,
        action: String,
        mut value: Option<String>,
    ) -> AppResult<UndoAction> {
        // Resolve relative dates once; a preview or a restart must not move "Tomorrow".
        if matches!(action.as_str(), "snooze" | "changeReminder" | "setDue") {
            let store = self.context.read_shared()?;
            let regional = self.context.clock.regional(&store.settings);
            let timestamp = crate::domain::regional::parse_when(
                value.as_deref().unwrap_or_default(), self.context.clock.now(),
                &regional.time_zone, &regional.locale,
            ).map_err(AppError::validation)?;
            value = Some(
                DateTime::from_timestamp(timestamp, 0)
                    .ok_or_else(|| AppError::validation("Choose a valid date"))?
                    .to_rfc3339(),
            );
        }
        self.queue(UndoOperation::Mail {
            email_id,
            action,
            value,
        })
    }
    pub fn queue_draft(
        &self,
        draft_id: u64,
        kind: &str,
        send_at: Option<i64>,
    ) -> AppResult<UndoAction> {
        let operation = match kind {
            "send" => {
                self.drafts.validate_send(draft_id)?;
                UndoOperation::SendDraft { draft_id }
            }
            "discard" => UndoOperation::DiscardDraft { draft_id },
            "schedule" => {
                self.drafts.validate_send(draft_id)?;
                let time = send_at.ok_or_else(|| AppError::validation("Choose a send time"))?;
                if time <= self.context.clock.now() + 30
                    || time > self.context.clock.now() + rules::MAX_DAYS_SEND_LATER * 86400
                {
                    return Err(AppError::validation(
                        "Choose a send time at least 30 seconds ahead, within 180 days",
                    ));
                }
                UndoOperation::ScheduleDraft {
                    draft_id,
                    send_at: time,
                }
            }
            _ => return Err(AppError::validation("Unknown draft action")),
        };
        self.queue(operation)
    }
    fn queue(&self, operation: UndoOperation) -> AppResult<UndoAction> {
        // Queuing changes the queue only, never a message: no copy of the mailbox.
        let pending = self.context.update_emails_with_events(&[], |store| {
            let subject = match &operation {
                UndoOperation::Mail {
                    email_id,
                    action,
                    value,
                } => {
                    // The message as the user sees it (its pending actions
                    // applied), the only one the validation needs.
                    let mut view = project(Store {
                        emails: store.email(*email_id).cloned().into_iter().collect(),
                        undo_actions: store
                            .undo_actions
                            .iter()
                            .filter(|a| a.operation.same_target(&operation))
                            .cloned()
                            .collect(),
                        ..Store::default()
                    });
                    let regional = self.context.clock.regional(&store.settings);
                    actions::apply_at(&mut view, *email_id, action, value.as_deref(),
                        self.context.clock.now(), &regional.time_zone, &regional.locale)
                        .map_err(AppError::validation)?;
                    view.email(*email_id)
                        .ok_or_else(|| AppError::missing("Message"))?
                        .subject
                        .clone()
                }
                _ => {
                    let id = operation.draft_id().unwrap();
                    if store.undo_actions.iter().any(|a| {
                        a.status != UndoStatus::Failed && a.operation.draft_id() == Some(id)
                    }) {
                        return Err(AppError::validation(
                            "This draft already has a pending action. Undo it first.",
                        ));
                    }
                    store
                        .drafts
                        .iter()
                        .find(|d| d.id == id)
                        .ok_or_else(|| AppError::missing("Draft"))?
                        .subject
                        .clone()
                }
            };
            let created_at_ms = self.context.clock.now_ms();
            let pending = UndoAction {
                id: store.new_id(),
                operation,
                subject,
                created_at_ms,
                expires_at_ms: created_at_ms
                    + i64::from(store.settings.undo_seconds.clamp(5, 30)) * 1000,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            };
            let account_id = account_of(store, &pending.operation);
            store.undo_actions.push(pending.clone());
            // The queued row is its own payload: the UI shows the undoable
            // action without reloading the workspace.
            Ok((
                (pending.clone(), account_id),
                vec![AppEvent::UndoChanged { action: pending }],
            ))
        })?;
        // After the commit: the cached deadline predates this action.
        self.schedule_dirty.store(true, Ordering::SeqCst);
        self.has_work.store(true, Ordering::SeqCst);
        self.notify();
        // The activity row of a logged action, written once the queue holds it
        // and best effort: the history never blocks the action.
        let (pending, account_id) = pending;
        if let Some(mut entry) = activity::for_undo_action(&pending) {
            entry.account_id = account_id;
            activity::record(&self.context, entry);
        }
        Ok(pending)
    }
    pub fn undo(&self, id: u64) -> AppResult<UndoAction> {
        let removed = self.context.update_emails_with_events(&[], |store| {
            let index = store
                .undo_actions
                .iter()
                .position(|a| a.id == id)
                .ok_or_else(|| AppError::missing("Undo action"))?;
            let action = &store.undo_actions[index];
            if action.status != UndoStatus::Pending
                || self.context.clock.now_ms() >= action.expires_at_ms
            {
                return Err(AppError::validation("The undo time has expired"));
            }
            let removed = store.undo_actions.remove(index);
            Ok((removed.clone(), vec![AppEvent::UndoRemoved { id: removed.id }]))
        })?;
        self.schedule_dirty.store(true, Ordering::SeqCst);
        self.notify();
        activity::set_undo_outcome(&self.context, removed.id, ActivityOutcome::Undone, None);
        Ok(removed)
    }
    /// Removes a notice the user is done with: a failure they have read, or a
    /// confirmation still being retried (`attempts > 0`) that they give up on
    /// ("Rinuncia"). Giving up is the only case where the projected local state
    /// returns. An action inside its undo window belongs to `undo`; one that is
    /// expired but never attempted is the queue's first commit, not a notice to
    /// drop, and one in flight is never removed (INV-008).
    pub fn dismiss(&self, id: u64) -> AppResult<()> {
        let gave_up = self.context.update_emails_with_events(&[], |store| {
            let now = self.context.clock.now_ms();
            // "Rinuncia" on a retrying action is the only removal that makes
            // the projected local state return: remember its target before
            // the row leaves the queue.
            let given_up = store
                .undo_actions
                .iter()
                .find(|a| {
                    a.id == id
                        && a.status == UndoStatus::Pending
                        && a.attempts > 0
                        && a.expires_at_ms <= now
                })
                .map(|a| (a.operation.clone(), a.error.clone()));
            let before = store.undo_actions.len();
            store.undo_actions.retain(|a| {
                !(a.id == id
                    && (a.status == UndoStatus::Failed
                        || (a.status == UndoStatus::Pending
                            && a.attempts > 0
                            && a.expires_at_ms <= now)))
            });
            // Only a row that actually left the queue is announced.
            let mut events = Vec::new();
            let mut gave_up = None;
            if store.undo_actions.len() != before {
                events.push(AppEvent::UndoRemoved { id });
                if let Some((operation, error)) = given_up {
                    events.extend(restore_events(store, &operation));
                    gave_up = Some(error);
                }
            }
            Ok((gave_up, events))
        })?;
        self.schedule_dirty.store(true, Ordering::SeqCst);
        self.notify();
        // "Rinuncia" on a retrying confirmation: the action never reached the
        // provider and the user stops waiting, so its row becomes a failure.
        if let Some(error) = gave_up {
            activity::set_undo_outcome(&self.context, id, ActivityOutcome::Failed, error);
        }
        Ok(())
    }
    /// Never retry a send or move whose server result was lost during shutdown.
    pub fn recover_interrupted(&self) -> AppResult<()> {
        if !self
            .context
            .read()?
            .undo_actions
            .iter()
            .any(|a| a.status == UndoStatus::Committing)
        {
            return Ok(());
        }
        let failed = self.context.update_with_events(|store| {
            let mut events = Vec::new();
            let mut marked = Vec::new();
            let mut failed = Vec::new();
            for action in &mut store.undo_actions {
                if action.status == UndoStatus::Committing {
                    action.status = UndoStatus::Failed;
                    action.error = Some("The app closed before this action was confirmed. Check the mailbox before trying again.".into());
                    // Startup recovery announces the rows it marked (Task 3.4
                    // handoff): the failure rows appear without a file watcher.
                    events.push(AppEvent::UndoChanged {
                        action: action.clone(),
                    });
                    marked.push(action.operation.clone());
                    failed.push((action.id, action.error.clone()));
                }
            }
            // The marked actions stop projecting: the rows they hid come back.
            // One announce per distinct target, whichever action named it.
            let mut seen = std::collections::HashSet::new();
            for operation in marked {
                let target = match &operation {
                    UndoOperation::Mail { email_id, .. } => (true, *email_id),
                    other => (false, other.draft_id().unwrap_or(0)),
                };
                if seen.insert(target) {
                    events.extend(restore_events(store, &operation));
                }
            }
            Ok((failed, events))
        })?;
        for (id, error) in failed {
            activity::set_undo_outcome(&self.context, id, ActivityOutcome::Failed, error);
        }
        // No notify: recovery only marks `Committing` rows `Failed`, and the
        // deadline schedule counts `Pending` rows — nothing to rearm. The
        // deadline thread runs its recovery gate before its first read anyway.
        self.schedule_dirty.store(true, Ordering::SeqCst);
        Ok(())
    }
    /// When the next confirmation comes due (ADR-0007 v3 decision 4, INV-005):
    /// the earliest effective deadline — the `retry_at_ms` backoff when set,
    /// else the undo-window expiry — over the `Pending` rows, or `None` when
    /// the queue is empty (the deadline thread then waits for a writer's
    /// signal alone). A due row that a same-target sibling holds back counts
    /// from the instant the sibling lets go (its undo window's expiry, or a
    /// short recheck while it commits), never from its own past deadline: the
    /// result is in the future whenever nothing can be claimed. A pending
    /// connectivity wake makes every queued row immediately due (EX-009): the
    /// next pass drains the wakes and decides per row which of them the wake
    /// actually retries.
    ///
    /// Uses the same cached schedule as the pass fast path; a full store read
    /// happens only when a writer invalidated the cache.
    pub fn next_deadline(&self) -> AppResult<Option<i64>> {
        if self.schedule_dirty.swap(false, Ordering::SeqCst)
            || self.earliest_due.load(Ordering::SeqCst) == i64::MIN
        {
            // Invalidate before reading, the closing pass's ordering rule: a
            // concurrent change either lands in this snapshot or leaves the
            // dirty flag set, so the cache is never stale-late.
            let snapshot = self.context.read().inspect_err(|_| {
                self.schedule_dirty.store(true, Ordering::SeqCst);
            })?;
            self.earliest_due.store(
                earliest_pending(&snapshot, self.context.clock.now_ms()),
                Ordering::SeqCst,
            );
        }
        let earliest = self.earliest_due.load(Ordering::SeqCst);
        if earliest == i64::MAX {
            return Ok(None);
        }
        if connectivity::has_wakes() {
            return Ok(Some(self.context.clock.now_ms()));
        }
        Ok(Some(earliest))
    }
    /// Sleeps the deadline thread until `deadline` passes (indefinitely when
    /// `None`) or a signal arrives: a queue writer changing the schedule, or a
    /// provider connection that succeeded (`connectivity::note_success`,
    /// EX-009). Nothing here re-checks on a timer (INV-005): both wake sources
    /// raise the waker flag under its mutex, which this wait holds from the
    /// flag check into the wait, so a signal racing the wait's start is never
    /// lost. A wake already pending when the wait is entered is covered by
    /// [`next_deadline`](Self::next_deadline), which reports it as due now.
    pub fn wait_for_work(&self, deadline: Option<i64>) {
        let (flag, condvar) = &*self.waker;
        let mut signaled = flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if *signaled {
                *signaled = false;
                return;
            }
            let Some(due_at_ms) = deadline else {
                // An empty queue waits for a signal alone (INV-005).
                signaled = condvar.wait(signaled).unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            };
            let remaining = due_at_ms - self.context.clock.now_ms();
            if remaining <= 0 {
                return;
            }
            signaled = condvar
                .wait_timeout(signaled, Duration::from_millis(remaining as u64))
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
    }
    /// The synchronous pass: claims every due confirmation, executes it on the
    /// calling thread, and settles its result at once. The desktop runs
    /// [`commit_due`](Self::commit_due) instead (ADR-0007 decision 4); this
    /// entry point keeps the queue usable where no worker registry exists —
    /// MCP and the suites that pin the retry semantics — with identical claim,
    /// retry, dismiss and recovery rules.
    pub fn process_due(&self) -> AppResult<bool> {
        self.run_pass(
            |_| Ok(()),
            |_, pending, claimed| {
                let ids: Vec<u64> = claimed.iter().map(|a| a.id).collect();
                let result = self.execute(pending, claimed);
                self.settle(&ids, &pending.operation, &result)
            },
        )
    }
    /// The deadline-driven dispatch pass (ADR-0007 v3 decision 4, INV-005):
    /// claims the due confirmations exactly like the synchronous pass —
    /// `Pending`→`Committing` in one store update, `same_target` batching — but
    /// every claimed batch becomes a `Priority::UserAction` job on the worker
    /// of the account it belongs to: mail actions on that account's worker,
    /// draft operations on the dedicated drafts worker ([`DRAFTS_ACCOUNT`]),
    /// so a slow or failing confirmation never delays another one (EX-004).
    /// The worker reports each result back through [`CommitOutcome`] and the
    /// queue settles it with the unchanged retry rules (INV-010). Returns
    /// whether any batch was claimed and dispatched.
    pub fn commit_due(&self, registry: &WorkerRegistry) -> AppResult<bool> {
        self.run_pass(
            // Workers exist for the accounts this snapshot lists (plus the
            // drafts worker), and a thread that finished is respawned here —
            // on the queue's own passes, never on a separate poll.
            |snapshot| registry.reconcile(&snapshot.accounts),
            |snapshot, pending, claimed| self.dispatch(registry, snapshot, pending, claimed),
        )
    }
    /// Qamille is about to quit (audit Q8): every `Pending` action stops being undoable now
    /// (`expires_at_ms` = now) and one backing off after a failed connection is attempted now,
    /// so the queue's next pass sends what would otherwise wait for the next start. `Committing`
    /// rows are already on their way and `Failed` ones are notices: both are left alone. Wakes
    /// the deadline thread, like a queue writer. Returns how many rows it changed.
    pub fn send_now(&self) -> AppResult<usize> {
        let now = self.context.clock.now_ms();
        let changed = self.context.update_emails_with_events(&[], |store| {
            let mut events = Vec::new();
            for action in store.undo_actions.iter_mut().filter(|a| a.status == UndoStatus::Pending) {
                let undoable = action.expires_at_ms > now;
                let backing_off = action.retry_at_ms.is_some_and(|at| at > now);
                if !undoable && !backing_off {
                    continue;
                }
                action.expires_at_ms = action.expires_at_ms.min(now);
                if backing_off {
                    action.retry_at_ms = Some(now);
                }
                // The notice stops counting down: the action cannot be undone any more.
                events.push(AppEvent::UndoChanged { action: action.clone() });
            }
            Ok((events.len(), events))
        })?;
        if changed > 0 {
            self.schedule_dirty.store(true, Ordering::SeqCst);
            self.has_work.store(true, Ordering::SeqCst);
            self.notify();
        }
        Ok(changed)
    }
    /// The actions that have not reached the provider yet, by what quitting does to them
    /// ([`Unsettled`]): `Pending` rows (undoable, due, or backing off) stay queued, `Committing`
    /// ones are on their way. `Failed` rows are notices, not work.
    pub fn unsettled(&self) -> AppResult<Unsettled> {
        // Polled while Qamille waits to quit: the shared snapshot, never a copy of the store.
        let store = self.context.read_shared()?;
        let mut left = Unsettled::default();
        for action in &store.undo_actions {
            let send = matches!(action.operation, UndoOperation::SendDraft { .. });
            match (action.status, send) {
                (UndoStatus::Pending, true) => left.sends += 1,
                (UndoStatus::Committing, true) => left.sending += 1,
                (UndoStatus::Pending, false) => left.actions += 1,
                (UndoStatus::Committing, false) => left.confirming += 1,
                (UndoStatus::Failed, _) => {}
            }
        }
        Ok(left)
    }
    /// One full queue pass, shared by both entry points: drain the
    /// connectivity wakes, read the snapshot, `prepare` it, claim every due
    /// `Pending` row and hand the batch to `on_claim`, then recompute the
    /// cached schedule. The 1.6 gating is unchanged: the fast path keeps an
    /// idle queue off the store, and a storage error leaves the flags set so
    /// the next pass retries.
    fn run_pass(
        &self,
        prepare: impl FnOnce(&Store) -> AppResult<()>,
        mut on_claim: impl FnMut(&Store, &UndoAction, &[UndoAction]) -> AppResult<()>,
    ) -> AppResult<bool> {
        if !self.has_work.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        // Idle fast path: while the earliest scheduled attempt is still ahead,
        // no connection woke the queue and nothing invalidated the cache, there
        // is nothing to do — return without touching the store, so a long
        // backoff does not keep full-store reads alive.
        if !self.schedule_dirty.load(Ordering::SeqCst)
            && self.earliest_due.load(Ordering::SeqCst) > self.context.clock.now_ms()
            && !connectivity::has_wakes()
        {
            self.has_work.store(true, Ordering::SeqCst);
            return Ok(false);
        }
        // Keep checking after a storage error as well as while a deadline is pending.
        self.has_work.store(true, Ordering::SeqCst);
        // Accounts whose provider connection succeeded since the last pass: their
        // queued confirmations are attempted now instead of at their deadline.
        let wakes = connectivity::drain_wakes();
        let snapshot = self.context.read()?;
        prepare(&snapshot)?;
        let mut changed = false;
        // Rows a claim of this pass already took (a claim drags every due
        // same-target sibling along): their snapshot copies are still `Pending`.
        let mut taken: Vec<u64> = Vec::new();
        for pending in snapshot.undo_actions.iter().filter(|a| {
            a.status == UndoStatus::Pending && a.expires_at_ms <= self.context.clock.now_ms()
        }) {
            if taken.contains(&pending.id) {
                continue;
            }
            // A retrying action waits for its own backoff, unless its account was
            // woken by a connection that succeeded (EX-009).
            let woken =
                account_of(&snapshot, &pending.operation).is_some_and(|id| wakes.contains(&id));
            if !woken && pending.retry_at_ms.is_some_and(|at| at > self.context.clock.now_ms()) {
                continue;
            }
            // Due but held back by a same-target sibling (committing, or queued
            // later and still inside its undo window): the claim would refuse
            // it, and a refused claim is still a store transaction. The closing
            // `reschedule` arms the wait for that sibling ([`blocked_until`]),
            // so the loop neither writes nor spins while the row is held.
            if blocked_until(&snapshot, pending, self.context.clock.now_ms()).is_some() {
                continue;
            }
            let claimed = self.claim(pending, woken)?;
            if claimed.is_empty() {
                // The store moved between the snapshot and the claim (a sibling
                // queued, or the row settled): `reschedule` reads it afresh.
                continue;
            }
            changed = true;
            taken.extend(claimed.iter().map(|a| a.id));
            on_claim(&snapshot, pending, &claimed)?;
        }
        self.reschedule()
            // A failed closing read leaves the flags set (see `reschedule`).
            .and(Ok(changed))
    }
    /// The claim, unchanged from the polling queue: one store update turns
    /// every due `Pending` row of the seed's target into `Committing`, never
    /// dragging a backing-off sibling into an early attempt, and announces the
    /// transition (the UI turns the rows into "confirming" without polling).
    fn claim(&self, pending: &UndoAction, woken: bool) -> AppResult<Vec<UndoAction>> {
        self.context.update_emails_with_events(&[], |store| {
            if !store
                .undo_actions
                .iter()
                .any(|a| a.id == pending.id && a.status == UndoStatus::Pending)
            {
                return Ok((Vec::new(), Vec::new()));
            }
            if store.undo_actions.iter().any(|a| {
                a.status != UndoStatus::Failed
                    && a.operation.same_target(&pending.operation)
                    && (a.status == UndoStatus::Committing
                        || a.expires_at_ms > self.context.clock.now_ms())
            }) {
                return Ok((Vec::new(), Vec::new()));
            }
            let mut claimed = Vec::new();
            for action in &mut store.undo_actions {
                if action.status == UndoStatus::Pending
                    && action.operation.same_target(&pending.operation)
                    // The same rule inside the claim: a due sibling must not
                    // drag a backing-off action into an early attempt.
                    && (woken
                        || !action.retry_at_ms.is_some_and(|at| at > self.context.clock.now_ms()))
                {
                    action.status = UndoStatus::Committing;
                    claimed.push(action.clone());
                }
            }
            // The transition is visible: the UI turns the row into
            // "confirming" without polling the queue.
            let mut events: Vec<AppEvent> = claimed
                .iter()
                .map(|action| AppEvent::UndoChanged {
                    action: action.clone(),
                })
                .collect();
            if let UndoOperation::SendDraft { draft_id } = pending.operation {
                if let Some(draft) = store.drafts.iter_mut().find(|d| d.id == draft_id) {
                    draft.send_at = None;
                    draft.updated = self.context.clock.now().max(draft.updated + 1);
                    events.push(AppEvent::DraftsChanged {
                        ids: vec![draft_id],
                    });
                }
            }
            Ok((claimed, events))
        })
    }
    /// Runs one claimed batch against the provider (synchronous path only).
    fn execute(&self, pending: &UndoAction, claimed: &[UndoAction]) -> AppResult<()> {
        match &pending.operation {
            UndoOperation::Mail { .. } => self.mail.act_queued(claimed),
            UndoOperation::SendDraft { draft_id } => self.drafts.send(*draft_id),
            UndoOperation::DiscardDraft { draft_id } => self.drafts.discard(*draft_id),
            UndoOperation::ScheduleDraft { draft_id, send_at } => {
                let committed = self.context.update_with_events(|store| {
                    let draft = store
                        .drafts
                        .iter_mut()
                        .find(|d| d.id == *draft_id)
                        .ok_or_else(|| AppError::missing("Draft"))?;
                    draft.send_at = Some(*send_at);
                    draft.updated = self.context.clock.now().max(draft.updated + 1);
                    Ok((
                        (),
                        vec![AppEvent::DraftsChanged {
                            ids: vec![*draft_id],
                        }],
                    ))
                });
                if committed.is_ok() {
                    // The settled send-later is a new scheduler deadline
                    // (INV-005): the thread sleeps until it instead of polling.
                    super::scheduler::note_schedule_change();
                }
                committed
            }
        }
    }
    /// Hands one claimed batch to the worker that owns it (dispatch path). A
    /// submission that never reached a worker is settled here instead: a queue
    /// refusal defers the rows ([`DEFERRAL_CODES`]), anything else fails them
    /// exactly like the synchronous path would.
    fn dispatch(
        &self,
        registry: &WorkerRegistry,
        snapshot: &Store,
        pending: &UndoAction,
        claimed: &[UndoAction],
    ) -> AppResult<()> {
        let ids: Vec<u64> = claimed.iter().map(|a| a.id).collect();
        let submission = match &pending.operation {
            UndoOperation::Mail { email_id, .. } => {
                let commands: Vec<(String, Option<String>)> = claimed
                    .iter()
                    .filter_map(|action| match &action.operation {
                        UndoOperation::Mail { action, value, .. } => {
                            Some((action.clone(), value.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                account_of(snapshot, &pending.operation)
                    .ok_or_else(|| AppError::missing("Message"))
                    .and_then(|account_id| {
                        registry.submit(
                            account_id,
                            Priority::UserAction,
                            Job::Act {
                                email_id: *email_id,
                                commands,
                                pending_ids: ids.clone(),
                            },
                        )
                    })
            }
            UndoOperation::SendDraft { draft_id } => registry.submit(
                DRAFTS_ACCOUNT,
                Priority::UserAction,
                Job::SendDraft {
                    draft_id: *draft_id,
                    pending_ids: ids.clone(),
                },
            ),
            UndoOperation::DiscardDraft { draft_id } => registry.submit(
                DRAFTS_ACCOUNT,
                Priority::UserAction,
                Job::DiscardDraft {
                    draft_id: *draft_id,
                    pending_ids: ids.clone(),
                },
            ),
            UndoOperation::ScheduleDraft { draft_id, send_at } => registry.submit(
                DRAFTS_ACCOUNT,
                Priority::UserAction,
                Job::ScheduleDraft {
                    draft_id: *draft_id,
                    send_at: *send_at,
                    pending_ids: ids.clone(),
                },
            ),
        };
        if let Err(error) = submission {
            self.settle(&ids, &pending.operation, &Err(error))?;
        }
        Ok(())
    }
    /// Applies one claimed batch's outcome to the queue rows — the unchanged
    /// 1.6 retry machinery. Success removes the rows; a deferral
    /// ([`DEFERRAL_CODES`]) puts them back to `Pending` with a short retry time
    /// without counting an attempt; any other failure either schedules the
    /// next backoff step (connect-class mail errors only, INV-010) or marks
    /// the rows definitively failed and restores the projection they hid
    /// (INV-008, EX-010).
    fn settle(
        &self,
        ids: &[u64],
        operation: &UndoOperation,
        result: &AppResult<()>,
    ) -> AppResult<()> {
        let failed_at_ms = self.context.clock.now_ms();
        let failed = self.context.update_emails_with_events(&[], |store| {
            let mut events = Vec::new();
            // The rows this settlement failed for good, for the activity log.
            let mut failed = Vec::new();
            match result {
                Ok(()) => {
                    // A mail action already dropped its queue rows in
                    // `commit_actions`; announce only the removals this
                    // transaction actually made, so no row is announced twice.
                    let removed: Vec<u64> = store
                        .undo_actions
                        .iter()
                        .filter(|a| ids.contains(&a.id))
                        .map(|a| a.id)
                        .collect();
                    store.undo_actions.retain(|a| !ids.contains(&a.id));
                    events.extend(removed.into_iter().map(|id| AppEvent::UndoRemoved { id }));
                }
                Err(error) if DEFERRAL_CODES.contains(&error.code.as_ref()) => {
                    // A deferral, not a failure: no attempt counted, no notice,
                    // the undo window (expires_at_ms) is left alone.
                    let retry_at = failed_at_ms + 3_000;
                    for action in store
                        .undo_actions
                        .iter_mut()
                        .filter(|a| ids.contains(&a.id))
                    {
                        action.status = UndoStatus::Pending;
                        action.retry_at_ms = Some(retry_at);
                        events.push(AppEvent::UndoChanged {
                            action: action.clone(),
                        });
                    }
                }
                Err(error) => {
                    let retry = is_retryable(operation, error);
                    let mut failed_now = false;
                    for action in store
                        .undo_actions
                        .iter_mut()
                        .filter(|a| ids.contains(&a.id))
                    {
                        // The reason stays on the notice in both cases.
                        action.error = Some(error.message.clone());
                        if retry {
                            // INV-010: the provider executed nothing, so the
                            // confirmation goes back in the queue with the
                            // next step of its backoff.
                            action.attempts = action.attempts.saturating_add(1);
                            action.retry_at_ms =
                                Some(failed_at_ms + backoff_ms(action.attempts));
                            action.status = UndoStatus::Pending;
                        } else {
                            action.status = UndoStatus::Failed;
                            failed_now = true;
                            failed.push((action.id, action.error.clone()));
                        }
                        // The retry schedule and the failure both update the
                        // visible row: the UI shows it without a reload.
                        events.push(AppEvent::UndoChanged {
                            action: action.clone(),
                        });
                    }
                    if failed_now {
                        // A definitive failure stops projecting the action:
                        // the row it hid comes back. A claimed batch shares
                        // one target (`same_target`), so one restore names it.
                        events.extend(restore_events(store, operation));
                    }
                }
            }
            Ok((failed, events))
        })?;
        for (id, error) in failed {
            activity::set_undo_outcome(&self.context, id, ActivityOutcome::Failed, error);
        }
        Ok(())
    }
    /// Settles claimed rows by id, re-reading their operation from the queue:
    /// the worker-thread entry, where the claiming pass has long returned.
    /// Rows a successful commit already consumed settle as nothing.
    fn settle_ids(&self, pending_ids: &[u64], result: &AppResult<()>) -> AppResult<()> {
        let snapshot = self.context.read()?;
        let Some(operation) = snapshot
            .undo_actions
            .iter()
            .find(|a| pending_ids.contains(&a.id))
            .map(|a| a.operation.clone())
        else {
            return Ok(());
        };
        self.settle(pending_ids, &operation, result)?;
        // Settling runs after the dispatching pass recomputed its schedule
        // cache: invalidate it and wake the deadline thread, so a new backoff
        // deadline is armed instead of slept through.
        self.schedule_dirty.store(true, Ordering::SeqCst);
        self.has_work.store(true, Ordering::SeqCst);
        self.notify();
        Ok(())
    }
    /// The closing recompute of the cached schedule, unchanged from the
    /// polling queue: reset the flags before reading so a concurrent enqueue
    /// can only turn them back on, then remember the earliest deadline so an
    /// idle queue costs no store reads.
    fn reschedule(&self) -> AppResult<()> {
        // Reset before reading so a concurrent enqueue can only turn the flag on.
        self.has_work.store(false, Ordering::SeqCst);
        // Invalidate before the snapshot: an enqueue that commits after this
        // read leaves the flag set, so the next pass recomputes the cache
        // instead of trusting a deadline that predates the new action.
        self.schedule_dirty.store(false, Ordering::SeqCst);
        let remaining = self.context.read().inspect_err(|_| {
            self.has_work.store(true, Ordering::SeqCst);
            // The cache was not recomputed: never trust it after a failure.
            self.schedule_dirty.store(true, Ordering::SeqCst);
        })?;
        let earliest = earliest_pending(&remaining, self.context.clock.now_ms());
        self.earliest_due.store(earliest, Ordering::SeqCst);
        if earliest != i64::MAX {
            self.has_work.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl CommitOutcome for UndoService {
    /// The worker's report for one dispatched confirmation (Task 3.2). The
    /// settlement is the queue's own machinery; a settlement that fails (a
    /// storage error) becomes a notice, because there is no calling thread to
    /// return it to — the rows stay `Committing` and startup recovery marks
    /// them `Failed`, the INV-008-safe direction.
    fn commit_finished(&self, pending_ids: &[u64], result: &AppResult<()>) {
        if pending_ids.is_empty() {
            return;
        }
        if let Err(error) = self.settle_ids(pending_ids, result) {
            self.context.events.emit(AppEvent::Notice {
                text: format!("The confirmation queue could not be updated: {}", error.message),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{Draft, Email};

    #[test]
    fn the_backoff_grows_and_stays_at_its_cap() {
        let steps: Vec<_> = (1..=6).map(backoff_ms).collect();
        assert_eq!(steps, [5_000, 15_000, 60_000, 300_000, 600_000, 600_000]);
        assert_eq!(backoff_ms(0), 5_000, "No attempt yet means the first step");
        assert_eq!(backoff_ms(u32::MAX), 600_000);
    }

    #[test]
    fn only_a_connect_class_mail_failure_is_retried() {
        let mail = UndoOperation::Mail {
            email_id: 1,
            action: "archive".into(),
            value: None,
        };
        let send = UndoOperation::SendDraft { draft_id: 2 };
        let offline = AppError::external("IMAP connect: os error 10060");
        assert!(is_retryable(&mail, &offline));
        // A delivery is reconciled by the outbox, never replayed by this queue.
        assert!(!is_retryable(&send, &offline));
        for definitive in [
            AppError::external("IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)"),
            AppError::external("Connection Lost"),
            AppError::external("Original message not found on the server: Q4. Sync mail and retry."),
            AppError::new("delivery_unknown", "Delivery is uncertain. Check the provider before sending again."),
            AppError::new("sent_not_saved", "Message was sent but its local copy could not be saved."),
            AppError::validation("Message not found"),
            // A worker crash mid-job is a lost outcome, never a retry (INV-008).
            AppError::new("worker_crash", "The queued action stopped unexpectedly; its result is unknown."),
            // The provider already executed it: a replay would apply it twice.
            AppError::new("remote_applied_local_failed", "IMAP connect: os error 10060"),
        ] {
            assert!(!is_retryable(&mail, &definitive), "{}", definitive.message);
        }
    }

    fn row(id: u64, email_id: u64, status: UndoStatus, expires_at_ms: i64) -> UndoAction {
        UndoAction {
            id,
            operation: UndoOperation::Mail {
                email_id,
                action: "archive".into(),
                value: None,
            },
            subject: String::new(),
            created_at_ms: expires_at_ms - 5_000,
            expires_at_ms,
            status,
            error: None,
            retry_at_ms: None,
            attempts: 0,
        }
    }

    #[test]
    fn a_due_row_waits_for_the_sibling_that_holds_it_and_never_for_the_past() {
        let now = 100_000;
        let mut store = Store::default();
        // Row 1 is due; 2 (same message) queued later, still in its window.
        store.undo_actions = vec![
            row(1, 7, UndoStatus::Pending, now - 10),
            row(2, 7, UndoStatus::Pending, now + 3_000),
        ];
        let held = &store.undo_actions[0];
        assert_eq!(blocked_until(&store, held, now), Some(now + 3_000));
        assert_eq!(earliest_pending(&store, now), now + 3_000);

        // A committing sibling is rechecked on the short bound.
        store.undo_actions[1].status = UndoStatus::Committing;
        assert_eq!(
            blocked_until(&store, &store.undo_actions[0], now),
            Some(now + COMMITTING_RECHECK_MS)
        );
        assert_eq!(earliest_pending(&store, now), now + COMMITTING_RECHECK_MS);

        // Nothing holds it once the sibling failed, expired, or is another message.
        store.undo_actions[1].status = UndoStatus::Failed;
        assert_eq!(blocked_until(&store, &store.undo_actions[0], now), None);
        assert_eq!(earliest_pending(&store, now), now - 10, "an unblocked due row is due now");
        store.undo_actions[1].status = UndoStatus::Pending;
        store.undo_actions[1].expires_at_ms = now;
        assert_eq!(blocked_until(&store, &store.undo_actions[0], now), None);
        store.undo_actions[1].expires_at_ms = now + 3_000;
        store.undo_actions[1].operation = UndoOperation::Mail {
            email_id: 8,
            action: "archive".into(),
            value: None,
        };
        assert_eq!(blocked_until(&store, &store.undo_actions[0], now), None);
    }

    #[test]
    fn a_held_row_does_not_hide_another_rows_earlier_deadline() {
        let now = 100_000;
        let mut store = Store::default();
        store.undo_actions = vec![
            row(1, 7, UndoStatus::Pending, now - 10),
            row(2, 8, UndoStatus::Pending, now + 1_000),
            row(3, 7, UndoStatus::Pending, now + 2_000),
        ];
        assert_eq!(earliest_pending(&store, now), now + 1_000);
        assert_eq!(earliest_pending(&Store::default(), now), i64::MAX);
    }

    mod quitting {
        //! «Esci» with actions still undoable: they are sent before Qamille leaves (audit Q8).
        use super::*;
        use crate::{
            application::{compute::NoCompute, ports::*},
            domain::model::{Account, Attachment},
            infrastructure::repository::MemoryRepository,
        };

        const NOW: i64 = 1_000_000;

        struct TestClock(AtomicI64);
        impl Clock for TestClock {
            fn now(&self) -> i64 {
                self.now_ms() / 1000
            }
            fn now_ms(&self) -> i64 {
                self.0.load(Ordering::SeqCst)
            }
        }
        #[derive(Default)]
        struct Recorder(Mutex<Vec<AppEvent>>);
        impl EventSink for Recorder {
            fn emit(&self, event: AppEvent) {
                self.0.lock().unwrap().push(event);
            }
        }
        struct NoMail;
        impl MailGateway for NoMail {
            fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
                Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
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
        struct NoFiles;
        impl AttachmentGateway for NoFiles {
            fn total_size(&self, paths: &[String]) -> AppResult<u64> {
                Ok(paths.len() as u64)
            }
            fn read_text(&self, _: &Attachment) -> AppResult<String> {
                Ok(String::new())
            }
        }

        /// One account with three messages, and the queue rows given.
        fn service(rows: Vec<UndoAction>) -> (UndoService, Arc<ServiceContext>, Arc<Recorder>) {
            let mut store = Store::default();
            store.accounts.push(Account { id: 1, email: "one@example.test".into(), ..Default::default() });
            for id in [10, 11, 12, 13] {
                store.emails.push(Email { id, account_id: 1, ..Default::default() });
            }
            store.undo_actions = rows;
            let recorder = Arc::new(Recorder::default());
            let context = Arc::new(ServiceContext {
                compute: Arc::new(NoCompute),
                repository: Arc::new(MemoryRepository::new(store)),
                mail: Arc::new(NoMail),
                secrets: Arc::new(NoSecrets),
                clock: Arc::new(TestClock(AtomicI64::new(NOW))),
                attachments: Arc::new(NoFiles),
                events: recorder.clone(),
            });
            let mail = MailService::new(context.clone());
            let drafts = DraftService::new(context.clone());
            (UndoService::new(context.clone(), mail, drafts), context, recorder)
        }
        fn statuses(context: &ServiceContext) -> Vec<(u64, UndoStatus, i64)> {
            context.read().unwrap().undo_actions.iter().map(|a| (a.id, a.status, a.expires_at_ms)).collect()
        }

        #[test]
        fn undo_send_now_makes_every_pending_action_due() {
            let _wakes = connectivity::test_guard();
            let mut retrying = row(4, 13, UndoStatus::Pending, NOW - 60_000);
            retrying.attempts = 2;
            retrying.retry_at_ms = Some(NOW + 300_000);
            let (undo, context, recorder) = service(vec![
                row(1, 10, UndoStatus::Pending, NOW + 20_000),
                row(2, 11, UndoStatus::Pending, NOW + 5_000),
                row(3, 12, UndoStatus::Pending, NOW + 30_000),
                retrying,
            ]);
            assert_eq!(undo.send_now().unwrap(), 4);
            // Every undo window ends now, and a confirmation backing off is attempted now.
            for action in context.read().unwrap().undo_actions {
                assert_eq!(action.expires_at_ms.min(NOW), action.expires_at_ms, "row {} still undoable", action.id);
                assert!(action.retry_at_ms.is_none_or(|at| at <= NOW), "row {} still backing off", action.id);
            }
            assert_eq!(undo.next_deadline().unwrap(), Some(NOW));
            // The rows are announced: the undo notices stop counting down.
            let changed = recorder.0.lock().unwrap().iter().filter(|e| matches!(e, AppEvent::UndoChanged { .. })).count();
            assert_eq!(changed, 4);
            // The queue's next pass takes them all.
            let workers = WorkerRegistry::new(
                MailService::new(context.clone()),
                DraftService::new(context.clone()),
                context.clone(),
                Arc::new(NullSink),
                Arc::new(undo.clone()) as Arc<dyn CommitOutcome>,
            );
            recorder.0.lock().unwrap().clear();
            assert!(undo.commit_due(&workers).unwrap());
            let mut claimed: Vec<u64> = recorder
                .0
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    AppEvent::UndoChanged { action } if action.status == UndoStatus::Committing => Some(action.id),
                    _ => None,
                })
                .collect();
            claimed.sort();
            assert_eq!(claimed, [1, 2, 3, 4]);
        }

        #[test]
        fn undo_send_now_leaves_failed_and_committing_alone() {
            let _wakes = connectivity::test_guard();
            let (undo, context, _) = service(vec![
                row(1, 10, UndoStatus::Failed, NOW + 20_000),
                row(2, 11, UndoStatus::Committing, NOW + 20_000),
                row(3, 12, UndoStatus::Pending, NOW + 20_000),
            ]);
            assert_eq!(undo.send_now().unwrap(), 1);
            assert_eq!(
                statuses(&context),
                [(1, UndoStatus::Failed, NOW + 20_000), (2, UndoStatus::Committing, NOW + 20_000), (3, UndoStatus::Pending, NOW)],
            );
            // Nothing left to make due: nothing written, nothing announced.
            let (undo, context, recorder) = service(vec![row(1, 10, UndoStatus::Failed, NOW + 20_000)]);
            assert_eq!(undo.send_now().unwrap(), 0);
            assert_eq!(statuses(&context), [(1, UndoStatus::Failed, NOW + 20_000)]);
            assert!(recorder.0.lock().unwrap().is_empty());
        }

        #[test]
        fn unsettled_tells_queued_sends_from_sends_on_their_way_and_from_other_actions() {
            let draft = |id, operation, status| UndoAction { operation, ..row(id, 0, status, NOW - 1) };
            let (undo, _, _) = service(vec![
                row(1, 10, UndoStatus::Pending, NOW + 20_000),
                row(2, 11, UndoStatus::Committing, NOW - 1),
                row(3, 12, UndoStatus::Failed, NOW - 1),
                draft(4, UndoOperation::SendDraft { draft_id: 20 }, UndoStatus::Pending),
                draft(5, UndoOperation::SendDraft { draft_id: 21 }, UndoStatus::Pending),
                draft(6, UndoOperation::SendDraft { draft_id: 22 }, UndoStatus::Committing),
                draft(7, UndoOperation::SendDraft { draft_id: 23 }, UndoStatus::Failed),
                // Scheduling and discarding a draft send nothing: they are actions like an archive.
                draft(8, UndoOperation::ScheduleDraft { draft_id: 24, send_at: NOW }, UndoStatus::Pending),
                draft(9, UndoOperation::DiscardDraft { draft_id: 25 }, UndoStatus::Committing),
            ]);
            let left = undo.unsettled().unwrap();
            // Queued rows stay for the next start; rows on their way are cut off by a quit and
            // startup recovery fails them, never retries them (INV-008).
            assert_eq!(left, Unsettled { sends: 2, sending: 1, actions: 2, confirming: 2 });
            assert_eq!(left.total(), 7);
            assert_eq!(service(vec![]).0.unsettled().unwrap(), Unsettled::default());
            assert_eq!(Unsettled::default().total(), 0);
        }
    }

    #[test]
    fn a_wake_is_matched_through_the_mail_or_draft_account() {
        let mut store = Store::default();
        store.emails.push(Email {
            id: 5,
            account_id: 2,
            ..Default::default()
        });
        store.drafts.push(Draft {
            id: 6,
            account_id: 3,
            ..Default::default()
        });
        let mail = |email_id| UndoOperation::Mail {
            email_id,
            action: "archive".into(),
            value: None,
        };
        assert_eq!(account_of(&store, &mail(5)), Some(2));
        assert_eq!(
            account_of(&store, &UndoOperation::SendDraft { draft_id: 6 }),
            Some(3)
        );
        assert_eq!(account_of(&store, &mail(99)), None);
        assert_eq!(
            account_of(&store, &UndoOperation::DiscardDraft { draft_id: 99 }),
            None,
            "A vanished target cannot be woken"
        );
    }
}
