//! Errori del server e loro forma HTTP: `{"error":{"code","message","retry_after_s"}}`
//! (D2 della specifica del server di calcolo). Nessun messaggio contiene segreti.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::accounts::DeviceInfo;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("Username o password non corretti.")]
    InvalidCredentials,
    #[error("La password è provvisoria: scegline una nuova prima di entrare.")]
    PasswordChangeRequired,
    #[error("Hai raggiunto il numero massimo di dispositivi: disattivane uno per entrare.")]
    DeviceLimit(Vec<DeviceInfo>),
    #[error("Troppi tentativi. Riprova tra {retry_after_s} secondi.")]
    RateLimited { retry_after_s: u64 },
    #[error("Il server è occupato. Riprova tra {retry_after_s} secondi.")]
    ServerBusy { retry_after_s: u64 },
    #[error("Questa operazione richiede una connessione https.")]
    TlsRequired,
    #[error("Credenziale mancante o non valida.")]
    Unauthorized,
    #[error("La richiesta è troppo grande.")]
    PayloadTooLarge,
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("Esiste già un utente con questo username.")]
    UsernameTaken,
    #[error("Esiste già un utente con questa email.")]
    EmailTaken,
    #[error("Metodo non consentito per questa rotta.")]
    MethodNotAllowed,
    #[error("Serve un abbonamento Qamille attivo per il calcolo sul server.")]
    SubscriptionRequired,
    #[error("La richiesta ha impiegato troppo tempo.")]
    Timeout,
    /// Il dettaglio va solo nel log del server, mai nella risposta.
    #[error("Errore interno del server.")]
    Internal(String),
    /// Un errore di un'estensione (l'enterprise): stato, codice e messaggio li sceglie lei;
    /// i campi di `extra` (un oggetto JSON) si aggiungono all'oggetto `error`.
    #[error("{message}")]
    Extension {
        status: StatusCode,
        code: &'static str,
        message: String,
        retry_after_s: Option<u64>,
        extra: Option<serde_json::Value>,
    },
}

impl ServerError {
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        ServerError::Internal(detail.to_string())
    }

    pub fn not_found() -> Self {
        ServerError::NotFound("Risorsa non trovata.".to_owned())
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ServerError::InvalidCredentials | ServerError::Unauthorized => StatusCode::UNAUTHORIZED,
            ServerError::PasswordChangeRequired => StatusCode::FORBIDDEN,
            ServerError::DeviceLimit(_)
            | ServerError::UsernameTaken
            | ServerError::EmailTaken => StatusCode::CONFLICT,
            ServerError::SubscriptionRequired => StatusCode::PAYMENT_REQUIRED,
            ServerError::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            ServerError::ServerBusy { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ServerError::TlsRequired | ServerError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ServerError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ServerError::NotFound(_) => StatusCode::NOT_FOUND,
            ServerError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ServerError::Timeout => StatusCode::REQUEST_TIMEOUT,
            ServerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ServerError::Extension { status, .. } => *status,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ServerError::InvalidCredentials => "invalid_credentials",
            ServerError::PasswordChangeRequired => "password_change_required",
            ServerError::DeviceLimit(_) => "device_limit",
            ServerError::RateLimited { .. } => "rate_limited",
            ServerError::ServerBusy { .. } => "server_busy",
            ServerError::TlsRequired => "tls_required",
            ServerError::Unauthorized => "unauthorized",
            ServerError::PayloadTooLarge => "payload_too_large",
            ServerError::InvalidRequest(_) => "invalid_request",
            ServerError::NotFound(_) => "not_found",
            ServerError::UsernameTaken => "username_taken",
            ServerError::EmailTaken => "email_taken",
            ServerError::MethodNotAllowed => "method_not_allowed",
            ServerError::SubscriptionRequired => "subscription_required",
            ServerError::Timeout => "timeout",
            ServerError::Internal(_) => "internal",
            ServerError::Extension { code, .. } => code,
        }
    }

    pub fn retry_after_s(&self) -> Option<u64> {
        match self {
            ServerError::RateLimited { retry_after_s }
            | ServerError::ServerBusy { retry_after_s } => Some(*retry_after_s),
            ServerError::Extension { retry_after_s, .. } => *retry_after_s,
            _ => None,
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorObject<'a>,
}

#[derive(Serialize)]
struct ErrorObject<'a> {
    code: &'static str,
    message: String,
    retry_after_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    devices: Option<&'a [DeviceInfo]>,
    #[serde(flatten)]
    extra: Option<&'a serde_json::Map<String, serde_json::Value>>,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        if let ServerError::Internal(detail) = &self {
            tracing::error!(detail = %detail, "errore interno");
        }
        let retry_after_s = self.retry_after_s();
        let devices = match &self {
            ServerError::DeviceLimit(list) => Some(list.as_slice()),
            _ => None,
        };
        let extra = match &self {
            ServerError::Extension { extra: Some(serde_json::Value::Object(map)), .. } => Some(map),
            _ => None,
        };
        let body = ErrorBody {
            error: ErrorObject {
                code: self.code(),
                message: self.to_string(),
                retry_after_s,
                devices,
                extra,
            },
        };
        let mut response = (self.status(), axum::Json(body)).into_response();
        if let Some(seconds) = retry_after_s
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        if matches!(self, ServerError::Unauthorized) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<rusqlite::Error> for ServerError {
    fn from(e: rusqlite::Error) -> Self {
        ServerError::internal(format!("database: {e}"))
    }
}
