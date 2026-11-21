//! The link of this computer to a Qamille server (spec A1, A2, A6, A7, INV-4, INV-10, INV-17).
//! `ServerAccount` is the port to the server's account routes; `ServerAccountService` governs the
//! session kept in the vault (`server-session`, one value written with one `set`), the consent per
//! address, the computation mode and the rules of the two editions. Passwords pass through these
//! calls and are never stored, logged or put in events.
use super::{AppError, AppResult, ServiceContext};
use crate::domain::{
    model::{AiProvider, ComputeMode, LlmConsent},
    server_account::{AccountInfo, BillingPage, CheckoutStatus, DeviceInfo, SERVER_API, ServerInfo, ServerSession},
    server_url::{ServerUrlError, normalize_server_url},
};
use std::sync::atomic::AtomicBool;
use crate::edition::Edition;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

/// The saved certificate authority, only for the address it was saved with (`None` for any
/// other, and for an authority saved without its address).
pub fn saved_ca(settings: &crate::domain::model::Settings, url: &str) -> Option<String> {
    settings
        .compute_ca_pem
        .clone()
        .filter(|pem| !pem.trim().is_empty())
        .filter(|_| settings.compute_ca_url.as_deref() == Some(url))
}

/// The vault key of the session (same value as `infrastructure::secret::SERVER_SESSION`).
pub const SERVER_SESSION: &str = "server-session";

/// Where account requests of a session go: only to the session's own address (INV-4).
#[derive(Clone)]
pub struct RemoteEndpoint {
    pub url: String,
    pub device_key: String,
    pub ca_pem: Option<String>,
    pub timeout: Duration,
}
impl std::fmt::Debug for RemoteEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteEndpoint").field("url", &self.url).field("device_key", &"<hidden>").finish()
    }
}

/// An address before any session: the probe and the sign-in routes.
#[derive(Clone, Debug)]
pub struct ServerAddress {
    pub url: String,
    pub ca_pem: Option<String>,
}

/// What `POST /v1/login` and `POST /v1/login/password` give.
#[derive(Clone)]
pub struct LoginGrant {
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub device_key: String,
}

/// The outcome of a sign-in that reached the server with valid credentials.
pub enum LoginOutcome {
    Signed(LoginGrant),
    /// A temporary password: it must be changed first (`POST /v1/login/password`).
    PasswordChangeRequired,
    /// The account has its maximum of devices: one must be disabled first.
    DeviceLimit(Vec<DeviceInfo>),
}

/// A sign-in request; the password never appears in `Debug`.
pub struct Credentials<'a> {
    pub identifier: &'a str,
    pub password: &'a str,
    pub device_name: &'a str,
}
impl std::fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("identifier", &self.identifier).field("device_name", &self.device_name).finish()
    }
}

/// What a browser sign-in on the server brings back to the app: a single-use code, valid only
/// with the verifier this app made for it (two PKCE: app→server, server→Google). Never in
/// `Debug`, logs or events.
#[derive(Clone)]
pub struct BrowserTicket {
    pub code: String,
    pub verifier: String,
}
impl std::fmt::Debug for BrowserTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrowserTicket(<hidden>)")
    }
}

/// The sign-in providers this app knows how to run in the browser.
pub const BROWSER_PROVIDERS: [&str; 1] = ["google"];

