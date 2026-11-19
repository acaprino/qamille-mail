//! Compute routes of spec D2: OpenAI-compatible where a standard exists (`/v1/models`,
//! `/v1/embeddings`, `/v1/audio/transcriptions`), `/qamille/v1/*` elsewhere.
//!
//! Speech recognition sits behind [`speech::SpeechBackend`] (the real one is
//! [`speech::WhisperBackend`]); up to `parallelism` transcriptions run at once (D2: the cores).
//!
//! The inference itself sits behind [`ComputeBackend`] (the real one is [`engine::OnnxBackend`],
//! tests use a fake without a model). Handlers run it on `spawn_blocking`, one inference at a
//! time per model with a bounded queue ([`Gate`]); beyond the queue the answer is
//! `rate_limited` with `retry_after_s`.
//!
//! Errors use the closed codes of D2 in the body `{"error":{"code","message","retry_after_s"}}`.
//! The access log records method, route pattern, status, duration and request size: never
//! texts, file names, vectors or credentials (INV-2).

mod chat;
pub mod engine;
pub mod parser;
pub mod llm_models;
pub mod speech;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, MatchedPath, Multipart, Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tower_http::timeout::TimeoutLayer;

pub use qamille_compute::embedder::{DIM, QUANTIZATION};
pub use qamille_compute::models::MODEL_ID;

/// Largest JSON body (a window of 200 000 characters to cut, or a batch of passages).
pub const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
/// Largest file for extraction and image embedding (D2: 25 MiB), plus room for multipart.
pub const MAX_FILE_BYTES: usize = qamille_compute::extract::MAX_INPUT_BYTES;
const MAX_UPLOAD_BODY: usize = MAX_FILE_BYTES + 256 * 1024;
/// Largest batch of `POST /v1/embeddings`.
pub const MAX_INPUTS: usize = 64;
/// Requests waiting for a model beyond the one running.
pub const QUEUE: usize = 16;
/// Pause suggested by `rate_limited`.
pub const QUEUE_RETRY_S: u64 = 2;
/// Pause suggested when one device key already has its requests in flight (remote mode).
pub const DEVICE_RETRY_S: u64 = 1;
/// Uploads read or held at once (each up to 25 MiB): about 100 MiB of request bodies at most.
pub const UPLOADS: usize = 4;
/// An upload must arrive within this time once its slot is free (a slow client is refused
/// instead of holding a slot).
pub const UPLOAD_DEADLINE: Duration = Duration::from_secs(60);
pub const UPLOAD_TOO_SLOW: &str = "The upload did not arrive in time.";
/// Largest embedding input, in bytes, refused before the model's permit: 2048 tokens of text
/// fit well within it, the app's 256-token passages are about 1 KiB.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;
pub use qamille_compute::embedder::MAX_INPUT_TOKENS;
/// Requests in flight at once per user in remote mode, unless configured (D2, A2).
pub const DEFAULT_MAX_IN_FLIGHT_PER_USER: usize = 2;
/// Pause suggested after a request ran past its route's time limit.
pub const TIMEOUT_RETRY_S: u64 = 30;
/// A request may wait in the queue, then extraction may retry once (two reader budgets).
pub const COMPUTE_TIMEOUT: Duration = Duration::from_secs(300);
/// One transcription may wait in the queue, then take up to 10 minutes.
pub const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(900);

/// The closed error codes of D2 (plus `invalid_request` and `not_found` for malformed calls, and
/// `forbidden` for a device key on a route of the administrator).
#[derive(Debug, Clone, PartialEq)]
pub enum ComputeError {
    ModelMissing(String),
    ModelDownloading { percent: Option<u8> },
    PausedForMemory { retry_after_s: u64 },
    PausedAfterFailure { retry_after_s: u64 },
    DocumentUnreadable(String),
    PayloadTooLarge,
    Unauthorized,
    /// The device key arrived over a connection that is not secure (`400 tls_required`).
    TlsRequired,
    RateLimited { retry_after_s: u64 },
    InvalidRequest(String),
    /// A valid credential that may not call this route (a device key on a model installation).
    Forbidden(String),
    /// The user has no subscription that opens remote compute (`402`).
    SubscriptionRequired,
    /// An error of the entitlement extension with its own status and code (a quota, say): it
    /// reaches the client as it is, with the fields of `extra` inside `error`.
    Entitlement {
        status: StatusCode,
        code: &'static str,
        message: String,
        retry_after_s: Option<u64>,
        extra: Option<serde_json::Value>,
    },
    NotFound,
    /// The assistant's provider does not answer (`503 llm_unavailable`, with a pause).
    LlmUnavailable { retry_after_s: u64 },
    /// The built-in model does not fit in the free memory (`503 llm_unavailable`, with a pause).
    LlmNotEnoughMemory { retry_after_s: u64 },
    /// The built-in model is still being downloaded by the server (`503 llm_unavailable`, with a pause).
    LlmPreparing { retry_after_s: u64 },
    /// The consent of the app does not match the provider now in use (`409`).
    LlmConsentRequired,
    /// The conversation does not fit the model's context (`400 llm_context_exceeded`).
    LlmContextExceeded,
    /// The message reaches the client (it never carries the user's content) and is not logged.
    Internal(String),
}

/// Messages already known to the app (and translated in `desktop/src/lib/backend-text.ts`).
pub const PAUSED_AFTER_FAILURE: &str = "Embedding paused after a worker failure; retry in one minute";
pub const PAUSED_FOR_MEMORY: &str = "Embedding paused: the computer needs more available memory";
pub const DOCUMENT_UNREADABLE: &str = "The document reader stopped on this file; the file was skipped";
pub const PAYLOAD_TOO_LARGE: &str = "Attachment exceeds the embedding memory budget (25 MiB)";
pub const MODEL_DOWNLOADING: &str = "The semantic search model is still downloading.";
pub const RATE_LIMITED: &str = "The compute server is busy; the request will be retried shortly.";
pub const TLS_REQUIRED: &str = "This operation requires an https connection.";
pub const UNAUTHORIZED: &str = "The compute server refused the credentials of this request.";
pub const SUBSCRIPTION_REQUIRED: &str = "An active Qamille subscription is required for compute on the server.";
pub const LLM_UNAVAILABLE: &str = "The assistant is temporarily unavailable.";
/// Its translation in `desktop/src/lib/backend-text.ts` belongs to wave 5.
pub const LLM_NOT_ENOUGH_MEMORY: &str = "The assistant model needs more free memory than this server has right now.";
/// The built-in model is being downloaded by the server on its own.
pub const LLM_PREPARING: &str = "The assistant model is being prepared on the server; try again in a few minutes.";
pub const LLM_CONSENT_REQUIRED: &str = "The assistant provider has changed: the consent must be given again.";
pub const LLM_CONTEXT_EXCEEDED: &str = "The conversation is too long for the assistant.";

