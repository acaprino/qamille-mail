//! Detach account-scoped permissions and pending writes outside the mail database.
use crate::application::AppResult;
use crate::model::Store;

pub fn revoke(id: u32) -> AppResult<()> {
    super::mcp_config::revoke_mailbox(id)?;
    super::contact_repository::FileContactStore::new(Store::data_dir()).revoke_mailbox(id)
}

/// The legacy native client uses the same sidecars and vault as the desktop.
pub fn retire_legacy(account: &crate::model::Account) -> Result<(), String> {
    revoke(account.id).map_err(|e| e.message)?;
    if account.resolved_provider_id() != "sample" {
        let primary = crate::application::sync::credential_key(account);
        super::secret::get_result(&primary)?;
        for key in [format!("calendar:{}", account.id), format!("contacts:{}", account.id), primary] {
            super::secret::remove(&key)?;
        }
        super::oauth::forget_access(&account.email);
    }
    Ok(())
}