/// The account routes of a Qamille server (spec A2). Errors carry the server's codes:
/// `unreachable`, `tls_untrusted` (the certificate is refused), `not_qamille`, `invalid_credentials`, `rate_limited`, `server_busy`,
/// `tls_required`, `unauthorized`, `invalid_request`, `server_error`.
pub trait ServerAccount: Send + Sync {
    fn info(&self, address: &ServerAddress) -> AppResult<ServerInfo>;
    fn login(&self, address: &ServerAddress, credentials: &Credentials<'_>) -> AppResult<LoginOutcome>;
    fn change_temporary_password(&self, address: &ServerAddress, credentials: &Credentials<'_>, new_password: &str) -> AppResult<LoginOutcome>;
    fn disable_device_with_password(&self, address: &ServerAddress, identifier: &str, password: &str, device_id: &str) -> AppResult<()>;
    fn devices(&self, endpoint: &RemoteEndpoint) -> AppResult<Vec<DeviceInfo>>;
    fn revoke_device(&self, endpoint: &RemoteEndpoint, device_id: &str) -> AppResult<()>;
    fn change_password(&self, endpoint: &RemoteEndpoint, current: &str, new: &str, sign_out_other_devices: bool) -> AppResult<()>;
    /// `DELETE /v1/devices/self`.
    fn sign_out(&self, endpoint: &RemoteEndpoint) -> AppResult<()>;
    fn delete_account(&self, endpoint: &RemoteEndpoint, password: &str) -> AppResult<()>;
    /// `POST /v1/signup`: always accepted (202), whether the email is free or not (A9).
    fn signup_start(&self, _address: &ServerAddress, _email: &str) -> AppResult<()> {
        Err(AppError::new("not_available", "This server does not offer sign-up"))
    }
    /// `POST /v1/signup/confirm`: creates the account and registers this device.
    fn signup_confirm(&self, _address: &ServerAddress, _signup: &SignUp<'_>) -> AppResult<LoginGrant> {
        Err(AppError::new("not_available", "This server does not offer sign-up"))
    }
    /// `POST /v1/password-reset`: always accepted (202).
    fn reset_start(&self, _address: &ServerAddress, _email: &str) -> AppResult<()> {
        Err(AppError::new("not_available", "This server does not offer password recovery"))
    }
    /// `POST /v1/password-reset/confirm`: sets the password and signs out every device.
    fn reset_confirm(&self, _address: &ServerAddress, _email: &str, _code: &str, _new_password: &str) -> AppResult<()> {
        Err(AppError::new("not_available", "This server does not offer password recovery"))
    }
    /// Opens the browser on `GET /v1/auth/{provider}/start` and waits on a loopback address of
    /// this computer for the server to come back with a code. With `reauth` the code only
    /// confirms an operation of the signed-in account (Google asks for the credentials again).
    /// `cancel` stops the wait.
    fn browser_sign_in(&self, _address: &ServerAddress, _provider: &str, _reauth: bool, _terms: Option<&str>, _cancel: &AtomicBool) -> AppResult<BrowserTicket> {
        Err(AppError::new("not_available", "This server does not offer browser sign-in"))
    }
    /// `POST /v1/login/browser`: registers this device with the code.
    fn redeem_browser(&self, _address: &ServerAddress, _ticket: &BrowserTicket, _device_name: &str) -> AppResult<LoginOutcome> {
        Err(AppError::new("not_available", "This server does not offer browser sign-in"))
    }
    /// `POST /v1/login/browser/disable-device`: at the device limit, with the same code.
    fn browser_disable_device(&self, _address: &ServerAddress, _ticket: &BrowserTicket, _device_id: &str) -> AppResult<()> {
        Err(AppError::new("not_available", "This server does not offer browser sign-in"))
    }
    /// `GET /v1/account`.
    fn account(&self, _endpoint: &RemoteEndpoint) -> AppResult<AccountInfo> {
        Err(AppError::new("not_available", "This server does not describe the account"))
    }
    /// `POST /v1/billing/checkout`: the processor's payment page for a plan.
    fn checkout(&self, _endpoint: &RemoteEndpoint, _plan_id: &str) -> AppResult<BillingPage> {
        Err(AppError::new("not_available", "This server does not sell subscriptions"))
    }
    /// `GET /v1/billing/checkout/{id}`.
    fn checkout_status(&self, _endpoint: &RemoteEndpoint, _id: &str) -> AppResult<CheckoutStatus> {
        Err(AppError::new("not_available", "This server does not sell subscriptions"))
    }
    /// `POST /v1/billing/portal`: the processor's page to manage the subscription.
    fn billing_portal(&self, _endpoint: &RemoteEndpoint) -> AppResult<BillingPage> {
        Err(AppError::new("not_available", "This server does not sell subscriptions"))
    }
    /// `DELETE /v1/account` confirmed by a browser sign-in just made (accounts without password).
    fn delete_account_reauthenticated(&self, _endpoint: &RemoteEndpoint, _ticket: &BrowserTicket) -> AppResult<()> {
        Err(AppError::new("not_available", "This server does not offer browser sign-in"))
    }
}

/// A page to open in the person's browser: `https://`, or `http://` only towards this computer.
/// Anything else from a server (a `file:` path, a custom scheme) is refused.
pub fn browser_url(url: &str) -> AppResult<String> {
    let url = url.trim();
    let refused = || AppError::new("invalid_url", "The server sent a page address this app does not open");
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(refused());
    }
    let (rest, https) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (rest, true),
        (None, Some(rest)) => (rest, false),
        _ => return Err(refused()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // The host without the port: `127.0.0.1.evil.example` is not the loopback.
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    };
    let host_ok = !host.is_empty() && !authority.contains('@') && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let loopback = host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost");
    if host_ok && (https || loopback) { Ok(url.to_string()) } else { Err(refused()) }
}

/// A sign-up confirmation; the password and the code never appear in `Debug`.
pub struct SignUp<'a> {
    pub email: &'a str,
    pub code: &'a str,
    pub password: &'a str,
    pub username: Option<&'a str>,
    pub device_name: &'a str,
    /// The version of the terms the person accepted, as `/v1/info` declared it (wave 9): sent
    /// only when the box was ticked.
    pub accepted_terms: Option<&'a str>,
}
impl std::fmt::Debug for SignUp<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignUp").field("email", &self.email).field("username", &self.username).finish()
    }
}

