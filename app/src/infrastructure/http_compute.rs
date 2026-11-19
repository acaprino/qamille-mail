//! `HttpCompute`: the only production implementation of [`ComputeGateway`] (spec D3), towards the
//! server of the session (INV-41: the app runs no compute server of its own).
//!
//! Over HTTP it keeps the semantics of the former frame protocol (D2, INV-3):
//! - the closed error codes of the server become `AppError::code`;
//! - no HTTP answer (connection refused or dropped, timeout) is `compute_unreachable`, a pause;
//!   the endpoint is looked up again (the supervisor restarts a dead local server) and an
//!   idempotent request is sent once more;
//! - on the content routes (extraction, image) a request that drops is repeated once after
//!   `/health` answers again; a second drop on the same document makes it
//!   `document_unreadable`; a drop with `/health` still silent stays a pause;
//! - a request that drops the connection twice while the server answers again in between (it
//!   crashed on that request and was restarted) fails as `internal`: the message spends one
//!   attempt instead of pausing the indexing for ever;
//! - a `401` makes the endpoint be looked up again once (a restarted local server has a new
//!   token in the discovery file);
//! - vectors are always renormalized and refused unless they have 256 values;
//! - a transcription window waits on its own thread, so a cancelled job returns at once.
//!
//! The prompts of the model card (`task: search result | query: …`, `title: … | text: …`) are
//! added here, by the client (D2).
use crate::application::compute::{ComputeGateway, ComputeHealth, EMBEDDING_BATCH, EMBEDDING_DIMENSIONS, EMBEDDING_MODEL_ID, SpeechSegment, codes};
use crate::application::{AppError, AppResult};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where a compute server answers and the credential for it.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `http://127.0.0.1:<port>` or the remote server's address, without a trailing `/`.
    pub base_url: String,
    pub token: String,
    /// `local` or the remote address (INV-8).
    pub identity: String,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint").field("base_url", &self.base_url).field("identity", &self.identity).finish_non_exhaustive()
    }
}

/// Provides the endpoint of each request: the session's address, or a fixed one.
pub trait EndpointSource: Send + Sync {
    fn endpoint(&self) -> AppResult<Endpoint>;
    /// `endpoint` gave no HTTP answer: find (or start) a server again next time.
    fn invalidate(&self, endpoint: &Endpoint);
    fn identity(&self) -> String;
    /// `endpoint` answered `401`: whether to send the request once more. The local server may
    /// have been restarted with a new token (look it up again); a remote session may have been
    /// replaced in the vault meanwhile (A11).
    fn renew(&self, endpoint: &Endpoint) -> bool {
        self.invalidate(endpoint);
        true
    }
    /// The `401` stands: the error the caller gets (the remote source marks the device unlinked).
    fn refused(&self, _endpoint: &Endpoint, error: AppError) -> AppError {
        error
    }
    /// A `402`: the account has no subscription for remote compute (the remote source says so
    /// once). A pause, like the others: the request is not blamed.
    fn unpaid(&self, _endpoint: &Endpoint, error: AppError) -> AppError {
        error
    }
    /// A `429 quota_exceeded`: the plan's quota of the month is used up (the remote source says so
    /// once per resource). A pause until the month renews: no wait, no second attempt.
    fn quota_used_up(&self, _endpoint: &Endpoint, error: AppError) -> AppError {
        error
    }
}

/// One address that never changes (a server started by a test, later the remote session).
pub struct FixedEndpoint(pub Endpoint);

impl EndpointSource for FixedEndpoint {
    fn endpoint(&self) -> AppResult<Endpoint> {
        Ok(self.0.clone())
    }
    fn invalidate(&self, _: &Endpoint) {}
    fn identity(&self) -> String {
        self.0.identity.clone()
    }
}

/// Largest answer read: 4 MiB of extracted text, JSON-escaped.
const MAX_ANSWER: u64 = 16 * 1024 * 1024;
const HEALTH_CACHE: Duration = Duration::from_secs(1);
/// A window may wait in the server's queue, then take up to 10 minutes (server: 900 s).
const TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(960);
/// Largest WAV window sent: 15 s of 16 kHz mono 16-bit is about 480 KB.
const MAX_WINDOW_BYTES: usize = 25 * 1024 * 1024;

pub const UNREACHABLE_MESSAGE: &str = "The compute server is not responding; the request will be retried.";
pub const DOCUMENT_UNREADABLE_MESSAGE: &str = "The document reader stopped on this file; the file was skipped";
pub const REQUEST_CRASHED_MESSAGE: &str = "The compute server stopped twice on this request; it will be tried again later.";