impl ComputeError {
    pub fn code(&self) -> &'static str {
        match self {
            ComputeError::ModelMissing(_) => "model_missing",
            ComputeError::ModelDownloading { .. } => "model_downloading",
            ComputeError::PausedForMemory { .. } => "paused_for_memory",
            ComputeError::PausedAfterFailure { .. } => "paused_after_failure",
            ComputeError::DocumentUnreadable(_) => "document_unreadable",
            ComputeError::PayloadTooLarge => "payload_too_large",
            ComputeError::Unauthorized => "unauthorized",
            ComputeError::TlsRequired => "tls_required",
            ComputeError::RateLimited { .. } => "rate_limited",
            ComputeError::InvalidRequest(_) => "invalid_request",
            ComputeError::Forbidden(_) => "forbidden",
            ComputeError::SubscriptionRequired => "subscription_required",
            ComputeError::Entitlement { code, .. } => code,
            ComputeError::NotFound => "not_found",
            ComputeError::LlmUnavailable { .. } | ComputeError::LlmNotEnoughMemory { .. } | ComputeError::LlmPreparing { .. } => "llm_unavailable",
            ComputeError::LlmConsentRequired => "llm_consent_required",
            ComputeError::LlmContextExceeded => "llm_context_exceeded",
            ComputeError::Internal(_) => "internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ComputeError::ModelMissing(_)
            | ComputeError::ModelDownloading { .. }
            | ComputeError::PausedForMemory { .. }
            | ComputeError::PausedAfterFailure { .. }
            | ComputeError::LlmUnavailable { .. }
            | ComputeError::LlmNotEnoughMemory { .. }
            | ComputeError::LlmPreparing { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ComputeError::LlmConsentRequired => StatusCode::CONFLICT,
            ComputeError::LlmContextExceeded => StatusCode::BAD_REQUEST,
            ComputeError::DocumentUnreadable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ComputeError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ComputeError::Unauthorized => StatusCode::UNAUTHORIZED,
            ComputeError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            ComputeError::InvalidRequest(_) | ComputeError::TlsRequired => StatusCode::BAD_REQUEST,
            ComputeError::Forbidden(_) => StatusCode::FORBIDDEN,
            ComputeError::SubscriptionRequired => StatusCode::PAYMENT_REQUIRED,
            ComputeError::Entitlement { status, .. } => *status,
            ComputeError::NotFound => StatusCode::NOT_FOUND,
            ComputeError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            ComputeError::ModelMissing(message)
            | ComputeError::DocumentUnreadable(message)
            | ComputeError::InvalidRequest(message)
            | ComputeError::Forbidden(message)
            | ComputeError::Entitlement { message, .. }
            | ComputeError::Internal(message) => message.clone(),
            ComputeError::ModelDownloading { .. } => MODEL_DOWNLOADING.to_owned(),
            ComputeError::PausedForMemory { .. } => PAUSED_FOR_MEMORY.to_owned(),
            ComputeError::PausedAfterFailure { .. } => PAUSED_AFTER_FAILURE.to_owned(),
            ComputeError::PayloadTooLarge => PAYLOAD_TOO_LARGE.to_owned(),
            ComputeError::Unauthorized => UNAUTHORIZED.to_owned(),
            ComputeError::TlsRequired => TLS_REQUIRED.to_owned(),
            ComputeError::SubscriptionRequired => SUBSCRIPTION_REQUIRED.to_owned(),
            ComputeError::RateLimited { .. } => RATE_LIMITED.to_owned(),
            ComputeError::NotFound => "Not found.".to_owned(),
            ComputeError::LlmUnavailable { .. } => LLM_UNAVAILABLE.to_owned(),
            ComputeError::LlmNotEnoughMemory { .. } => LLM_NOT_ENOUGH_MEMORY.to_owned(),
            ComputeError::LlmPreparing { .. } => LLM_PREPARING.to_owned(),
            ComputeError::LlmConsentRequired => LLM_CONSENT_REQUIRED.to_owned(),
            ComputeError::LlmContextExceeded => LLM_CONTEXT_EXCEEDED.to_owned(),
        }
    }

    pub fn retry_after_s(&self) -> Option<u64> {
        match self {
            ComputeError::PausedForMemory { retry_after_s }
            | ComputeError::PausedAfterFailure { retry_after_s }
            | ComputeError::RateLimited { retry_after_s }
            | ComputeError::LlmUnavailable { retry_after_s }
            | ComputeError::LlmNotEnoughMemory { retry_after_s }
            | ComputeError::LlmPreparing { retry_after_s } => Some(*retry_after_s),
            ComputeError::Entitlement { retry_after_s, .. } => *retry_after_s,
            ComputeError::ModelDownloading { .. } => Some(10),
            _ => None,
        }
    }
}

impl IntoResponse for ComputeError {
    fn into_response(self) -> Response {
        let retry_after_s = self.retry_after_s();
        let mut body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.message(),
                "retry_after_s": retry_after_s,
            }
        });
        // The extension's own fields (a quota's resource, limit, use, renewal) go inside `error`,
        // never over the closed ones.
        if let ComputeError::Entitlement { extra: Some(serde_json::Value::Object(fields)), .. } = &self
            && let Some(error) = body["error"].as_object_mut()
        {
            for (key, value) in fields {
                error.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        let mut response = (self.status(), Json(body)).into_response();
        response.extensions_mut().insert(ErrorCode(self.code()));
        if let Some(seconds) = retry_after_s
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        if matches!(self, ComputeError::Unauthorized) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

/// The code of an error response, for the access log.
#[derive(Clone, Copy)]
struct ErrorCode(&'static str);

/// State of one model as `GET /health` and `GET /qamille/v1/models/{id}` report it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelStatus {
    pub id: String,
    pub kind: String,
    /// `ready`, `loading`, `downloading` or `missing`.
    pub state: String,
    pub percent: Option<u8>,
    pub done_bytes: u64,
    pub total_bytes: u64,
    /// The last installation failure, if the model is still missing.
    pub error: Option<String>,
    /// In memory right now (it leaves after 60 s without requests).
    pub loaded: bool,
    pub dimensions: usize,
    pub quantization: String,
}

/// The inference behind the routes. Every method may block: handlers call it on
/// `spawn_blocking`.
pub trait ComputeBackend: Send + Sync + 'static {
    /// Passages of at most 256 tokens sharing 20 (D2), with the model's tokenizer.
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError>;
    /// One normalized vector of `DIM` values per already prompted input.
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError>;
    /// [`Self::embed`] and the model tokens of the inputs (the per-user counters, the OpenAI
    /// `usage`); backends that cannot count say 0.
    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        self.embed(inputs).map(|vectors| (vectors, 0))
    }
    fn embed_image(&self, bytes: &[u8]) -> Result<Vec<f32>, ComputeError>;
    fn extract(&self, name: &str, bytes: &[u8]) -> Result<String, ComputeError>;
    fn models(&self) -> Vec<ModelStatus>;
    /// Starts installing `id` and returns at once; progress is in [`Self::models`].
    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError>;
    /// The embedding model in use and its quantization label: what `/health` declares and the
    /// app records with its index (D4).
    fn embedding_identity(&self) -> (String, String) {
        (MODEL_ID.to_owned(), QUANTIZATION.to_owned())
    }
    /// Called every few seconds: frees what has been idle (the model after 60 s).
    fn release_idle(&self) {}
    /// The server stops: child processes end now (a blocking task still running may keep the
    /// backend alive past the runtime, so its drop cannot be relied on).
    fn shutdown(&self) {}
}

/// At most `permits` running requests per gate, at most `queue` waiting.
///
/// The permit is owned: the handler moves it into its blocking work, so a request whose client
/// went away keeps its slot until the work really ends. The waiting count is held by a guard,
/// so a request dropped while it waits (a timeout, a closed connection) leaves the queue.
pub struct Gate {
    running: Arc<Semaphore>,
    permits: usize,
    waiting: AtomicUsize,
    queue: usize,
}

struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Gate {
    pub fn new(queue: usize) -> Self {
        Self::with_permits(1, queue)
    }

    pub fn with_permits(permits: usize, queue: usize) -> Self {
        let permits = permits.max(1);
        Gate { running: Arc::new(Semaphore::new(permits)), permits, waiting: AtomicUsize::new(0), queue }
    }

    pub fn permits(&self) -> usize {
        self.permits
    }

    /// Requests holding a permit now.
    pub fn running(&self) -> usize {
        self.permits.saturating_sub(self.running.available_permits())
    }

    /// Requests waiting for a permit now.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    pub async fn enter(&self) -> Result<OwnedSemaphorePermit, ComputeError> {
        if let Ok(permit) = Arc::clone(&self.running).try_acquire_owned() {
            return Ok(permit);
        }
        if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.queue {
            self.waiting.fetch_sub(1, Ordering::SeqCst);
            return Err(ComputeError::RateLimited { retry_after_s: QUEUE_RETRY_S });
        }
        let _waiting = Waiting(&self.waiting);
        Arc::clone(&self.running)
            .acquire_owned()
            .await
            .map_err(|_| ComputeError::Internal("The compute server is stopping.".to_owned()))
    }
}

/// Who may call the compute routes. `serve`: the key of an active device of an active user,
/// verified like the account routes ([`crate::http::DeviceAuth`]). Models are installed only
/// on the administrative listener (A1, D2). `Fixed` is a hidden fixed credential, only for
/// the tests of the compute routes: no serving code may use it.
#[derive(Clone)]
pub enum ComputeAuth {
    #[doc(hidden)]
    Fixed([u8; 32]),
    Devices(Arc<DeviceGate>),
}

/// Who a request comes from, once its credential was verified. Kept in the request extensions
/// for the per-user counters; it never holds the credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// The fixed credential of the compute tests.
    Fixed,
    /// A device of a user of the remote server; `key_sha256` (hex) names its key without it.
    Device { user_id: String, key_sha256: String },
}

/// Failed device authentications per origin before `rate_limited` (only the database query a
/// wrong key costs is at stake: keys are 32 random bytes).
pub const DEVICE_AUTH_MAX_FAILURES: u32 = 120;
pub const DEVICE_AUTH_WINDOW: Duration = Duration::from_secs(60);

/// The remote mode's checks: device keys through the user database, a limit per origin on
/// failed keys, and at most `max_in_flight` requests at once for one **user**, whatever the
/// device (spec A2, D2: 2 by default). The administrator key is never accepted here: models are
/// installed on the administrative listener (`/admin/api/models/{id}/install`).
pub struct DeviceGate {
    accounts: Arc<crate::accounts::Accounts>,
    /// Con un `Entitlement`: senza diritto al calcolo, `402 subscription_required`.
    entitlement: Option<Arc<dyn crate::ext::Entitlement>>,
    trusted_proxies: Arc<[std::net::IpAddr]>,
    /// Addresses from which the public listener accepts plain text (`plaintext_from`), like the
    /// account routes.
    plaintext_from: Arc<[std::net::IpAddr]>,
    max_in_flight: usize,
    in_flight: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    failures: crate::limiter::FailureLimiter,
}

impl DeviceGate {
    pub fn new(accounts: Arc<crate::accounts::Accounts>, trusted_proxies: Arc<[std::net::IpAddr]>, max_in_flight: usize) -> Self {
        Self::with_failures(accounts, trusted_proxies, max_in_flight, DEVICE_AUTH_MAX_FAILURES, DEVICE_AUTH_WINDOW)
    }

    /// With another limit on failed keys (tests).
    pub fn with_failures(accounts: Arc<crate::accounts::Accounts>, trusted_proxies: Arc<[std::net::IpAddr]>, max_in_flight: usize, max_failures: u32, window: Duration) -> Self {
        DeviceGate {
            accounts,
            entitlement: None,
            trusted_proxies,
            plaintext_from: Arc::from(Vec::new()),
            max_in_flight: max_in_flight.max(1),
            in_flight: std::sync::Mutex::new(std::collections::HashMap::new()),
            failures: crate::limiter::FailureLimiter::new(max_failures, window),
        }
    }

    /// The addresses that may use plain text (`plaintext_from`), as for the account routes.
    pub fn with_plaintext_from(mut self, plaintext_from: Arc<[std::net::IpAddr]>) -> Self {
        self.plaintext_from = plaintext_from;
        self
    }

    /// The entitlement that opens remote compute (none: every active user may compute).
    pub fn with_entitlement(mut self, entitlement: Option<Arc<dyn crate::ext::Entitlement>>) -> Self {
        self.entitlement = entitlement;
        self
    }

    /// A slot among the requests in flight of this user, or `None` beyond the limit.
    fn enter(self: &Arc<Self>, user_id: &str) -> Option<InFlightSlot> {
        let mut map = self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = map.entry(user_id.to_owned()).or_insert(0);
        if *count >= self.max_in_flight {
            return None;
        }
        *count += 1;
        Some(InFlightSlot { gate: Arc::clone(self), user_id: user_id.to_owned() })
    }

    /// The origin of a request (socket, or `X-Forwarded-For` from a trusted proxy), with whether
    /// the connection is secure: the same rule as the login and the account routes.
    fn origin(&self, headers: &HeaderMap, socket: Option<std::net::SocketAddr>) -> Option<crate::origin::ClientOrigin> {
        socket.map(|socket| crate::origin::resolve_with_plaintext(socket, headers, &self.trusted_proxies, &self.plaintext_from))
    }

    /// Requests of `user_id` in flight now (tests).
    pub fn in_flight(&self, user_id: &str) -> usize {
        self.in_flight.lock().map(|map| map.get(user_id).copied().unwrap_or(0)).unwrap_or(0)
    }
}

/// One request of a user in flight; dropped when the response is ready (or the client went
/// away).
struct InFlightSlot {
    gate: Arc<DeviceGate>,
    user_id: String,
}

impl Drop for InFlightSlot {
    fn drop(&mut self) {
        let mut map = self.gate.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(count) = map.get_mut(&self.user_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.user_id);
            }
        }
    }
}

impl ComputeAuth {
    #[doc(hidden)]
    pub fn fixed(token: &str) -> Self {
        ComputeAuth::Fixed(Sha256::digest(token.as_bytes()).into())
    }

    /// The caller of a request with these headers, if its credential is valid. Remote: an origin
    /// over its limit of failed keys is refused before any query (`rate_limited`), and every
    /// failed key counts.
    async fn caller(&self, headers: &HeaderMap, socket: Option<std::net::SocketAddr>) -> Result<Option<Caller>, ComputeError> {
        let Some(token) = bearer(headers) else { return Ok(None) };
        match self {
            ComputeAuth::Fixed(expected) => {
                let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
                Ok(bool::from(digest.ct_eq(expected)).then_some(Caller::Fixed))
            }
            ComputeAuth::Devices(gate) => {
                let origin = gate.origin(headers, socket);
                // A device key is a password: it never travels over a connection that is not secure.
                if origin.is_some_and(|origin| !origin.secure) {
                    return Err(ComputeError::TlsRequired);
                }
                let ip = origin.map(|origin| origin.ip);
                if let Some(ip) = ip
                    && let Err(crate::error::ServerError::RateLimited { retry_after_s }) = gate.failures.check(ip, Instant::now())
                {
                    return Err(ComputeError::RateLimited { retry_after_s });
                }
                match gate.accounts.authenticate(token).await {
                    Ok(auth) => Ok(Some(Caller::Device { key_sha256: auth.key_sha256().to_owned(), user_id: auth.user_id })),
                    Err(_) => {
                        if let Some(ip) = ip {
                            gate.failures.record_failure(ip, Instant::now());
                        }
                        Ok(None)
                    }
                }
            }
        }
    }
}

/// The socket of a request, when the server runs with connect info.
fn socket_of(request: &Request) -> Option<std::net::SocketAddr> {
    request.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>().map(|info| info.0)
}