/// What a probe of an address found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Probe {
    Found { info: ServerInfo },
    Unreachable,
    NotQamille,
    Incompatible { info: ServerInfo },
    HttpsRequired,
    Invalid,
}

/// The outcome of connecting, changing the temporary password or changing server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectOutcome {
    Connected { status: ServerStatus },
    PasswordChangeRequired,
    DeviceLimit { devices: Vec<DeviceInfo> },
}

/// How a sign-out went: `offline` when the server did not answer and the device stays in its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignOut {
    SignedOut,
    SignedOutOffline,
}

/// What the interface may know about the link: never the device key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerStatus {
    pub edition: Edition,
    pub connected: bool,
    pub url: Option<String>,
    pub username: Option<String>,
    pub device_id: Option<String>,
    pub compute_mode: ComputeMode,
    pub consented_urls: Vec<String>,
}

/// Where the assistant of the server stands (gateway LLM, L5): the interface warns before a
/// request would be refused with `409 llm_consent_required`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LlmStatus {
    /// Not connected, or the server has no assistant.
    Unavailable,
    Ready { label: String, external: bool },
    /// The server declares another provider than the one agreed to (or none was agreed to).
    ConsentNeeded { label: String, external: bool },
}

/// `GET /v1/info` is read again at most this often to warn about a changed provider.
const LLM_INFO_TTL: Duration = Duration::from_secs(600);

/// A browser sign-in waiting at the device limit.
struct PendingBrowser {
    address: ServerAddress,
    ticket: BrowserTicket,
    device_name: String,
    change_server: bool,
}

const STARTUP_SIGN_OUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ServerAccountService {
    context: Arc<ServiceContext>,
    account: Arc<dyn ServerAccount>,
    edition: Edition,
    /// A browser sign-in stopped at the device limit: its code stays valid for a few minutes, to
    /// disable a device and finish. Only in memory.
    pending_browser: std::sync::Mutex<Option<PendingBrowser>>,
    /// Raised by «Cancel» while the browser sign-in waits.
    browser_cancel: AtomicBool,
    /// The certificate authority written in the open form: it applies to the address being
    /// connected and is saved only with the session it helped create. The session in use
    /// keeps its own (security review: a CA pasted for a new server must not cover the old one).
    pending_ca: std::sync::Mutex<Option<String>>,
    /// The last `info` read for `llm_status`, with when and for which address.
    llm_info: std::sync::Mutex<Option<(std::time::Instant, String, ServerInfo)>>,
    llm_info_ttl: Duration,
}

fn url_error(error: ServerUrlError) -> AppError {
    AppError::new(error.code(), error.message())
}

impl ServerAccountService {
    pub fn new(context: Arc<ServiceContext>, account: Arc<dyn ServerAccount>, edition: Edition) -> Self {
        Self {
            context,
            account,
            edition,
            pending_browser: std::sync::Mutex::new(None),
            browser_cancel: AtomicBool::new(false),
            pending_ca: std::sync::Mutex::new(None),
            llm_info: std::sync::Mutex::new(None),
            llm_info_ttl: LLM_INFO_TTL,
        }
    }
    /// Another interval between two reads of `info` (tests).
    pub fn with_llm_info_ttl(mut self, ttl: Duration) -> Self {
        self.llm_info_ttl = ttl;
        self
    }
    pub fn edition(&self) -> &Edition {
        &self.edition
    }

