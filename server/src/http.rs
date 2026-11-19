//! Router pubblico e amministrativo, estrattori (`DeviceAuth`, `AdminAuth`) e gestori.
//!
//! La pagina `/admin` e le rotte `/admin/api/*` esistono solo sul router
//! amministrativo (INV-18). Le rotte di registrazione esistono solo con la
//! registrazione accesa, quelle del recupero solo con la posta configurata.
//! Ogni risposta porta `Cache-Control: no-store`. Il log di accesso registra
//! metodo, rotta (lo schema, non il percorso), esito e durata: mai corpi,
//! credenziali o indirizzi.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{
    ConnectInfo, DefaultBodyLimit, FromRef, FromRequest, FromRequestParts, MatchedPath, Path,
    Request, State,
};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::time::Instant;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::accounts::{Accounts, AuthContext};
use crate::config::{AdminKey, Edition};
use crate::error::ServerError;
use crate::ext::{
    AccountExtension, AdminPage, AdminUsersExtension, Entitlement, InfoExtension, JsonMap,
    ReauthExtension,
};
use crate::limiter::FailureLimiter;
use crate::origin::{self, ClientOrigin};

/// Corpo massimo di una richiesta.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Durata massima di una richiesta.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Versione dell'API dichiarata da `GET /v1/info`.
pub const API_VERSION: u32 = 1;

/// Stato condiviso dei router.
#[derive(Clone)]
pub struct AppState {
    pub accounts: Arc<Accounts>,
    pub trusted_proxies: Arc<[IpAddr]>,
    /// Indirizzi da cui l'ascolto pubblico accetta il chiaro (`plaintext_from`).
    pub plaintext_from: Arc<[IpAddr]>,
    pub edition: Edition,
    pub admin: Option<Arc<AdminState>>,
    /// Il diritto di usare il calcolo; senza, ogni utente attivo può usarlo.
    pub entitlement: Option<Arc<dyn Entitlement>>,
    /// L'assistente in uso, che `/admin` può cambiare a caldo: `GET /v1/info` lo dichiara; vuoto
    /// senza provider (il campo `llm` manca, come un `null`).
    pub llm: crate::llm::LlmHolder,
    /// Ciò che `GET /v1/info` dichiara della sincronizzazione; `None` senza `QAMILLE_SYNC_KEY`.
    pub sync: Option<Arc<crate::sync::SyncInfo>>,
    /// Campi in più di `GET /v1/info`.
    pub info: Option<InfoExtension>,
    /// Campi in più di `GET /v1/account`.
    pub account: Option<AccountExtension>,
    /// Campi in più di `GET /admin/api/users`.
    pub admin_users: Option<AdminUsersExtension>,
    /// Conferma della cancellazione dell'account rifacendo l'accesso con un provider.
    pub reauth: Option<ReauthExtension>,
}

/// Chiave amministratore (solo il suo SHA-256) e limite per origine dedicato.
pub struct AdminState {
    key_sha256: [u8; 32],
    limiter: FailureLimiter,
    /// Indirizzi da cui l'ascolto amministrativo accetta il chiaro (oltre al loopback).
    plaintext_from: Vec<IpAddr>,
}

impl AdminState {
    pub fn new(
        key: &AdminKey,
        max_failures: u32,
        window: Duration,
        plaintext_from: Vec<IpAddr>,
    ) -> Self {
        AdminState {
            key_sha256: Sha256::digest(key.expose().as_bytes()).into(),
            limiter: FailureLimiter::new(max_failures, window),
            plaintext_from: plaintext_from
                .into_iter()
                .map(|ip| ip.to_canonical())
                .collect(),
        }
    }

    pub fn limiter(&self) -> &FailureLimiter {
        &self.limiter
    }

    /// Confronto a tempo costante (sugli SHA-256, così anche la lunghezza non trapela).
    fn matches(&self, candidate: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(candidate.as_bytes()).into();
        digest.ct_eq(&self.key_sha256).into()
    }
}

// --- Estrattori -----------------------------------------------------------------

