use super::{AppResult, ServiceContext, drafts::DraftService, events::AppEvent};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Duration,
};

/// How long a scheduled send that the credentials vault refused waits before the vault is asked
/// again, unless it opens first ([`SchedulerService::credentials_opened`]). The overdue `send_at`
/// alone would be a deadline in the past, and the scheduler thread would spin on it.
const VAULT_RECHECK_MS: i64 = 60_000;

/// Writers that create or move a deadline announce it here, so the scheduler
/// thread sleeps exactly until the next deadline instead of polling at a fixed
/// rate (ADR-0007 decision 4, INV-005). Process-global like `connectivity`:
/// the MCP process enqueues deliveries without holding a SchedulerService.
fn waker() -> &'static Arc<(Mutex<bool>, Condvar)> {
    static WAKER: std::sync::OnceLock<Arc<(Mutex<bool>, Condvar)>> = std::sync::OnceLock::new();
    WAKER.get_or_init(|| Arc::new((Mutex::new(false), Condvar::new())))
}
/// Call after committing a new or moved scheduler deadline (outbox enqueue or
/// retry, a settled scheduled send, a snooze/reminder/due write).
pub fn note_schedule_change() {
    let (flag, condvar) = &**waker();
    *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    condvar.notify_all();
}

/// Timing belongs to the host; the use case is deterministic and has no UI loop.
/// Failed scheduled sends are disarmed to avoid repeated SMTP attempts. A send
/// the credentials vault refuses never failed: it stays armed and is held.
#[derive(Clone)]
pub struct SchedulerService {
    context: Arc<ServiceContext>,
    drafts: DraftService,
    failed: Arc<Mutex<HashSet<u64>>>,
    /// Scheduled sends the vault refused (`vault_*`: credentials of another
    /// Windows user or computer, a key Windows cannot open right now), by draft,
    /// with the instant the vault is asked again. Their `send_at` stays as it
    /// was: nothing reached the provider, and they leave once the vault opens.
    held: Arc<Mutex<HashMap<u64, i64>>>,
}
impl SchedulerService {
    pub fn new(context: Arc<ServiceContext>, drafts: DraftService) -> Self {
        Self {
            context,
            drafts,
            failed: Arc::new(Mutex::new(HashSet::new())),
            held: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    /// Never lost to a poisoned lock: a held send missing here would be an
    /// overdue deadline, and the scheduler thread would spin on it.
    fn held(&self) -> MutexGuard<'_, HashMap<u64, i64>> {
        self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// The credentials vault opened (`vault_unlock`): the sends it held back are
    /// due now instead of at their recheck. Wakes the scheduler thread when
    /// there was one.
    pub fn credentials_opened(&self) {
        let released = {
            let mut held = self.held();
            let any = !held.is_empty();
            held.clear();
            any
        };
        if released {
            note_schedule_change();
        }
    }
    /// The earliest deadline the scheduler must act on, in milliseconds:
    /// snoozes, reminders, scheduled sends and queued outbox deliveries.
    /// `None` means nothing is due, so the thread waits for a writer's note
    /// alone (INV-005).
    pub fn next_deadline(&self) -> AppResult<Option<i64>> {
        let store = self.context.read_shared()?;
        let held = self.held().clone();
        let mut best: Option<i64> = None;
        let mut consider_ms = |ms: i64| {
            best = Some(best.map_or(ms, |current: i64| current.min(ms)));
        };
        let ms = |seconds: i64| seconds.saturating_mul(1000);
        for email in &store.emails {
            if let Some(at) = email.snoozed_until {
                consider_ms(ms(at));
            }
            if let Some(at) = email.reminder_at {
                consider_ms(ms(at));
            }
        }
        for draft in &store.drafts {
            if let Some(at) = draft.send_at {
                // A send the vault holds back is due at its recheck, never at its past time.
                consider_ms(held.get(&draft.id).map_or(ms(at), |&recheck| recheck.max(ms(at))));
            }
        }
        for delivery in &store.outbox {
            if matches!(
                delivery.status,
                crate::domain::outbox::DeliveryStatus::Queued
                    | crate::domain::outbox::DeliveryStatus::Scheduled
            ) {
                consider_ms(ms(delivery.send_at));
            }
        }
        Ok(best)
    }
    /// The context clock in milliseconds. The host uses it to cap the wait for
    /// the interim cross-process bridge, keeping a single time source.
    pub fn now_ms(&self) -> i64 {
        self.context.clock.now_ms()
    }
    /// Sleeps until `deadline` passes (indefinitely when `None`) or a writer
    /// notes a schedule change; the note flag is consumed here, so a note
    /// racing the wait's start cannot be lost (same protocol as the undo
    /// thread's waker).
    pub fn wait_for_deadline(&self, deadline: Option<i64>) {
        let (flag, condvar) = &**waker();
        let mut signaled = flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if *signaled {
                *signaled = false;
                return;
            }
            let Some(due_ms) = deadline else {
                signaled = condvar.wait(signaled).unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            };
            let remaining = due_ms - self.context.clock.now_ms();
            if remaining <= 0 {
                return;
            }
            signaled = condvar
                .wait_timeout(signaled, Duration::from_millis(remaining as u64))
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
    }
    /// Snoozes and reminders that came due: one committed wake that announces
    /// exactly the messages it touched (INV-004). Runs before any send so a
    /// delivery failure can never skip a wake.
    pub fn wake_due(&self) -> AppResult<()> {
        let now = self.context.clock.now();
        let snapshot = self.context.read()?;
        if !snapshot.emails.iter().any(|e| {
            e.snoozed_until.is_some_and(|t| t <= now) || e.reminder_at.is_some_and(|t| t <= now)
        }) {
            return Ok(());
        }
        self.context.update_with_events(|store| {
            // Collect the touched messages before the wake clears the very
            // timestamps that identify them, then announce them per account
            // (INV-004): a wake reloads neither the list nor the workspace.
            let mut touched: std::collections::BTreeMap<u32, Vec<u64>> = Default::default();
            for email in store.emails.iter().filter(|e| {
                e.snoozed_until.is_some_and(|t| t <= now)
                    || e.reminder_at.is_some_and(|t| t <= now)
            }) {
                touched.entry(email.account_id).or_default().push(email.id);
            }
            store.wake_snoozed(now);
            for email in &mut store.emails {
                if email.reminder_at.is_some_and(|t| t <= now) {
                    email.reminder_at = None;
                    if !email.replied {
                        email.read = false;
                        email.pinned = true;
                    }
                }
            }
            let mut events = Vec::new();
            for (account_id, ids) in touched {
                events.push(AppEvent::EmailChanged { account_id, ids });
                events.push(AppEvent::CountsChanged { account_id });
            }
            Ok(((), events))
        })?;
        Ok(())
    }
    /// Outbox deliveries and scheduled sends that came due. A send failure
    /// disarms the draft and reports it; the wake half already ran, so the
    /// failure skips nothing (review 3.3 requirement). A send the credentials
    /// vault refuses is held instead, armed and without a notice (`held`).
    pub fn send_due(&self) -> AppResult<Vec<String>> {
        super::mcp_outbox::tick(self.context.clone())?;
        let now = self.context.clock.now();
        let now_ms = self.context.clock.now_ms();
        let snapshot = self.context.read()?;
        let mut notices = Vec::new();
        // A held send whose draft is gone, or no longer scheduled, is forgotten.
        self.held()
            .retain(|id, _| snapshot.drafts.iter().any(|d| d.id == *id && d.send_at.is_some()));
        for draft in snapshot
            .drafts
            .iter()
            .filter(|d| d.send_at.is_some_and(|t| t <= now))
            .filter(|d| {
                !snapshot.undo_actions.iter().any(|a| {
                    a.status != crate::domain::undo::UndoStatus::Failed
                        && a.operation.draft_id() == Some(d.id)
                })
            })
        {
            if self
                .failed
                .lock()
                .map(|f| f.contains(&draft.id))
                .unwrap_or(true)
            {
                continue;
            }
            if self.held().get(&draft.id).is_some_and(|&recheck| recheck > now_ms) {
                continue;
            }
            // The credentials before the claim: while the vault cannot open them
            // the send is held, still armed, with no event and no notice.
            if self.drafts.validate_send(draft.id).is_err_and(|error| error.is_vault()) {
                self.held().insert(draft.id, now_ms + VAULT_RECHECK_MS);
                continue;
            }
            self.held().remove(&draft.id);
            // Persist the claim before contacting SMTP. A crash or an ambiguous
            // network failure must not trigger another automatic send on restart.
            let claimed = self.context.update_with_events(|store| {
                let Some(current) = store.drafts.iter_mut().find(|d| d.id == draft.id) else {
                    return Ok((false, Vec::new()));
                };
                if !current.send_at.is_some_and(|t| t <= now) {
                    return Ok((false, Vec::new()));
                };
                current.send_at = None;
                current.updated = now.max(current.updated + 1);
                Ok((
                    true,
                    vec![AppEvent::DraftsChanged {
                        ids: vec![draft.id],
                    }],
                ))
            })?;
            if !claimed {
                continue;
            }
            if let Err(e) = self.drafts.send(draft.id) {
                // The vault shut between the check and the send. It refused before
                // any provider contact (`validate_send` heads the delivery's enqueue):
                // the claim is undone and the send held, as above. A claim that
                // cannot be undone falls through to the failure: never a silent disarm.
                if e.is_vault() && self.rearm(draft.id, draft.send_at).is_ok() {
                    self.held().insert(draft.id, now_ms + VAULT_RECHECK_MS);
                    continue;
                }
                if let Ok(mut failed) = self.failed.lock() {
                    failed.insert(draft.id);
                }
                notices.push(format!(
                    "Scheduled message ‘{}’: {}",
                    draft.subject, e.message
                ));
                // Also outside the window (a Windows notification, desktop `notifications.rs`):
                // once, since the draft is now in `failed` and its claim cleared `send_at`.
                self.context.events.emit(AppEvent::ScheduledSendFailed {
                    draft_id: draft.id,
                    subject: draft.subject.clone(),
                });
            }
        }
        Ok(notices)
    }
    /// Puts back the schedule a claim cleared, unless the draft was scheduled
    /// again or removed meanwhile. The drafts list follows (`DraftsChanged`,
    /// the same event as the claim's): the send is armed again, not failed.
    fn rearm(&self, id: u64, send_at: Option<i64>) -> AppResult<()> {
        self.context.update_with_events(|store| {
            let Some(current) = store.drafts.iter_mut().find(|d| d.id == id && d.send_at.is_none()) else {
                return Ok(((), Vec::new()));
            };
            current.send_at = send_at;
            current.updated = self.context.clock.now().max(current.updated + 1);
            Ok(((), vec![AppEvent::DraftsChanged { ids: vec![id] }]))
        })
    }
    /// One pass: wakes first (committed even if the send half fails), then
    /// deliveries and scheduled sends.
    pub fn tick(&self) -> AppResult<Vec<String>> {
        self.wake_due()?;
        self.send_due()
    }
}