    /// The session in the vault, if any. An unreadable value counts as none.
    pub fn session(&self) -> AppResult<Option<ServerSession>> {
        Self::parse_session(&self.context.secrets.get(SERVER_SESSION)?)
    }
    /// The session for a request made before signing in: a session in the credentials of another
    /// Windows user or computer (no passphrase opens them) is none, so the sign-in can go on.
    fn session_for_sign_in(&self) -> AppResult<Option<ServerSession>> {
        Self::parse_session(&self.context.secrets.get_for_sign_in(SERVER_SESSION)?)
    }
    fn parse_session(raw: &str) -> AppResult<Option<ServerSession>> {
        if raw.trim().is_empty() {
            return Ok(None);
        }
        Ok(serde_json::from_str(raw).ok())
    }
    /// The saved CA, only for the address it was saved with.
    fn ca_pem(&self, url: &str) -> AppResult<Option<String>> {
        Ok(saved_ca(&self.context.read_shared()?.settings, url))
    }
    fn pending(&self) -> Option<String> {
        self.pending_ca.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// The CA for a request to `url` before a session for it exists: the one written in the
    /// form; the saved one only when no session for another address could be exposed by it.
    fn ca_for(&self, url: &str) -> AppResult<Option<String>> {
        let session_url = self.session_for_sign_in()?.map(|s| s.url);
        Ok(match session_url {
            Some(current) if current != url => self.pending(),
            _ => self.pending().or(self.ca_pem(url)?),
        })
    }
    /// The endpoint of the session: the only way a device key leaves the vault.
    pub fn endpoint(&self) -> AppResult<Option<RemoteEndpoint>> {
        let Some(session) = self.session()? else { return Ok(None) };
        let ca_pem = self.ca_pem(&session.url)?;
        Ok(Some(RemoteEndpoint { url: session.url, device_key: session.device_key, ca_pem, timeout: REQUEST_TIMEOUT }))
    }
    fn require_endpoint(&self) -> AppResult<RemoteEndpoint> {
        self.endpoint()?.ok_or_else(|| AppError::new("not_connected", "This computer is not connected to a Qamille server"))
    }
    /// The normalized address, refused when a Professional build is asked for another server.
    fn address(&self, url: &str) -> AppResult<ServerAddress> {
        let url = normalize_server_url(url).map_err(url_error)?;
        if let Some(preset) = self.edition.preset() {
            if url != preset {
                return Err(AppError::new("preset_only", "This edition connects only to its own Qamille server"));
            }
        }
        let ca_pem = self.ca_for(&url)?;
        Ok(ServerAddress { url, ca_pem })
    }

    pub fn status(&self) -> AppResult<ServerStatus> {
        let session = self.session()?;
        let settings = self.context.read_shared()?.settings.clone();
        Ok(ServerStatus {
            edition: self.edition.clone(),
            connected: session.is_some(),
            url: session.as_ref().map(|s| s.url.clone()),
            username: session.as_ref().map(|s| s.username.clone()),
            device_id: session.as_ref().map(|s| s.device_id.clone()),
            compute_mode: settings.compute_mode,
            consented_urls: settings.compute_consented_urls,
        })
    }

    /// `GET /v1/info` only: the one request allowed before consent (INV-10).
    pub fn probe(&self, url: &str) -> AppResult<Probe> {
        let address = match self.address(url) {
            Ok(address) => address,
            Err(error) if error.code == "https_required" => return Ok(Probe::HttpsRequired),
            Err(error) if error.code == "invalid_url" => return Ok(Probe::Invalid),
            Err(error) => return Err(error),
        };
        Ok(match self.account.info(&address) {
            Ok(info) if info.api == SERVER_API => Probe::Found { info },
            Ok(info) => Probe::Incompatible { info },
            Err(error) if error.code == "unreachable" => Probe::Unreachable,
            Err(error) if error.code == "not_qamille" => Probe::NotQamille,
            Err(error) => return Err(error),
        })
    }

    /// The person agreed to send content to this address (INV-10).
    pub fn record_consent(&self, url: &str) -> AppResult<()> {
        let address = self.address(url)?;
        self.context.update(|store| {
            let consented = &mut store.settings.compute_consented_urls;
            if !consented.contains(&address.url) {
                consented.push(address.url.clone());
            }
            Ok(())
        })
    }
    /// The person agreed to the provider the server declares now for the assistant (INV-34).
    /// What is saved comes from `GET /v1/info`, never from the interface. A server without an
    /// assistant saves nothing.
    pub fn record_llm_consent(&self, url: &str) -> AppResult<()> {
        let address = self.address(url)?;
        self.require_consent(&address)?;
        let info = self.account.info(&address)?;
        let Some(llm) = info.llm else { return Ok(()) };
        // The label travels in a header: a server cannot make it carry a line break.
        if llm.label.trim().is_empty() || llm.label.chars().any(char::is_control) {
            return Err(AppError::new("invalid_provider", "The server declared an assistant provider this app cannot use"));
        }
        self.context.update(|store| {
            store.settings.llm_consent = Some(LlmConsent { url: address.url.clone(), label: llm.label.clone(), external: llm.external });
            Ok(())
        })
    }
    /// What the connected server declares now, kept for a few minutes. `None` without a session
    /// or when the server does not answer.
    pub fn current_info(&self) -> AppResult<Option<ServerInfo>> {
        let Some(session) = self.session()? else { return Ok(None) };
        let cached = self
            .llm_info
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|(at, url, _)| url == &session.url && at.elapsed() < self.llm_info_ttl)
            .map(|(_, _, info)| info);
        Ok(match cached {
            Some(info) => Some(info),
            None => {
                let address = self.address(&session.url)?;
                let Ok(info) = self.account.info(&address) else { return Ok(None) };
                *self.llm_info.lock().unwrap_or_else(|e| e.into_inner()) = Some((std::time::Instant::now(), session.url.clone(), info.clone()));
                Some(info)
            }
        })
    }
    /// The normalized address of `url` when the server there declares the mailbox
    /// synchronization (`GET /v1/info`, never the interface's word), `None` when it does not.
    pub fn sync_declared(&self, url: &str) -> AppResult<Option<String>> {
        let address = self.address(url)?;
        let info = self.account.info(&address)?;
        Ok(info.sync.filter(|s| s.available).map(|_| address.url))
    }
    /// The declared provider of the session's server against the consent, to warn beforehand.
    pub fn llm_status(&self) -> AppResult<LlmStatus> {
        let Some(session) = self.session()? else { return Ok(LlmStatus::Unavailable) };
        let Some(info) = self.current_info()? else { return Ok(LlmStatus::Unavailable) };
        let Some(llm) = info.llm else { return Ok(LlmStatus::Unavailable) };
        let agreed = self.context.read_shared()?.settings.llm_consent.clone();
        let same = agreed.is_some_and(|c| c.url == session.url && c.label == llm.label && c.external == llm.external);
        Ok(if same {
            LlmStatus::Ready { label: llm.label, external: llm.external }
        } else {
            LlmStatus::ConsentNeeded { label: llm.label, external: llm.external }
        })
    }
    fn require_consent(&self, address: &ServerAddress) -> AppResult<()> {
        if self.context.read_shared()?.settings.compute_consented_urls.contains(&address.url) {
            Ok(())
        } else {
            Err(AppError::new("consent_required", "Agree to send content to this server before signing in"))
        }
    }
    /// One `set` of the session, then the remote mode.
    fn store_session(&self, address: &ServerAddress, grant: LoginGrant) -> AppResult<ServerStatus> {
        let session = ServerSession {
            url: address.url.clone(),
            user_id: grant.user_id,
            username: grant.username,
            device_id: grant.device_id,
            device_key: grant.device_key,
        };
        let value = serde_json::to_string(&session).map_err(|_| AppError::storage("The server session could not be saved"))?;
        self.context.secrets.set(SERVER_SESSION, &value)?;
        // A server that sells subscriptions, and an account without one: the computation stays
        // on this computer until the person subscribes (remote would only pause). Any other
        // answer, or none, keeps the rule of A8: remote after signing in.
        let probe = RemoteEndpoint {
            url: session.url.clone(),
            device_key: session.device_key.clone(),
            ca_pem: address.ca_pem.clone(),
            timeout: Duration::from_secs(10),
        };
        let mode = match self.account.account(&probe) {
            Ok(info) if info.billing_enabled && !info.subscription.as_ref().is_some_and(|s| s.entitled) => ComputeMode::Off,
            _ => ComputeMode::Server,
        };
        // The CA is bound to its address (`compute_ca_url`): between the two writes the new
        // session is never paired with the old server's CA, whichever is read first.
        let ca = address.ca_pem.clone();
        let url = address.url.clone();
        self.context.update(|store| {
            // The server declared an assistant and the person agreed to its provider at this
            // address: it is the default unless they chose another one.
            let agreed = store.settings.llm_consent.as_ref().is_some_and(|c| c.url == url);
            if agreed && store.settings.ai_provider == AiProvider::Offline {
                store.settings.ai_provider = AiProvider::QamilleServer;
            }
            store.settings.compute_mode = mode;
            store.settings.compute_ca_url = ca.as_ref().map(|_| url.clone());
            store.settings.compute_ca_pem = ca.clone();
            Ok(())
        })?;
        *self.pending_ca.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.status()
    }
    fn finish(&self, address: &ServerAddress, outcome: LoginOutcome, before_store: impl FnOnce()) -> AppResult<ConnectOutcome> {
        Ok(match outcome {
            LoginOutcome::Signed(grant) => {
                before_store();
                ConnectOutcome::Connected { status: self.store_session(address, grant)? }
            }
            LoginOutcome::PasswordChangeRequired => ConnectOutcome::PasswordChangeRequired,
            LoginOutcome::DeviceLimit(devices) => ConnectOutcome::DeviceLimit { devices },
        })
    }

