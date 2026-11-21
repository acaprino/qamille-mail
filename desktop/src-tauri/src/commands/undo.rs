use crate::runtime::{DesktopState, blocking};
use qamille::{application::AppResult, domain::undo::UndoAction};
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
    /// The actions still not confirmed when the wait ended: 0 means everything left.
    pub remaining: u32,
}

/// Qamille is quitting (the undo queue's close guard, `lib/undo-close.ts`, audit Q8): every action
/// still undoable is sent now, and this waits, at most `wait_ms`, until the queue is empty.
#[tauri::command]
pub async fn undo_send_now(state: State<'_, DesktopState>, wait_ms: u32) -> AppResult<UndoSendResult> {
    let s = state.undo.clone();
    blocking(move || {
        s.send_now()?;
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms.min(SEND_NOW_LIMIT_MS)));
        loop {
            let remaining = s.unsettled()?;
            if remaining == 0 || Instant::now() >= deadline {
                return Ok(UndoSendResult { remaining: u32::try_from(remaining).unwrap_or(u32::MAX) });
            }
            std::thread::sleep(SEND_NOW_STEP);
        }
    })
    .await
}

/// «Esci» found actions that did not leave: a Windows question over the main window, like
/// `meeting_confirm_close`. `true`: quit anyway, they go at the next start. Esc and the X keep
/// Qamille open, the harmless choice.
#[tauri::command]
pub async fn undo_quit_anyway(window: tauri::WebviewWindow, remaining: u32) -> AppResult<bool> {
    let italian = crate::window_shell::italian();
    let (stay, leave) = if italian { ("Resta aperto", "Esci comunque") } else { ("Stay open", "Quit anyway") };
    let text = crate::background_rules::unsent_text(italian, remaining);
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
