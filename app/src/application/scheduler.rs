use super::{AppResult, ServiceContext, drafts::DraftService, events::AppEvent};
use std::{
    collections::HashSet,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

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
/// Failed scheduled sends are disarmed to avoid repeated SMTP attempts.
#[derive(Clone)]
pub struct SchedulerService {
    context: Arc<ServiceContext>,
    drafts: DraftService,
    failed: Arc<Mutex<HashSet<u64>>>,
}
impl SchedulerService {
    pub fn new(context: Arc<ServiceContext>, drafts: DraftService) -> Self {
        Self {
            context,
            drafts,
            failed: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    /// The earliest deadline the scheduler must act on, in milliseconds:
    /// snoozes, reminders, scheduled sends and queued outbox deliveries.
    /// `None` means nothing is due, so the thread waits for a writer's note
    /// alone (INV-005).
    pub fn next_deadline(&self) -> AppResult<Option<i64>> {
        let store = self.context.read_shared()?;
        let mut best: Option<i64> = None;
        let mut consider = |seconds: i64| {
            let ms = seconds.saturating_mul(1000);
            best = Some(best.map_or(ms, |current: i64| current.min(ms)));
        };
        for email in &store.emails {
            if let Some(at) = email.snoozed_until {
                consider(at);
            }
            if let Some(at) = email.reminder_at {
                consider(at);
            }
        }
        for draft in &store.drafts {
            if let Some(at) = draft.send_at {
                consider(at);
            }
        }
        for delivery in &store.outbox {
            if matches!(
                delivery.status,
                crate::domain::outbox::DeliveryStatus::Queued
                    | crate::domain::outbox::DeliveryStatus::Scheduled
            ) {
                consider(delivery.send_at);
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
    /// failure skips nothing (review 3.3 requirement).
    pub fn send_due(&self) -> AppResult<Vec<String>> {
        super::mcp_outbox::tick(self.context.clone())?;
        let now = self.context.clock.now();
        let snapshot = self.context.read()?;
        let mut notices = Vec::new();
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
    /// One pass: wakes first (committed even if the send half fails), then
    /// deliveries and scheduled sends.
    pub fn tick(&self) -> AppResult<Vec<String>> {
        self.wake_due()?;
        self.send_due()
    }
}