    /// Signs in to `url` and keeps the session. Without consent nothing but `info` is sent.
    pub fn connect(&self, url: &str, credentials: &Credentials<'_>) -> AppResult<ConnectOutcome> {
        let address = self.address(url)?;
        self.require_consent(&address)?;
        // The session is saved in the vault: no login (and no device) when it would be refused.
        self.context.secrets.check_writable()?;
        let outcome = self.account.login(&address, credentials)?;
        self.finish(&address, outcome, || {})
    }
    /// Replaces a temporary password and registers this device in one request. With
    /// `change_server` (the sign-in that asked for the new password was a change of server) the
    /// rules of [`ServerAccountService::change_server`] hold: self-hosted only, another address,
    /// and the old device signs out once the new session is kept.
    pub fn change_temporary_password(&self, url: &str, credentials: &Credentials<'_>, new_password: &str, change_server: bool) -> AppResult<ConnectOutcome> {
        if change_server && self.edition.preset().is_some() {
            return Err(AppError::new("not_available", "This edition cannot change server"));
        }
        let address = self.address(url)?;
        let old = if change_server { self.endpoint()? } else { None };
        if old.as_ref().is_some_and(|old| old.url == address.url) {
            return Err(AppError::new("same_server", "This computer is already connected to this server"));
        }
        self.require_consent(&address)?;
        self.context.secrets.check_writable()?;
        let outcome = self.account.change_temporary_password(&address, credentials, new_password)?;
        let account = self.account.clone();
        self.finish(&address, outcome, move || {
            if let Some(old) = old {
                let _ = account.sign_out(&old);
            }
        })
    }
    /// At the device limit: disables one device of the same account with the password.
    pub fn disable_device_with_password(&self, url: &str, identifier: &str, password: &str, device_id: &str) -> AppResult<()> {
        let address = self.address(url)?;
        self.require_consent(&address)?;
        self.account.disable_device_with_password(&address, identifier, password, device_id)
    }

