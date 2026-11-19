//! The notice about the old local compute mode (wave 2): status, freeing the models (only with
//! the person's confirmation) and dismissing it.
use crate::runtime::{DesktopState, blocking};
use qamille::application::{AppResult, legacy_local::LegacyLocalStatus};
use tauri::State;

#[tauri::command]
pub async fn legacy_local_status(state: State<'_, DesktopState>) -> AppResult<LegacyLocalStatus> {
    let service = state.legacy_local.clone();
    blocking(move || service.status()).await
}
#[tauri::command]
pub async fn legacy_local_free_space(state: State<'_, DesktopState>, confirmed: bool) -> AppResult<u64> {
    let service = state.legacy_local.clone();
    blocking(move || service.free_space(confirmed)).await
}
#[tauri::command]
pub async fn legacy_local_dismiss(state: State<'_, DesktopState>) -> AppResult<()> {
    let service = state.legacy_local.clone();
    blocking(move || service.dismiss()).await
}
