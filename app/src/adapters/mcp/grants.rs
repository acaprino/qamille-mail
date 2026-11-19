//! Permission checks at provider boundaries, including long batches.
use crate::{
    application::{AppError, AppResult, ports::*},
    domain::outbox::{ProviderCapabilities, RemoteDraft},
    infrastructure::mcp_config::{self, Profile},
    model::{Access, Account, Email},
};
use std::sync::{Arc, atomic::AtomicBool};
pub struct GuardedMail {
    pub inner: Arc<dyn MailGateway>,
    pub repository: Arc<dyn StoreRepository>,
    pub name: String,
    pub granted: Profile,
}
impl GuardedMail {
    fn check(&self, account: &Account, access: Access) -> AppResult<()> {
        let store = self.repository.read()?;
        let current = mcp_config::profile(&self.name, store.settings.mcp_access)?;
        if serde_json::to_value(&current)? != serde_json::to_value(&self.granted)?
            || mcp_config::rank(current.access) < mcp_config::rank(access)
            || current
                .account_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&account.id))
            || store.account(account.id).is_none()
        {
            return Err(AppError::new(
                "permission_denied",
                "Assistant permissions changed; start a new request with the current grants",
            ));
        }
        Ok(())
    }
}
impl MailGateway for GuardedMail {
    fn capabilities(&self, a: &Account) -> ProviderCapabilities {
        if self.check(a, Access::ReadOnly).is_ok() {
            self.inner.capabilities(a)
        } else {
            ProviderCapabilities {
                provider_id: "unavailable".into(),
                ..Default::default()
            }
        }
    }
    fn send_remote_draft(
        &self,
        a: &Account,
        name: &str,
        m: OutgoingMessage,
        id: &str,
        revision: &str,
    ) -> AppResult<()> {
        self.check(a, Access::Send)?;
        self.inner.send_remote_draft(a, name, m, id, revision)
    }
    fn authorize_mcp_delivery(&self, p: Option<&str>, id: u32) -> AppResult<()> {
        self.inner.authorize_mcp_delivery(p, id)
    }
    fn list_labels(&self, a: &Account) -> AppResult<serde_json::Value> {
        self.check(a, Access::ReadOnly)?;
        self.inner.list_labels(a)
    }
    fn manage_label(
        &self,
        a: &Account,
        action: &str,
        name: &str,
        new_name: Option<&str>,
    ) -> AppResult<serde_json::Value> {
        self.check(a, Access::Triage)?;
        self.inner.manage_label(a, action, name, new_name)
    }
    fn list_remote_drafts(&self, a: &Account) -> AppResult<Vec<RemoteDraft>> {
        self.check(a, Access::ReadOnly)?;
        self.inner.list_remote_drafts(a)
    }
    fn save_remote_draft(&self, a: &Account, d: &RemoteDraft) -> AppResult<RemoteDraft> {
        self.check(a, Access::Triage).map_err(|error| AppError::new("cloud_save_not_started", error.message))?;
        self.inner.save_remote_draft(a, d)
    }
    fn delete_remote_draft(&self, a: &Account, id: &str) -> AppResult<()> {
        self.check(a, Access::Triage)?;
        self.inner.delete_remote_draft(a, id)
    }
    fn sync_with_policy(&self, a: &Account, r: &SyncRequest<'_>) -> AppResult<PolicySyncBatch> {
        self.check(a, Access::Triage)?;
        self.inner.sync_with_policy(a, r)
    }
    fn load_body(&self, a: &Account, e: &Email) -> AppResult<Email> {
        self.check(a, Access::ReadOnly)?;
        self.inner.load_body(a, e)
    }
    fn load_body_preview(&self, a: &Account, e: &Email) -> AppResult<Email> {
        self.check(a, Access::ReadOnly)?;
        self.inner.load_body_preview(a, e)
    }
    fn authorize_read(&self, a: &Account) -> AppResult<()> {
        self.check(a, Access::ReadOnly)?;
        self.inner.authorize_read(a)
    }
    fn search(&self, a: &Account, r: &crate::application::ports::RemoteSearchRequest, c: &AtomicBool) -> AppResult<crate::application::ports::RemoteSearchPage> {
        self.check(a, Access::ReadOnly)?;
        self.inner.search(a, r, c)
    }
    fn history_batch(&self, a: &Account, r: &crate::application::ports::HistoryRequest) -> AppResult<crate::application::ports::HistoryBatch> {
        self.check(a, Access::ReadOnly)?;
        self.inner.history_batch(a, r)
    }
    fn apply_message(&self, a: &Account, e: &Email, action: RemoteAction) -> AppResult<()> {
        self.check(a, Access::Triage)?;
        self.inner.apply_message(a, e, action)
    }
    fn resolve_identity(&self, a: &Account, e: &Email) -> AppResult<Email> {
        self.check(a, Access::Triage)?;
        self.inner.resolve_identity(a, e)
    }
    fn watch(&self, a: &Account) -> AppResult<Box<dyn MailWatch>> {
        self.check(a, Access::ReadOnly)?;
        self.inner.watch(a)
    }
    fn originals(&self, a: &Account, e: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        self.check(a, Access::ReadOnly)?;
        self.inner.originals(a, e)
    }
    fn attachment_bytes(&self, a: &Account, e: &Email, attachment_id: u64) -> AppResult<Vec<u8>> {
        self.check(a, Access::ReadOnly)?;
        self.inner.attachment_bytes(a, e, attachment_id)
    }
    fn thread(&self, a: &Account, e: &Email) -> AppResult<Vec<Email>> {
        self.check(a, Access::ReadOnly)?;
        self.inner.thread(a, e)
    }
    fn sync(&self, a: &Account) -> AppResult<SyncBatch> {
        self.check(a, Access::Triage)?;
        self.inner.sync(a)
    }
    fn apply(&self, a: &Account, id: u32, action: RemoteAction) -> AppResult<()> {
        self.check(a, Access::Triage)?;
        self.inner.apply(a, id, action)
    }
    fn send(&self, a: &Account, n: &str, m: OutgoingMessage) -> AppResult<()> {
        self.check(a, Access::Send)?;
        self.inner.send(a, n, m)
    }
    fn check_login(&self, a: &Account) -> AppResult<()> {
        self.check(a, Access::ReadOnly)?;
        self.inner.check_login(a)
    }
}