/// Origine della richiesta (socket o, dietro un proxy fidato, `X-Forwarded-For`).
#[derive(Debug, Clone, Copy)]
pub struct Origin(pub ClientOrigin);

impl<S> FromRequestParts<S> for Origin
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let ConnectInfo(socket) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| {
                ServerError::internal("ConnectInfo mancante: il server va avviato con connect info")
            })?;
        Ok(Origin(origin::resolve_with_plaintext(
            socket,
            &parts.headers,
            &state.trusted_proxies,
            &state.plaintext_from,
        )))
    }
}

/// Come [`Origin`], ma rifiuta con `tls_required` una connessione non sicura.
#[derive(Debug, Clone, Copy)]
pub struct SecureOrigin(pub ClientOrigin);

impl<S> FromRequestParts<S> for SecureOrigin
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Origin(origin) = Origin::from_request_parts(parts, state).await?;
        if !origin.secure {
            return Err(ServerError::TlsRequired);
        }
        Ok(SecureOrigin(origin))
    }
}

pub(crate) fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

/// La chiave di un dispositivo attivo di un utente attivo, da
/// `Authorization: Bearer spk_dev_…`. Come il calcolo e la sincronizzazione: prima la
/// connessione sicura (`tls_required`: la chiave vale come una password), poi il limite per
/// origine sulle chiavi sbagliate (`rate_limited`, prima di ogni interrogazione del database).
#[derive(Debug, Clone)]
pub struct DeviceAuth(pub AuthContext);

impl<S> FromRequestParts<S> for DeviceAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let SecureOrigin(origin) = SecureOrigin::from_request_parts(parts, state).await?;
        let state = AppState::from_ref(state);
        let failures = state.accounts.device_auth_limiter();
        failures.check(origin.ip, Instant::now())?;
        let key = bearer(parts).ok_or(ServerError::Unauthorized)?;
        match state.accounts.authenticate(key).await {
            Ok(auth) => Ok(DeviceAuth(auth)),
            Err(error) => {
                if matches!(error, ServerError::Unauthorized) {
                    failures.record_failure(origin.ip, Instant::now());
                }
                Err(error)
            }
        }
    }
}

/// L'origine di una richiesta all'ascolto amministrativo, su connessione sicura:
/// come [`SecureOrigin`], ma accetta il chiaro anche dagli indirizzi di
/// `admin_plaintext_from` (per esempio il gateway della rete del compose, da cui
/// arriva la porta pubblicata su `127.0.0.1` dell'host).
#[derive(Debug, Clone, Copy)]
pub struct AdminOrigin(pub ClientOrigin);

impl<S> FromRequestParts<S> for AdminOrigin
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let admin = state.admin.as_ref().ok_or_else(ServerError::not_found)?;
        let ConnectInfo(socket) = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
            .ok_or_else(|| {
                ServerError::internal("ConnectInfo mancante: il server va avviato con connect info")
            })?;
        // Solo `admin_plaintext_from`: l'elenco del chiaro dell'ascolto pubblico non vale qui.
        let origin = origin::resolve_with_plaintext(
            socket,
            &parts.headers,
            &state.trusted_proxies,
            &admin.plaintext_from,
        );
        if !origin.secure {
            return Err(ServerError::TlsRequired);
        }
        Ok(AdminOrigin(origin))
    }
}

/// La chiave amministratore, su connessione sicura e sotto il suo limite per origine.
#[derive(Debug, Clone, Copy)]
pub struct AdminAuth;

impl<S> FromRequestParts<S> for AdminAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AdminOrigin(origin) = AdminOrigin::from_request_parts(parts, state).await?;
        let state = AppState::from_ref(state);
        let admin = state.admin.as_ref().ok_or_else(ServerError::not_found)?;
        let attempt = admin.limiter.begin(origin.ip, Instant::now())?;
        match bearer(parts) {
            Some(token) if admin.matches(token) => {
                attempt.succeeded();
                Ok(AdminAuth)
            }
            _ => {
                attempt.failed(Instant::now());
                Err(ServerError::Unauthorized)
            }
        }
    }
}

