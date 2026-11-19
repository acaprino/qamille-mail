//! The compute port (spec D3): every inference of the app (passages, vectors of texts and
//! images, document text) goes through [`ComputeGateway`], implemented over HTTP towards
//! `qamille-server` (the local child on `127.0.0.1`, or a remote server). The app keeps the
//! search itself: BM25, scores, fusion and the index files.
//!
//! Errors carry the closed codes of D2 in `AppError::code`. [`is_transient`] tells the pauses
//! that say nothing about the content (the message is retried and never loses an attempt,
//! INV-3) from the failures of that content (`document_unreadable`, `payload_too_large`).
use super::AppResult;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The embedding model of the index (D4).
pub const EMBEDDING_MODEL_ID: &str = "embeddinggemma-2";
/// Vector length served and stored (D2).
pub const EMBEDDING_DIMENSIONS: usize = 256;
/// Passages per embedding request: one request per batch, cancellation between batches.
pub const EMBEDDING_BATCH: usize = 16;
/// Identity of the local server (INV-8): the same at every launch, whatever its port.
pub const LOCAL_IDENTITY: &str = "local";
/// Prefix of the speech models of the server (`whisper-base`).
pub const SPEECH_MODEL_PREFIX: &str = "whisper-";
/// Quantization of the local model's files (D4: recorded, never blocking).
pub const EMBEDDING_QUANTIZATION: &str = "q4f16";

/// A `401` from the remote server after sign-in (A8, EX-15): the session stays; the person signs
/// in again from Settings → Compute. Translated in `desktop/src/lib/backend-text.ts`.
pub const UNLINKED_MESSAGE: &str = "This device is no longer connected to the server";
/// Search returned word matches only (D8). Translated in `backend-text.ts`.
pub const SEMANTIC_UNAVAILABLE_MESSAGE: &str = "Search by meaning is not available: showing word matches only";
/// The words of A11 for an assistant (MCP results are not translated by the interface).
pub const MCP_NOT_CONNECTED_MESSAGE: &str = "ricerca per significato non disponibile: Qamille non è collegato al server, accedi dall'app";
/// The MCP form of [`SEMANTIC_UNAVAILABLE_MESSAGE`] for every other cause.
pub const MCP_SEMANTIC_UNAVAILABLE_MESSAGE: &str = "ricerca per significato non disponibile";
/// D4: the server's embedding model differs from the index's. Translated in `backend-text.ts`.
pub const MODEL_MISMATCH_MESSAGE: &str = "The server uses a different model from the index: reindex to search by meaning again";
/// Remote mode without a usable session (signed out, or no consent for its address): nothing is
/// sent anywhere. Translated in `backend-text.ts`.
pub const NOT_CONNECTED_MESSAGE: &str = "Not connected to the Qamille server";
/// Said once per session when the server answers `402`.
pub const SUBSCRIPTION_REQUIRED_MESSAGE: &str = "Your Qamille subscription is not active: compute on the server is paused. Subscribe in Settings → Compute, or continue without a server.";

/// What a pause for a used-up quota says, by the resource the server names (`compute`,
/// `transcription` or `llm`). Fixed texts: the interface translates them.
pub fn quota_exceeded_message(resource: &str) -> &'static str {
    match resource {
        "transcription" => "The transcription minutes of your plan are used up for this month: transcription on the server is paused until they renew.",
        "llm" => "The assistant tokens of your plan are used up for this month: the assistant on the server is paused until they renew.",
        _ => "The search tokens of your plan are used up for this month: indexing and semantic search on the server are paused until they renew.",
    }
}

