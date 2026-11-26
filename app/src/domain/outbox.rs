//! Durable delivery records shared by every host. A claimed delivery is never retried blindly.
use super::model::Draft;
use serde::{Deserialize, Serialize};
/// Callers supply stable managed attachment references, such as
/// `workspace:/outbox-files/<snapshot>/report.pdf`, rather than machine paths.
pub fn message_id(draft: &Draft) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "<qamille-{:x}@qamille.local>",
        Sha256::digest(serde_json::to_vec(draft).unwrap_or_default())
    )
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Queued,
    Scheduled,
    Sending,
    Sent,
    SentNotSaved,
    Unknown,
    Failed,
    Cancelled,
    NeedsReview,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub id: u64,
    pub key: String,
    pub request_hash: String,
    #[serde(default)]
    pub receipt_message_id: Option<String>,
    pub draft: Draft,
    #[serde(default)]
    pub plain: String,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub attachment_bytes: u64,
    pub status: DeliveryStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub send_at: i64,
    #[serde(default)]
    pub requested_send_at: Option<i64>,
    pub error: Option<String>,
    pub owner_pid: Option<u32>,
    #[serde(default)]
    pub requires_mcp_send: bool,
    #[serde(default)]
    pub source_profile: Option<String>,
}

impl Delivery {
    /// Keep the identity needed for idempotent replay, while releasing completed message payloads.
    pub fn compact(&mut self, message_id: String) {
        self.receipt_message_id = Some(message_id);
        self.draft = Draft {
            id: self.draft.id,
            account_id: self.draft.account_id,
            updated: self.draft.updated,
            request_id: std::mem::take(&mut self.draft.request_id),
            ..Default::default()
        };
        self.plain.clear();
        self.html = None;
        self.sender_name.clear();
        self.attachment_bytes = 0;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ProviderCapabilities {
    pub provider_id: String,
    pub sync: bool,
    pub remote_actions: bool,
    pub remote_search: bool,
    pub remote_drafts: bool,
    pub native_threads: bool,
    pub labels: bool,
    pub send: bool,
    pub max_attachment_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteDraft {
    pub id: String,
    pub revision: String,
    pub raw: String,
    pub thread_id: Option<String>,
    pub draft: Draft,
}