/// Corpo JSON con rifiuti nella forma d'errore del server. Il messaggio non
/// riporta mai il testo dell'errore di serde, che può contenere valori del corpo.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ServerError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(JsonBody(value)),
            Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                Err(ServerError::PayloadTooLarge)
            }
            Err(_) => Err(ServerError::InvalidRequest(
                "Corpo della richiesta non valido: serve un JSON con i campi previsti.".to_owned(),
            )),
        }
    }
}

// --- Richieste e risposte --------------------------------------------------------

macro_rules! redacted_debug {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

/// `POST /v1/login`.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub identifier: String,
    pub password: String,
    pub device_name: String,
}
redacted_debug!(LoginRequest);

/// `POST /v1/login/password`.
#[derive(Deserialize)]
pub struct TemporaryPasswordRequest {
    pub identifier: String,
    pub temporary_password: String,
    pub new_password: String,
    pub device_name: String,
}
redacted_debug!(TemporaryPasswordRequest);

/// `POST /v1/login/disable-device`.
#[derive(Deserialize)]
pub struct DisableDeviceRequest {
    pub identifier: String,
    pub password: String,
    pub device_id: String,
}
redacted_debug!(DisableDeviceRequest);

fn default_true() -> bool {
    true
}

/// `PUT /v1/account/password`.
#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    #[serde(default = "default_true")]
    pub sign_out_other_devices: bool,
}
redacted_debug!(ChangePasswordRequest);

/// `DELETE /v1/account`: con la password, oppure, per chi entra con Google, con il codice di
/// un accesso appena rifatto nel browser (`login_code` e `code_verifier`).
#[derive(Deserialize)]
pub struct DeleteAccountRequest {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub login_code: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
}
redacted_debug!(DeleteAccountRequest);

/// `POST /admin/api/users`.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub max_devices: Option<u32>,
}

/// `POST /admin/api/users/{id}/max-devices`. `null`: vale il massimo predefinito.
#[derive(Debug, Deserialize)]
pub struct MaxDevicesRequest {
    pub max_devices: Option<u32>,
}

#[derive(Serialize)]
struct DeviceView {
    id: String,
    name: String,
    created_at: String,
    last_seen_at: Option<String>,
    current: bool,
}

// --- Router ----------------------------------------------------------------------

/// Le rotte dell'account, senza `/health` (con il calcolo montato `/health` è il suo).
fn account_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/info", get(info))
        .route("/v1/login", post(login))
        .route("/v1/login/password", post(login_password))
        .route("/v1/login/disable-device", post(login_disable_device))
        .route("/v1/account/password", put(change_password))
        .route("/v1/account/devices", get(list_devices))
        .route("/v1/account/devices/{id}", delete(revoke_device))
        .route("/v1/devices/self", delete(revoke_self))
        .route("/v1/account", get(account).delete(delete_account))
}

/// Rotte amministrative senza stato né strati. Con `AdminPage::Extension` manca la pagina
/// del core (`/admin`, `/admin/`, `/admin/app.css`, `/admin/app.js`): la monta l'estensione.
fn admin_routes(page: AdminPage) -> Router<AppState> {
    let mut routes = Router::new();
    if page == AdminPage::Core {
        routes = routes
            .route("/admin", get(admin_page_index))
            .route("/admin/", get(admin_page_index))
            .route("/admin/app.css", get(admin_page_css))
            .route("/admin/app.js", get(admin_page_js));
    }
    routes
        .route(
            "/admin/api/users",
            get(admin_list_users).post(admin_create_user),
        )
        .route("/admin/api/users/{id}", delete(admin_delete_user))
        .route("/admin/api/users/{id}/devices", get(admin_list_devices))
        .route(
            "/admin/api/users/{id}/reset-password",
            post(admin_reset_password),
        )
        .route("/admin/api/users/{id}/disable", post(admin_disable_user))
        .route("/admin/api/users/{id}/enable", post(admin_enable_user))
        .route(
            "/admin/api/users/{id}/max-devices",
            post(admin_set_max_devices),
        )
        .route(
            "/admin/api/users/{id}/devices/{device_id}",
            delete(admin_revoke_device),
        )
}

