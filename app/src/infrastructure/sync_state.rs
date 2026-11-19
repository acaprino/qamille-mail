//! What this computer remembers of the mailbox synchronization: `mailbox-sync.json` in the data
//! folder, written atomically. It holds ids, versions, times and HMACs, never a credential. The save
//! before the last one stays in `mailbox-sync.json.bak`: a damaged file gives that one back (the
//! switches of the person survive, and an old copy only costs a merge), and with neither the next
//! pass starts over (nothing is lost, the server holds it).
use crate::application::{
    AppError, AppResult,
    sync::{GoogleClients, SyncState, SyncStateStore},
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const FILE_NAME: &str = "mailbox-sync.json";

pub struct FileSyncState {
    path: PathBuf,
}

impl FileSyncState {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join(FILE_NAME) }
    }
    fn backup(&self) -> PathBuf {
        self.path.with_extension("json.bak")
    }
    /// `Ok(None)` when there is no file, or it is damaged; an error when it exists and cannot be
    /// read (an empty answer would let the next save erase what it holds).
    fn read(path: &Path) -> AppResult<Option<SyncState>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text).ok()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(AppError::storage("The synchronization state could not be read")),
        }
    }
}

/// A name no other save uses at the same time (two saves never share a temporary file).
fn unique(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    path.with_extension(format!("json.{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

impl SyncStateStore for FileSyncState {
    fn load(&self) -> AppResult<SyncState> {
        if let Some(state) = Self::read(&self.path)? {
            return Ok(state);
        }
        Ok(Self::read(&self.backup()).ok().flatten().unwrap_or_default())
    }
    fn save(&self, state: &SyncState) -> AppResult<()> {
        let text = serde_json::to_string(state).map_err(|_| AppError::storage("The synchronization state could not be saved"))?;
        let temporary = unique(&self.path);
        std::fs::write(&temporary, text).map_err(|_| AppError::storage("The synchronization state could not be saved"))?;
        // The file in place is good (it loads): it becomes the previous save.
        if matches!(Self::read(&self.path), Ok(Some(_))) {
            let _ = std::fs::copy(&self.path, self.backup());
        }
        std::fs::rename(&temporary, &self.path).map_err(|_| {
            let _ = std::fs::remove_file(&temporary);
            AppError::storage("The synchronization state could not be saved")
        })
    }
}

/// The Google client of this build: its secret is baked in, so it never travels in an item.
pub struct AppGoogleClients;
impl GoogleClients for AppGoogleClients {
    fn is_app_client(&self, client_id: &str) -> bool {
        super::google_oauth_config::is_app_client(client_id)
    }
}

/// Revokes the token at Google (`oauth::REVOKE_URL`).
pub struct GoogleRevoker;
impl crate::application::sync::TokenRevoker for GoogleRevoker {
    fn revoke(&self, token: &str) -> AppResult<()> {
        super::oauth::revoke(super::oauth::REVOKE_URL, token).map_err(|_| AppError::new("revoke_failed", "Google did not revoke the access"))
    }
}