/// The permit of `gate` for this request, once its caller is checked again: a device revoked
/// (or a user disabled) while the request waited in the queue is dropped with `401` (EX-15).
async fn admitted(state: &ComputeState, caller: Option<&Caller>, gate: &Gate) -> Result<OwnedSemaphorePermit, ComputeError> {
    let permit = gate.enter().await?;
    if let (ComputeAuth::Devices(devices), Some(Caller::Device { key_sha256, .. })) = (&state.auth, caller)
        && !devices.accounts.device_active(key_sha256).await
    {
        return Err(ComputeError::Unauthorized);
    }
    Ok(permit)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

/// What one request of a user cost, for the per-user counters (D5): never its content.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Usage {
    pub requests: u64,
    pub audio_seconds: f64,
    pub embedded_tokens: u64,
    /// Token in entrata e in uscita dell'assistente (gateway LLM).
    pub llm_prompt_tokens: u64,
    pub llm_completion_tokens: u64,
}

impl Usage {
    pub fn new(requests: u64, audio_seconds: f64, embedded_tokens: u64) -> Self {
        Usage { requests, audio_seconds, embedded_tokens, llm_prompt_tokens: 0, llm_completion_tokens: 0 }
    }

    /// Lo stesso consumo, con i token dell'assistente.
    pub fn with_llm(mut self, prompt_tokens: u64, completion_tokens: u64) -> Usage {
        self.llm_prompt_tokens = prompt_tokens;
        self.llm_completion_tokens = completion_tokens;
        self
    }
}

/// Where the per-user counters go (the user database in remote mode).
pub trait UsageSink: Send + Sync + 'static {
    fn record(&self, user_id: &str, usage: Usage);
}

/// The cost of the request in progress, filled by the handlers and recorded once at the end.
#[derive(Clone, Default)]
struct UsageTicket(Arc<std::sync::Mutex<Usage>>);

impl UsageTicket {
    fn add(&self, change: impl FnOnce(&mut Usage)) {
        if let Ok(mut usage) = self.0.lock() {
            change(&mut usage);
        }
    }
}

#[derive(Clone)]
pub struct ComputeState {
    pub backend: Arc<dyn ComputeBackend>,
    pub speech: Arc<dyn speech::SpeechBackend>,
    /// The `llm-*` models of the built-in assistant.
    pub llm_models: Arc<dyn llm_models::LlmModels>,
    pub auth: ComputeAuth,
    /// Per-user counters (remote mode only).
    pub usage: Option<Arc<dyn UsageSink>>,
    /// The mode `/health` declares (`remote` in `serve`).
    pub mode: &'static str,
    /// The assistant's gateway (`POST /v1/chat/completions`), swapped at run time from `/admin`.
    llm: crate::llm::LlmHolder,
    /// The body limit and the time limit (seconds) of the assistant's route; `None`: no route.
    llm_route: Option<(usize, u64)>,
    /// The assistant's settings from `/admin` (`/admin/api/llm`); `None`: not on this server.
    llm_admin: Option<Arc<crate::llm::LlmAdmin>>,
    /// The embedding model chosen from `/admin` (`/admin/api/embedding`); `None`: not on this server.
    embedding_admin: Option<Arc<crate::embedding::EmbeddingAdmin>>,
    model_gate: Arc<Gate>,
    parser_gate: Arc<Gate>,
    speech_gate: Arc<Gate>,
    /// Uploads being read or held in memory (images, documents, audio), whatever route: a slow
    /// client holds one of these, never a model's permit. Per user, the in-flight limit applies.
    upload_gate: Arc<Gate>,
    upload_deadline: Duration,
}

impl ComputeState {
    pub fn new(backend: Arc<dyn ComputeBackend>, auth: ComputeAuth, mode: &'static str) -> Self {
        Self::with_queue(backend, auth, mode, QUEUE)
    }

    pub fn with_queue(backend: Arc<dyn ComputeBackend>, auth: ComputeAuth, mode: &'static str, queue: usize) -> Self {
        ComputeState {
            backend,
            speech: Arc::new(speech::NoSpeech),
            llm_models: Arc::new(llm_models::NoLlmModels),
            auth,
            usage: None,
            mode,
            llm: crate::llm::LlmHolder::default(),
            llm_route: None,
            llm_admin: None,
            embedding_admin: None,
            model_gate: Arc::new(Gate::new(queue)),
            parser_gate: Arc::new(Gate::new(queue)),
            speech_gate: Arc::new(Gate::with_permits(speech::default_parallelism(), speech::SPEECH_QUEUE)),
            upload_gate: Arc::new(Gate::with_permits(UPLOADS, queue.max(UPLOADS))),
            upload_deadline: UPLOAD_DEADLINE,
        }
    }

    /// With speech recognition: at most `parallelism` transcriptions at once.
    pub fn with_speech(self, speech: Arc<dyn speech::SpeechBackend>, parallelism: usize) -> Self {
        self.with_speech_queue(speech, parallelism, speech::SPEECH_QUEUE)
    }

    pub fn with_speech_queue(mut self, speech: Arc<dyn speech::SpeechBackend>, parallelism: usize, queue: usize) -> Self {
        self.speech = speech;
        self.speech_gate = Arc::new(Gate::with_permits(parallelism, queue));
        self
    }

    /// With the installable language models (`llm-*`): they join `/health` and the admin routes.
    pub fn with_llm_models(mut self, models: Arc<dyn llm_models::LlmModels>) -> Self {
        self.llm_models = models;
        self
    }

    /// With another deadline for uploads (tests).
    pub fn with_upload_deadline(mut self, deadline: Duration) -> Self {
        self.upload_deadline = deadline;
        self
    }

    /// With the assistant's gateway: `POST /v1/chat/completions` and its models in `/v1/models`.
    pub fn with_llm(mut self, gateway: Arc<crate::llm::LlmGateway>) -> Self {
        self.llm_route = Some((gateway.config().max_request_bytes, gateway.config().timeout_secs));
        self.llm = crate::llm::LlmHolder::default();
        self.llm.replace(Some(gateway));
        self
    }

    /// With the assistant that `/admin` can change at run time: the route is always there (`404`
    /// while no provider is set), its limits from the configuration the server started with.
    pub fn with_llm_admin(mut self, holder: crate::llm::LlmHolder, route: (usize, u64), admin: Arc<crate::llm::LlmAdmin>) -> Self {
        self.llm = holder;
        self.llm_route = Some(route);
        self.llm_admin = Some(admin);
        self
    }

    /// With the embedding model that `/admin` can change at run time (`backend` already answers
    /// with the model it chose).
    pub fn with_embedding_admin(mut self, admin: Arc<crate::embedding::EmbeddingAdmin>) -> Self {
        self.embedding_admin = Some(admin);
        self
    }

    /// The assistant's gateway in use, if a provider is set.
    pub fn llm_gateway(&self) -> Option<Arc<crate::llm::LlmGateway>> {
        self.llm.get()
    }

    /// With per-user counters.
    pub fn with_usage(mut self, usage: Arc<dyn UsageSink>) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Every model of this server: embeddings, speech and language models.
    pub fn all_models(&self) -> Vec<ModelStatus> {
        let mut models = self.backend.models();
        models.extend(self.speech.models());
        models.extend(self.llm_models.models());
        models
    }

    /// Starts installing a model (embedding, `whisper-<size>` or `llm-*`); blocking.
    pub fn install_model(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        if id.starts_with(qamille_compute::llm_catalog::ID_PREFIX) {
            self.llm_models.install(id)
        } else if id.starts_with(speech::ID_PREFIX) {
            self.speech.install(id)
        } else {
            self.backend.install(id)
        }
    }

