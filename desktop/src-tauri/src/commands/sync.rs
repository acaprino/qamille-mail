//! The mailbox synchronization between this person's computers (wave 7): status, switches, the
//! notices of mailboxes removed elsewhere and the revocation of Google tokens. The credentials
//! never pass through here: the commands carry ids and switches only, and the service reads the
//! vault itself.
use crate::runtime::{DesktopState, blocking};
use qamille::application::{
    AppResult,
    sync::{MailboxRevoke, SyncReport, SyncStatus},
};
use tauri::State;

#[tauri::command]
pub async fn sync_status(state: State<'_, DesktopState>) -> AppResult<SyncStatus> {
    let sync = state.sync.clone();
    blocking(move || sync.status()).await
}
/// The general switch «Sincronizza le caselle tra i miei computer».
#[tauri::command]
pub async fn sync_set_enabled(state: State<'_, DesktopState>, enabled: bool) -> AppResult<()> {
    let sync = state.sync.clone();
    blocking(move || sync.set_enabled(enabled)).await
}
/// «Sincronizza questa casella».
#[tauri::command]
pub async fn sync_set_mailbox(state: State<'_, DesktopState>, account_id: u32, enabled: bool) -> AppResult<()> {
    let sync = state.sync.clone();
    blocking(move || sync.set_mailbox(account_id, enabled)).await
}
#[tauri::command]
pub async fn sync_now(state: State<'_, DesktopState>) -> AppResult<SyncReport> {
    let sync = state.sync.clone();
    blocking(move || sync.sync_now()).await
}
/// «Rimuovi anche qui» (`remove_here`) or «Tienila solo qui».
#[tauri::command]
pub async fn sync_resolve_removal(state: State<'_, DesktopState>, item_id: String, remove_here: bool) -> AppResult<()> {
    let (sync, accounts) = (state.sync.clone(), state.accounts.clone());
    blocking(move || {
        sync.resolve_removal(&item_id, remove_here, &accounts)?;
        Ok(())
    })
    .await
}
/// Asks Google to revoke the access of one mailbox (the retry of a failed revocation).
#[tauri::command]
pub async fn sync_revoke_mailbox(state: State<'_, DesktopState>, account_id: u32) -> AppResult<MailboxRevoke> {
    let sync = state.sync.clone();
    blocking(move || sync.revoke_mailbox(account_id)).await
}
/// «Revoca gli accessi alle caselle», after a device was disabled.
#[tauri::command]
pub async fn sync_revoke_all(state: State<'_, DesktopState>) -> AppResult<Vec<MailboxRevoke>> {
    let sync = state.sync.clone();
    blocking(move || sync.revoke_all()).await
}
/// Removes a mailbox from this computer; the others are asked about it at the next pass.
#[tauri::command]
pub async fn account_remove(state: State<'_, DesktopState>, account_id: u32) -> AppResult<()> {
    let (sync, accounts) = (state.sync.clone(), state.accounts.clone());
    blocking(move || {
        let removed = accounts.remove_account(account_id)?;
        sync.after_local_removal(&removed.provider_id, &removed.email)
    })
    .await
}
