use crate::runtime::{DesktopState, blocking};
use qamille::application::AppResult;
use tauri::State;
#[tauri::command]
pub async fn connect_google(
    window: tauri::WebviewWindow,
    state: State<'_, DesktopState>,
    login_hint: Option<String>,
) -> AppResult<()> {
    let s = state.accounts.clone();
    let result = blocking(move || s.connect_google(login_hint.as_deref().unwrap_or(""))).await;
    back_from_browser(&window);
    // A new or renewed mailbox goes to the other computers within seconds.
    state.sync.nudge();
    result
}
/// A sign-in ended in the browser: flash the taskbar button once, without taking the focus
/// from the browser (audit F20, T19).
pub(crate) fn back_from_browser(window: &tauri::WebviewWindow) {
    if !window.is_focused().unwrap_or(true) {
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}
/// Ends a Google sign-in that is still waiting for the browser.
#[tauri::command]
pub fn cancel_google_sign_in(state: State<'_, DesktopState>) {
    state.accounts.cancel_google();
}
#[tauri::command]
pub async fn connect_password(
    state: State<'_, DesktopState>,
    name: String,
    email: String,
    password: String,
) -> AppResult<()> {
    let s = state.accounts.clone();
    let result = blocking(move || s.connect_password(&name, &email, &password)).await;
    state.sync.nudge();
    result
}

#[tauri::command]
pub async fn get_account_storage(
    state: State<'_, DesktopState>,
    account_id: u32,
) -> AppResult<qamille::domain::storage::StorageUsage> {
    let service = state.storage.clone();
    blocking(move || service.get(account_id)).await
}

#[tauri::command]
pub async fn update_account_storage(
    state: State<'_, DesktopState>,
    account_id: u32,
    policy: qamille::domain::storage::AccountStoragePolicy,
    confirm_cleanup: bool,
) -> AppResult<qamille::domain::storage::StorageUsage> {
    let service = state.storage.clone();
    blocking(move || service.update(account_id, policy, confirm_cleanup)).await
}
