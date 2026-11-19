//! Which compute server this process talks to (spec D3, A1, A11). Read at every indexing step,
//! search, extraction and transcription job, so a change takes effect from the next one; a job
//! keeps the gateway it started with ([`ComputeGateway::resolve`]).
//!
//! - `Settings.compute_mode = Off` (or a process the factory was not given the store and the
//!   vault of): no request leaves the process (`compute_not_connected`); the app computes
//!   nothing itself (INV-41).
//! - `Server`, with a session in the vault (`server-session`) whose address is in
//!   `Settings.compute_consented_urls`: [`HttpCompute`] towards `session.url` with
//!   `Authorization: Bearer <device key>` and the session's certificate authority (`tls.rs`). The
//!   key goes only to that address (INV-4); before every request the mode and the consent are
//!   checked again (`authorize()`, D7, INV-10).
//! - `Server` without a usable session: no request leaves the process (`compute_not_connected`,
//!   a pause).
//!
//! A `401` from the remote server re-reads the vault: a session that changed meanwhile (the
//! window signed in again) is tried once; otherwise the session stays, the device is marked
//! unlinked and `AppEvent::Notice` «This device is no longer connected to the server» is said
//! once per session (A8, EX-15).
use super::http_compute::{Endpoint, EndpointSource, HttpCompute};
use crate::application::compute::{self, ComputeGateway, ComputeHealth, SpeechSegment, codes};
use crate::application::events::AppEvent;
use crate::application::ports::{EventSink, SecretStore, StoreRepository};
use crate::application::server_account::SERVER_SESSION;
use crate::application::{AppError, AppResult};
use crate::domain::model::ComputeMode;
use crate::domain::server_account::ServerSession;
use crate::model::Store;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

static OVERRIDE: RwLock<Option<Arc<dyn ComputeGateway>>> = RwLock::new(None);
static GLOBAL: RwLock<Option<Arc<ComputeFactory>>> = RwLock::new(None);

/// The factory of this process: without [`configure`] it is `Off`.
pub fn global() -> Arc<ComputeFactory> {
    if let Ok(slot) = GLOBAL.read()
        && let Some(factory) = slot.as_ref()
    {
        return factory.clone();
    }
    let mut slot = GLOBAL.write().unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.get_or_insert_with(|| Arc::new(ComputeFactory::new(None))).clone()
}

/// Gives the factory of this process the store (mode, consent, certificate authority) and the
/// vault (the session) it chooses from. The window and the MCP processes call it once at start.
pub fn configure(repository: Arc<dyn StoreRepository>, secrets: Arc<dyn SecretStore>) {
    let previous = global();
    let factory = Arc::new(ComputeFactory::new(Some(Links { repository, secrets })));
    if let Ok(sink) = previous.notices.sink.read()
        && let Some(sink) = sink.as_ref()
    {
        factory.set_notices(sink.clone());
    }
    *GLOBAL.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(factory);
}

/// Where the assistant of the server is reached: the session's address, its device key and
/// authority, and the consent header. Never kept: read again at every call, so a sign-out or
/// a changed consent takes effect at once.
#[derive(Clone)]
pub struct AssistantLink {
    pub url: String,
    pub key: String,
    pub ca_pem: Option<String>,
    /// The value of `X-Qamille-Llm-Consent`.
    pub consent: String,
}
impl std::fmt::Debug for AssistantLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantLink").field("url", &self.url).field("key", &"<hidden>").finish()
    }
}

/// The assistant link of this process (`None`: nothing leaves the process).
pub fn assistant_link() -> Option<AssistantLink> {
    global().assistant_link()
}

/// The gateway chosen now for this process's `data/` folder.
pub fn current() -> Arc<dyn ComputeGateway> {
    if let Ok(chosen) = OVERRIDE.read()
        && let Some(gateway) = chosen.as_ref()
    {
        return gateway.clone();
    }
    global().current()
}

/// A gateway that follows [`current`] at every call: the one `ServiceContext::compute` holds.
pub fn routed() -> Arc<dyn ComputeGateway> {
    Arc::new(RoutedCompute { factory: None })
}