pub mod codes {
    pub const MODEL_MISSING: &str = "model_missing";
    pub const MODEL_DOWNLOADING: &str = "model_downloading";
    pub const PAUSED_FOR_MEMORY: &str = "paused_for_memory";
    pub const PAUSED_AFTER_FAILURE: &str = "paused_after_failure";
    pub const DOCUMENT_UNREADABLE: &str = "document_unreadable";
    pub const PAYLOAD_TOO_LARGE: &str = "payload_too_large";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const INTERNAL: &str = "internal";
    /// Client side: no HTTP answer (connection refused or dropped, timeout, server not started).
    pub const UNREACHABLE: &str = "compute_unreachable";
    /// Client side: server mode without a session (or mode `Off`), or without consent for the session's address
    /// (INV-10). No request was sent. A pause: signing in clears it.
    pub const NOT_CONNECTED: &str = "compute_not_connected";
    /// Client side (D4): the server's embedding model or dimensions differ from the index's.
    pub const MODEL_MISMATCH: &str = "model_mismatch";
    /// A valid credential that may not call that route (a device key on a model installation).
    pub const FORBIDDEN: &str = "forbidden";
    /// `402`: the account has no subscription that opens remote compute (spec «Vendita
    /// Professional», V5). A pause: subscribing, or going back to this computer, clears it.
    pub const SUBSCRIPTION_REQUIRED: &str = "subscription_required";
    /// `429` with this code (wave 9): the plan's quota of the month is used up. A pause until the
    /// month renews, not a busy server: no wait and no second attempt.
    pub const QUOTA_EXCEEDED: &str = "quota_exceeded";
}

/// The pauses of D2: model not there yet, memory, a recent failure, a full queue, or no
/// answer at all. `unauthorized` is not one of them (A8).
pub fn is_transient(code: &str) -> bool {
    matches!(
        code,
        codes::MODEL_MISSING
            | codes::MODEL_DOWNLOADING
            | codes::PAUSED_FOR_MEMORY
            | codes::PAUSED_AFTER_FAILURE
            | codes::RATE_LIMITED
            | codes::UNREACHABLE
            | codes::NOT_CONNECTED
            | codes::SUBSCRIPTION_REQUIRED
            | codes::QUOTA_EXCEEDED
    )
}

/// What an MCP search says when its results are words only (A11): not connected (no session,
/// no consent, a device the server refused) or any other cause.
pub fn mcp_semantic_notice(code: &str) -> &'static str {
    match code {
        codes::UNAUTHORIZED | codes::NOT_CONNECTED | "compute_unauthorized" => MCP_NOT_CONNECTED_MESSAGE,
        _ => MCP_SEMANTIC_UNAVAILABLE_MESSAGE,
    }
}

/// The model that made a set of vectors (D4): files of passages record it, `/health` declares it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingIdentity {
    pub model: String,
    pub dimensions: usize,
    #[serde(default)]
    pub quantization: String,
}

impl EmbeddingIdentity {
    /// The model of the local mode (and of passage files older than D4).
    pub fn local() -> Self {
        EmbeddingIdentity { model: EMBEDDING_MODEL_ID.into(), dimensions: EMBEDDING_DIMENSIONS, quantization: EMBEDDING_QUANTIZATION.into() }
    }
    /// Vectors of the two can be compared: same model and dimensions (quantization is only
    /// recorded, D4).
    pub fn same_vectors(&self, other: &EmbeddingIdentity) -> bool {
        self.model == other.model && self.dimensions == other.dimensions
    }
}

/// One model as the server declares it in `GET /health`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputeModel {
    pub id: String,
    /// `ready`, `loading`, `downloading` or `missing`.
    pub state: String,
    #[serde(default)]
    pub percent: Option<u8>,
    #[serde(default)]
    pub done_bytes: u64,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub dimensions: usize,
    #[serde(default)]
    pub quantization: String,
}

impl ComputeModel {
    /// Installed on the server (loaded or not).
    pub fn ready(&self) -> bool {
        matches!(self.state.as_str(), "ready" | "loading")
    }
}

/// One recognized segment, in milliseconds from the start of the audio sent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeechSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// What `GET /health` says to a caller with valid credentials.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputeHealth {
    pub version: String,
    pub models: Vec<ComputeModel>,
    /// The embedding model the server serves (`/health` → `embedding`); `None` from a server
    /// that does not say, which counts as the local model.
    #[serde(default)]
    pub embedding: Option<EmbeddingIdentity>,
}

impl ComputeHealth {
    pub fn model(&self, id: &str) -> Option<&ComputeModel> {
        self.models.iter().find(|model| model.id == id)
    }
    /// The embedding model the server serves.
    pub fn embedding_identity(&self) -> EmbeddingIdentity {
        self.embedding.clone().unwrap_or_else(EmbeddingIdentity::local)
    }
    /// The server's embedding model is installed there.
    pub fn embedding_ready(&self) -> bool {
        let identity = self.embedding_identity();
        self.model(&identity.model).is_some_and(ComputeModel::ready)
    }
}