/// Router dell'ascolto pubblico: le rotte del core (con `/health` se `health`) e quelle
/// delle estensioni, unite una volta sola sotto gli strati comuni. Panico di axum su una rotta
/// doppia: lo intercetta chi chiama.
pub(crate) fn compose_public(state: &AppState, health: bool, extensions: &[Router]) -> Router {
    let mut routes = account_routes();
    if health {
        routes = routes.route("/health", get(health_route));
    }
    let mut router = routes.with_state(state.clone());
    for extension in extensions {
        router = router.merge(extension.clone());
    }
    finish(router)
}

/// Router dell'ascolto amministrativo: ogni risposta porta anche le intestazioni di
/// sicurezza della pagina. `core_extra` sono le rotte del core che vogliono il suo stato (i
/// modelli del calcolo), `extensions` quelle delle estensioni.
pub(crate) fn compose_admin(
    state: &AppState,
    page: AdminPage,
    core_extra: Option<Router<AppState>>,
    extensions: &[Router],
) -> Router {
    let mut routes = admin_routes(page);
    if let Some(extra) = core_extra {
        routes = routes.merge(extra);
    }
    let mut router = routes.with_state(state.clone());
    for extension in extensions {
        // Defence in depth: whatever the extension's handlers extract, `/admin/api/` never
        // runs without the admin key.
        router = router.merge(extension.clone().route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_admin_key,
        )));
    }
    finish(router).layer(middleware::from_fn(admin_security_headers))
}

/// Strato delle rotte di estensione: sotto `/admin/api/` serve la chiave amministratore.
async fn require_admin_key(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ServerError> {
    if !request.uri().path().starts_with("/admin/api/") {
        return Ok(next.run(request).await);
    }
    let (mut parts, body) = request.into_parts();
    AdminAuth::from_request_parts(&mut parts, &state).await?;
    Ok(next.run(Request::from_parts(parts, body)).await)
}

/// CSP della pagina `/admin` (e di ogni risposta dell'ascolto amministrativo).
pub const ADMIN_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; form-action 'none'; base-uri 'none'";

async fn admin_security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(ADMIN_CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    response
}

fn finish(routes: Router) -> Router {
    routes
        .fallback(fallback)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(middleware::from_fn(finish_response))
}

async fn fallback() -> ServerError {
    ServerError::not_found()
}

fn is_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
}

/// Ultimo strato: errori senza corpo JSON nella forma del server,
/// `Cache-Control: no-store`, log di accesso senza dati sensibili.
pub(crate) async fn finish_response(request: Request, next: Next) -> Response {
    let method: Method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "(nessuna)".to_owned());
    let started = Instant::now();
    let mut response = next.run(request).await;
    if !is_json(&response) {
        let replacement = match response.status() {
            StatusCode::BAD_REQUEST => Some(ServerError::InvalidRequest(
                "Richiesta non valida.".to_owned(),
            )),
            StatusCode::NOT_FOUND => Some(ServerError::not_found()),
            StatusCode::METHOD_NOT_ALLOWED => Some(ServerError::MethodNotAllowed),
            StatusCode::PAYLOAD_TOO_LARGE => Some(ServerError::PayloadTooLarge),
            StatusCode::REQUEST_TIMEOUT => Some(ServerError::Timeout),
            _ => None,
        };
        if let Some(error) = replacement {
            let allow = response.headers().get(header::ALLOW).cloned();
            response = error.into_response();
            if let Some(allow) = allow {
                response.headers_mut().insert(header::ALLOW, allow);
            }
        }
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    tracing::info!(
        method = %method,
        route = %route,
        status = response.status().as_u16(),
        duration_ms = started.elapsed().as_millis() as u64,
        "richiesta"
    );
    response
}

// --- Gestori pubblici ------------------------------------------------------------

