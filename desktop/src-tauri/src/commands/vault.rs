//! The key of the credentials (phase 3, Task 7): where it stands, the transfer passphrase that
//! opens it on another Windows user or computer, and starting over when it does not open here.
//! The passphrase goes straight to `secret`, which wipes it after use; it is never logged.
use crate::runtime::blocking;
use qamille::application::AppResult;
use qamille::infrastructure::gateways::vault_error;
use qamille::infrastructure::secret::{self, VaultStatus};

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

#[tauri::command]
pub async fn vault_unlock(passphrase: String) -> AppResult<VaultStatus> {
    blocking(move || secret::unlock_with_transfer_passphrase(passphrase).map_err(vault_error)).await
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
