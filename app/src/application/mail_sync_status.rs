//! Transient sync state shared by manual downloads and the background worker.
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Syncing,
    Complete,
    Error,
}

#[derive(Clone, Serialize)]
pub struct AccountSyncStatus {
    pub account_id: u32,
    pub phase: SyncPhase,
    pub added: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
pub struct MailSyncStatus {
    pub revision: u64,
    pub accounts: Vec<AccountSyncStatus>,
}

#[derive(Clone, Default)]
pub(super) struct SyncProgress {
    status: Arc<Mutex<MailSyncStatus>>,
}

impl SyncProgress {
    pub fn snapshot(&self) -> MailSyncStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The caller emits the typed `SyncProgress` event after this returns, so a
    /// snapshot pulled on that event already holds the errors it announces.
    pub fn update(&self, account_id: u32, phase: SyncPhase, added: usize, errors: Vec<String>) {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        status.revision += 1;
        status.accounts.retain(|account| account.account_id != account_id);
        status.accounts.push(AccountSyncStatus { account_id, phase, added, errors });
    }
}
