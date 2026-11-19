use super::{AppError, AppResult, events::AppEvent, ports::*};
use crate::domain::model::{Account, Settings, Store};
use std::sync::Arc;

pub struct ServiceContext {
    pub repository: Arc<dyn StoreRepository>,
    pub mail: Arc<dyn MailGateway>,
    pub secrets: Arc<dyn SecretStore>,
    pub clock: Arc<dyn Clock>,
    pub attachments: Arc<dyn AttachmentGateway>,
    pub events: Arc<dyn EventSink>,
    /// Every inference (spec D3). In the app it routes each call to the endpoint the factory
    /// chooses at that moment (local server, or the remote server of the session).
    pub compute: Arc<dyn super::compute::ComputeGateway>,
}
impl ServiceContext {
    /// The compute gateway of this moment: a job that starts now keeps it to the end (D3).
    pub fn compute_now(&self) -> Arc<dyn super::compute::ComputeGateway> {
        super::compute::pinned(&self.compute)
    }
    pub fn cleanup_mail_cache(&self) -> AppResult<()> {
        self.repository.reconcile_storage().map_err(|error| {
            AppError::new(
                "cleanup_pending",
                format!(
                    "Changes are saved. Local cleanup will retry: {}",
                    error.message
                ),
            )
        })
    }
    /// A copy the caller may change.
    pub fn read(&self) -> AppResult<Store> {
        self.repository.read()
    }
    /// The committed store, shared: no copy when nothing was written since.
    pub fn read_shared(&self) -> AppResult<Arc<Store>> {
        self.repository.read_shared()
    }
    pub fn update<T>(&self, operation: impl FnOnce(&mut Store) -> AppResult<T>) -> AppResult<T> {
        self.run_update(None, operation)
    }
    /// `update` for an operation that changes no message except those whose id
    /// is in `emails` (see `StoreRepository::transaction_on_emails`): a
    /// single-message action then neither copies nor compares the mailbox.
    /// Debug builds panic when the operation changes another message.
    pub fn update_emails<T>(
        &self,
        emails: &[u64],
        operation: impl FnOnce(&mut Store) -> AppResult<T>,
    ) -> AppResult<T> {
        self.run_update(Some(emails), operation)
    }
    fn run_update<T>(
        &self,
        emails: Option<&[u64]>,
        operation: impl FnOnce(&mut Store) -> AppResult<T>,
    ) -> AppResult<T> {
        // Maintenance (reconcile_storage, cache cleanup) is a background job of
        // its own — `application::maintenance::MaintenanceService`, ADR-0007
        // decision 2. No user write runs it in line, neither before nor after
        // the transaction; a damaged optional index is quarantined by the next
        // maintenance pass instead.
        let mut op = Some(operation);
        let mut result = None;
        let mut attempt = |store: &mut Store| {
            let Some(operation) = op.take() else {
                // The repository retried after an attempt whose operation had
                // already run met a busy database at its commit. A `FnOnce`
                // cannot be replayed on the fresh snapshot, and writing the
                // stale one would lose a concurrent commit: refuse the retry.
                // Nothing was stored; the caller may run the change again.
                return Err(AppError::storage(
                    "Another process held the mail database while this change was committing; \
                     nothing was saved",
                ));
            };
            result = Some(operation(store)?);
            Ok(())
        };
        match emails {
            Some(emails) => self.repository.transaction_on_emails(emails, &mut attempt)?,
            None => self.repository.transaction(&mut attempt)?,
        }
        result.ok_or_else(|| AppError::storage("Repository did not execute the transaction"))
    }
    /// Like `update`, and the only way a writer announces UI events: the closure
    /// returns the events its change produced, and they are emitted in that order
    /// only after the repository committed. A failed operation or commit emits
    /// nothing; events are never emitted from inside the transaction.
    pub fn update_with_events<T>(
        &self,
        operation: impl FnOnce(&mut Store) -> AppResult<(T, Vec<AppEvent>)>,
    ) -> AppResult<T> {
        let (value, events) = self.update(operation)?;
        self.emit_all(events);
        Ok(value)
    }
    /// `update_with_events` for an operation that changes no message except
    /// those in `emails` (see `update_emails`).
    pub fn update_emails_with_events<T>(
        &self,
        emails: &[u64],
        operation: impl FnOnce(&mut Store) -> AppResult<(T, Vec<AppEvent>)>,
    ) -> AppResult<T> {
        let (value, events) = self.update_emails(emails, operation)?;
        self.emit_all(events);
        Ok(value)
    }
    fn emit_all(&self, events: Vec<AppEvent>) {
        for event in events {
            // A change event without ids announces nothing and would make the
            // UI read for nothing: drop it, preserving the order of the rest.
            if event.is_empty_change() {
                continue;
            }
            self.events.emit(event);
        }
    }
    pub fn account(&self, store: &Store, id: u32) -> AppResult<Account> {
        let mut account = store
            .account(id)
            .cloned()
            .ok_or_else(|| AppError::missing("Account"))?;
        if account.resolved_provider_id() != "sample" {
            let key=if account.resolved_provider_id()=="gmail"{account_key(&account.email)}else{provider_account_key(account.resolved_provider_id(),account.id)};
            account.password = self.secrets.get(&key)?;
            if account.password.is_empty() {
                return Err(AppError::new(
                    "sign_in_required",
                    "Reconnect this mail account in Settings",
                ));
            }
        }
        Ok(account)
    }
    pub fn settings(&self, store: &Store) -> AppResult<Settings> {
        let mut settings = store.settings.clone();
        settings.ai_key = self.secrets.get(AI_KEY)?;
        settings.google_client_secret = self.secrets.get(GOOGLE_SECRET)?;
        Ok(settings)
    }
}