/// Replaces the gateway of this process (tests only; `None` restores the default).
#[cfg(any(test, feature = "test-support"))]
pub fn set_override(gateway: Option<Arc<dyn ComputeGateway>>) {
    if let Ok(mut chosen) = OVERRIDE.write() {
        *chosen = gateway;
    }
}

/// Where the compute side of this process tells the person about a failure that does not clear
/// by itself (the window's event sink; processes without a window have none).
pub fn set_notices(sink: Arc<dyn EventSink>) {
    global().set_notices(sink);
}

struct Links {
    repository: Arc<dyn StoreRepository>,
    secrets: Arc<dyn SecretStore>,
}

/// Notices said once per key (the text of a failure, or a session that became unlinked).
#[derive(Default)]
pub struct Notices {
    sink: RwLock<Option<Arc<dyn EventSink>>>,
    seen: Mutex<HashSet<String>>,
}

impl Notices {
    pub fn once(&self, key: &str, text: &str) {
        let first = self.seen.lock().map(|mut seen| seen.insert(key.to_owned())).unwrap_or(false);
        if !first {
            return;
        }
        if let Ok(slot) = self.sink.read()
            && let Some(sink) = slot.as_ref()
        {
            let sink = sink.clone();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink.emit(AppEvent::Notice { text: text.to_owned() })));
        }
    }
    /// A notice said once may be said again (the condition cleared in between).
    pub fn forget(&self, key: &str) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.remove(key);
        }
    }
}

/// What the factory chose, for the interface (Settings → Compute).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// The mode is `Off`, or the factory has no store: nothing is sent.
    Off,
    /// The session's server, consented (its address).
    Server(String),
    /// Server mode without a session, or without consent for its address.
    NotConnected,
}

/// A session, its certificate authority and the fingerprint naming it (never the key itself).
#[derive(Clone)]
struct Link {
    session: ServerSession,
    ca_pem: Option<String>,
    fingerprint: String,
}

impl Link {
    fn endpoint(&self) -> Endpoint {
        Endpoint { base_url: self.session.url.clone(), token: self.session.device_key.clone(), identity: self.session.url.clone() }
    }
}

