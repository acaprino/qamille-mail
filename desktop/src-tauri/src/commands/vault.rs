//! The key of the credentials (phase 3, Task 7): where it stands, the transfer passphrase that
//! opens it on another Windows user or computer, and starting over when it does not open here.
//! The passphrase goes straight to `secret`, which wipes it after use; it is never logged.
use crate::runtime::{DesktopState, blocking};
use qamille::application::AppResult;
use qamille::infrastructure::gateways::vault_error;
use qamille::infrastructure::secret::{self, VaultStatus};
use tauri::State;

#[tauri::command]
pub async fn vault_status() -> AppResult<VaultStatus> {
    blocking(|| Ok(secret::status())).await
}

/// Argon2id runs here (about a second): off the main thread.
#[tauri::command]
pub async fn vault_set_transfer_passphrase(passphrase: String) -> AppResult<VaultStatus> {
    blocking(move || secret::set_transfer_passphrase(passphrase).map_err(vault_error)).await
}

#[tauri::command]
pub async fn vault_remove_transfer_passphrase() -> AppResult<VaultStatus> {
    blocking(|| secret::remove_transfer_passphrase().map_err(vault_error)).await
}

/// The credentials open: what the vault held back leaves now, not at its next recheck. Scheduled
/// sends and the undo queue's sends and actions waited for it (a `vault_*` refusal holds them,
/// never fails them), and none of them reached a provider.
#[tauri::command]
pub async fn vault_unlock(state: State<'_, DesktopState>, passphrase: String) -> AppResult<VaultStatus> {
    let (scheduler, undo) = (state.scheduler.clone(), state.undo.clone());
    blocking(move || {
        let status = secret::unlock_with_transfer_passphrase(passphrase).map_err(vault_error)?;
        scheduler.credentials_opened();
        if let Err(error) = undo.credentials_opened() {
            // The vault is open either way: the queue's own backoff still retries what it holds.
            crate::native_feel::report(&format!("the undo queue was not woken after the vault opened: {}", error.message));
        }
        Ok(status)
    })
    .await
}

/// The credentials that do not open here go to a folder of their own in the data folder, never
/// deleted; the answer is that folder's name. `unopenable`: Windows still cannot open the key
/// after «Try again», and the person confirmed.
#[tauri::command]
pub async fn vault_start_over(unopenable: Option<bool>) -> AppResult<String> {
    blocking(move || secret::start_over(unopenable.unwrap_or(false)).map_err(vault_error)).await
}

/// The person was told where a sign-in set the old credentials aside.
#[tauri::command]
pub async fn vault_dismiss_set_aside() -> AppResult<()> {
    blocking(|| secret::dismiss_set_aside_notice().map_err(vault_error)).await
}
