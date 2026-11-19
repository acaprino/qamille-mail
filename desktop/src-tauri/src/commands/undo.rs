use crate::runtime::{DesktopState, blocking};
use qamille::{application::AppResult, domain::undo::UndoAction};
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
