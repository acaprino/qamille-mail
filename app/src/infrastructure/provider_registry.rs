//! Select a provider adapter independently for each account. Unknown IDs never
//! silently fall back to Gmail. All application services keep using MailGateway.
use crate::{
    application::{AppError, AppResult, ports::*},
    domain::outbox::{ProviderCapabilities, RemoteDraft},
    model::{Account, Email, Store},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    adapters: BTreeMap<String, Arc<dyn MailGateway>>,
}
impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(
        &mut self,
        id: impl Into<String>,
        adapter: Arc<dyn MailGateway>,
    ) -> AppResult<()> {
        let id = id.into();
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
        {
            return Err(AppError::validation(
                "Provider IDs use lowercase letters, digits, underscores and hyphens",
            ));
        }
        if self.adapters.contains_key(&id) {
            return Err(AppError::validation("Provider adapter already registered"));
        }
        self.adapters.insert(id, adapter);
        Ok(())
    }
    pub fn installed_ids(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }
    fn adapter(&self, account: &Account) -> AppResult<&Arc<dyn MailGateway>> {
        self.adapters
            .get(account.resolved_provider_id())
            .ok_or_else(|| {
                AppError::new(
                    "unsupported_provider",
                    format!(
                        "No installed adapter for provider '{}'",
                        account.resolved_provider_id()
                    ),
                )
            })
    }
    /// Current shipping adapters. Later connectors are registered without changing services.
    pub fn initial(attachments: std::path::PathBuf) -> Self {
        let gmail: Arc<dyn MailGateway> = Arc::new(super::gateways::GmailGateway::new(attachments));
        let mut adapters = BTreeMap::new();
        adapters.insert("gmail".into(), gmail.clone());
        adapters.insert("sample".into(), gmail);
        Self { adapters }
    }
}
impl MailGateway for ProviderRegistry {
    fn capabilities(&self, a: &Account) -> ProviderCapabilities {
        self.adapter(a)
            .map(|adapter| {
                let mut c = adapter.capabilities(a);
                c.provider_id = a.resolved_provider_id().into();
                c
            })
            .unwrap_or_else(|_| ProviderCapabilities {
                provider_id: a.resolved_provider_id().into(),
                ..Default::default()
            })
    }
    fn authorize_mcp_delivery(&self, profile: Option<&str>, account_id: u32) -> AppResult<()> {
        // This is an application grant, not an authentication method of one provider.
        let store = super::repository::JsonStoreRepository::new(Store::path()).read()?;
        let account = store
            .account(account_id)
            .ok_or_else(|| AppError::missing("Account"))?;
        self.adapter(account)?;
        let grant =
            super::mcp_config::profile(profile.unwrap_or("default"), store.settings.mcp_access)?;
        if grant.access != crate::model::Access::Send
            || grant
                .account_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&account_id))
        {
            return Err(AppError::new(
                "permission_denied",
                "This MCP profile no longer allows sending from this account",
            ));
        }
        Ok(())
    }
    fn list_labels(&self, a: &Account) -> AppResult<Value> {
        self.adapter(a)?.list_labels(a)
    }
    fn manage_label(
        &self,
        a: &Account,
        action: &str,
        name: &str,
        new_name: Option<&str>,
    ) -> AppResult<Value> {
        self.adapter(a)?.manage_label(a, action, name, new_name)
    }
    fn list_remote_drafts(&self, a: &Account) -> AppResult<Vec<RemoteDraft>> {
        self.adapter(a)?.list_remote_drafts(a)
    }
    fn save_remote_draft(&self, a: &Account, draft: &RemoteDraft) -> AppResult<RemoteDraft> {
        self.adapter(a)?.save_remote_draft(a, draft)
    }
    fn delete_remote_draft(&self, a: &Account, id: &str) -> AppResult<()> {
        self.adapter(a)?.delete_remote_draft(a, id)
    }
    fn send_remote_draft(
        &self,
        a: &Account,
        sender: &str,
        message: OutgoingMessage,
        id: &str,
        revision: &str,
    ) -> AppResult<()> {
        self.adapter(a)?
            .send_remote_draft(a, sender, message, id, revision)
    }
    fn sync_with_policy(
        &self,
        a: &Account,
        request: &SyncRequest<'_>,
    ) -> AppResult<PolicySyncBatch> {
        self.adapter(a)?.sync_with_policy(a, request)
    }
    fn sync(&self, a: &Account) -> AppResult<SyncBatch> {
        self.adapter(a)?.sync(a)
    }
    fn load_body(&self, a: &Account, email: &Email) -> AppResult<Email> {
        self.adapter(a)?.load_body(a, email)
    }
    fn load_body_preview(&self, a: &Account, email: &Email) -> AppResult<Email> {
        self.adapter(a)?.load_body_preview(a, email)
    }
    fn authorize_read(&self, a: &Account) -> AppResult<()> {
        self.adapter(a)?.authorize_read(a)
    }
    fn search(
        &self,
        a: &Account,
        request: &crate::application::ports::RemoteSearchRequest,
        cancel: &AtomicBool,
    ) -> AppResult<crate::application::ports::RemoteSearchPage> {
        self.adapter(a)?.search(a, request, cancel)
    }
    fn history_batch(&self, a: &Account, request: &crate::application::ports::HistoryRequest) -> AppResult<crate::application::ports::HistoryBatch> {
        self.adapter(a)?.history_batch(a, request)
    }
    fn apply_message(&self, a: &Account, email: &Email, action: RemoteAction) -> AppResult<()> {
        self.adapter(a)?.apply_message(a, email, action)
    }
    fn resolve_identity(&self, a: &Account, email: &Email) -> AppResult<Email> {
        self.adapter(a)?.resolve_identity(a, email)
    }
    fn apply(&self, a: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        self.adapter(a)?.apply(a, uid, action)
    }
    fn watch(&self, a: &Account) -> AppResult<Box<dyn MailWatch>> {
        self.adapter(a)?.watch(a)
    }
    fn originals(&self, a: &Account, emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        self.adapter(a)?.originals(a, emails)
    }
    fn attachment_bytes(&self, a: &Account, email: &Email, attachment_id: u64) -> AppResult<Vec<u8>> {
        self.adapter(a)?.attachment_bytes(a, email, attachment_id)
    }
    fn thread(&self, a: &Account, email: &Email) -> AppResult<Vec<Email>> {
        self.adapter(a)?.thread(a, email)
    }
    fn send(&self, a: &Account, name: &str, message: OutgoingMessage) -> AppResult<()> {
        self.adapter(a)?.send(a, name, message)
    }
    fn check_login(&self, a: &Account) -> AppResult<()> {
        self.adapter(a)?.check_login(a)
    }
}