/// The SHA-256 of address, key and certificate authority: tells two links apart (a gateway is
/// built for one of them, its TLS bound to that authority) without keeping the key.
fn fingerprint(session: &ServerSession, ca_pem: Option<&str>) -> String {
    let digest = Sha256::digest(format!("{}\n{}\n{}", session.url, session.device_key, ca_pem.unwrap_or("")).as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub struct ComputeFactory {
    links: Option<Links>,
    remote: Mutex<Option<(String, Arc<dyn ComputeGateway>)>>,
    notices: Arc<Notices>,
    /// Fingerprints of the sessions the remote server refused (`401`).
    unlinked: Arc<Mutex<HashSet<String>>>,
    /// Builds the gateway of a session (tests: towards a server of their own, without TLS).
    remote_builder: Box<dyn Fn(Arc<dyn EndpointSource>, Option<&str>) -> AppResult<Arc<dyn ComputeGateway>> + Send + Sync>,
}

impl ComputeFactory {
    /// A factory choosing from `links` (none: always `Off`).
    fn new(links: Option<Links>) -> Self {
        ComputeFactory {
            links,
            remote: Mutex::new(None),
            notices: Arc::new(Notices::default()),
            unlinked: Arc::default(),
            remote_builder: Box::new(|source, ca| Ok(Arc::new(HttpCompute::remote(source, ca)?))),
        }
    }

    /// A factory of its own (tests, and processes that are not the window).
    pub fn with(repository: Arc<dyn StoreRepository>, secrets: Arc<dyn SecretStore>) -> Self {
        Self::new(Some(Links { repository, secrets }))
    }

    pub fn set_notices(&self, sink: Arc<dyn EventSink>) {
        if let Ok(mut slot) = self.notices.sink.write() {
            *slot = Some(sink);
        }
    }

    /// A gateway following this factory at every call.
    pub fn routed(self: &Arc<Self>) -> Arc<dyn ComputeGateway> {
        Arc::new(RoutedCompute { factory: Some(self.clone()) })
    }

    /// The session of the vault, if any; an unreadable value counts as none.
    fn session(links: &Links) -> Option<ServerSession> {
        let raw = links.secrets.get(SERVER_SESSION).ok()?;
        if raw.trim().is_empty() {
            return None;
        }
        serde_json::from_str(&raw).ok()
    }

    /// `authorize()` of D7 and INV-10: server mode, a session, and consent for its address.
    fn link(links: &Links) -> AppResult<Option<Link>> {
        let store = links.repository.read_shared()?;
        let settings = &store.settings;
        if settings.compute_mode != ComputeMode::Server {
            return Ok(None);
        }
        let not_connected = || AppError::new(codes::NOT_CONNECTED, compute::NOT_CONNECTED_MESSAGE);
        let session = Self::session(links).ok_or_else(not_connected)?;
        if !settings.compute_consented_urls.contains(&session.url) {
            return Err(not_connected());
        }
        // Only the authority saved for this very address (security review of wave 3).
        let ca_pem = crate::application::server_account::saved_ca(settings, &session.url);
        Ok(Some(Link { fingerprint: fingerprint(&session, ca_pem.as_deref()), session, ca_pem }))
    }

    /// A session, consent for its address (INV-10) and consent to the provider the server
    /// declared at that address (INV-34). The assistant is the person's own choice of provider,
    /// so it does not depend on `compute_mode`.
    pub fn assistant_link(&self) -> Option<AssistantLink> {
        let links = self.links.as_ref()?;
        let session = Self::session(links)?;
        let store = links.repository.read_shared().ok()?;
        let settings = &store.settings;
        if !settings.compute_consented_urls.contains(&session.url) {
            return None;
        }
        let consent = settings.llm_consent.as_ref().filter(|c| c.url == session.url)?;
        let ca_pem = crate::application::server_account::saved_ca(settings, &session.url);
        Some(AssistantLink { consent: consent.header(), url: session.url, key: session.device_key, ca_pem })
    }

    pub fn route(&self) -> Route {
        let Some(links) = &self.links else { return Route::Off };
        match Self::link(links) {
            Ok(None) => Route::Off,
            Ok(Some(link)) => Route::Server(link.session.url),
            Err(_) => Route::NotConnected,
        }
    }

    /// The remote server refused the session in the vault now (`401`), and it was not replaced.
    pub fn unlinked(&self) -> bool {
        let Some(links) = &self.links else { return false };
        match Self::link(links) {
            Ok(Some(link)) => self.unlinked.lock().map(|set| set.contains(&link.fingerprint)).unwrap_or(false),
            _ => false,
        }
    }

    pub fn current(&self) -> Arc<dyn ComputeGateway> {
        let off = || -> Arc<dyn ComputeGateway> { Arc::new(Unavailable(AppError::new(codes::NOT_CONNECTED, compute::NOT_CONNECTED_MESSAGE))) };
        let Some(links) = &self.links else { return off() };
        let link = match Self::link(links) {
            Ok(None) => return off(),
            Ok(Some(link)) => link,
            Err(error) => return Arc::new(Unavailable(error)),
        };
        let mut slot = self.remote.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((fingerprint, gateway)) = slot.as_ref()
            && *fingerprint == link.fingerprint
        {
            return gateway.clone();
        }
        let source = Arc::new(SessionSource {
            link: Mutex::new(link.clone()),
            repository: links.repository.clone(),
            secrets: links.secrets.clone(),
            notices: self.notices.clone(),
            unlinked: self.unlinked.clone(),
        });
        match (self.remote_builder)(source, link.ca_pem.as_deref()) {
            Ok(gateway) => {
                *slot = Some((link.fingerprint, gateway.clone()));
                gateway
            }
            // A certificate authority that cannot be read: nothing is sent.
            Err(error) => Arc::new(Unavailable(error)),
        }
    }
}

/// The session's server as an endpoint: `authorize()` before every request, the vault read
/// again after a `401`.
struct SessionSource {
    link: Mutex<Link>,
    repository: Arc<dyn StoreRepository>,
    secrets: Arc<dyn SecretStore>,
    notices: Arc<Notices>,
    unlinked: Arc<Mutex<HashSet<String>>>,
}

impl SessionSource {
    fn links(&self) -> Links {
        Links { repository: self.repository.clone(), secrets: self.secrets.clone() }
    }
}

impl EndpointSource for SessionSource {
    fn endpoint(&self) -> AppResult<Endpoint> {
        let link = self.link.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        // INV-10: still in server mode, and still consented for this address, at every request.
        match ComputeFactory::link(&self.links())? {
            Some(now) if now.session.url == link.session.url => Ok(link.endpoint()),
            _ => Err(AppError::new(codes::NOT_CONNECTED, compute::NOT_CONNECTED_MESSAGE)),
        }
    }
    fn invalidate(&self, _: &Endpoint) {}
    fn identity(&self) -> String {
        self.link.lock().map(|link| link.session.url.clone()).unwrap_or_default()
    }
    /// A `401`: the session in the vault may have changed meanwhile (A11). Only a new key for
    /// the same address and authority is tried here: another server, or another authority, gets
    /// a gateway of its own from the next `current()` (its TLS is built for it).
    fn renew(&self, endpoint: &Endpoint) -> bool {
        let Ok(Some(now)) = ComputeFactory::link(&self.links()) else { return false };
        let mut link = self.link.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if now.fingerprint == link.fingerprint || now.session.device_key == endpoint.token {
            return false;
        }
        if now.session.url != link.session.url || now.ca_pem != link.ca_pem {
            return false;
        }
        *link = now;
        true
    }
    fn refused(&self, _: &Endpoint, _: AppError) -> AppError {
        let fingerprint = self.link.lock().map(|link| link.fingerprint.clone()).unwrap_or_default();
        // The vault holds another link already: this one ends, nothing to tell.
        if let Ok(Some(now)) = ComputeFactory::link(&self.links())
            && now.fingerprint != fingerprint
        {
            return AppError::new(codes::UNAUTHORIZED, compute::UNLINKED_MESSAGE);
        }
        if let Ok(mut set) = self.unlinked.lock() {
            set.insert(fingerprint.clone());
        }
        self.notices.once(&format!("unlinked:{fingerprint}"), compute::UNLINKED_MESSAGE);
        AppError::new(codes::UNAUTHORIZED, compute::UNLINKED_MESSAGE)
    }
    fn unpaid(&self, _: &Endpoint, _: AppError) -> AppError {
        let fingerprint = self.link.lock().map(|link| link.fingerprint.clone()).unwrap_or_default();
        self.notices.once(&format!("unpaid:{fingerprint}"), compute::SUBSCRIPTION_REQUIRED_MESSAGE);
        AppError::new(codes::SUBSCRIPTION_REQUIRED, compute::SUBSCRIPTION_REQUIRED_MESSAGE)
    }
    fn quota_used_up(&self, _: &Endpoint, error: AppError) -> AppError {
        let fingerprint = self.link.lock().map(|link| link.fingerprint.clone()).unwrap_or_default();
        // The text names the resource: each one is said once, and the others stay open.
        self.notices.once(&format!("quota:{fingerprint}:{}", error.message), &error.message);
        error
    }
}

/// `Off`, or server mode without a usable session: every call fails at once, nothing is sent.
struct Unavailable(AppError);

impl Unavailable {
    fn fail<T>(&self) -> AppResult<T> {
        Err(AppError::new(self.0.code, self.0.message.clone()))
    }
}

impl ComputeGateway for Unavailable {
    fn chunk_text(&self, _: &str) -> AppResult<Vec<String>> {
        self.fail()
    }
    fn embed_query(&self, _: &str) -> AppResult<Vec<f32>> {
        self.fail()
    }
    fn embed_document(&self, _: &str, _: &str) -> AppResult<Vec<f32>> {
        self.fail()
    }
    fn embed_image(&self, _: &[u8]) -> AppResult<Vec<f32>> {
        self.fail()
    }
    fn extract_document(&self, _: &str, _: &[u8]) -> AppResult<String> {
        self.fail()
    }
    fn health(&self) -> AppResult<ComputeHealth> {
        self.fail()
    }
    fn transcribe_window(&self, _: &[u8], _: &str, _: &str, _: &AtomicBool) -> AppResult<Vec<SpeechSegment>> {
        self.fail()
    }
    fn identity(&self) -> String {
        "remote".into()
    }
    fn is_remote(&self) -> bool {
        true
    }
}

/// Each call goes to the gateway the factory chooses at that moment.
struct RoutedCompute {
    factory: Option<Arc<ComputeFactory>>,
}

impl RoutedCompute {
    fn now(&self) -> Arc<dyn ComputeGateway> {
        match &self.factory {
            Some(factory) => factory.current(),
            None => current(),
        }
    }
}

impl ComputeGateway for RoutedCompute {
    fn chunk_text(&self, text: &str) -> AppResult<Vec<String>> {
        self.now().chunk_text(text)
    }
    fn embed_query(&self, text: &str) -> AppResult<Vec<f32>> {
        self.now().embed_query(text)
    }
    fn embed_document(&self, title: &str, text: &str) -> AppResult<Vec<f32>> {
        self.now().embed_document(title, text)
    }
    fn embed_documents(&self, title: &str, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        self.now().embed_documents(title, texts)
    }
    fn embed_image(&self, bytes: &[u8]) -> AppResult<Vec<f32>> {
        self.now().embed_image(bytes)
    }
    fn extract_document(&self, name: &str, bytes: &[u8]) -> AppResult<String> {
        self.now().extract_document(name, bytes)
    }
    fn health(&self) -> AppResult<ComputeHealth> {
        self.now().health()
    }
    fn transcribe_window(&self, wav: &[u8], model: &str, language: &str, cancel: &AtomicBool) -> AppResult<Vec<SpeechSegment>> {
        self.now().transcribe_window(wav, model, language, cancel)
    }
    fn identity(&self) -> String {
        self.now().identity()
    }
    fn resolve(&self) -> Option<Arc<dyn ComputeGateway>> {
        Some(self.now())
    }
    fn is_remote(&self) -> bool {
        self.now().is_remote()
    }
}

/// The notices of the global factory (the window's tests read them).
pub fn notices() -> Arc<Notices> {
    global().notices.clone()
}

impl ComputeFactory {
    /// The notices of this factory.
    pub fn notices(&self) -> Arc<Notices> {
        self.notices.clone()
    }
    /// Tests: the gateway of a session is built by `builder` (a server of the test, plain HTTP).
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_remote_builder(mut self, builder: impl Fn(Arc<dyn EndpointSource>, Option<&str>) -> AppResult<Arc<dyn ComputeGateway>> + Send + Sync + 'static) -> Self {
        self.remote_builder = Box::new(builder);
        self
    }
}

/// What Settings → Compute shows about the compute server (D7): never a key.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct ComputeStatus {
    /// `server`, `off` or `not_connected`.
    pub route: String,
    /// The remote server's address.
    pub url: Option<String>,
    /// The remote server answered with this session's credentials.
    pub reachable: bool,
    /// The remote server refused this device (`401`): «Sign in again» (A8, EX-15).
    pub unlinked: bool,
    pub version: Option<String>,
    /// The models of the remote server (ready, downloading with a percentage, missing).
    pub models: Vec<crate::application::compute::ComputeModel>,
    /// D4: the server's embedding model and the index's, when they differ.
    pub model_mismatch: Option<ModelMismatch>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ModelMismatch {
    pub server: crate::application::compute::EmbeddingIdentity,
    pub index: crate::application::compute::EmbeddingIdentity,
}

impl ComputeFactory {
    /// Reads `/health` of the server of the session.
    pub fn status(&self) -> ComputeStatus {
        let route = self.route();
        let mut status = ComputeStatus::default();
        match &route {
            Route::Off => {
                status.route = "off".into();
                return status;
            }
            Route::NotConnected => {
                status.route = "not_connected".into();
                return status;
            }
            Route::Server(url) => {
                status.route = "server".into();
                status.url = Some(url.clone());
            }
        }
        match self.current().health() {
            Ok(health) => {
                status.reachable = true;
                status.version = Some(health.version.clone());
                let server = health.embedding_identity();
                if let Some(index) = super::rag_segments::index_identity(&Store::data_dir())
                    && !index.same_vectors(&server)
                {
                    status.model_mismatch = Some(ModelMismatch { server, index });
                }
                status.models = health.models;
            }
            Err(error) if error.code == codes::UNAUTHORIZED => status.unlinked = true,
            Err(_) => {}
        }
        status.unlinked |= self.unlinked();
        status
    }
}

/// [`ComputeFactory::status`] of this process.
pub fn status() -> ComputeStatus {
    global().status()
}
