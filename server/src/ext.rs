//! I punti di estensione del core: tratti e tipi che l'enterprise (o un altro crate)
//! implementa per cambiare il comportamento senza toccare i file del core.

use crate::error::ServerError;

pub type BoxFut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Una risorsa del server che può richiedere un diritto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Compute,
    /// La trascrizione (`/v1/audio/transcriptions`): una risorsa a parte, così una quota
    /// finita non blocca anche la ricerca per significato.
    Transcription,
    Llm,
    Sync,
}

/// Il diritto di un utente a una risorsa. Senza diritto: `ServerError::SubscriptionRequired`
/// (o un'altra `ServerError`). Senza `Entitlement` ogni utente attivo può usare il calcolo.
pub trait Entitlement: Send + Sync + 'static {
    fn require<'a>(
        &'a self,
        user_id: &'a str,
        resource: Resource,
    ) -> BoxFut<'a, Result<(), ServerError>>;
}

/// Chi vuole sapere che un utente o un dispositivo non esistono più (per ripulire i suoi
/// dati). Si chiama **dopo il commit**, sullo stesso thread di chi ha fatto l'operazione:
/// non deve bloccare né fallire.
pub trait AccountObserver: Send + Sync + 'static {
    fn user_deleted(&self, user_id: &str);
    fn device_revoked(&self, device_id: &str);
}

/// Chi può opporsi alla cancellazione di un utente (per esempio perché ha un abbonamento che
/// continua a incassare). Si chiama **prima** della cancellazione, con l'id dell'utente, dopo che
/// la persona ha confermato con la password (o l'amministratore ha chiesto): un `Err` ferma tutto
/// e il suo messaggio arriva a chi ha chiesto. Un `Ok` può aver fatto qualcosa (disdire).
pub type DeletionGuard =
    std::sync::Arc<dyn Fn(String /* user_id */) -> BoxFut<'static, Result<(), ServerError>> + Send + Sync>;

/// Un oggetto JSON da arricchire o da aggiungere a una risposta del core.
pub type JsonMap = serde_json::Map<String, serde_json::Value>;

/// Aggiunge campi a `GET /v1/info`, dopo quelli del core.
pub type InfoExtension = std::sync::Arc<dyn Fn(&mut JsonMap) + Send + Sync>;

/// Campi in più di `GET /v1/account` per un utente (dopo quelli del core).
pub type AccountExtension = std::sync::Arc<
    dyn Fn(String /* user_id */) -> BoxFut<'static, Result<JsonMap, ServerError>> + Send + Sync,
>;

/// Campi in più di `GET /admin/api/users`: `top` nel corpo, `per_user` (o
/// `per_user_default` se l'utente manca) in ogni elemento di `users`.
pub struct AdminUsersExtra {
    pub top: JsonMap,
    pub per_user: std::collections::HashMap<String, JsonMap>,
    pub per_user_default: JsonMap,
}

pub type AdminUsersExtension =
    std::sync::Arc<dyn Fn() -> BoxFut<'static, Result<AdminUsersExtra, ServerError>> + Send + Sync>;

/// Conferma, rifacendo l'accesso con un provider esterno, chi cancella il proprio account:
/// `(origine, codice, verificatore)` → id dell'utente che si è appena autenticato.
pub type ReauthExtension = std::sync::Arc<
    dyn Fn(std::net::IpAddr, &str, &str) -> Result<String, ServerError> + Send + Sync,
>;

/// Chi serve la pagina `/admin`: il core (la sua) o un'estensione (con la sua, montata come
/// rotta). Le rotte `/admin/api/*` del core ci sono sempre.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AdminPage {
    #[default]
    Core,
    Extension,
}

/// `400 invalid_code`: un codice monouso (di conferma, o di un accesso rifatto) sbagliato,
/// scaduto o già usato. La stessa risposta del core e delle estensioni.
pub fn invalid_code() -> ServerError {
    ServerError::Extension {
        status: axum::http::StatusCode::BAD_REQUEST,
        code: "invalid_code",
        message: "Codice non valido o scaduto.".to_owned(),
        retry_after_s: None,
        extra: None,
    }
}
