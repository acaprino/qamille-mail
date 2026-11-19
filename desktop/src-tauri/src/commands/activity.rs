//! The activity log for the sidebar panel and the Activity page (centro attività):
//! read-only queries plus the "seen" marker. The rows are written by the services
//! (`application::activity`), never from here.
use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{AppResult, activity},
    domain::activity::{ActivityEntry, ActivityFilter},
};
use tauri::State;

/// Rows the sidebar panel may ask for.
const RECENT_MAX: u32 = 60;
/// Rows one page of the Activity page may ask for.
const PAGE_MAX: u32 = 200;

/// Whether this store keeps a history: false with the JSON store, and the UI then
/// says the history needs the SQLite engine.
#[tauri::command]
pub async fn activity_available(state: State<'_, DesktopState>) -> AppResult<bool> {
    let repository = state.repository();
    blocking(move || Ok(repository.activity_supported())).await
}
/// The newest rows, for the panel (it asks for 3).
#[tauri::command]
pub async fn activity_recent(state: State<'_, DesktopState>, limit: u32) -> AppResult<Vec<ActivityEntry>> {
    let repository = state.repository();
    blocking(move || repository.activity_page(None, limit.clamp(1, RECENT_MAX), ActivityFilter::All)).await
}
/// One page of the full history, newest first; `before_seq` is the last row of the previous page.
#[tauri::command]
pub async fn activity_page(
    state: State<'_, DesktopState>,
    before_seq: Option<i64>,
    limit: u32,
    filter: ActivityFilter,
) -> AppResult<Vec<ActivityEntry>> {
    let repository = state.repository();
    blocking(move || repository.activity_page(before_seq, limit.clamp(1, PAGE_MAX), filter)).await
}
/// Errors and failed actions the user has not seen yet: the dot on the history icon.
#[tauri::command]
pub async fn activity_unseen_errors(state: State<'_, DesktopState>) -> AppResult<u32> {
    let repository = state.repository();
    blocking(move || repository.unseen_activity_errors()).await
}
/// The panel or the page was opened: everything up to now is seen.
#[tauri::command]
pub async fn activity_mark_seen(state: State<'_, DesktopState>) -> AppResult<()> {
    let context = state.context();
    blocking(move || activity::mark_seen(&context)).await
}
