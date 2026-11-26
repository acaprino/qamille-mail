use super::{
    AppError, AppResult, ServiceContext,
    ports::{GOOGLE_SECRET, GoogleIdentity, account_key},
};
use crate::domain::model::{Account, AccountKind, SignatureBinding};
use std::sync::Arc;

/// What a removed mailbox was: enough for the synchronization to delete its item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedAccount {
    pub provider_id: String,
    pub email: String,
}

#[derive(Clone)]
pub struct AccountService {
    context: Arc<ServiceContext>,
    identity: Arc<dyn GoogleIdentity>,
    revoke_local_grants: fn(u32) -> AppResult<()>,
}
impl AccountService {
    pub fn new(context: Arc<ServiceContext>, identity: Arc<dyn GoogleIdentity>) -> Self {
        Self { context, identity, revoke_local_grants: |_| Ok(()) }
    }
    pub fn with_grant_cleanup(mut self, cleanup: fn(u32) -> AppResult<()>) -> Self {
        self.revoke_local_grants = cleanup;
        self
    }
    /// `login_hint` is the address the person expects to use; empty lets Google ask.
    pub fn connect_google(&self, login_hint: &str) -> AppResult<()> {
        // Before the browser opens: a sign-in whose token could not be saved is not started.
        self.context.secrets.check_writable()?;
        let store = self.context.read_shared()?;
        let secret = self.context.secrets.get_for_sign_in(GOOGLE_SECRET)?;
        let (client_id, secret) = self.identity.client_credentials(&store.settings.google_client_id, &secret)?;
        let (email, token) = self.identity.sign_in(&client_id, &secret, login_hint)?;
        self.connect(None, &email, &token, &client_id)
    }
    /// Removes a mailbox from this computer: its credential in the vault, its messages and its
    /// drafts. The mail stays on the provider.
    pub fn remove_account(&self, id: u32) -> AppResult<RemovedAccount> {
        let store = self.context.read_shared()?;
        let account = store.account(id).cloned().ok_or_else(|| AppError::missing("Account"))?;
        let removed = RemovedAccount { provider_id: account.resolved_provider_id().to_string(), email: account.email.clone() };
        (self.revoke_local_grants)(id)?;
        self.context.update(|store| {
            let current = store.account(id).ok_or_else(|| AppError::missing("Account"))?;
            if !current.email.eq_ignore_ascii_case(&account.email) || current.resolved_provider_id() != account.resolved_provider_id() {
                return Err(AppError::new("identity_changed", "Mailbox changed; refresh before removing it"));
            }
            if account.resolved_provider_id() != "sample" {
                let primary = super::sync::credential_key(&account);
                // A locked vault otherwise accepts a removal as a no-op.
                self.context.secrets.get(&primary)?;
                for key in [format!("calendar:{id}"), format!("contacts:{id}"), primary] {
                    self.context.secrets.remove(&key)?;
                }
            }
            store.detach_mailbox(id, self.context.clock.now());
            Ok(())
        })?;
        Ok(removed)
    }
    /// Stops a Google sign-in that is still waiting for the browser.
    pub fn cancel_google(&self) {
        self.identity.cancel_sign_in();
    }
    pub fn connect_password(&self, name: &str, email: &str, password: &str) -> AppResult<()> {
        self.connect(Some(name), email, password, "")
    }
    /// `name` is what the person typed; `None` (Google sign-in) names a new account after its address
    /// and leaves the name of an account that is already there alone.
    fn connect(&self, name: Option<&str>, email: &str, password: &str, client_id: &str) -> AppResult<()> {
        self.context.secrets.check_writable()?;
        let rename = name.is_some();
        let name = name.unwrap_or(email).trim().to_string();
        let email = email.trim().to_lowercase();
        if !email.contains('@') || email.contains([' ', '\r', '\n']) || password.trim().is_empty() {
            return Err(AppError::validation(
                "Enter an email address and valid sign-in details",
            ));
        }
        let mut account = Account {
            provider_id: "gmail".into(),
            name,
            email: email.clone(),
            username: email.clone(),
            password: password.into(),
            oauth_client_id: client_id.into(),
            kind: AccountKind::Gmail,
            color: [65, 106, 219],
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            archive_folder: "[Gmail]/All Mail".into(),
            trash_folder: "[Gmail]/Trash".into(),
            signature: SignatureBinding::MostRecent,
            fetch_count: 200,
            ..Default::default()
        };
        self.context.mail.check_login(&account)?;
        account.password.clear();
        self.context.update(|store| {
            self.context.secrets.set(&account_key(&email), password)?;
            if let Some(old) = store.accounts.iter_mut().find(|a| {
                a.email.eq_ignore_ascii_case(&email) && a.resolved_provider_id() == "gmail"
            }) {
                old.oauth_client_id = account.oauth_client_id;
                old.provider_id = "gmail".into();
                if rename {
                    old.name = account.name;
                }
            } else {
                account.id = store.allocate_mailbox_id().ok_or_else(|| AppError::storage("Mailbox identifiers exhausted"))?;
                store.accounts.push(account);
            }
            Ok(())
        })
    }
}
