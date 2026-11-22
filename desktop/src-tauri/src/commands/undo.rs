use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{AppResult, undo::Unsettled},
    domain::undo::UndoAction,
};
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::State;

#[tauri::command]
pub async fn queue_mail_action(state: State<'_, DesktopState>, id: u64, action: String, value: Option<String>) -> AppResult<UndoAction> {
    let s = state.undo.clone();
    blocking(move || s.queue_mail(id, action, value)).await
}
#[tauri::command]
pub async fn queue_draft_action(state: State<'_, DesktopState>, id: u64, action: String, send_at: Option<i64>) -> AppResult<UndoAction> {
    let s = state.undo.clone();
    blocking(move || s.queue_draft(id, &action, send_at)).await
}
#[tauri::command]
pub async fn undo_action(state: State<'_, DesktopState>, id: u64) -> AppResult<UndoAction> {
    let s = state.undo.clone();
    blocking(move || s.undo(id)).await
}
#[tauri::command]
pub async fn dismiss_action(state: State<'_, DesktopState>, id: u64) -> AppResult<()> {
    let s = state.undo.clone();
    blocking(move || s.dismiss(id)).await
}

/// The longest a page may make Qamille wait for the queue: a way out must stay a way out.
const SEND_NOW_LIMIT_MS: u32 = 30_000;
/// How often the wait reads the queue: a confirmation takes a second or two.
const SEND_NOW_STEP: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, Serialize)]
pub struct UndoSendResult {
    /// The actions still not confirmed when the wait ended, and the sends that failed during it: 0
    /// means everything left.
    pub remaining: u32,
    /// The same actions by what quitting does to them (`sends`, `sending`, `actions`,
    /// `confirming`): the page hands them to `undo_quit_anyway`, which words each.
    #[serde(flatten)]
    pub left: Unsettled,
    /// Sends queued when the wait began that failed during it (no connection): no longer queued, but
    /// they did not leave (`background_rules::not_sent`).
    pub failed: u32,
}

/// The drafts whose send is still in the undo queue (waiting or on its way), and every draft.
fn queued_sends(context: &qamille::application::ServiceContext) -> AppResult<(Vec<u64>, Vec<u64>)> {
    use qamille::domain::undo::{UndoOperation, UndoStatus};
    let store = context.read_shared()?;
    let queued = store
        .undo_actions
        .iter()
        .filter(|action| matches!(action.status, UndoStatus::Pending | UndoStatus::Committing))
        .filter_map(|action| match action.operation {
            UndoOperation::SendDraft { draft_id } => Some(draft_id),
            _ => None,
        })
        .collect();
    Ok((queued, store.drafts.iter().map(|draft| draft.id).collect()))
}

/// Qamille is quitting (the undo queue's close guard, `lib/undo-close.ts`, audit Q8): every action
/// still undoable is sent now, and this waits, at most `wait_ms`, until the queue is empty. A send that
/// fails meanwhile leaves the queue without leaving the computer: it is counted as `failed`, so that
/// «Esci» asks instead of quitting in silence.
#[tauri::command]
pub async fn undo_send_now(state: State<'_, DesktopState>, wait_ms: u32) -> AppResult<UndoSendResult> {
    let s = state.undo.clone();
    let context = state.context();
    blocking(move || {
        let (started, _) = queued_sends(&context)?;
        s.send_now()?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms.min(SEND_NOW_LIMIT_MS)));
        loop {
            let left = s.unsettled()?;
            if left.total() == 0 || Instant::now() >= deadline {
                let (queued, drafts) = queued_sends(&context)?;
                let failed = crate::background_rules::not_sent(&started, &queued, &drafts);
                let remaining = u32::try_from(left.total()).unwrap_or(u32::MAX).saturating_add(failed);
                return Ok(UndoSendResult { remaining, left, failed });
            }
            std::thread::sleep(SEND_NOW_STEP);
        }
    })
    .await
}

/// «Esci» found actions that did not leave (`left`, from `undo_send_now`): a Windows question over
/// the main window, like `meeting_confirm_close`, saying what quitting does to each kind. `true`:
/// quit anyway; what is queued goes at the next start, what is on its way may be lost (the next
/// start marks it failed, never retries it). Esc and the X keep Qamille open, the harmless choice.
#[tauri::command]
pub async fn undo_quit_anyway(window: tauri::WebviewWindow, left: Unsettled, failed: Option<u32>) -> AppResult<bool> {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let left = crate::background_rules::QueueLeft {
        sends: count(left.sends),
        sending: count(left.sending),
        actions: count(left.actions),
        confirming: count(left.confirming),
        failed: failed.unwrap_or(0),
    };
    if left.total() == 0 {
        // Nothing to lose: no question with nothing in it.
        return Ok(true);
    }
    let italian = crate::window_shell::italian();
    let (stay, leave) = if italian { ("Resta aperto", "Esci comunque") } else { ("Stay open", "Quit anyway") };
    let text = crate::background_rules::unsent_text(italian, left);
    blocking(move || {
        Ok(rfd::MessageDialog::new()
            .set_parent(&window)
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Qamille")
            .set_description(text)
            .set_buttons(rfd::MessageButtons::OkCancelCustom(stay.into(), leave.into()))
            .show()
            == rfd::MessageDialogResult::Custom(leave.into()))
    })
    .await
}
