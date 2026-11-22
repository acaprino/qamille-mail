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
    /// The same by what quitting does to them (`sends`, `sending`, `actions`, `confirming`, and
    /// `failed`, the sends this wait made due that failed): the page hands them to
    /// `undo_quit_anyway`, which words each.
    #[serde(flatten)]
    pub left: Unsettled,
}

/// Qamille is quitting (the undo queue's close guard, `lib/undo-close.ts`, audit Q8): every action
/// still undoable is sent now, and this waits, at most `wait_ms`, until the queue is empty. A send that
/// fails meanwhile leaves the queue without leaving the computer: it comes back as `failed`
/// (`UndoService::unsettled_since`), so that «Esci» asks instead of quitting in silence. At session end
/// (`session_end`: the session-end step, then `settleUndoQueue`) nobody can answer: what did not leave
/// is a line in `native.log` from the call that waits, and nothing asks. The sends the first call
/// made due are kept (`SESSION_SENDS`), so one that fails before the second call is still counted.
#[tauri::command]
pub async fn undo_send_now(state: State<'_, DesktopState>, wait_ms: u32, session_end: Option<bool>) -> AppResult<UndoSendResult> {
    let s = state.undo.clone();
    let session_end = session_end == Some(true);
    blocking(move || {
        let mut started = s.sends_in_queue()?;
        if session_end {
            let mut kept = SESSION_SENDS.lock().unwrap_or_else(|e| e.into_inner());
            kept.extend(started.iter().copied());
            kept.sort_unstable();
            kept.dedup();
            started.clone_from(&kept);
        }
        s.send_now()?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms.min(SEND_NOW_LIMIT_MS)));
        loop {
            let queued = s.unsettled()?;
            if queued.queued() == 0 || Instant::now() >= deadline {
                let left = s.unsettled_since(&started)?;
                if session_end && wait_ms > 0 && left.total() > 0 {
                    crate::native_feel::report(&crate::background_rules::session_end_line(count(left.failed), count(left.sends), count(left.sending), count(left.queued() - left.sends - left.sending)));
                }
                return Ok(UndoSendResult { remaining: count(left.total()), left });
            }
            std::thread::sleep(SEND_NOW_STEP);
        }
    })
    .await
}

/// The sends in the queue at the first call of the session end (`undo_send_now` with `session_end`).
static SESSION_SENDS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// «Esci» found actions that did not leave (`left`, from `undo_send_now`): a Windows question over
/// the main window, like `meeting_confirm_close`, saying what quitting does to each kind. `true`:
/// quit anyway; what is queued goes at the next start, what is on its way may be lost (the next
/// start marks it failed, never retries it). Esc and the X keep Qamille open, the harmless choice.
#[tauri::command]
pub async fn undo_quit_anyway(window: tauri::WebviewWindow, left: Unsettled) -> AppResult<bool> {
    let left = crate::background_rules::QueueLeft {
        sends: count(left.sends),
        sending: count(left.sending),
        actions: count(left.actions),
        confirming: count(left.confirming),
        failed: count(left.failed),
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
