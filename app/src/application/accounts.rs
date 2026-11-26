use super::{
    AppError, AppResult, ServiceContext,
    ports::{GOOGLE_SECRET, GoogleIdentity, SecretStore, account_key},
};
use crate::domain::model::{Account, AccountKind, SignatureBinding, Store};
use std::sync::Arc;

/// What a removed mailbox was: enough for the synchronization to delete its item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedAccount {
    pub provider_id: String,
    pub email: String,
}

/// Holds off contact delivery while the vault and the mail repository detach the mailbox.
pub type MailboxRetirement = fn(u32, &mut dyn FnMut() -> AppResult<()>) -> AppResult<()>;

#[derive(Clone)]
pub struct AccountService {
    context: Arc<ServiceContext>,
    identity: Arc<dyn GoogleIdentity>,
    retire_mailbox: MailboxRetirement,
}
impl AccountService {
    pub fn new(context: Arc<ServiceContext>, identity: Arc<dyn GoogleIdentity>) -> Self {
        Self { context, identity, retire_mailbox: |_, detach| detach() }
    }
    pub fn with_grant_cleanup(mut self, cleanup: MailboxRetirement) -> Self {
        self.retire_mailbox = cleanup;
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
        let primary = retirement_key(&account);
        let mut committed = false;
        let result = (self.retire_mailbox)(id, &mut || {
            self.context.update(|store| {
                let current = store.account(id).ok_or_else(|| AppError::missing("Account"))?;
                if !current.email.eq_ignore_ascii_case(&account.email) || current.resolved_provider_id() != account.resolved_provider_id() {
                    return Err(AppError::new("identity_changed", "Mailbox changed; refresh before removing it"));
                }
                // Validate the vault without changing it. Only a committed removal may delete keys.
                if !primary.is_empty() { self.context.secrets.get(&primary)?; }
                store.detach_mailbox(id, self.context.clock.now());
                store.settings.pending_mailbox_cleanup.insert(id, primary.clone());
                Ok(())
            })?;
            committed = true;
            self.clear_credentials(id)
        });
        if committed {
            self.settle_cleanup(id, result);
        } else {
            result?;
            return Err(AppError::storage("Mailbox retirement did not commit"));
        }
        Ok(removed)
    }

    fn clear_credentials(&self, id: u32) -> AppResult<()> {
        // Connecting a mailbox sets its credential under this same repository writer lock.
        self.context.repository.transaction(&mut |store| {
            clear_retired_credentials(store, id, self.context.secrets.as_ref())
        })
    }

    fn settle_cleanup(&self, id: u32, result: AppResult<()>) {
        let result = result.and_then(|()| self.context.update(|store| {
            store.settings.pending_mailbox_cleanup.remove(&id);
            Ok(())
        }));
        if let Err(error) = result {
            eprintln!("Mailbox {id} cleanup will resume after restart: {error}");
        }
    }

    pub fn retry_mailbox_cleanup(&self) -> AppResult<()> {
        let pending: Vec<_> = self.context.read_shared()?.settings.pending_mailbox_cleanup.keys().copied().collect();
        for id in pending {
            let result = (self.retire_mailbox)(id, &mut || self.clear_credentials(id));
            self.settle_cleanup(id, result);
        }
        Ok(())
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

pub(crate) fn retirement_key(account: &Account) -> String {
    if account.resolved_provider_id() == "sample" { String::new() } else { super::sync::credential_key(account) }
}

pub(crate) fn clear_retired_credentials(store: &Store, id: u32, secrets: &dyn SecretStore) -> AppResult<()> {
    let Some(primary) = store.settings.pending_mailbox_cleanup.get(&id) else { return Ok(()); };
    if store.account(id).is_some() { return Err(AppError::storage("Pending cleanup belongs to an active mailbox")); }
    if primary.is_empty() { return Ok(()); }
    // The real vault accepts remove as a no-op while locked. Leave the journal for a retry.
    secrets.get(primary)?;
    for key in [format!("calendar:{id}"), format!("contacts:{id}")] {
        secrets.remove(&key)?;
    }
    if !store.accounts.iter().any(|account| super::sync::credential_key(account) == *primary) {
        secrets.remove(primary)?;
        if let Some(email) = primary.strip_prefix("gmail:") { super::sync::forget_access_hook(email); }
    }
    Ok(())
}