    pub fn devices(&self) -> AppResult<Vec<DeviceInfo>> {
        self.account.devices(&self.require_endpoint()?)
    }
    pub fn revoke_device(&self, device_id: &str) -> AppResult<()> {
        self.account.revoke_device(&self.require_endpoint()?, device_id)
    }
    pub fn change_password(&self, current: &str, new: &str, sign_out_other_devices: bool) -> AppResult<()> {
        self.account.change_password(&self.require_endpoint()?, current, new, sign_out_other_devices)
    }

    /// Self-hosted only: the old session stays until the new sign-in succeeds; then the old
    /// device signs out (its error ignored) and the new session is written with one `set`.
    pub fn change_server(&self, url: &str, credentials: &Credentials<'_>) -> AppResult<ConnectOutcome> {
        if self.edition.preset().is_some() {
            return Err(AppError::new("not_available", "This edition cannot change server"));
        }
        let address = self.address(url)?;
        let old = self.endpoint()?;
        if old.as_ref().is_some_and(|old| old.url == address.url) {
            return Err(AppError::new("same_server", "This computer is already connected to this server"));
        }
        self.require_consent(&address)?;
        let outcome = self.account.login(&address, credentials)?;
        let account = self.account.clone();
        self.finish(&address, outcome, move || {
            if let Some(old) = old {
                let _ = account.sign_out(&old);
            }
        })
    }