pub trait ComputeGateway: Send + Sync {
    /// Passages of at most 256 model tokens sharing 20 (D2).
    fn chunk_text(&self, text: &str) -> AppResult<Vec<String>>;
    fn embed_query(&self, text: &str) -> AppResult<Vec<f32>>;
    fn embed_document(&self, title: &str, text: &str) -> AppResult<Vec<f32>>;
    /// The passages of one document in one request (D2: "input a lotti").
    fn embed_documents(&self, title: &str, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        texts.iter().map(|text| self.embed_document(title, text)).collect()
    }
    fn embed_image(&self, bytes: &[u8]) -> AppResult<Vec<f32>>;
    /// Text of a document (PDF, Office, …); at most 25 MiB in, 4 MiB out.
    fn extract_document(&self, name: &str, bytes: &[u8]) -> AppResult<String>;
    fn health(&self) -> AppResult<ComputeHealth>;
    /// One WAV window through `POST /v1/audio/transcriptions` (`verbose_json`) with the speech
    /// model `model` (`whisper-base`) and a language code or `auto`. Returns at once with the
    /// code `cancelled` when `cancel` is set.
    fn transcribe_window(&self, wav: &[u8], model: &str, language: &str, cancel: &AtomicBool) -> AppResult<Vec<SpeechSegment>>;
    /// `local`, or the remote server's address: stable across launches (INV-8).
    fn identity(&self) -> String;
    /// A gateway that routes each call to the endpoint chosen at that moment (the factory of
    /// spec D3) gives the gateway chosen now, so a job ends where it began; a gateway towards one
    /// endpoint gives `None` (it is that endpoint).
    fn resolve(&self) -> Option<Arc<dyn ComputeGateway>> {
        None
    }
    /// The endpoint is a remote server (D7: consent and `authorize()` apply, models are the
    /// administrator's).
    fn is_remote(&self) -> bool {
        self.identity() != LOCAL_IDENTITY
    }
}

/// A gateway that serves nothing: every call fails with `compute_not_connected`, no request is
/// made. For services and tests that make no inference.
pub struct NoCompute;

impl NoCompute {
    fn fail<T>() -> AppResult<T> {
        Err(super::AppError::new(codes::NOT_CONNECTED, NOT_CONNECTED_MESSAGE))
    }
}

impl ComputeGateway for NoCompute {
    fn chunk_text(&self, _: &str) -> AppResult<Vec<String>> {
        Self::fail()
    }
    fn embed_query(&self, _: &str) -> AppResult<Vec<f32>> {
        Self::fail()
    }
    fn embed_document(&self, _: &str, _: &str) -> AppResult<Vec<f32>> {
        Self::fail()
    }
    fn embed_image(&self, _: &[u8]) -> AppResult<Vec<f32>> {
        Self::fail()
    }
    fn extract_document(&self, _: &str, _: &[u8]) -> AppResult<String> {
        Self::fail()
    }
    fn health(&self) -> AppResult<ComputeHealth> {
        Self::fail()
    }
    fn transcribe_window(&self, _: &[u8], _: &str, _: &str, _: &AtomicBool) -> AppResult<Vec<SpeechSegment>> {
        Self::fail()
    }
    fn identity(&self) -> String {
        LOCAL_IDENTITY.into()
    }
}

/// The gateway of this moment: `gateway` itself, or the one it routes to now.
pub fn pinned(gateway: &Arc<dyn ComputeGateway>) -> Arc<dyn ComputeGateway> {
    gateway.resolve().unwrap_or_else(|| gateway.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pauses_are_transient_and_content_failures_are_not() {
        for code in [codes::MODEL_MISSING, codes::MODEL_DOWNLOADING, codes::PAUSED_FOR_MEMORY, codes::PAUSED_AFTER_FAILURE, codes::RATE_LIMITED, codes::UNREACHABLE, codes::NOT_CONNECTED, codes::SUBSCRIPTION_REQUIRED, codes::QUOTA_EXCEEDED] {
            assert!(is_transient(code), "{code}");
        }
        for code in [codes::DOCUMENT_UNREADABLE, codes::PAYLOAD_TOO_LARGE, codes::UNAUTHORIZED, codes::INTERNAL, codes::MODEL_MISMATCH, codes::FORBIDDEN, "external"] {
            assert!(!is_transient(code), "{code}");
        }
    }
}