enum Failure {
    /// The server answered with an error.
    Answer(AppError, Option<u64>),
    /// No HTTP answer (the detail, an address or an OS error, is not kept).
    Transport,
    /// 404: the route (or the model it names) does not exist on this server.
    NotFound(AppError),
}

/// Prefix of the compute routes, and the one servers used before the rebranding.
const ROUTE_PREFIX: &str = "/qamille/v1/";
const LEGACY_ROUTE_PREFIX: &str = "/sparklone/v1/";

enum Body<'a> {
    Empty,
    Json(Value),
    Bytes { content_type: String, data: &'a [u8] },
}

type HealthCache = Arc<Mutex<Option<(Instant, ComputeHealth)>>>;

/// The request rules of D2, shared by [`HttpCompute`] and the threads of its transcriptions.
#[derive(Clone)]
struct Client {
    source: Arc<dyn EndpointSource>,
    agent: ureq::Agent,
    health_agent: ureq::Agent,
    health: HealthCache,
}

pub struct HttpCompute {
    client: Client,
    transcription_agent: ureq::Agent,
    /// One `/health` request at a time: concurrent misses of the cache wait for it.
    health_fetch: Mutex<()>,
}

/// With `tls`: the operating system's verifier, or the session's certificate authority (D3).
fn agent_with(global: Duration, tls: Option<&ureq::tls::TlsConfig>) -> ureq::Agent {
    let builder = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(global))
        // A redirect would carry the request (and its credential) to another address.
        .max_redirects(0);
    match tls {
        Some(tls) => builder.tls_config(tls.clone()).build().into(),
        None => builder.build().into(),
    }
}

/// The `&'static` form of a server code (unknown codes become `external`).
fn static_code(code: &str, status: u16) -> &'static str {
    match code {
        codes::MODEL_MISSING => codes::MODEL_MISSING,
        codes::MODEL_DOWNLOADING => codes::MODEL_DOWNLOADING,
        codes::PAUSED_FOR_MEMORY => codes::PAUSED_FOR_MEMORY,
        codes::PAUSED_AFTER_FAILURE => codes::PAUSED_AFTER_FAILURE,
        codes::DOCUMENT_UNREADABLE => codes::DOCUMENT_UNREADABLE,
        codes::PAYLOAD_TOO_LARGE => codes::PAYLOAD_TOO_LARGE,
        codes::UNAUTHORIZED => codes::UNAUTHORIZED,
        codes::RATE_LIMITED => codes::RATE_LIMITED,
        codes::INTERNAL => codes::INTERNAL,
        codes::SUBSCRIPTION_REQUIRED => codes::SUBSCRIPTION_REQUIRED,
        codes::QUOTA_EXCEEDED => codes::QUOTA_EXCEEDED,
        _ => match status {
            401 => codes::UNAUTHORIZED,
            402 => codes::SUBSCRIPTION_REQUIRED,
            413 => codes::PAYLOAD_TOO_LARGE,
            429 => codes::RATE_LIMITED,
            500..=599 => codes::INTERNAL,
            _ => "external",
        },
    }
}

/// No HTTP answer. The transport detail (an address, an OS error) stays out of the message.
fn unreachable() -> AppError {
    AppError::new(codes::UNREACHABLE, UNREACHABLE_MESSAGE)
}