    fn forget_session(&self) -> AppResult<()> {
        self.context.secrets.remove(SERVER_SESSION)?;
        *self.llm_info.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.context.update(|store| {
            store.settings.compute_mode = ComputeMode::Off;
            // The consent to the server's assistant goes with the session.
            store.settings.llm_consent = None;
            // The consent to keep the mailboxes on the server goes with the session too.
            store.settings.mailbox_sync_consent = None;
            store.settings.mailbox_sync = false;
            if store.settings.ai_provider == AiProvider::QamilleServer {
                store.settings.ai_provider = AiProvider::Offline;
            }
            Ok(())
        })
    }
    /// Signs this device out. The session goes even when the server does not answer.
    pub fn sign_out(&self) -> AppResult<SignOut> {
        let Some(endpoint) = self.endpoint()? else {
            self.forget_session()?;
            return Ok(SignOut::SignedOut);
        };
        let outcome = match self.account.sign_out(&endpoint) {
            // Already revoked on the server: signed out all the same.
            Ok(()) => SignOut::SignedOut,
            Err(error) if error.code == "unauthorized" => SignOut::SignedOut,
            Err(_) => SignOut::SignedOutOffline,
        };
        self.forget_session()?;
        Ok(outcome)
    }
    /// Deletes the account on the server, then signs out like `sign_out`.
    pub fn delete_account(&self, password: &str) -> AppResult<()> {
        let endpoint = self.require_endpoint()?;
        self.account.delete_account(&endpoint, password)?;
        self.forget_session()
    }
    /// `Off` never touches the session; `Server` needs one.
    pub fn set_mode(&self, mode: ComputeMode) -> AppResult<()> {
        if mode == ComputeMode::Server && self.session()?.is_none() {
            return Err(AppError::new("not_connected", "This computer is not connected to a Qamille server"));
        }
        self.context.update(|store| {
            store.settings.compute_mode = mode;
            Ok(())
        })
    }
    /// Professional, at start: a session for another address (a `data/` folder that came from a
    /// self-hosted build) signs out there within 5 s and is removed (EX-20).
    pub fn startup_check(&self) -> AppResult<bool> {
        let Some(preset) = self.edition.preset() else { return Ok(false) };
        let Some(mut endpoint) = self.endpoint()? else { return Ok(false) };
        if endpoint.url == preset {
            return Ok(false);
        }
        endpoint.timeout = STARTUP_SIGN_OUT;
        let _ = self.account.sign_out(&endpoint);
        self.forget_session()?;
        Ok(true)
    }
    /// Professional only, with consent: asks the server to mail a sign-up code (INV-17).
    pub fn signup_start(&self, url: &str, email: &str) -> AppResult<()> {
        if self.edition.preset().is_none() {
            return Err(AppError::new("not_available", "Accounts are created by the server administrator"));
        }
        let address = self.address(url)?;
        self.require_consent(&address)?;
        self.account.signup_start(&address, email.trim())
    }
    /// Creates the account with the mailed code and keeps the session, like `connect`.
    pub fn signup_confirm(&self, url: &str, signup: &SignUp<'_>) -> AppResult<ServerStatus> {
        if self.edition.preset().is_none() {
            return Err(AppError::new("not_available", "Accounts are created by the server administrator"));
        }
        let address = self.address(url)?;
        self.require_consent(&address)?;
        self.context.secrets.check_writable()?;
        let grant = self.account.signup_confirm(&address, signup)?;
        self.store_session(&address, grant)
    }
    /// «Forgot password?»: asks the server to mail a code. Both editions; with consent, like every
    /// request that carries something of the person (INV-10).
    pub fn reset_start(&self, url: &str, email: &str) -> AppResult<()> {
        let address = self.address(url)?;
        self.require_consent(&address)?;
        self.account.reset_start(&address, email.trim())
    }
    /// Sets the new password with the mailed code. No session is created: the person signs in.
    pub fn reset_confirm(&self, url: &str, email: &str, code: &str, new_password: &str) -> AppResult<()> {
        let address = self.address(url)?;
        self.require_consent(&address)?;
        self.account.reset_confirm(&address, email.trim(), code.trim(), new_password)
    }
    /// The certificate authority of the person's organization for the address in the open form,
    /// or none. Saved with the session only once a sign-in succeeds.
    pub fn set_ca_pem(&self, pem: Option<String>) -> AppResult<()> {
        let pem = pem.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        if pem.as_ref().is_some_and(|p| !p.contains("-----BEGIN CERTIFICATE-----") || p.len() > 256 * 1024) {
            return Err(AppError::validation("The certificate authority file holds no PEM certificate"));
        }
        *self.pending_ca.lock().unwrap_or_else(|e| e.into_inner()) = pem;
        Ok(())
    }
    /// The form was closed: the CA written in it is forgotten («Cancel» changes nothing).
    pub fn forget_pending_ca(&self) {
        *self.pending_ca.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    fn provider(provider: &str) -> AppResult<&'static str> {
        BROWSER_PROVIDERS
            .iter()
            .copied()
            .find(|known| *known == provider)
            .ok_or_else(|| AppError::validation("Unknown sign-in provider"))
    }

    fn browser_outcome(&self, pending: PendingBrowser, outcome: LoginOutcome) -> AppResult<ConnectOutcome> {
        match outcome {
            LoginOutcome::DeviceLimit(devices) => {
                *self.pending_browser.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
                Ok(ConnectOutcome::DeviceLimit { devices })
            }
            LoginOutcome::PasswordChangeRequired => Err(AppError::new("server_error", "The Qamille server answered with an error")),
            LoginOutcome::Signed(grant) => {
                let old = if pending.change_server { self.endpoint()? } else { None };
                let account = self.account.clone();
                self.finish(&pending.address, LoginOutcome::Signed(grant), move || {
                    if let Some(old) = old {
                        let _ = account.sign_out(&old);
                    }
                })
            }
        }
    }