    /// Removes a language model (`llm-*`); blocking. Other models are not removable.
    pub fn remove_model(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        if id.starts_with(qamille_compute::llm_catalog::ID_PREFIX) {
            self.llm_models.remove(id)
        } else {
            Err(ComputeError::NotFound)
        }
    }

    /// The models with what `/admin` and the command line show before an installation.
    pub fn models_with_details(&self) -> Vec<(ModelStatus, Option<llm_models::LlmDetails>)> {
        self.all_models()
            .into_iter()
            .map(|model| {
                let details = (model.kind == "llm").then(|| self.llm_models.details(&model.id)).flatten();
                (model, details)
            })
            .collect()
    }
}

/// The compute routes and their layers, without a fallback.
fn routes(state: ComputeState) -> Router<ComputeState> {
    let llm_routes = state.llm_route.map(|(body_limit, timeout_secs)| chat::routes(&state, body_limit, timeout_secs));
    let json_limit = DefaultBodyLimit::max(MAX_JSON_BYTES);
    let upload_limit = DefaultBodyLimit::max(MAX_UPLOAD_BODY);
    let timeout = |limit: Duration| -> TimeoutLayer { TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, limit) };
    let protected = Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/embeddings", post(embeddings).layer(json_limit))
        .route("/qamille/v1/chunks", post(chunks).layer(json_limit))
        .route("/qamille/v1/embeddings/image", post(embed_image).layer(upload_limit))
        .route("/qamille/v1/extract", post(extract).layer(upload_limit))
        .route("/qamille/v1/models/{id}", get(model_status))
        // The same routes under the names used before the rebrandings, for apps not yet updated.
        .route("/qemille/v1/chunks", post(chunks).layer(json_limit))
        .route("/qemille/v1/embeddings/image", post(embed_image).layer(upload_limit))
        .route("/qemille/v1/extract", post(extract).layer(upload_limit))
        .route("/qemille/v1/models/{id}", get(model_status))
        .route("/sparklone/v1/chunks", post(chunks).layer(json_limit))
        .route("/sparklone/v1/embeddings/image", post(embed_image).layer(upload_limit))
        .route("/sparklone/v1/extract", post(extract).layer(upload_limit))
        .route("/sparklone/v1/models/{id}", get(model_status))
        .layer(timeout(COMPUTE_TIMEOUT))
        // A window of a slow machine with a large model may take minutes (the old limit: 10).
        .route("/v1/audio/transcriptions", post(transcriptions).layer::<_, std::convert::Infallible>(upload_limit).layer::<_, std::convert::Infallible>(timeout(TRANSCRIPTION_TIMEOUT)))
        .route_layer(middleware::from_fn_with_state(state, require_auth));
    let mut router = Router::new()
        .route("/health", get(health).layer(timeout(COMPUTE_TIMEOUT)))
        .merge(protected);
    if let Some(llm_routes) = llm_routes {
        router = router.merge(llm_routes);
    }
    router
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .layer(middleware::from_fn(finish_response))
}

/// Router of the compute routes alone, for the tests (with [`ComputeAuth::fixed`]): `/health`
/// and the compute routes, nothing of the accounts.
#[doc(hidden)]
pub fn fixed_router(state: ComputeState) -> Router {
    routes(state.clone())
        .fallback(fallback)
        .layer(middleware::from_fn(finish_response))
        .with_state(state)
}

/// The compute routes of `serve` (remote mode), to merge into the public router: `/health`
/// answers anyone with `{status, version}` and a valid key also with models, state and
/// capabilities; every other route takes a device key ([`ComputeAuth::Devices`]). No fallback:
/// the public router has its own.
pub fn remote_router(state: ComputeState) -> Router {
    routes(state.clone()).with_state(state)
}

/// Frees idle resources every 10 seconds until the runtime stops.
pub fn spawn_idle_release(backend: Arc<dyn ComputeBackend>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        loop {
            tick.tick().await;
            let backend = Arc::clone(&backend);
            let _ = tokio::task::spawn_blocking(move || backend.release_idle()).await;
        }
    });
}

/// Who is calling and whether the user may use `resource`: the key, the limit on failed keys
/// per origin and the `Entitlement` (`402`). Shared by the compute routes and the assistant's.
async fn authorize(
    state: &ComputeState,
    headers: &HeaderMap,
    socket: Option<std::net::SocketAddr>,
    resource: crate::ext::Resource,
) -> Result<Caller, ComputeError> {
    let caller = state.auth.caller(headers, socket).await?.ok_or(ComputeError::Unauthorized)?;
    if let (ComputeAuth::Devices(gate), Caller::Device { user_id, .. }) = (&state.auth, &caller)
        && let Some(entitlement) = &gate.entitlement
    {
        match entitlement.require(user_id, resource).await {
            Ok(()) => {}
            Err(crate::error::ServerError::SubscriptionRequired) => {
                return Err(ComputeError::SubscriptionRequired);
            }
            Err(crate::error::ServerError::Extension { status, code, message, retry_after_s, extra }) => {
                return Err(ComputeError::Entitlement { status, code, message, retry_after_s, extra });
            }
            Err(error) => return Err(ComputeError::Internal(format!("{error}"))),
        }
    }
    Ok(caller)
}

async fn require_auth(State(state): State<ComputeState>, mut request: Request, next: Next) -> Response {
    // The transcription has its own resource: a spent quota of audio must not stop the search.
    let resource = if request.uri().path() == "/v1/audio/transcriptions" {
        crate::ext::Resource::Transcription
    } else {
        crate::ext::Resource::Compute
    };
    let caller = match authorize(&state, request.headers(), socket_of(&request), resource).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response(),
    };
    let mut _slot = None;
    if let (ComputeAuth::Devices(gate), Caller::Device { user_id, .. }) = (&state.auth, &caller) {
        match gate.enter(user_id) {
            Some(slot) => _slot = Some(slot),
            None => return ComputeError::RateLimited { retry_after_s: DEVICE_RETRY_S }.into_response(),
        }
    }
    let ticket = UsageTicket::default();
    request.extensions_mut().insert(caller.clone());
    request.extensions_mut().insert(ticket.clone());
    let response = next.run(request).await;
    if let (Some(sink), Caller::Device { user_id, .. }) = (&state.usage, &caller) {
        let mut usage = ticket.0.lock().map(|usage| *usage).unwrap_or_default();
        usage.requests = 1;
        sink.record(user_id, usage);
    }
    response
}

async fn fallback() -> ComputeError {
    ComputeError::NotFound
}

fn is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
}

/// Last layer: errors without a JSON body in the D2 form, `Cache-Control: no-store`, and the
/// access log (route pattern, status, duration, size; no content).
async fn finish_response(request: Request, next: Next) -> Response {
    let method: Method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "(none)".to_owned());
    let size = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let started = Instant::now();
    let mut response = next.run(request).await;
    if !is_json(&response) {
        let replacement = match response.status() {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY | StatusCode::UNSUPPORTED_MEDIA_TYPE => {
                Some(ComputeError::InvalidRequest("Invalid request.".to_owned()))
            }
            StatusCode::NOT_FOUND => Some(ComputeError::NotFound),
            StatusCode::METHOD_NOT_ALLOWED => Some(ComputeError::InvalidRequest("Method not allowed for this route.".to_owned())),
            StatusCode::PAYLOAD_TOO_LARGE => Some(ComputeError::PayloadTooLarge),
            // A request that waited too long in a busy server says nothing about its content: a
            // pause, so the message keeps its attempts (INV-3).
            StatusCode::REQUEST_TIMEOUT => Some(ComputeError::RateLimited { retry_after_s: TIMEOUT_RETRY_S }),
            _ => None,
        };
        if let Some(error) = replacement {
            let status = response.status();
            response = error.into_response();
            if status == StatusCode::METHOD_NOT_ALLOWED {
                *response.status_mut() = status;
            }
        }
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let code = response.extensions().get::<ErrorCode>().map(|code| code.0);
    tracing::info!(
        method = %method,
        route = %route,
        status = response.status().as_u16(),
        bytes = size,
        duration_ms = started.elapsed().as_millis() as u64,
        code = code.unwrap_or(""),
        "compute"
    );
    response
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, ComputeError> + Send + 'static) -> Result<T, ComputeError> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|_| Err(ComputeError::Internal("The compute task stopped unexpectedly.".to_owned())))
}

