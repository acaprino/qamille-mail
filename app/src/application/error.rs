use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}
pub type AppResult<T> = Result<T, AppError>;

/// Codes of the errors of the credentials vault (`infrastructure::secret`, phase 3, Task 7): the
/// interface words each in its own language.
pub mod vault_codes {
    /// The key belongs to another Windows user or computer, and no transfer passphrase opens it.
    pub const FOREIGN: &str = "vault_foreign";
    /// The key belongs to another Windows user or computer; the transfer passphrase opens it.
    pub const LOCKED: &str = "vault_locked";
    pub const DAMAGED: &str = "vault_damaged";
    /// Windows or the memory cannot open the key right now: try again, never start over.
    pub const UNAVAILABLE: &str = "vault_unavailable";
    /// A file of the vault cannot be read or written right now.
    pub const STORAGE: &str = "vault_storage";
    pub const WRONG_PASSPHRASE: &str = "vault_wrong_passphrase";
    pub const NO_TRANSFER: &str = "vault_no_transfer";
    pub const NEEDS_UNLOCK: &str = "vault_needs_unlock";
    /// The transfer passphrase is too short or too long.
    pub const PASSPHRASE: &str = "vault_passphrase";
    pub const NOTHING_TO_START_OVER: &str = "vault_nothing_to_start_over";
    pub const NEEDS_DPAPI: &str = "vault_needs_dpapi";
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("validation", message)
    }
    pub fn missing(kind: &str) -> Self {
        Self::new("not_found", format!("{kind} not found"))
    }
    pub fn external(message: impl Into<String>) -> Self {
        Self::new("external", message)
    }
    pub fn storage(message: impl Into<String>) -> Self {
        Self::new("storage", message)
    }
    /// The credentials vault refused (`vault_codes`): the credentials do not open on this Windows
    /// user, or cannot be read right now. It answers before any provider is contacted, so the work
    /// that needed them can wait for the vault instead of failing (scheduler, undo queue).
    pub fn is_vault(&self) -> bool {
        self.code.starts_with("vault_")
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for AppError {}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::storage(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::storage(error.to_string())
    }
}