    /// «Continue with Google» (spec «Vendita Professional», V3): the browser signs in on the
    /// server, which hands this app a single-use code for this device. Consent first (INV-10);
    /// with `change_server` (self-hosted only) the old session goes once the new one exists.
    /// `accepted_terms`: the version read from `/v1/info`, when the person ticked the box before
    /// the browser opened (a new account needs it, an existing one signs in anyway).
    pub fn connect_with_browser(&self, url: &str, provider: &str, device_name: &str, change_server: bool, accepted_terms: Option<&str>) -> AppResult<ConnectOutcome> {
        let provider = Self::provider(provider)?;
        if change_server && self.edition.preset().is_some() {
            return Err(AppError::new("not_available", "This edition cannot change server"));
        }
        let address = self.address(url)?;
        if change_server && self.endpoint()?.is_some_and(|old| old.url == address.url) {
            return Err(AppError::new("same_server", "This computer is already connected to this server"));
        }
        self.require_consent(&address)?;
        self.context.secrets.check_writable()?;
        *self.pending_browser.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.browser_cancel.store(false, std::sync::atomic::Ordering::Release);
        let ticket = self.account.browser_sign_in(&address, provider, false, accepted_terms, &self.browser_cancel)?;
        let outcome = self.account.redeem_browser(&address, &ticket, device_name)?;
        self.browser_outcome(PendingBrowser { address, ticket, device_name: device_name.to_string(), change_server }, outcome)
    }

    /// Stops a browser sign-in that waits for the browser.
    pub fn cancel_browser_sign_in(&self) {
        self.browser_cancel.store(true, std::sync::atomic::Ordering::Release);
    }

    /// At the device limit of a browser sign-in: disables one device with the same code, then
    /// finishes the sign-in.
    pub fn browser_disable_device(&self, device_id: &str) -> AppResult<ConnectOutcome> {
        let pending = self
            .pending_browser
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| AppError::new("invalid_code", "The sign-in expired. Sign in again"))?;
        if let Err(error) = self.account.browser_disable_device(&pending.address, &pending.ticket, device_id) {
            // A server that did not answer, or answered busy, has not used the code: the person
            // can pick the device again. Any other refusal ends this sign-in.
            if matches!(error.code, "unreachable" | "tls_untrusted" | "rate_limited" | "server_busy" | "server_error") {
                *self.pending_browser.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
            }
            return Err(error);
        }
        let outcome = self.account.redeem_browser(&pending.address, &pending.ticket, &pending.device_name)?;
        self.browser_outcome(pending, outcome)
    }

    /// `GET /v1/account` of the session.
    pub fn account_info(&self) -> AppResult<AccountInfo> {
        self.account.account(&self.require_endpoint()?)
    }

    /// The processor's payment page for a plan, to open in the browser.
    pub fn start_checkout(&self, plan_id: &str) -> AppResult<BillingPage> {
        let mut page = self.account.checkout(&self.require_endpoint()?, plan_id.trim())?;
        page.url = browser_url(&page.url)?;
        Ok(page)
    }

    pub fn checkout_status(&self, id: &str) -> AppResult<CheckoutStatus> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(AppError::validation("Unknown payment session"));
        }
        self.account.checkout_status(&self.require_endpoint()?, id)
    }

    /// The processor's page to manage the subscription, to open in the browser.
    pub fn billing_portal(&self) -> AppResult<BillingPage> {
        let mut page = self.account.billing_portal(&self.require_endpoint()?)?;
        page.url = browser_url(&page.url)?;
        Ok(page)
    }

    /// Deletes an account without password: a new browser sign-in on the session's server
    /// confirms it, then this device signs out like `sign_out`.
    pub fn delete_account_with_browser(&self, provider: &str) -> AppResult<()> {
        let provider = Self::provider(provider)?;
        let endpoint = self.require_endpoint()?;
        let address = ServerAddress { url: endpoint.url.clone(), ca_pem: endpoint.ca_pem.clone() };
        self.browser_cancel.store(false, std::sync::atomic::Ordering::Release);
        let ticket = self.account.browser_sign_in(&address, provider, true, None, &self.browser_cancel)?;
        self.account.delete_account_reauthenticated(&endpoint, &ticket)?;
        self.forget_session()
    }

    /// The first start of a Professional build is behind the person.
    pub fn finish_onboarding(&self) -> AppResult<()> {
        self.context.update(|store| {
            store.settings.onboarding_done = true;
            Ok(())
        })
    }
}