/// `work`, or `invalid_request` if the upload takes longer than `deadline` ([`UPLOAD_DEADLINE`]).
async fn within_upload_deadline<T>(deadline: Duration, work: impl std::future::Future<Output = T>) -> Result<T, ComputeError> {
    tokio::time::timeout(deadline, work).await.map_err(|_| ComputeError::InvalidRequest(UPLOAD_TOO_SLOW.to_owned()))
}

/// The `Content-Length` the client declared, if any.
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    headers.get(header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

/// The body, refused beyond `limit` bytes. A body that cannot be read (too large, or a client
/// that went away mid-body) is never answered with its content.
async fn read_limited(body: axum::body::Body, limit: usize) -> Result<Bytes, ComputeError> {
    axum::body::to_bytes(body, limit).await.map_err(|_| ComputeError::PayloadTooLarge)
}

/// No model, the server's embedding model in use (`active`), or the built-in id: the apps
/// released so far always send it, after checking the model `/health` declares against their index.
fn check_model(model: Option<&str>, active: &str) -> Result<(), ComputeError> {
    match model {
        None | Some(MODEL_ID) => Ok(()),
        Some(model) if model == active => Ok(()),
        Some(_) => Err(ComputeError::ModelMissing(format!("This server serves only {active}."))),
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ComputeError> {
    // Never echo serde's message: it can quote the body.
    serde_json::from_slice(body).map_err(|_| ComputeError::InvalidRequest("Invalid JSON body for this route.".to_owned()))
}

// --- Handlers --------------------------------------------------------------------

async fn health(State(state): State<ComputeState>, request: Request) -> Response {
    let headers = request.headers();
    let mut body = serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    });
    // A wrong key, or an origin over its limit of wrong keys, sees the anonymous answer.
    if matches!(state.auth.caller(headers, socket_of(&request)).await, Ok(Some(_))) {
        let parallelism = state.speech_gate.permits();
        let all = state.clone();
        let models = tokio::task::spawn_blocking(move || all.all_models()).await.unwrap_or_default();
        let speech_ready = models.iter().any(|model| model.id.starts_with(speech::ID_PREFIX) && model.state == "ready");
        let (embedding_model, quantization) = state.backend.embedding_identity();
        body["mode"] = serde_json::json!(state.mode);
        body["models"] = serde_json::json!(models);
        body["embedding"] = serde_json::json!({
            "model": embedding_model,
            "dimensions": DIM,
            "quantization": quantization,
        });
        body["limits"] = serde_json::json!({
            "max_file_bytes": MAX_FILE_BYTES,
            "max_text_bytes": qamille_compute::extract::MAX_TEXT_BYTES,
            "max_inputs": MAX_INPUTS,
            "queue": QUEUE,
            "transcription_parallelism": parallelism,
        });
        body["capabilities"] = serde_json::json!({
            "embeddings": true,
            "chunks": true,
            "image_embeddings": embedding_model == MODEL_ID,
            "extract": true,
            "transcriptions": speech_ready,
        });
        if let ComputeAuth::Devices(gate) = &state.auth {
            body["limits"]["max_in_flight_per_user"] = serde_json::json!(gate.max_in_flight);
        }
    }
    Json(body).into_response()
}

async fn list_models(State(state): State<ComputeState>) -> Result<Response, ComputeError> {
    let assistant = state.llm.get().map(|gateway| gateway.models()).unwrap_or_default();
    let models = blocking(move || Ok(state.all_models())).await?;
    let mut data: Vec<_> = models
        .iter()
        // The assistant models are listed below, from the gateway, once.
        .filter(|model| model.kind != "llm")
        .map(|model| serde_json::json!({"id": model.id, "object": "model", "owned_by": "qamille", "state": model.state}))
        .collect();
    data.extend(
        assistant
            .iter()
            .map(|id| serde_json::json!({"id": id, "object": "model", "owned_by": "qamille", "state": "ready"})),
    );
    Ok(Json(serde_json::json!({"object": "list", "data": data})).into_response())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EmbeddingInput {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct EmbeddingsRequest {
    #[serde(default)]
    model: Option<String>,
    input: EmbeddingInput,
    #[serde(default)]
    dimensions: Option<usize>,
}

async fn embeddings(State(state): State<ComputeState>, caller: Option<axum::Extension<Caller>>, ticket: Option<axum::Extension<UsageTicket>>, body: Bytes) -> Result<Response, ComputeError> {
    let request: EmbeddingsRequest = parse_json(&body)?;
    drop(body);
    let (active, _) = state.backend.embedding_identity();
    check_model(request.model.as_deref(), &active)?;
    if request.dimensions.is_some_and(|d| d != DIM) {
        return Err(ComputeError::InvalidRequest(format!("This server serves {DIM} dimensions.")));
    }
    let inputs = match request.input {
        EmbeddingInput::One(text) => vec![text],
        EmbeddingInput::Many(texts) => texts,
    };
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(ComputeError::InvalidRequest(format!("Send between 1 and {MAX_INPUTS} inputs.")));
    }
    // Before the model's permit: no input can be longer than the model takes (its tokens are
    // checked again under the permit, see `MAX_INPUT_TOKENS`).
    if inputs.iter().any(|input| input.len() > MAX_INPUT_BYTES) {
        return Err(ComputeError::PayloadTooLarge);
    }
    let permit = admitted(&state, caller.as_ref().map(|c| &c.0), &state.model_gate).await?;
    let backend = Arc::clone(&state.backend);
    // The model is named after the vectors are made: the one that made them, even if the
    // panel changed it while this request waited its turn.
    let ((vectors, tokens), active) = blocking(move || {
        let _permit = permit;
        Ok((backend.embed_counted(&inputs)?, backend.embedding_identity().0))
    })
    .await?;
    if let Some(axum::Extension(ticket)) = ticket {
        ticket.add(|usage| usage.embedded_tokens += tokens);
    }
    let data: Vec<_> = vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| serde_json::json!({"object": "embedding", "index": index, "embedding": embedding}))
        .collect();
    Ok(Json(serde_json::json!({
        "object": "list",
        "data": data,
        "model": active,
        "usage": {"prompt_tokens": tokens, "total_tokens": tokens},
    }))
    .into_response())
}

#[derive(Deserialize)]
struct ChunksRequest {
    #[serde(default)]
    model: Option<String>,
    text: String,
}

async fn chunks(State(state): State<ComputeState>, caller: Option<axum::Extension<Caller>>, body: Bytes) -> Result<Response, ComputeError> {
    let request: ChunksRequest = parse_json(&body)?;
    drop(body);
    check_model(request.model.as_deref(), &state.backend.embedding_identity().0)?;
    let permit = admitted(&state, caller.as_ref().map(|c| &c.0), &state.model_gate).await?;
    let backend = Arc::clone(&state.backend);
    let chunks = blocking(move || {
        let _permit = permit;
        backend.chunk(&request.text)
    })
    .await?;
    Ok(Json(serde_json::json!({"chunks": chunks})).into_response())
}

async fn embed_image(State(state): State<ComputeState>, caller: Option<axum::Extension<Caller>>, headers: HeaderMap, body: axum::body::Body) -> Result<Response, ComputeError> {
    // With another embedding model images are not searched: said before the upload is read or
    // any slot is taken, not after megabytes that nobody will use.
    if state.backend.embedding_identity().0 != MODEL_ID {
        return Err(ComputeError::DocumentUnreadable(crate::embedding::IMAGES_UNAVAILABLE.to_owned()));
    }
    if declared_length(&headers).is_some_and(|length| length > MAX_FILE_BYTES as u64) {
        return Err(ComputeError::PayloadTooLarge);
    }
    // The body is read (capped, within its deadline) before the model's permit, so a slow
    // upload holds only one of the few upload slots, never the model.
    let upload = state.upload_gate.enter().await?;
    let body = within_upload_deadline(state.upload_deadline, read_limited(body, MAX_FILE_BYTES)).await??;
    if body.is_empty() {
        return Err(ComputeError::InvalidRequest("The image is empty.".to_owned()));
    }
    let permit = admitted(&state, caller.as_ref().map(|c| &c.0), &state.model_gate).await?;
    let backend = Arc::clone(&state.backend);
    let (vector, model) = blocking(move || {
        let _permit = permit;
        let _upload = upload;
        Ok((backend.embed_image(&body)?, backend.embedding_identity().0))
    })
    .await?;
    Ok(Json(serde_json::json!({"model": model, "embedding": vector})).into_response())
}

async fn extract(State(state): State<ComputeState>, caller: Option<axum::Extension<Caller>>, headers: HeaderMap, mut multipart: Multipart) -> Result<Response, ComputeError> {
    let invalid = || ComputeError::InvalidRequest("Send a multipart body with `name` and `file`.".to_owned());
    if declared_length(&headers).is_some_and(|length| length > MAX_UPLOAD_BODY as u64) {
        return Err(ComputeError::PayloadTooLarge);
    }
    // The body first (an upload slot, its deadline), then the parser's permit.
    let upload = state.upload_gate.enter().await?;
    let read = async {
        let mut name: Option<String> = None;
        let mut file: Option<(Option<String>, Bytes)> = None;
        while let Some(field) = multipart.next_field().await.map_err(|error| {
            if error.status() == StatusCode::PAYLOAD_TOO_LARGE { ComputeError::PayloadTooLarge } else { invalid() }
        })? {
            match field.name() {
                Some("name") => name = Some(field.text().await.map_err(|_| invalid())?),
                Some("file") => {
                    let file_name = field.file_name().map(str::to_owned);
                    let bytes = field.bytes().await.map_err(|error| {
                        if error.status() == StatusCode::PAYLOAD_TOO_LARGE { ComputeError::PayloadTooLarge } else { invalid() }
                    })?;
                    file = Some((file_name, bytes));
                }
                _ => {}
            }
        }
        Ok::<_, ComputeError>((name, file))
    };
    let (name, file) = within_upload_deadline(state.upload_deadline, read).await??;
    let (file_name, bytes) = file.ok_or_else(invalid)?;
    let name = name.or(file_name).ok_or_else(invalid)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ComputeError::PayloadTooLarge);
    }
    let permit = admitted(&state, caller.as_ref().map(|c| &c.0), &state.parser_gate).await?;
    let backend = Arc::clone(&state.backend);
    let text = blocking(move || {
        let _permit = permit;
        let _upload = upload;
        backend.extract(&name, &bytes)
    })
    .await?;
    Ok(Json(serde_json::json!({"text": text})).into_response())
}

