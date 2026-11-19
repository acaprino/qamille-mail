//! The first start after the local compute mode (wave 2, compito 5): the app no longer runs a
//! compute server or models on the PC. A server left running by an older version is stopped,
//! its files go, and the person is told once, with the choice of freeing the space of the
//! models (never on its own).
use super::{AppError, AppResult, ServiceContext};
use serde::Serialize;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

/// The guide for the server on the PC with Docker (the public repository of the future).
pub const SERVER_GUIDE_URL: &str = "https://github.com/acaprino/qamille-mail/blob/main/docs/server-sul-pc.md";

pub trait LegacyLocalPort: Send + Sync {
    /// Stops the leftover local server, only if it answers `GET /health` with the recorded
    /// token; deletes `compute-server.json` and `compute-server.lock`. True if there was a record.
    fn stop_leftover(&self) -> bool;
    /// Bytes of `data/models` and `data/speech-models`.
    fn models_bytes(&self) -> u64;
    /// Deletes the two folders; the bytes freed.
    fn free_models(&self) -> AppResult<u64>;
    /// Deletes `qamille-server(.exe)` next to the executable, if there and not in use.
    fn remove_old_program(&self);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LegacyLocalStatus {
    pub show: bool,
    pub models_bytes: u64,
    pub had_server: bool,
    pub guide_url: &'static str,
}

pub struct LegacyLocalService {
    context: Arc<ServiceContext>,
    port: Arc<dyn LegacyLocalPort>,
    had_server: AtomicBool,
}

impl LegacyLocalService {
    pub fn new(context: Arc<ServiceContext>, port: Arc<dyn LegacyLocalPort>) -> Self {
        Self { context, port, had_server: AtomicBool::new(false) }
    }
    /// At start: stops, cleans and works out the notice. `show` is
    /// `(had_server || models_bytes > 0) && !legacy_local_notice_dismissed`.
    pub fn startup(&self) -> AppResult<LegacyLocalStatus> {
        let had_server = self.port.stop_leftover();
        self.port.remove_old_program();
        self.had_server.store(had_server, Ordering::SeqCst);
        self.status()
    }
    pub fn status(&self) -> AppResult<LegacyLocalStatus> {
        let models_bytes = self.port.models_bytes();
        let had_server = self.had_server.load(Ordering::SeqCst);
        let dismissed = self.context.read_shared()?.settings.legacy_local_notice_dismissed;
        Ok(LegacyLocalStatus { show: (had_server || models_bytes > 0) && !dismissed, models_bytes, had_server, guide_url: SERVER_GUIDE_URL })
    }
    /// Never on its own: without `confirmed` nothing is deleted.
    pub fn free_space(&self, confirmed: bool) -> AppResult<u64> {
        if !confirmed {
            return Err(AppError::validation("confirmation_required"));
        }
        self.port.free_models()
    }
    pub fn dismiss(&self) -> AppResult<()> {
        self.context.update(|store| {
            store.settings.legacy_local_notice_dismissed = true;
            Ok(())
        })
    }
}