/// 256 values, finite, renormalized (D2: the client always renormalizes).
fn vector(value: &Value) -> AppResult<Vec<f32>> {
    let values = value.as_array().ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned no vector."))?;
    if values.len() != EMBEDDING_DIMENSIONS {
        return Err(AppError::new(codes::INTERNAL, format!("The compute server returned {} values, expected {EMBEDDING_DIMENSIONS}.", values.len())));
    }
    let mut out = Vec::with_capacity(EMBEDDING_DIMENSIONS);
    for value in values {
        let value = value.as_f64().map(|v| v as f32).filter(|v| v.is_finite()).ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned an invalid vector."))?;
        out.push(value);
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= 1e-9 {
        return Err(AppError::new(codes::INTERNAL, "The compute server returned an invalid vector."));
    }
    out.iter_mut().for_each(|x| *x /= norm);
    Ok(out)
}

fn multipart(name: &str, data: &[u8]) -> (String, Vec<u8>) {
    let mut seed = [0u8; 12];
    let _ = getrandom::fill(&mut seed);
    let boundary = format!("qamille-{}", seed.iter().map(|b| format!("{b:02x}")).collect::<String>());
    // The name travels in its own field; quotes and line ends would break the header.
    let safe: String = name.chars().map(|c| if c.is_control() || c == '"' { '_' } else { c }).collect();
    let mut body = Vec::with_capacity(data.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{safe}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"document\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Client {
    fn send(&self, agent: &ureq::Agent, endpoint: &Endpoint, method: &str, path: &str, body: &Body) -> Result<Value, Failure> {
        let url = format!("{}{path}", endpoint.base_url);
        let auth = format!("Bearer {}", endpoint.token);
        let result = match (method, body) {
            ("GET", _) => agent.get(&url).header("Authorization", &auth).call(),
            (_, Body::Empty) => agent.post(&url).header("Authorization", &auth).send_empty(),
            (_, Body::Json(value)) => agent.post(&url).header("Authorization", &auth).header("Content-Type", "application/json").send(serde_json::to_vec(value).unwrap_or_default().as_slice()),
            (_, Body::Bytes { content_type, data }) => agent.post(&url).header("Authorization", &auth).header("Content-Type", content_type.as_str()).send(*data),
        };
        let mut response = match result {
            Ok(response) => response,
            Err(_) => return Err(Failure::Transport),
        };
        let status = response.status().as_u16();
        let bytes = response.body_mut().with_config().limit(MAX_ANSWER).read_to_vec().map_err(|_| Failure::Transport)?;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(value);
        }
        let error = &value["error"];
        let retry_after = |value: &Value| value["error"]["retry_after_s"].as_u64();
        let code = static_code(error["code"].as_str().unwrap_or(""), status);
        let message = if code == codes::QUOTA_EXCEEDED {
            // The server's sentence is in Italian and carries a date: the app says it with a fixed text.
            crate::application::compute::quota_exceeded_message(error["resource"].as_str().unwrap_or_default()).to_owned()
        } else {
            error["message"].as_str().filter(|m| !m.is_empty()).map(str::to_owned).unwrap_or_else(|| format!("The compute server answered {status}."))
        };
        let error = AppError::new(code, message);
        if status == 404 {
            return Err(Failure::NotFound(error));
        }
        Err(Failure::Answer(error, retry_after(&value)))
    }

    /// One request with the retry rules of D2, on `agent`.
    fn call(&self, agent: &ureq::Agent, method: &str, path: &str, body: Body, content: bool) -> AppResult<Value> {
        let mut transport_failures = 0;
        let mut busy = 0;
        let mut rediscovered = false;
        let mut path = path.to_owned();
        loop {
            let endpoint = self.source.endpoint()?;
            match self.send(agent, &endpoint, method, &path, &body) {
                Ok(value) => return Ok(value),
                // A server not yet updated after the rebranding knows the routes by their old name.
                Err(Failure::NotFound(error)) => match path.strip_prefix(ROUTE_PREFIX) {
                    Some(rest) => path = format!("{LEGACY_ROUTE_PREFIX}{rest}"),
                    None => return Err(error),
                },
                // A full queue clears in seconds: wait for it a few times before pausing.
                Err(Failure::Answer(error, retry)) if error.code == codes::RATE_LIMITED && busy < 3 && retry.unwrap_or(1) <= 5 => {
                    busy += 1;
                    std::thread::sleep(Duration::from_secs(retry.unwrap_or(1).max(1)));
                }
                // The local server may have been restarted with a new token, or the remote session
                // replaced in the vault: once more if the source says so.
                Err(Failure::Answer(error, _)) if error.code == codes::UNAUTHORIZED => {
                    if rediscovered || !self.source.renew(&endpoint) {
                        return Err(self.source.refused(&endpoint, error));
                    }
                    rediscovered = true;
                    self.forget_health();
                }
                Err(Failure::Answer(error, _)) if error.code == codes::SUBSCRIPTION_REQUIRED => {
                    return Err(self.source.unpaid(&endpoint, error));
                }
                Err(Failure::Answer(error, _)) if error.code == codes::QUOTA_EXCEEDED => {
                    return Err(self.source.quota_used_up(&endpoint, error));
                }
                Err(Failure::Answer(error, _)) => return Err(error),
                Err(Failure::Transport) => {
                    self.source.invalidate(&endpoint);
                    self.forget_health();
                    transport_failures += 1;
                    if transport_failures < 2 && !content {
                        continue;
                    }
                    // The server must be back before the request is blamed.
                    if !self.server_answers() {
                        return Err(unreachable());
                    }
                    if transport_failures >= 2 {
                        return Err(if content {
                            AppError::new(codes::DOCUMENT_UNREADABLE, DOCUMENT_UNREADABLE_MESSAGE)
                        } else {
                            // Not a pause: a request that brings the server down would loop for ever.
                            AppError::new(codes::INTERNAL, REQUEST_CRASHED_MESSAGE)
                        });
                    }
                }
            }
        }
    }

    fn server_answers(&self) -> bool {
        let Ok(endpoint) = self.source.endpoint() else { return false };
        matches!(self.send(&self.health_agent, &endpoint, "GET", "/health", &Body::Empty), Ok(value) if value["models"].is_array())
    }

    fn forget_health(&self) {
        if let Ok(mut cached) = self.health.lock() {
            *cached = None;
        }
    }
}

impl HttpCompute {
    pub fn new(source: Arc<dyn EndpointSource>) -> Self {
        Self::with_tls(source, None)
    }

    /// Towards a remote server: TLS with the system's verifier, or with `ca_pem` (D3, `tls.rs`).
    pub fn remote(source: Arc<dyn EndpointSource>, ca_pem: Option<&str>) -> AppResult<Self> {
        Ok(Self::with_tls(source, Some(super::tls::config(ca_pem)?)))
    }

    fn with_tls(source: Arc<dyn EndpointSource>, tls: Option<ureq::tls::TlsConfig>) -> Self {
        let tls = tls.as_ref();
        HttpCompute {
            client: Client {
                source,
                // A request may wait in the server's queue and a reader may take its budget.
                agent: agent_with(Duration::from_secs(330), tls),
                health_agent: agent_with(Duration::from_secs(5), tls),
                health: Arc::new(Mutex::new(None)),
            },
            transcription_agent: agent_with(TRANSCRIPTION_TIMEOUT, tls),
            health_fetch: Mutex::new(()),
        }
    }

    fn call(&self, method: &str, path: &str, body: Body, content: bool) -> AppResult<Value> {
        self.client.call(&self.client.agent, method, path, body, content)
    }

    fn cached_health(&self) -> Option<ComputeHealth> {
        let cached = self.client.health.lock().ok()?;
        cached.as_ref().filter(|(at, _)| at.elapsed() < HEALTH_CACHE).map(|(_, health)| health.clone())
    }

    fn embed_inputs(&self, inputs: Vec<String>) -> AppResult<Vec<Vec<f32>>> {
        let count = inputs.len();
        let answer = self.call("POST", "/v1/embeddings", Body::Json(json!({"model": EMBEDDING_MODEL_ID, "input": inputs, "dimensions": EMBEDDING_DIMENSIONS})), false)?;
        let data = answer["data"].as_array().ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned no vectors."))?;
        if data.len() != count {
            return Err(AppError::new(codes::INTERNAL, "The compute server returned a different number of vectors."));
        }
        let mut ordered: Vec<Option<Vec<f32>>> = vec![None; count];
        for (position, item) in data.iter().enumerate() {
            let index = item["index"].as_u64().map_or(position, |i| i as usize);
            let slot = ordered.get_mut(index).ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned a different number of vectors."))?;
            *slot = Some(vector(&item["embedding"])?);
        }
        ordered.into_iter().map(|v| v.ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned a different number of vectors."))).collect()
    }
}

/// The multipart body of `/v1/audio/transcriptions`, as the OpenAI API takes it.
fn transcription_body(wav: &[u8], model: &str, language: &str) -> (String, Vec<u8>) {
    let mut seed = [0u8; 12];
    let _ = getrandom::fill(&mut seed);
    let boundary = format!("qamille-{}", seed.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let mut body = Vec::with_capacity(wav.len() + 1024);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    };
    field("model", model);
    field("response_format", "verbose_json");
    if language != "auto" {
        field("language", language);
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// `verbose_json` segments (seconds) in milliseconds; a bare `text` covers the whole audio.
fn speech_segments(answer: &Value, wav_ms: u64) -> AppResult<Vec<SpeechSegment>> {
    let seconds = |value: &Value| (value.as_f64().unwrap_or(0.0).max(0.0) * 1000.0) as u64;
    if let Some(rows) = answer["segments"].as_array() {
        return Ok(rows
            .iter()
            .map(|row| SpeechSegment {
                start_ms: seconds(&row["start"]),
                end_ms: if row["end"].is_number() { seconds(&row["end"]) } else { wav_ms },
                text: row["text"].as_str().unwrap_or("").trim().to_owned(),
            })
            .collect());
    }
    match answer["text"].as_str() {
        Some(text) => Ok(vec![SpeechSegment { start_ms: 0, end_ms: wav_ms, text: text.trim().to_owned() }]),
        None => Err(AppError::new(codes::INTERNAL, "The compute server returned no transcription.")),
    }
}

/// Milliseconds of a 16 kHz mono 16-bit WAV (the windows the app sends).
fn wav_ms(wav: &[u8]) -> u64 {
    (wav.len().saturating_sub(44) as u64) * 1000 / 32_000
}

fn document_prompt(title: &str, text: &str) -> String {
    let t = if title.trim().is_empty() { "none" } else { title.trim() };
    format!("title: {t} | text: {text}")
}

impl ComputeGateway for HttpCompute {
    fn chunk_text(&self, text: &str) -> AppResult<Vec<String>> {
        let answer = self.call("POST", "/qamille/v1/chunks", Body::Json(json!({"model": EMBEDDING_MODEL_ID, "title": "", "text": text})), false)?;
        let chunks = answer["chunks"].as_array().ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned no passages."))?;
        Ok(chunks.iter().filter_map(|chunk| chunk.as_str().map(str::to_owned)).collect())
    }

    fn embed_query(&self, text: &str) -> AppResult<Vec<f32>> {
        Ok(self.embed_inputs(vec![format!("task: search result | query: {text}")])?.remove(0))
    }

    fn embed_document(&self, title: &str, text: &str) -> AppResult<Vec<f32>> {
        Ok(self.embed_inputs(vec![document_prompt(title, text)])?.remove(0))
    }

    fn embed_documents(&self, title: &str, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        // Callers already send batches of this size; a longer list is cut the same way.
        for batch in texts.chunks(EMBEDDING_BATCH) {
            out.extend(self.embed_inputs(batch.iter().map(|text| document_prompt(title, text)).collect())?);
        }
        Ok(out)
    }

    fn embed_image(&self, bytes: &[u8]) -> AppResult<Vec<f32>> {
        let answer = self.call("POST", "/qamille/v1/embeddings/image", Body::Bytes { content_type: "application/octet-stream".into(), data: bytes }, true)?;
        vector(&answer["embedding"])
    }

    fn extract_document(&self, name: &str, bytes: &[u8]) -> AppResult<String> {
        let (content_type, body) = multipart(name, bytes);
        let answer = self.call("POST", "/qamille/v1/extract", Body::Bytes { content_type, data: &body }, true)?;
        answer["text"].as_str().map(str::to_owned).ok_or_else(|| AppError::new(codes::INTERNAL, "The compute server returned no text."))
    }

    fn health(&self) -> AppResult<ComputeHealth> {
        if let Some(health) = self.cached_health() {
            return Ok(health);
        }
        let _single = self.health_fetch.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(health) = self.cached_health() {
            return Ok(health);
        }
        let mut answer = self.client.call(&self.client.health_agent, "GET", "/health", Body::Empty, false)?;
        if !answer["models"].is_array() {
            // The credential was not recognized: a restarted local server, or a replaced remote
            // session. Once more if the source says so.
            let endpoint = self.client.source.endpoint()?;
            let refused = || AppError::new(codes::UNAUTHORIZED, "The compute server refused the credentials of this request.");
            if !self.client.source.renew(&endpoint) {
                return Err(self.client.source.refused(&endpoint, refused()));
            }
            answer = self.client.call(&self.client.health_agent, "GET", "/health", Body::Empty, false)?;
            if !answer["models"].is_array() {
                let endpoint = self.client.source.endpoint()?;
                return Err(self.client.source.refused(&endpoint, refused()));
            }
        }
        let health = ComputeHealth {
            version: answer["version"].as_str().unwrap_or_default().to_owned(),
            models: serde_json::from_value(answer["models"].clone()).map_err(|_| AppError::new(codes::INTERNAL, "The compute server returned an unreadable status."))?,
            embedding: serde_json::from_value(serde_json::json!({
                "model": answer["embedding"]["model"],
                "dimensions": answer["embedding"]["dimensions"],
                "quantization": answer["embedding"]["quantization"].as_str().unwrap_or_default(),
            }))
            .ok(),
        };
        if let Ok(mut cached) = self.client.health.lock() {
            *cached = Some((Instant::now(), health.clone()));
        }
        Ok(health)
    }

    fn transcribe_window(&self, wav: &[u8], model: &str, language: &str, cancel: &AtomicBool) -> AppResult<Vec<SpeechSegment>> {
        if wav.len() > MAX_WINDOW_BYTES {
            return Err(AppError::new(codes::PAYLOAD_TOO_LARGE, "The audio window exceeds the transcription limit."));
        }
        let (content_type, body) = transcription_body(wav, model, language);
        let duration = wav_ms(wav);
        let client = self.client.clone();
        let agent = self.transcription_agent.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        // A cancelled job must not wait for the window: the request finishes on its own.
        std::thread::Builder::new()
            .name("compute-transcription".into())
            .spawn(move || {
                let answer = client.call(&agent, "POST", "/v1/audio/transcriptions", Body::Bytes { content_type, data: &body }, false);
                let _ = sender.send(answer);
            })
            .map_err(|_| AppError::new(codes::INTERNAL, "Cannot start the transcription request."))?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::new("cancelled", "Operazione annullata."));
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => return speech_segments(&answer?, duration),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(AppError::new(codes::INTERNAL, "The compute task stopped unexpectedly."));
                }
            }
        }
    }

    fn identity(&self) -> String {
        self.client.source.identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    /// A tiny HTTP server answering each connection with the next scripted reply; `None`
    /// closes the connection without answering.
    fn script(replies: Vec<Option<(u16, String)>>) -> (String, Arc<Mutex<Vec<String>>>) {
        let (url, seen, _) = script_with_bodies(replies);
        (url, seen)
    }

    type Log = Arc<Mutex<Vec<String>>>;

    /// [`script`], also keeping each request body as text.
    fn script_with_bodies(replies: Vec<Option<(u16, String)>>) -> (String, Log, Log) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let body_log = bodies.clone();
        std::thread::spawn(move || {
            let mut replies = replies.into_iter();
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() { break; }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") { length = value.trim().parse().unwrap(); }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock().unwrap().push(first.split_whitespace().nth(1).unwrap_or("").to_owned());
                body_log.lock().unwrap().push(String::from_utf8_lossy(&body).into_owned());
                match replies.next().flatten() {
                    Some((status, body)) => {
                        let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    }
                    None => drop(stream),
                }
            }
        });
        (format!("http://127.0.0.1:{port}"), seen, bodies)
    }

    fn compute(base_url: String) -> HttpCompute {
        HttpCompute::new(Arc::new(FixedEndpoint(Endpoint { base_url, token: "t".repeat(64), identity: "local".into() })))
    }

    fn embedding(values: Vec<f32>) -> String {
        json!({"object": "list", "data": [{"object": "embedding", "index": 0, "embedding": values}]}).to_string()
    }

    const HEALTH: &str = r#"{"status":"ok","version":"0.1.0","models":[]}"#;

    #[test]
    fn closed_codes_and_pauses_cross_http() {
        let body = |code: &str| json!({"error": {"code": code, "message": "m", "retry_after_s": 60}}).to_string();
        for (status, code, transient) in [(503, "model_missing", true), (503, "paused_for_memory", true), (503, "paused_after_failure", true), (503, "model_downloading", true), (422, "document_unreadable", false), (413, "payload_too_large", false), (401, "unauthorized", false), (500, "internal", false)] {
            // A 401 is asked once more after looking the endpoint up again.
            let (url, _) = script(vec![Some((status, body(code))), Some((status, body(code)))]);
            let error = compute(url).embed_query("q").unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(crate::application::compute::is_transient(error.code), transient, "{code}");
        }
    }

    #[test]
    fn vectors_are_renormalized_and_must_have_256_values() {
        let (url, seen) = script(vec![Some((200, embedding(vec![2.0; 256]))), Some((200, embedding(vec![1.0; 255])))]);
        let compute = compute(url);
        let v = compute.embed_query("città").unwrap();
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
        assert_eq!(compute.embed_document("t", "x").unwrap_err().code, codes::INTERNAL);
        assert_eq!(seen.lock().unwrap().as_slice(), ["/v1/embeddings", "/v1/embeddings"]);
    }

    #[test]
    fn a_dropped_connection_is_a_pause_then_retried_once() {
        let (url, seen) = script(vec![None, Some((200, embedding(vec![1.0; 256])))]);
        assert_eq!(compute(url).embed_query("q").unwrap().len(), 256);
        assert_eq!(seen.lock().unwrap().len(), 2);
        let (url, _) = script(vec![None, None]);
        let error = compute(url).embed_query("q").unwrap_err();
        assert_eq!(error.code, codes::UNREACHABLE);
        assert!(crate::application::compute::is_transient(error.code));
        let refused = compute("http://127.0.0.1:9".into()).chunk_text("x").unwrap_err();
        assert_eq!(refused.code, codes::UNREACHABLE);
    }

    #[test]
    fn a_document_that_drops_twice_while_the_server_answers_is_unreadable() {
        let (url, seen) = script(vec![None, Some((200, HEALTH.into())), None, Some((200, HEALTH.into()))]);
        let error = compute(url).extract_document("crash.pdf", b"%PDF").unwrap_err();
        assert_eq!(error.code, codes::DOCUMENT_UNREADABLE);
        assert_eq!(seen.lock().unwrap().as_slice(), ["/qamille/v1/extract", "/health", "/qamille/v1/extract", "/health"]);
        // The server drops once and comes back: the document is read.
        let (url, _) = script(vec![None, Some((200, HEALTH.into())), Some((200, json!({"text": "ok"}).to_string()))]);
        assert_eq!(compute(url).extract_document("a.pdf", b"%PDF").unwrap(), "ok");
        // The server stays silent: a pause, not the document's fault.
        let (url, _) = script(vec![None, None]);
        assert_eq!(compute(url).embed_image(b"png").unwrap_err().code, codes::UNREACHABLE);
    }

    #[test]
    fn a_server_from_before_the_rebranding_is_asked_with_the_old_route_name() {
        let missing = json!({"error": {"code": "not_found", "message": "no route"}}).to_string();
        let (url, seen) = script(vec![Some((404, missing.clone())), Some((200, json!({"chunks": ["a"]}).to_string()))]);
        assert_eq!(compute(url).chunk_text("a").unwrap(), vec!["a"]);
        assert_eq!(seen.lock().unwrap().as_slice(), ["/qamille/v1/chunks", "/sparklone/v1/chunks"]);
        // Missing under both names: one retry, then the 404 is the answer.
        let (url, seen) = script(vec![Some((404, missing.clone())), Some((404, missing))]);
        assert!(compute(url).chunk_text("a").is_err());
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn quota_exceeded_is_its_own_pause_and_not_a_busy_server() {
        let quota = |resource: &str| {
            json!({"error": {"code": "quota_exceeded", "message": "Hai finito i token.", "retry_after_s": 3600, "resource": resource, "limit": 100, "used": 120, "renews_at": "2026-12-01T00:00:00.000Z"}}).to_string()
        };
        // One answer only: no wait (3600 s) and no second attempt.
        let (url, seen) = script(vec![Some((429, quota("compute")))]);
        let started = Instant::now();
        let error = compute(url).embed_query("q").unwrap_err();
        assert_eq!(error.code, "quota_exceeded");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(crate::application::compute::is_transient(error.code));
        assert!(error.message.contains("search tokens"), "{}", error.message);
        let (url, _) = script(vec![Some((429, quota("transcription")))]);
        assert!(compute(url).embed_query("q").unwrap_err().message.contains("transcription"));
        // A 429 without that code is still a busy server.
        let busy = json!({"error": {"code": "rate_limited", "message": "busy", "retry_after_s": 60}}).to_string();
        let (url, _) = script(vec![Some((429, busy))]);
        assert_eq!(compute(url).embed_query("q").unwrap_err().code, "rate_limited");
    }

    #[test]
    fn a_full_queue_is_waited_for() {
        let busy = json!({"error": {"code": "rate_limited", "message": "busy", "retry_after_s": 1}}).to_string();
        let (url, seen) = script(vec![Some((429, busy)), Some((200, json!({"chunks": ["a", "b"]}).to_string()))]);
        assert_eq!(compute(url).chunk_text("a. b").unwrap(), vec!["a", "b"]);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn health_without_models_means_the_credentials_were_refused() {
        let anonymous = r#"{"status":"ok","version":"0.1.0"}"#;
        let (url, seen) = script(vec![Some((200, anonymous.into())), Some((200, anonymous.into()))]);
        assert_eq!(compute(url).health().unwrap_err().code, codes::UNAUTHORIZED);
        assert_eq!(seen.lock().unwrap().len(), 2, "asked again once after a new lookup");
    }

    /// Gives `first` until invalidated, then `second`; counts the invalidations.
    struct Rotating {
        first: Endpoint,
        second: Endpoint,
        invalidated: std::sync::atomic::AtomicUsize,
    }

    impl EndpointSource for Rotating {
        fn endpoint(&self) -> AppResult<Endpoint> {
            Ok(if self.invalidated.load(Ordering::SeqCst) == 0 { self.first.clone() } else { self.second.clone() })
        }
        fn invalidate(&self, _: &Endpoint) {
            self.invalidated.fetch_add(1, Ordering::SeqCst);
        }
        fn identity(&self) -> String {
            "local".into()
        }
    }

    #[test]
    fn a_401_looks_the_local_server_up_again_once() {
        let refused = json!({"error": {"code": "unauthorized", "message": "m", "retry_after_s": null}}).to_string();
        let (old, old_seen) = script(vec![Some((401, refused))]);
        let (new, new_seen) = script(vec![Some((200, json!({"chunks": ["a"]}).to_string()))]);
        let endpoint = |base_url: String, token: &str| Endpoint { base_url, token: token.repeat(64), identity: "local".into() };
        let source = Arc::new(Rotating { first: endpoint(old, "o"), second: endpoint(new, "n"), invalidated: Default::default() });
        let compute = HttpCompute::new(source.clone());
        assert_eq!(compute.chunk_text("a").unwrap(), vec!["a"]);
        assert_eq!(source.invalidated.load(Ordering::SeqCst), 1);
        assert_eq!((old_seen.lock().unwrap().len(), new_seen.lock().unwrap().len()), (1, 1));
    }

    #[test]
    fn a_request_that_brings_the_server_down_twice_spends_an_attempt() {
        // Dropped twice, while the restarted server answers `/health` in between: the request
        // is at fault, and a pause would retry it for ever.
        let (url, seen) = script(vec![None, None, Some((200, HEALTH.into()))]);
        let error = compute(url).chunk_text("a huge passage").unwrap_err();
        assert_eq!(error.code, codes::INTERNAL);
        assert!(!crate::application::compute::is_transient(error.code));
        assert_eq!(error.message, REQUEST_CRASHED_MESSAGE);
        assert_eq!(seen.lock().unwrap().as_slice(), ["/qamille/v1/chunks", "/qamille/v1/chunks", "/health"]);
    }

    #[test]
    fn concurrent_health_reads_share_one_request() {
        let (url, seen) = script(vec![Some((200, HEALTH.into()))]);
        let compute = Arc::new(compute(url));
        let readers: Vec<_> = (0..4).map(|_| { let compute = compute.clone(); std::thread::spawn(move || compute.health().map(|h| h.version)) }).collect();
        for reader in readers {
            assert_eq!(reader.join().unwrap().unwrap(), "0.1.0");
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    fn window(seconds: u64) -> Vec<u8> {
        let mut wav = vec![0u8; 44];
        wav[..4].copy_from_slice(b"RIFF");
        wav.extend(std::iter::repeat_n(1u8, (seconds * 32_000) as usize));
        wav
    }

    #[test]
    fn a_window_is_sent_as_openai_multipart_and_its_segments_come_back_in_ms() {
        let answer = json!({"text": "ciao a tutti", "segments": [{"start": 0.5, "end": 1.25, "text": " ciao"}, {"start": 1.25, "text": "a tutti "}]}).to_string();
        let (url, seen, bodies) = script_with_bodies(vec![Some((200, answer)), Some((200, json!({"text": " solo testo "}).to_string()))]);
        let compute = compute(url);
        let segments = compute.transcribe_window(&window(15), "whisper-base", "it", &AtomicBool::new(false)).unwrap();
        assert_eq!(segments, vec![
            SpeechSegment { start_ms: 500, end_ms: 1250, text: "ciao".into() },
            SpeechSegment { start_ms: 1250, end_ms: 15_000, text: "a tutti".into() },
        ]);
        assert_eq!(seen.lock().unwrap()[0], "/v1/audio/transcriptions");
        let body = bodies.lock().unwrap()[0].clone();
        for (field, value) in [("model", "whisper-base"), ("response_format", "verbose_json"), ("language", "it")] {
            assert!(body.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n")), "{field}");
        }
        assert!(body.contains("filename=\"audio.wav\""));
        let auto = compute.transcribe_window(&window(2), "whisper-base", "auto", &AtomicBool::new(false)).unwrap();
        assert_eq!(auto, vec![SpeechSegment { start_ms: 0, end_ms: 2000, text: "solo testo".into() }]);
        assert!(!bodies.lock().unwrap()[1].contains("name=\"language\""), "auto sends no language");
    }

    #[test]
    fn a_cancelled_window_returns_at_once() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(20));
        });
        let compute = compute(format!("http://127.0.0.1:{port}"));
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let began = Instant::now();
        let error = compute.transcribe_window(&window(1), "whisper-base", "it", &cancel).unwrap_err();
        assert_eq!(error.code, "cancelled");
        assert!(began.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_redirect_is_never_followed() {
        let (elsewhere, seen) = script(vec![Some((200, embedding(vec![1.0; 256])))]);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buffer = [0u8; 8192];
                let _ = stream.read(&mut buffer);
                let _ = write!(stream, "HTTP/1.1 307 Moved\r\nLocation: {elsewhere}/v1/embeddings\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let error = compute(format!("http://127.0.0.1:{port}")).embed_query("q").unwrap_err();
        assert_eq!(error.code, "external");
        std::thread::sleep(Duration::from_millis(100));
        assert!(seen.lock().unwrap().is_empty(), "the credential never followed the redirect");
    }

    #[test]
    fn multipart_names_cannot_break_the_header() {
        let (content_type, body) = multipart("a\"b\r\n.pdf", b"X");
        assert!(content_type.starts_with("multipart/form-data; boundary=qamille-"));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("a_b__.pdf"));
        assert!(!text.contains("a\"b"));
        let _ = std::io::sink().write_all(&body);
    }
}
