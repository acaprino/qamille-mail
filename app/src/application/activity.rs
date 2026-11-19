//! Writing the activity log. Every function is best effort: the log is a record
//! of something that already happened, so a failing write is reported on stderr
//! and never fails the action it describes. Callers write after the store
//! transaction they record has committed (the SQLite write slot is not
//! reentrant), then the UI is told once with `ActivityChanged`.
use super::{AppResult, ServiceContext, events::AppEvent};
use crate::domain::{
    activity::{
        ActivityKind, ActivityOutcome, ActivitySource, FAILURE_REPEAT_MS, LOGGED_MAIL_ACTIONS,
        NewActivity, code,
    },
    undo::{UndoAction, UndoOperation},
};

/// Appends one row and announces it; the row's `seq`, or `None` when the store
/// keeps no log or the write failed.
pub fn record(context: &ServiceContext, entry: NewActivity) -> Option<i64> {
    let seq = match context.repository.append_activity(&entry, context.clock.now_ms()) {
        Ok(seq) => seq?,
        Err(error) => {
            eprintln!("activity log: could not record {}: {}", entry.code, error.message);
            return None;
        }
    };
    context.events.emit(AppEvent::ActivityChanged { ids: vec![seq as u64] });
    Some(seq)
}

/// Records a failure unless the newest row of its family (the same account, the
/// success code or this failure code) is already this failure with the same
/// message and is younger than [`FAILURE_REPEAT_MS`]: a sync that fails every
/// minute, or a model install that backs off and retries, is one row until the
/// message changes, a success comes in between, or the outage has gone on or come
/// back after hours. The check reads the database, so it also holds after a restart.
pub fn record_failure(context: &ServiceContext, entry: NewActivity, success_code: &str) -> Option<i64> {
    match context
        .repository
        .last_activity(&[success_code, entry.code.as_str()], entry.account_id)
    {
        Ok(Some(last))
            if last.code == entry.code
                && last.detail == entry.detail
                && context.clock.now_ms() - last.at_ms < FAILURE_REPEAT_MS =>
        {
            return None;
        }
        Ok(_) => {}
        // Unknown history: a repeated row is better than a lost error.
        Err(error) => eprintln!("activity log: could not look up {}: {}", entry.code, error.message),
    }
    record(context, entry)
}

/// Corrects the outcome of a user action's row (undone inside the window, or
/// failed after it) and announces the change when a row matched.
pub fn set_undo_outcome(
    context: &ServiceContext,
    undo_id: u64,
    outcome: ActivityOutcome,
    detail: Option<String>,
) {
    match context
        .repository
        .set_activity_outcome(undo_id, outcome, detail.as_deref())
    {
        Ok(Some(seq)) => context.events.emit(AppEvent::ActivityChanged { ids: vec![seq as u64] }),
        Ok(None) => {}
        Err(error) => eprintln!("activity log: could not update undo {undo_id}: {}", error.message),
    }
}

/// The user opened the panel or the page: everything up to now is seen, so the
/// dot on the history icon goes out. Not a user-visible failure when the store
/// keeps no log (the call then does nothing).
pub fn mark_seen(context: &ServiceContext) -> AppResult<()> {
    context.repository.set_activity_seen_ms(context.clock.now_ms())
}

/// The row for a queued user action, or `None` when the operation does not
/// enter the log. The account is not known here: the caller fills `account_id`
/// from the store it already holds.
pub fn for_undo_action(action: &UndoAction) -> Option<NewActivity> {
    let (code, ref_kind, ref_id) = match &action.operation {
        UndoOperation::Mail { email_id, action: name, .. } => {
            if !LOGGED_MAIL_ACTIONS.contains(&name.as_str()) {
                return None;
            }
            (name.clone(), "email", *email_id)
        }
        UndoOperation::SendDraft { draft_id } => (code::SEND.to_string(), "draft", *draft_id),
        UndoOperation::DiscardDraft { draft_id } => (code::DISCARD_DRAFT.to_string(), "draft", *draft_id),
        UndoOperation::ScheduleDraft { draft_id, .. } => {
            (code::SCHEDULE_DRAFT.to_string(), "draft", *draft_id)
        }
    };
    Some(NewActivity {
        kind: ActivityKind::Action,
        code,
        source: ActivitySource::User,
        subject: Some(action.subject.clone()).filter(|subject| !subject.is_empty()),
        ref_kind: Some(ref_kind.into()),
        ref_id: Some(ref_id),
        undo_id: Some(action.id),
        ..Default::default()
    })
}
