//! Ports for the contacts feature: local storage and the Google People gateway.
use super::{AppError, AppResult};
use crate::domain::{
    contacts::{ContactFields, ContactsState},
    model::Account,
};

pub trait ContactStore: Send + Sync {
    fn read(&self) -> AppResult<ContactsState>;
    /// One atomic read-modify-write. Only a successful operation commits.
    fn transaction(&self, operation: &mut dyn FnMut(&mut ContactsState) -> AppResult<()>) -> AppResult<()>;
    /// Held while queued Google writes are sent, so two processes never send the same write twice.
    fn lock_flush(&self) -> AppResult<Box<dyn Send>> {
        Ok(Box::new(()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteContact {
    pub remote_id: String,
    pub etag: String,
    pub fields: ContactFields,
    /// Provider data this app does not manage, returned unchanged on update.
    pub unmanaged: serde_json::Value,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemotePage {
    pub items: Vec<RemoteContact>,
    pub next_page: Option<String>,
    pub next_sync_token: Option<String>,
}

pub trait ContactGateway: Send + Sync {
    /// `etag_conflict` marks a stale etag on update; `sync_token_expired` marks an expired token on list.
    fn list(&self, account: &Account, other: bool, page: Option<&str>, sync_token: Option<&str>) -> AppResult<RemotePage>;
    fn create(&self, account: &Account, fields: &ContactFields) -> AppResult<RemoteContact>;
    fn update(&self, account: &Account, remote_id: &str, etag: &str, fields: &ContactFields, unmanaged: &serde_json::Value) -> AppResult<RemoteContact>;
    fn delete(&self, account: &Account, remote_id: &str) -> AppResult<()>;
    /// One card as the server has it now; used to resolve a stale etag.
    fn get(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact>;
    fn promote_other(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact>;
    /// Opens the consent flow for read or write access.
    fn authorize(&self, account: &Account, write: bool) -> AppResult<()>;
}

/// Gateway for builds and tests without a remote provider: every call is unsupported.
pub struct NullContactGateway;
fn unsupported<T>() -> AppResult<T> {
    Err(AppError::new("unsupported_capability", "Remote contacts are unavailable"))
}
impl ContactGateway for NullContactGateway {
    fn list(&self, _: &Account, _: bool, _: Option<&str>, _: Option<&str>) -> AppResult<RemotePage> {
        unsupported()
    }
    fn create(&self, _: &Account, _: &ContactFields) -> AppResult<RemoteContact> {
        unsupported()
    }
    fn update(&self, _: &Account, _: &str, _: &str, _: &ContactFields, _: &serde_json::Value) -> AppResult<RemoteContact> {
        unsupported()
    }
    fn delete(&self, _: &Account, _: &str) -> AppResult<()> {
        unsupported()
    }
    fn get(&self, _: &Account, _: &str) -> AppResult<RemoteContact> {
        unsupported()
    }
    fn promote_other(&self, _: &Account, _: &str) -> AppResult<RemoteContact> {
        unsupported()
    }
    fn authorize(&self, _: &Account, _: bool) -> AppResult<()> {
        unsupported()
    }
}