async fn model_status(State(state): State<ComputeState>, Path(id): Path<String>) -> Result<Response, ComputeError> {
    let models = blocking(move || Ok(state.all_models())).await?;
    let model = models
        .into_iter()
        .find(|model| model.id == id)
        .ok_or_else(|| ComputeError::ModelMissing(format!("This server serves only {MODEL_ID}.")))?;
    Ok(Json(model).into_response())
}

/// Starts installing a model (embedding or `whisper-<size>`).
async fn install(state: &ComputeState, id: String) -> Result<ModelStatus, ComputeError> {
    let state = state.clone();
    blocking(move || state.install_model(&id)).await
}

/// Model routes of the **administrative** listener in remote mode (D2: models are the
/// administrator's), behind [`crate::http::AdminAuth`]: TLS (or loopback), the limit of failed
/// keys per origin, the same `401` for any wrong key. `GET /admin/api/models`,
/// `POST /admin/api/models/{id}/install`.
pub fn admin_routes(state: ComputeState) -> Router<crate::http::AppState> {
    Router::new()
        .route("/admin/api/models", get(admin_models))
        .route("/admin/api/models/{id}/install", post(admin_install))
        .route("/admin/api/models/{id}", axum::routing::delete(admin_remove))
        .route("/admin/api/llm", get(admin_llm).put(admin_llm_save).delete(admin_llm_reset))
        .route("/admin/api/embedding", get(admin_embedding).put(admin_embedding_save).delete(admin_embedding_reset))
        .layer(axum::Extension(state))
}

fn embedding_admin(state: &ComputeState) -> Result<Arc<crate::embedding::EmbeddingAdmin>, crate::error::ServerError> {
    state.embedding_admin.clone().ok_or_else(crate::error::ServerError::not_found)
}

/// `GET /admin/api/embedding`: the embedding model saved and the one in use; of the service's key
/// only its last four characters.
async fn admin_embedding(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>) -> Result<Response, crate::error::ServerError> {
    let admin = embedding_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.view()).await.map_err(crate::error::ServerError::internal)?;
    Ok(Json(view).into_response())
}

/// `PUT /admin/api/embedding`: tries the service with one call, then saves it in the data folder
/// and applies it at once. A service whose vectors are not of the index's length changes nothing.
async fn admin_embedding_save(
    _admin: crate::http::AdminAuth,
    axum::Extension(state): axum::Extension<ComputeState>,
    crate::http::JsonBody(input): crate::http::JsonBody<crate::embedding::EmbeddingSettingsInput>,
) -> Result<Response, crate::error::ServerError> {
    let admin = embedding_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.update(input))
        .await
        .map_err(crate::error::ServerError::internal)?
        .map_err(crate::error::ServerError::InvalidRequest)?;
    Ok(Json(view).into_response())
}

/// `DELETE /admin/api/embedding`: back to the local model.
async fn admin_embedding_reset(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>) -> Result<Response, crate::error::ServerError> {
    let admin = embedding_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.reset())
        .await
        .map_err(crate::error::ServerError::internal)?
        .map_err(crate::error::ServerError::InvalidRequest)?;
    Ok(Json(view).into_response())
}

fn llm_admin(state: &ComputeState) -> Result<Arc<crate::llm::LlmAdmin>, crate::error::ServerError> {
    state.llm_admin.clone().ok_or_else(crate::error::ServerError::not_found)
}

/// `GET /admin/api/llm`: the assistant's settings, where they come from and what runs now; of the
/// provider's key only where it comes from and its last four characters.
async fn admin_llm(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>) -> Result<Response, crate::error::ServerError> {
    let admin = llm_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.view()).await.map_err(crate::error::ServerError::internal)?;
    Ok(Json(view).into_response())
}

/// `PUT /admin/api/llm`: saves the settings in the data folder and applies them at once (a local
/// model this server did not start with: at the next start, `restart_required`).
async fn admin_llm_save(
    _admin: crate::http::AdminAuth,
    axum::Extension(state): axum::Extension<ComputeState>,
    crate::http::JsonBody(input): crate::http::JsonBody<crate::llm::LlmSettingsInput>,
) -> Result<Response, crate::error::ServerError> {
    let admin = llm_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.update(input))
        .await
        .map_err(crate::error::ServerError::internal)?
        .map_err(|error| crate::error::ServerError::InvalidRequest(error.to_string()))?;
    Ok(Json(view).into_response())
}