async fn health_route() -> Response {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn info(State(state): State<AppState>) -> Response {
    let mut body = JsonMap::new();
    body.insert("version".to_owned(), serde_json::json!(env!("CARGO_PKG_VERSION")));
    body.insert("api".to_owned(), serde_json::json!(API_VERSION));
    body.insert("edition".to_owned(), serde_json::json!(state.edition.as_str()));
    if let Some(gateway) = state.llm.get() {
        body.insert("llm".to_owned(), serde_json::json!(gateway.info()));
    }
    if let Some(sync) = &state.sync {
        body.insert("sync".to_owned(), serde_json::json!(**sync));
    }
    if let Some(extension) = &state.info {
        extension(&mut body);
    }
    Json(body).into_response()
}

/// `GET /v1/account`: chi è collegato (i campi delle estensioni si aggiungono ai suoi).
async fn account(
    State(state): State<AppState>,
    DeviceAuth(auth): DeviceAuth,
) -> Result<Response, ServerError> {
    let profile = state.accounts.profile(&auth.user_id).await?;
    let mut body = JsonMap::new();
    body.insert("user_id".to_owned(), serde_json::json!(profile.user_id));
    body.insert("username".to_owned(), serde_json::json!(profile.username));
    body.insert("email".to_owned(), serde_json::json!(profile.email));
    body.insert("has_password".to_owned(), serde_json::json!(profile.has_password));
    body.insert("device_id".to_owned(), serde_json::json!(auth.device_id));
    if let Some(extension) = &state.account {
        for (key, value) in extension(auth.user_id.clone()).await? {
            body.insert(key, value);
        }
    }
    Ok(Json(body).into_response())
}

async fn login(
    State(state): State<AppState>,
    SecureOrigin(origin): SecureOrigin,
    JsonBody(body): JsonBody<LoginRequest>,
) -> Result<Response, ServerError> {
    let ok = state
        .accounts
        .login(
            origin.ip,
            &body.identifier,
            &body.password,
            &body.device_name,
        )
        .await?;
    Ok(Json(ok).into_response())
}

async fn login_password(
    State(state): State<AppState>,
    SecureOrigin(origin): SecureOrigin,
    JsonBody(body): JsonBody<TemporaryPasswordRequest>,
) -> Result<Response, ServerError> {
    let ok = state
        .accounts
        .change_temporary_password(
            origin.ip,
            &body.identifier,
            &body.temporary_password,
            &body.new_password,
            &body.device_name,
        )
        .await?;
    Ok(Json(ok).into_response())
}

async fn login_disable_device(
    State(state): State<AppState>,
    SecureOrigin(origin): SecureOrigin,
    JsonBody(body): JsonBody<DisableDeviceRequest>,
) -> Result<StatusCode, ServerError> {
    state
        .accounts
        .disable_device_with_password(origin.ip, &body.identifier, &body.password, &body.device_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn change_password(
    State(state): State<AppState>,
    SecureOrigin(origin): SecureOrigin,
    DeviceAuth(auth): DeviceAuth,
    JsonBody(body): JsonBody<ChangePasswordRequest>,
) -> Result<StatusCode, ServerError> {
    state
        .accounts
        .change_password(
            origin.ip,
            &auth,
            &body.current_password,
            &body.new_password,
            body.sign_out_other_devices,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_devices(
    State(state): State<AppState>,
    DeviceAuth(auth): DeviceAuth,
) -> Result<Response, ServerError> {
    let devices: Vec<DeviceView> = state
        .accounts
        .list_devices(&auth.user_id)
        .await?
        .into_iter()
        .map(|d| DeviceView {
            current: d.id == auth.device_id,
            id: d.id,
            name: d.name,
            created_at: d.created_at,
            last_seen_at: d.last_seen_at,
        })
        .collect();
    Ok(Json(serde_json::json!({ "devices": devices })).into_response())
}

async fn revoke_device(
    State(state): State<AppState>,
    DeviceAuth(auth): DeviceAuth,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ServerError> {
    state.accounts.revoke_device(&auth, &device_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_self(
    State(state): State<AppState>,
    DeviceAuth(auth): DeviceAuth,
) -> Result<StatusCode, ServerError> {
    state.accounts.revoke_own_device(&auth).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_account(
    State(state): State<AppState>,
    SecureOrigin(origin): SecureOrigin,
    DeviceAuth(auth): DeviceAuth,
    JsonBody(body): JsonBody<DeleteAccountRequest>,
) -> Result<StatusCode, ServerError> {
    match (&body.password, &body.login_code, &body.code_verifier, &state.reauth) {
        (Some(password), _, _, _) => {
            state.accounts.delete_account(origin.ip, &auth, password).await?;
        }
        (None, Some(code), Some(verifier), Some(reauth)) => {
            let user_id = reauth(origin.ip, code, verifier)?;
            if user_id != auth.user_id {
                return Err(crate::ext::invalid_code());
            }
            state.accounts.delete_user(&auth.user_id).await?;
        }
        _ => {
            return Err(ServerError::InvalidRequest(
                "Conferma con la password o rifacendo l'accesso.".to_owned(),
            ));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- Pagina /admin ----------------------------------------------------------------

const ADMIN_INDEX_HTML: &str = include_str!("http/admin/index.html");
const ADMIN_APP_CSS: &str = include_str!("http/admin/app.css");
const ADMIN_APP_JS: &str = include_str!("http/admin/app.js");

fn static_page(content_type: &'static str, body: &'static str) -> Response {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
        .into_response()
}

async fn admin_page_index(_origin: AdminOrigin) -> Response {
    static_page("text/html; charset=utf-8", ADMIN_INDEX_HTML)
}

async fn admin_page_css(_origin: AdminOrigin) -> Response {
    static_page("text/css; charset=utf-8", ADMIN_APP_CSS)
}

async fn admin_page_js(_origin: AdminOrigin) -> Response {
    static_page("text/javascript; charset=utf-8", ADMIN_APP_JS)
}

// --- Gestori amministrativi --------------------------------------------------------

async fn admin_list_users(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Response, ServerError> {
    let users = state.accounts.list_users().await?;
    let mut users = serde_json::to_value(users).map_err(ServerError::internal)?;
    let extra = match &state.admin_users {
        Some(extension) => Some(extension().await?),
        None => None,
    };
    if let (Some(extra), Some(list)) = (&extra, users.as_array_mut()) {
        for user in list {
            let id = user["id"].as_str().unwrap_or_default().to_owned();
            let fields = extra.per_user.get(&id).unwrap_or(&extra.per_user_default);
            for (key, value) in fields {
                user[key] = value.clone();
            }
        }
    }
    let mut body = serde_json::json!({
        "users": users,
        "default_max_devices": state.accounts.default_max_devices(),
    });
    if state.llm.get().is_some() {
        body["llm"] = serde_json::json!(true);
    }
    if let Some(extra) = extra {
        for (key, value) in extra.top {
            body[key] = value;
        }
    }
    Ok(Json(body).into_response())
}

async fn admin_create_user(
    State(state): State<AppState>,
    _admin: AdminAuth,
    JsonBody(body): JsonBody<CreateUserRequest>,
) -> Result<Response, ServerError> {
    let created = state
        .accounts
        .create_user(&body.username, body.email.as_deref(), body.max_devices)
        .await?;
    Ok((StatusCode::CREATED, Json(created)).into_response())
}

async fn admin_delete_user(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ServerError> {
    state.accounts.delete_user(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_list_devices(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
) -> Result<Response, ServerError> {
    let devices = state.accounts.list_devices(&user_id).await?;
    Ok(Json(serde_json::json!({ "devices": devices })).into_response())
}

async fn admin_reset_password(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
) -> Result<Response, ServerError> {
    let temporary = state.accounts.reset_password(&user_id).await?;
    Ok(Json(temporary).into_response())
}

async fn admin_disable_user(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ServerError> {
    state.accounts.disable_user(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_enable_user(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ServerError> {
    state.accounts.enable_user(&user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_set_max_devices(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(user_id): Path<String>,
    JsonBody(body): JsonBody<MaxDevicesRequest>,
) -> Result<StatusCode, ServerError> {
    state
        .accounts
        .set_max_devices(&user_id, body.max_devices)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_revoke_device(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path((user_id, device_id)): Path<(String, String)>,
) -> Result<StatusCode, ServerError> {
    state
        .accounts
        .revoke_device_admin(&user_id, &device_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