/// `DELETE /admin/api/llm`: drops the saved settings; the configuration (file and environment) applies again.
async fn admin_llm_reset(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>) -> Result<Response, crate::error::ServerError> {
    let admin = llm_admin(&state)?;
    let view = tokio::task::spawn_blocking(move || admin.reset())
        .await
        .map_err(crate::error::ServerError::internal)?
        .map_err(|error| crate::error::ServerError::InvalidRequest(error.to_string()))?;
    Ok(Json(view).into_response())
}

fn admin_error(error: ComputeError) -> crate::error::ServerError {
    match error {
        ComputeError::Internal(detail) => crate::error::ServerError::internal(detail),
        ComputeError::NotFound => crate::error::ServerError::not_found(),
        other => crate::error::ServerError::InvalidRequest(other.message()),
    }
}

async fn admin_models(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>) -> Result<Response, crate::error::ServerError> {
    let (models, host) = blocking(move || Ok((state.models_with_details(), state.llm_models.host()))).await.map_err(admin_error)?;
    let models: Vec<serde_json::Value> = models
        .into_iter()
        .map(|(model, details)| {
            let mut value = serde_json::json!(model);
            if let Some(details) = details {
                value["llm"] = serde_json::json!(details);
            }
            value
        })
        .collect();
    Ok(Json(serde_json::json!({"models": models, "host": host})).into_response())
}

async fn admin_remove(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>, Path(id): Path<String>) -> Result<Response, crate::error::ServerError> {
    let status = blocking(move || state.remove_model(&id)).await.map_err(admin_error)?;
    Ok(Json(status).into_response())
}

async fn admin_install(_admin: crate::http::AdminAuth, axum::Extension(state): axum::Extension<ComputeState>, Path(id): Path<String>) -> Result<Response, crate::error::ServerError> {
    let status = install(&state, id).await.map_err(admin_error)?;
    let code = if status.state == "ready" { StatusCode::OK } else { StatusCode::ACCEPTED };
    Ok((code, Json(status)).into_response())
}

/// `POST /v1/audio/transcriptions`: multipart `file` (WAV), `model` (`whisper-<size>`),
/// `language` (a code or `auto`, optional), `response_format` (`json` by default as in the
/// OpenAI API, `verbose_json` with segments in seconds, or `text`).
async fn transcriptions(State(state): State<ComputeState>, caller: Option<axum::Extension<Caller>>, ticket: Option<axum::Extension<UsageTicket>>, headers: HeaderMap, mut multipart: Multipart) -> Result<Response, ComputeError> {
    let invalid = || ComputeError::InvalidRequest("Send a multipart body with `file` and `model`.".to_owned());
    if declared_length(&headers).is_some_and(|length| length > MAX_UPLOAD_BODY as u64) {
        return Err(ComputeError::PayloadTooLarge);
    }
    // The body first (an upload slot, its deadline), then the recognizer's permit.
    let upload = state.upload_gate.enter().await?;
    let too_large = |error: axum::extract::multipart::MultipartError| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE { ComputeError::PayloadTooLarge } else { invalid() }
    };
    let read = async {
        let (mut file, mut model, mut language, mut format) = (None, None, None, None);
        while let Some(field) = multipart.next_field().await.map_err(too_large)? {
            match field.name() {
                Some("file") => file = Some(field.bytes().await.map_err(too_large)?),
                Some("model") => model = Some(field.text().await.map_err(|_| invalid())?),
                Some("language") => language = Some(field.text().await.map_err(|_| invalid())?),
                Some("response_format") => format = Some(field.text().await.map_err(|_| invalid())?),
                // `prompt`, `temperature`, `timestamp_granularities[]`: accepted, not used.
                _ => {}
            }
        }
        Ok::<_, ComputeError>((file, model, language, format))
    };
    let (file, model, language, format) = within_upload_deadline(state.upload_deadline, read).await??;
    let audio = file.filter(|bytes| !bytes.is_empty()).ok_or_else(invalid)?;
    if audio.len() > MAX_FILE_BYTES {
        return Err(ComputeError::PayloadTooLarge);
    }
    if !qamille_compute::speech::looks_like_wav(&audio) {
        return Err(ComputeError::DocumentUnreadable("The audio is not a WAV file.".to_owned()));
    }
    let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty()).ok_or_else(invalid)?;
    let language = language.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).unwrap_or_else(|| "auto".to_owned());
    if !qamille_compute::speech::valid_language(&language) {
        return Err(ComputeError::InvalidRequest("Invalid transcription language.".to_owned()));
    }
    let format = format.map(|f| f.trim().to_owned()).unwrap_or_else(|| "json".to_owned());
    if !matches!(format.as_str(), "json" | "verbose_json" | "text") {
        return Err(ComputeError::InvalidRequest("Unsupported response_format: use json, verbose_json or text.".to_owned()));
    }
    // The duration the quota charges comes from the samples really present, not from the
    // header's byte rate; a header that gives none is refused before any quota or whisper.
    let duration = qamille_compute::speech::wav_seconds(&audio).ok_or_else(|| ComputeError::InvalidRequest("The WAV header is incomplete or unsupported: use PCM audio with a `fmt ` chunk before `data`.".to_owned()))?;
    if let Some(axum::Extension(ticket)) = &ticket {
        ticket.add(|usage| usage.audio_seconds += duration);
    }
    let permit = admitted(&state, caller.as_ref().map(|c| &c.0), &state.speech_gate).await?;
    let input = speech::TranscriptionInput { audio: audio.to_vec(), model, language: language.clone(), threads: speech::threads(&state.speech_gate) };
    drop(audio);
    // The client going away drops this future: the flag stops the recognizer.
    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = speech::CancelOnDrop(Arc::clone(&cancel));
    let backend = Arc::clone(&state.speech);
    let segments = blocking(move || {
        let _permit = permit;
        let _upload = upload;
        backend.transcribe(&input, &cancel)
    })
    .await?;
    let text = segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join(" ");
    Ok(match format.as_str() {
        "text" => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response(),
        "json" => Json(serde_json::json!({"text": text})).into_response(),
        _ => {
            let rows: Vec<_> = segments
                .iter()
                .enumerate()
                .map(|(id, segment)| {
                    serde_json::json!({
                        "id": id, "seek": 0,
                        "start": segment.start_ms as f64 / 1000.0, "end": segment.end_ms as f64 / 1000.0,
                        "text": segment.text, "tokens": [], "temperature": 0.0,
                        "avg_logprob": 0.0, "compression_ratio": 0.0, "no_speech_prob": 0.0,
                    })
                })
                .collect();
                        Json(serde_json::json!({"task": "transcribe", "language": language, "duration": duration, "text": text, "segments": rows})).into_response()
        }
    })
}

/// For tests and the CLI: a vector of `DIM` with unit length.
pub fn is_unit(vector: &[f32]) -> bool {
    vector.len() == DIM && (vector.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn a_request_past_its_time_limit_is_a_pause_not_a_failure() {
        // The TimeoutLayer answers 408 without a body; the client must see a transient code, so
        // a wait in a busy queue costs the message no attempt and skips no attachment (INV-3).
        let router: Router = Router::new()
            .route("/slow", get(|| async { StatusCode::REQUEST_TIMEOUT }))
            .layer(middleware::from_fn(finish_response));
        let response = router.oneshot(Request::builder().uri("/slow").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "rate_limited");
        assert_eq!(body["error"]["retry_after_s"], TIMEOUT_RETRY_S);
    }
}
