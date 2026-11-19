//! Utenti, dispositivi, accesso, password e autenticazione della chiave.
//!
//! Regole (A3 della specifica):
//! - chi non può entrare (sconosciuto, disattivato, senza password, provvisoria
//!   scaduta, password sbagliata) riceve sempre `InvalidCredentials` dopo una
//!   verifica argon2;
//! - il limite per origine si controlla prima della prenotazione del turno e di argon2;
//! - le verifiche argon2 sono al più 4 insieme, e chi non trova posto entro 2 s
//!   riceve `ServerBusy`;
//! - il massimo di dispositivi si controlla dentro la transazione `BEGIN IMMEDIATE`
//!   che registra il dispositivo.

use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tokio::time::Instant;

use crate::config::Config;
use crate::db::Db;
use crate::error::ServerError;
use crate::ext::{AccountObserver, DeletionGuard};
use crate::limiter::{FailureLimiter, LoginThrottle, normalize_identifier};
use crate::password;

/// Prefisso delle chiavi dei dispositivi.
pub const DEVICE_KEY_PREFIX: &str = "spk_dev_";
/// Verifiche argon2 contemporanee.
pub const ARGON2_SLOTS: usize = 4;
/// Attesa massima di un posto per argon2.
pub const ARGON2_WAIT: Duration = Duration::from_secs(2);
/// `last_seen_at` si aggiorna al più ogni 5 minuti.
pub const LAST_SEEN_INTERVAL: chrono::TimeDelta = chrono::TimeDelta::minutes(5);
/// Password attuali sbagliate di fila con la stessa chiave prima della revoca.
pub const WRONG_PASSWORD_STREAK_LIMIT: i64 = 10;

const MAX_IDENTIFIER_CHARS: usize = 320;
const MAX_USERNAME_CHARS: usize = 64;
const MAX_EMAIL_CHARS: usize = 254;
const MAX_DEVICE_NAME_CHARS: usize = 100;

/// Orologio da parete, iniettabile nei test.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Orologio fermo che si sposta a mano (per i test).
pub struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    pub fn new(start: DateTime<Utc>) -> Self {
        ManualClock(Mutex::new(start))
    }

    pub fn advance(&self, by: chrono::TimeDelta) {
        let mut now = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *now += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

pub fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Un dispositivo attivo, come lo vedono l'utente e l'amministratore.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
}

/// Esito di un accesso riuscito. Il `Debug` non stampa la chiave.
#[derive(Clone, Serialize)]
pub struct LoginOk {
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub device_key: String,
}

impl fmt::Debug for LoginOk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginOk")
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

/// Chi sta chiamando, ricavato da una chiave valida.
#[derive(Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub device_id: String,
    key_sha256: String,
}

impl AuthContext {
    /// Lo SHA-256 (esadecimale) della chiave usata: la nomina senza contenerla.
    pub fn key_sha256(&self) -> &str {
        &self.key_sha256
    }
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthContext")
            .field("user_id", &self.user_id)
            .field("device_id", &self.device_id)
            .finish_non_exhaustive()
    }
}

/// Una password provvisoria appena generata. Il `Debug` non la stampa.
#[derive(Clone, Serialize)]
pub struct TemporaryPassword {
    pub temporary_password: String,
    pub temporary_expires_at: String,
}

impl fmt::Debug for TemporaryPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TemporaryPassword")
            .field("temporary_expires_at", &self.temporary_expires_at)
            .finish_non_exhaustive()
    }
}

/// Utente appena creato. Il `Debug` non stampa la provvisoria.
#[derive(Clone, Serialize)]
pub struct CreatedUser {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub temporary_password: String,
    pub temporary_expires_at: String,
}

impl fmt::Debug for CreatedUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedUser")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("email", &self.email)
            .field("temporary_expires_at", &self.temporary_expires_at)
            .finish_non_exhaustive()
    }
}

/// Riga dell'elenco utenti per l'amministratore. Mai l'hash.
#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub password_temporary: bool,
    pub temporary_expires_at: Option<String>,
    /// `None`: vale `default_max_devices`.
    pub max_devices: Option<u32>,
    pub disabled_at: Option<String>,
    pub created_at: String,
    pub active_devices: u32,
    /// Contatori di calcolo (D5): solo numeri, mai contenuti.
    pub usage: ComputeUsage,
}

/// L'account visto da chi è collegato (`GET /v1/account`).
#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    /// Ha una password (chi è nato con Google può non averla).
    pub has_password: bool,
    /// Le identità esterne collegate (`google`).
    pub identities: Vec<String>,
}

/// Contatori di calcolo di un utente: richieste, secondi di audio trascritti, token
/// embeddati.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ComputeUsage {
    pub requests: u64,
    pub audio_seconds: f64,
    pub embedded_tokens: u64,
    /// Token in entrata e in uscita dell'assistente (gateway LLM).
    pub llm_prompt_tokens: u64,
    pub llm_completion_tokens: u64,
}

#[derive(Clone)]
struct UserRow {
    id: String,
    username: String,
    password_hash: Option<String>,
    temporary: bool,
    temporary_expires_at: Option<String>,
    disabled: bool,
}

impl UserRow {
    fn temporary_expired(&self, now: DateTime<Utc>) -> bool {
        match self
            .temporary_expires_at
            .as_deref()
            .and_then(parse_timestamp)
        {
            Some(expires) => now >= expires,
            None => true,
        }
    }

    /// Con la password giusta, l'utente può entrare?
    fn usable(&self, now: DateTime<Utc>) -> bool {
        !self.disabled
            && self.password_hash.is_some()
            && !(self.temporary && self.temporary_expired(now))
    }
}

const USER_COLUMNS: &str =
    "id, username, password_hash, password_temporary, temporary_expires_at, disabled_at";

fn user_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: row.get(0)?,
        username: row.get(1)?,
        password_hash: row.get(2)?,
        temporary: row.get::<_, i64>(3)? != 0,
        temporary_expires_at: row.get(4)?,
        disabled: row.get::<_, Option<String>>(5)?.is_some(),
    })
}

fn find_by_identifier(conn: &Connection, identifier: &str) -> rusqlite::Result<Option<UserRow>> {
    let identifier = identifier.trim();
    let sql = if identifier.contains('@') {
        format!("SELECT {USER_COLUMNS} FROM users WHERE email = ?1 COLLATE NOCASE")
    } else {
        format!("SELECT {USER_COLUMNS} FROM users WHERE username = ?1 COLLATE NOCASE")
    };
    conn.query_row(&sql, [identifier], user_from_row).optional()
}

fn find_by_id(conn: &Connection, user_id: &str) -> rusqlite::Result<Option<UserRow>> {
    let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1");
    conn.query_row(&sql, [user_id], user_from_row).optional()
}

fn active_devices(conn: &Connection, user_id: &str) -> rusqlite::Result<Vec<DeviceInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, created_at, last_seen_at FROM devices
         WHERE user_id = ?1 AND revoked_at IS NULL ORDER BY created_at, id",
    )?;
    stmt.query_map([user_id], |row| {
        Ok(DeviceInfo {
            id: row.get(0)?,
            name: row.get(1)?,
            created_at: row.get(2)?,
            last_seen_at: row.get(3)?,
        })
    })?
    .collect()
}

fn revoke_device_rows(
    conn: &Connection,
    user_id: &str,
    device_id: &str,
    now: &str,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE devices SET revoked_at = ?3 WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL",
        params![device_id, user_id, now],
    )?;
    // Solo le chiavi di un dispositivo di questo utente: mai quelle di un altro.
    conn.execute(
        "UPDATE device_keys SET revoked_at = ?3
         WHERE device_id = ?1 AND revoked_at IS NULL
           AND device_id IN (SELECT id FROM devices WHERE id = ?1 AND user_id = ?2)",
        params![device_id, user_id, now],
    )?;
    Ok(changed > 0)
}

/// Il posto prenotato da una verifica della password attuale nella serie della
/// chiave (`wrong_password_streak`).
///
/// Il posto si prende prima di argon2 e resta occupato solo se la verifica finisce
/// con una password sbagliata ([`StreakSlot::keep`]) o se l'operazione con la
/// password giusta è stata applicata (la sua transazione azzera la serie). In ogni
/// altro caso torna libero: server occupato, errore interno, hash non riuscito,
/// utente cambiato nel frattempo, e anche richiesta interrotta (tempo scaduto o
/// client scollegato), grazie al `Drop`.
///
/// Prenotazione e rilascio passano entrambi dal blocco della connessione e
/// guardano lo stesso stato: un rilascio arrivato prima che la prenotazione sia
/// partita la annulla, così nessun posto resta occupato per sbaglio.
pub(crate) struct StreakSlot {
    db: Db,
    key_sha256: String,
    state: Arc<SlotState>,
    armed: bool,
}

#[derive(Default)]
struct SlotState {
    reserved: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
}

impl StreakSlot {
    fn new(db: Db, key_sha256: String) -> Self {
        StreakSlot {
            db,
            key_sha256,
            state: Arc::new(SlotState::default()),
            armed: true,
        }
    }

    /// Prende il posto, se la chiave non è revocata e la serie è sotto il limite.
    async fn reserve(&self) -> Result<bool, ServerError> {
        let key = self.key_sha256.clone();
        let state = Arc::clone(&self.state);
        self.db
            .blocking(move |conn| {
                if state.released.load(Ordering::SeqCst) {
                    return Ok(false);
                }
                let changed = conn.execute(
                    "UPDATE device_keys SET wrong_password_streak = wrong_password_streak + 1
                     WHERE key_sha256 = ?1 AND revoked_at IS NULL AND wrong_password_streak < ?2",
                    params![key, WRONG_PASSWORD_STREAK_LIMIT],
                )?;
                state.reserved.store(changed > 0, Ordering::SeqCst);
                Ok(changed > 0)
            })
            .await
    }

    /// Il posto resta occupato (password sbagliata, o serie già azzerata dalla
    /// transazione dell'operazione).
    fn keep(mut self) {
        self.armed = false;
    }

    /// Libera il posto.
    async fn release(mut self) {
        self.armed = false;
        let _ = Self::release_in(&self.db, &self.key_sha256, &self.state).await;
    }

    /// Dopo l'operazione con la password giusta: applicata, il posto è già stato
    /// azzerato nella sua transazione; non applicata, si libera.
    async fn settle(self, applied: &Result<(), ServerError>) {
        if applied.is_ok() {
            self.keep();
        } else {
            self.release().await;
        }
    }

    async fn release_in(db: &Db, key: &str, state: &Arc<SlotState>) -> Result<(), ServerError> {
        let key = key.to_owned();
        let state = Arc::clone(state);
        db.blocking(move |conn| {
            state.released.store(true, Ordering::SeqCst);
            if state.reserved.swap(false, Ordering::SeqCst) {
                conn.execute(
                    "UPDATE device_keys SET wrong_password_streak = MAX(wrong_password_streak - 1, 0)
                     WHERE key_sha256 = ?1 AND revoked_at IS NULL",
                    [&key],
                )?;
            }
            Ok(())
        })
        .await
    }
}

impl Drop for StreakSlot {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Richiesta interrotta: il rilascio continua in un'attività a parte.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let db = self.db.clone();
            let key = self.key_sha256.clone();
            let state = Arc::clone(&self.state);
            handle.spawn(async move {
                let _ = StreakSlot::release_in(&db, &key, &state).await;
            });
        }
    }
}

/// Chiave del rallentamento per un utente noto.
fn throttle_user_key(user_id: &str) -> String {
    format!("u:{user_id}")
}

/// Un nome breve e senza segreti per un identificativo che non è di nessun utente (gli
/// eventi di controllo non riportano mai ciò che la persona ha scritto): i primi 8 caratteri
/// esadecimali dello SHA-256 dell'identificativo normalizzato.
fn short_hash(identifier: &str) -> String {
    let digest = Sha256::digest(normalize_identifier(identifier).as_bytes());
    hex::encode(&digest[..4])
}

fn key_sha256(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

fn new_device_key() -> String {
    format!(
        "{DEVICE_KEY_PREFIX}{}",
        hex::encode(password::random_bytes_32())
    )
}

fn valid_key_format(key: &str) -> bool {
    key.strip_prefix(DEVICE_KEY_PREFIX).is_some_and(|hex_part| {
        hex_part.len() == 64
            && hex_part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn validate_identifier(identifier: &str) -> Result<(), ServerError> {
    let trimmed = identifier.trim();
    if trimmed.is_empty() {
        return Err(ServerError::InvalidRequest(
            "Indica username o email.".to_owned(),
        ));
    }
    if trimmed.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(ServerError::InvalidRequest(
            "Username o email troppo lunghi.".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_device_name(name: &str) -> Result<String, ServerError> {
    let trimmed = name.trim();
    let chars = trimmed.chars().count();
    if chars == 0 || chars > MAX_DEVICE_NAME_CHARS || trimmed.chars().any(char::is_control) {
        return Err(ServerError::InvalidRequest(format!(
            "Il nome del dispositivo deve avere da 1 a {MAX_DEVICE_NAME_CHARS} caratteri."
        )));
    }
    Ok(trimmed.to_owned())
}

pub fn validate_new_password(new_password: &str) -> Result<(), ServerError> {
    password::validate(new_password).map_err(|e| match e {
        password::PasswordPolicyError::TooCommon => ServerError::Extension {
            status: axum::http::StatusCode::BAD_REQUEST,
            code: "password_too_common",
            message: e.to_string(),
            retry_after_s: None,
            extra: None,
        },
        _ => ServerError::InvalidRequest(e.to_string()),
    })
}

/// Username: 1–64 caratteri tra lettere senza accenti, cifre, `.`, `_`, `-`.
pub fn validate_username(username: &str) -> Result<String, ServerError> {
    let trimmed = username.trim();
    if trimmed.contains('@') {
        return Err(ServerError::InvalidRequest(
            "Lo username non può contenere @.".to_owned(),
        ));
    }
    let ok = !trimmed.is_empty()
        && trimmed.chars().count() <= MAX_USERNAME_CHARS
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok {
        return Err(ServerError::InvalidRequest(format!(
            "Lo username deve avere da 1 a {MAX_USERNAME_CHARS} caratteri tra lettere senza accenti, cifre, punto, trattino e trattino basso."
        )));
    }
    Ok(trimmed.to_owned())
}

/// Email: solo un indirizzo nudo (`addr-spec`), in ASCII, e normalizzato una volta
/// per tutte in minuscolo. Questa forma vale per tutto: quota e chiave dei codici,
/// destinatario della posta, riga dell'utente. Così l'indice `NOCASE` di SQLite
/// (che confronta solo l'ASCII) e le chiavi dei codici sono d'accordo.
///
/// Rifiutati: nome visualizzato, parentesi angolari, commenti, virgolette, spazi,
/// caratteri non ASCII (un dominio internazionale va scritto in punycode, `xn--…`),
/// punti all'inizio, alla fine o doppi nella parte locale, domini senza punto o con
/// etichette non valide.
pub fn validate_email(email: &str) -> Result<String, ServerError> {
    let trimmed = email.trim();
    let invalid = || ServerError::InvalidRequest("Email non valida.".to_owned());
    if trimmed.len() > MAX_EMAIL_CHARS || !trimmed.is_ascii() {
        return Err(invalid());
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(invalid());
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~.-".contains(&b));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = domain.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !local_ok || !domain_ok {
        return Err(invalid());
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// I campi di un utente nuovo (vedi [`Accounts::insert_user`]).
pub struct NewUser<'a> {
    pub username: &'a str,
    pub email: Option<&'a str>,
    pub password_hash: Option<&'a str>,
    pub created_at: &'a str,
}

/// Servizio degli account.
pub struct Accounts {
    db: Db,
    clock: Arc<dyn Clock>,
    default_max_devices: u32,
    temporary_password_days: u32,
    device_inactive_days: u32,
    login_limiter: FailureLimiter,
    /// Chiavi di dispositivo sbagliate per origine sulle rotte dell'account (come sul calcolo).
    device_auth_limiter: FailureLimiter,
    throttle: LoginThrottle,
    argon2_slots: Arc<Semaphore>,
    argon2_verifications: AtomicU64,
    observers: RwLock<Vec<Arc<dyn AccountObserver>>>,
    deletion_guard: RwLock<Option<DeletionGuard>>,
}

impl Accounts {
    pub fn new(db: Db, config: &Config, clock: Arc<dyn Clock>) -> Self {
        Accounts {
            db,
            clock,
            default_max_devices: config.default_max_devices,
            temporary_password_days: config.temporary_password_days,
            device_inactive_days: config.device_inactive_days,
            login_limiter: FailureLimiter::new(
                config.login_max_failures_per_origin,
                Duration::from_secs(config.login_failure_window_secs),
            ),
            device_auth_limiter: FailureLimiter::new(
                config.compute_max_auth_failures_per_origin,
                crate::compute::DEVICE_AUTH_WINDOW,
            ),
            throttle: LoginThrottle::new(),
            argon2_slots: Arc::new(Semaphore::new(ARGON2_SLOTS)),
            argon2_verifications: AtomicU64::new(0),
            observers: RwLock::new(Vec::new()),
            deletion_guard: RwLock::new(None),
        }
    }

    /// Aggiunge chi viene avvisato quando un utente è cancellato o un dispositivo revocato.
    pub fn add_observer(&self, observer: Arc<dyn AccountObserver>) {
        self.observers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(observer);
    }

    /// Chi può fermare la cancellazione di un utente (l'enterprise, per gli abbonamenti).
    pub fn set_deletion_guard(&self, guard: DeletionGuard) {
        *self
            .deletion_guard
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(guard);
    }

    async fn check_deletion(&self, user_id: &str) -> Result<(), ServerError> {
        let guard = self
            .deletion_guard
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        match guard {
            Some(guard) => guard(user_id.to_owned()).await,
            None => Ok(()),
        }
    }

    fn notify(&self, call: impl Fn(&dyn AccountObserver)) {
        let observers = self
            .observers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for observer in observers.iter() {
            call(observer.as_ref());
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Verifiche argon2 fatte da questo servizio (per i test e la diagnostica).
    pub fn argon2_verifications(&self) -> u64 {
        self.argon2_verifications.load(Ordering::SeqCst)
    }

    /// I posti per argon2: i test li occupano per provare `server_busy`.
    pub fn argon2_semaphore(&self) -> &Arc<Semaphore> {
        &self.argon2_slots
    }

    pub fn login_limiter(&self) -> &FailureLimiter {
        &self.login_limiter
    }

    /// Il limite per origine sulle chiavi di dispositivo sbagliate delle rotte dell'account.
    pub fn device_auth_limiter(&self) -> &FailureLimiter {
        &self.device_auth_limiter
    }

    pub fn throttle(&self) -> &LoginThrottle {
        &self.throttle
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    fn temporary_expiry(&self) -> String {
        timestamp(self.now() + chrono::TimeDelta::days(i64::from(self.temporary_password_days)))
    }

    async fn argon2_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, ServerError> {
        match tokio::time::timeout(ARGON2_WAIT, Arc::clone(&self.argon2_slots).acquire_owned())
            .await
        {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(ServerError::internal("semaforo argon2 chiuso")),
            Err(_) => Err(ServerError::ServerBusy {
                retry_after_s: ARGON2_WAIT.as_secs(),
            }),
        }
    }

    async fn argon2_verify(
        &self,
        hash: Option<String>,
        password: &str,
    ) -> Result<bool, ServerError> {
        let permit = self.argon2_permit().await?;
        self.argon2_verifications.fetch_add(1, Ordering::SeqCst);
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            password::verify(hash.as_deref(), &password)
        })
        .await
        .map_err(|e| ServerError::internal(format!("verifica argon2 interrotta: {e}")))
    }

    pub async fn argon2_hash(&self, password: &str) -> Result<String, ServerError> {
        let permit = self.argon2_permit().await?;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            password::hash(&password)
        })
        .await
        .map_err(|e| ServerError::internal(format!("hash argon2 interrotto: {e}")))?
        .map_err(ServerError::internal)
    }

    /// Chiave del rallentamento: l'id dell'utente se l'identificativo è di un
    /// utente (username ed email finiscono nella stessa voce), altrimenti
    /// l'identificativo normalizzato.
    fn throttle_key(user: Option<&UserRow>, identifier: &str) -> String {
        match user {
            Some(user) => throttle_user_key(&user.id),
            None => format!("i:{}", normalize_identifier(identifier)),
        }
    }

    /// La chiave del rallentamento per questo identificativo (per i test e la
    /// diagnostica).
    pub async fn throttle_key_for(&self, identifier: &str) -> Result<String, ServerError> {
        let lookup = identifier.to_owned();
        let user = self
            .db
            .blocking(move |conn| Ok(find_by_identifier(conn, &lookup)?))
            .await?;
        Ok(Self::throttle_key(user.as_ref(), identifier))
    }

    /// Azzera il rallentamento di un utente (per esempio dopo il recupero della password).
    pub fn clear_throttle(&self, user_id: &str) {
        self.throttle.succeeded(&throttle_user_key(user_id));
    }

    /// Controllo comune delle rotte con identificativo e password: limite per
    /// origine, turno prenotato, una verifica argon2, stessa risposta per tutti i
    /// casi in cui non si entra. Accetta la provvisoria valida.
    ///
    /// Il tentativo conta come in corso fin dalla prenotazione, sia nel limite per
    /// origine sia nel rallentamento: richieste mandate insieme si mettono in fila
    /// e non superano la soglia prima che argon2 abbia finito.
    async fn check_credentials(
        &self,
        origin: IpAddr,
        identifier: &str,
        password: &str,
    ) -> Result<UserRow, ServerError> {
        let attempt = self.login_limiter.begin(origin, Instant::now())?;

        // La stessa lettura per identificativi noti e sconosciuti: nessuna
        // differenza di tempo tra i due casi.
        let lookup = identifier.to_owned();
        let user = self
            .db
            .blocking(move |conn| Ok(find_by_identifier(conn, &lookup)?))
            .await?;
        // Tetto di tentativi in corso per account: da molte origini non si indovina una
        // password e non si occupano i posti di argon2 (`rate_limited`, senza verifica).
        let ticket = self.throttle.try_begin(
            &Self::throttle_key(user.as_ref(), identifier),
            Instant::now(),
        )?;
        tokio::time::sleep_until(ticket.start()).await;

        // Dopo l'attesa si rilegge l'utente: può essere cambiato nel frattempo.
        let lookup = identifier.to_owned();
        let user = self
            .db
            .blocking(move |conn| Ok(find_by_identifier(conn, &lookup)?))
            .await?;
        let hash = user.as_ref().and_then(|u| u.password_hash.clone());
        let matches = self.argon2_verify(hash, password).await?;
        let now = self.now();
        match user {
            Some(user) if matches && user.usable(now) => {
                ticket.succeeded();
                attempt.succeeded();
                Ok(user)
            }
            other => {
                let at = Instant::now();
                attempt.failed(at);
                ticket.failed(at);
                match other {
                    Some(user) => tracing::warn!(
                        target: "audit", event = "login_failed", user_id = %user.id, origin = %origin
                    ),
                    None => tracing::warn!(
                        target: "audit", event = "login_failed", identifier_hash = %short_hash(identifier), origin = %origin
                    ),
                }
                Err(ServerError::InvalidCredentials)
            }
        }
    }

    /// Dentro una transazione: l'utente esiste, è attivo e ha ancora l'hash
    /// verificato (nessun cambio concorrente). Restituisce il suo massimo.
    fn lock_user(
        conn: &Connection,
        user_id: &str,
        expected_hash: &str,
    ) -> Result<Option<u32>, ServerError> {
        let row: Option<(Option<String>, Option<i64>, Option<String>)> = conn
            .query_row(
                "SELECT password_hash, max_devices, disabled_at FROM users WHERE id = ?1",
                [user_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match row {
            Some((Some(hash), max, None)) if hash == expected_hash => {
                Ok(max.map(|m| m.clamp(0, i64::from(u32::MAX)) as u32))
            }
            _ => Err(ServerError::InvalidCredentials),
        }
    }

    /// Registra un utente nella transazione di chi chiama (nasce con `password_temporary = 0`);
    /// senza hash non può entrare con la password. Restituisce l'id. Nessun controllo di
    /// unicità: lo fa chi chiama, nella stessa transazione.
    pub fn insert_user(
        tx: &rusqlite::Transaction<'_>,
        user: NewUser<'_>,
    ) -> rusqlite::Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO users (id, username, email, password_hash, password_temporary, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![id, user.username, user.email, user.password_hash, user.created_at],
        )?;
        Ok(id)
    }

    /// Nuova password definitiva: cancella lo stato di provvisoria.
    pub fn set_password(
        tx: &rusqlite::Transaction<'_>,
        user_id: &str,
        password_hash: &str,
        _now: &str,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "UPDATE users SET password_hash = ?2, password_temporary = 0,
             temporary_expires_at = NULL WHERE id = ?1",
            params![user_id, password_hash],
        )?;
        Ok(())
    }

    /// Revoca tutti i dispositivi attivi dell'utente e le loro chiavi, dentro la transazione di
    /// chi chiama; restituisce gli id dei dispositivi revocati ora. Chi chiama, dopo il commit,
    /// avvisa gli osservatori con [`Accounts::notify_devices_revoked`].
    pub fn revoke_all_devices(
        tx: &rusqlite::Transaction<'_>,
        user_id: &str,
        now: &str,
    ) -> rusqlite::Result<Vec<String>> {
        let revoked = {
            let mut stmt =
                tx.prepare("SELECT id FROM devices WHERE user_id = ?1 AND revoked_at IS NULL")?;
            stmt.query_map(params![user_id], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        tx.execute(
            "UPDATE device_keys SET revoked_at = ?2
             WHERE revoked_at IS NULL
               AND device_id IN (SELECT id FROM devices WHERE user_id = ?1)",
            params![user_id, now],
        )?;
        tx.execute(
            "UPDATE devices SET revoked_at = ?2 WHERE user_id = ?1 AND revoked_at IS NULL",
            params![user_id, now],
        )?;
        Ok(revoked)
    }

    /// Le righe di un dispositivo revocato (solo per i test).
    #[doc(hidden)]
    pub fn revoke_device_rows(
        conn: &Connection,
        user_id: &str,
        device_id: &str,
        now: &str,
    ) -> rusqlite::Result<bool> {
        revoke_device_rows(conn, user_id, device_id, now)
    }

    /// Avvisa gli osservatori che questi dispositivi sono stati revocati (dopo il commit di
    /// [`Accounts::revoke_all_devices`]).
    pub fn notify_devices_revoked(&self, device_ids: &[String]) {
        for id in device_ids {
            self.notify(|o| o.device_revoked(id));
        }
    }

    /// L'utente con questa email (maiuscole non contano): `(id, disattivato)`.
    pub fn user_by_email(
        tx: &rusqlite::Transaction<'_>,
        email: &str,
    ) -> rusqlite::Result<Option<(String, bool)>> {
        tx.query_row(
            "SELECT id, disabled_at IS NOT NULL FROM users WHERE email = ?1 COLLATE NOCASE",
            [email],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    }

    /// Uno username già preso (maiuscole non contano).
    pub fn username_taken(conn: &Connection, username: &str) -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE username = ?1 COLLATE NOCASE)",
            [username],
            |r| r.get(0),
        )
    }

    /// Username e massimo di dispositivi di un utente attivo; `None` se manca o è disattivato.
    pub fn active_user_for_device(
        tx: &rusqlite::Transaction<'_>,
        user_id: &str,
    ) -> rusqlite::Result<Option<(String, Option<u32>)>> {
        let row: Option<(String, Option<i64>, Option<String>)> = tx
            .query_row(
                "SELECT username, max_devices, disabled_at FROM users WHERE id = ?1",
                [user_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((username, max, None)) => Some((
                username,
                max.map(|m| m.clamp(0, i64::from(u32::MAX)) as u32),
            )),
            _ => None,
        })
    }

    /// Registra dispositivo e chiave, con il controllo del massimo nella stessa
    /// transazione del chiamante.
    pub fn insert_device(
        conn: &rusqlite::Transaction<'_>,
        user_id: &str,
        max_devices: u32,
        device_name: &str,
        now: &str,
    ) -> Result<(String, String), ServerError> {
        if max_devices > 0 {
            let active: i64 = conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE user_id = ?1 AND revoked_at IS NULL",
                [user_id],
                |r| r.get(0),
            )?;
            if active >= i64::from(max_devices) {
                return Err(ServerError::DeviceLimit(active_devices(conn, user_id)?));
            }
        }
        let device_id = uuid::Uuid::new_v4().to_string();
        let key = new_device_key();
        conn.execute(
            "INSERT INTO devices (id, user_id, name, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![device_id, user_id, device_name, now],
        )?;
        conn.execute(
            "INSERT INTO device_keys (key_sha256, device_id, created_at) VALUES (?1, ?2, ?3)",
            params![key_sha256(&key), device_id, now],
        )?;
        Ok((device_id, key))
    }

    /// `POST /v1/login`.
    pub async fn login(
        &self,
        origin: IpAddr,
        identifier: &str,
        password: &str,
        device_name: &str,
    ) -> Result<LoginOk, ServerError> {
        validate_identifier(identifier)?;
        let device_name = validate_device_name(device_name)?;
        let user = self.check_credentials(origin, identifier, password).await?;
        if user.temporary {
            return Err(ServerError::PasswordChangeRequired);
        }
        let default_max = self.default_max_devices;
        let now = timestamp(self.now());
        self.db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let hash = user.password_hash.clone().unwrap_or_default();
                let max = Self::lock_user(&tx, &user.id, &hash)?.unwrap_or(default_max);
                let (device_id, device_key) =
                    Self::insert_device(&tx, &user.id, max, &device_name, &now)?;
                tx.commit()?;
                Ok(LoginOk {
                    user_id: user.id,
                    username: user.username,
                    device_id,
                    device_key,
                })
            })
            .await
    }

    /// `POST /v1/login/password`: sostituisce la provvisoria e registra il primo
    /// dispositivo nella stessa transazione.
    pub async fn change_temporary_password(
        &self,
        origin: IpAddr,
        identifier: &str,
        temporary_password: &str,
        new_password: &str,
        device_name: &str,
    ) -> Result<LoginOk, ServerError> {
        validate_identifier(identifier)?;
        let device_name = validate_device_name(device_name)?;
        validate_new_password(new_password)?;
        let user = self
            .check_credentials(origin, identifier, temporary_password)
            .await?;
        if !user.temporary {
            return Err(ServerError::InvalidRequest(
                "La password non è provvisoria: accedi con la tua password.".to_owned(),
            ));
        }
        if new_password == temporary_password {
            return Err(ServerError::InvalidRequest(
                "La nuova password deve essere diversa da quella provvisoria.".to_owned(),
            ));
        }
        let new_hash = self.argon2_hash(new_password).await?;
        let default_max = self.default_max_devices;
        let now = timestamp(self.now());
        self.db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let old_hash = user.password_hash.clone().unwrap_or_default();
                let max = Self::lock_user(&tx, &user.id, &old_hash)?.unwrap_or(default_max);
                tx.execute(
                    "UPDATE users SET password_hash = ?2, password_temporary = 0,
                     temporary_expires_at = NULL WHERE id = ?1",
                    params![user.id, new_hash],
                )?;
                let (device_id, device_key) =
                    Self::insert_device(&tx, &user.id, max, &device_name, &now)?;
                tx.commit()?;
                tracing::info!(
                    target: "audit", event = "password_changed", user_id = %user.id, device_id = %device_id, temporary = true
                );
                Ok(LoginOk {
                    user_id: user.id,
                    username: user.username,
                    device_id,
                    device_key,
                })
            })
            .await
    }

    /// `POST /v1/login/disable-device`: con la password (anche provvisoria)
    /// disattiva un dispositivo dello stesso utente.
    pub async fn disable_device_with_password(
        &self,
        origin: IpAddr,
        identifier: &str,
        password: &str,
        device_id: &str,
    ) -> Result<(), ServerError> {
        validate_identifier(identifier)?;
        let user = self.check_credentials(origin, identifier, password).await?;
        self.revoke_user_device(&user.id, device_id, "user_password").await
    }

    /// Autentica la chiave di un dispositivo: chiave e dispositivo non revocati,
    /// utente non disattivato.
    /// La chiave con questo SHA-256 è ancora di un dispositivo attivo di un utente attivo
    /// (richieste rimaste in coda mentre l'amministratore la revocava, EX-15). Un errore del
    /// database conta come no.
    pub async fn device_active(&self, key_sha256: &str) -> bool {
        let hash = key_sha256.to_owned();
        self.db
            .blocking(move |conn| {
                let found: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM device_keys k
                         JOIN devices d ON d.id = k.device_id
                         JOIN users u ON u.id = d.user_id
                         WHERE k.key_sha256 = ?1 AND k.revoked_at IS NULL
                           AND d.revoked_at IS NULL AND u.disabled_at IS NULL",
                        [&hash],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(found.is_some())
            })
            .await
            .unwrap_or(false)
    }

    pub async fn authenticate(&self, key: &str) -> Result<AuthContext, ServerError> {
        if !valid_key_format(key) {
            return Err(ServerError::Unauthorized);
        }
        let hash = key_sha256(key);
        let now = self.now();
        let inactive_days = self.device_inactive_days;
        let (result, expired) = self
            .db
            .blocking(move |conn| {
                let row: Option<(String, String, Option<String>, String)> = conn
                    .query_row(
                        "SELECT d.id, d.user_id, d.last_seen_at, d.created_at
                         FROM device_keys k
                         JOIN devices d ON d.id = k.device_id
                         JOIN users u ON u.id = d.user_id
                         WHERE k.key_sha256 = ?1 AND k.revoked_at IS NULL
                           AND d.revoked_at IS NULL AND u.disabled_at IS NULL",
                        [&hash],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                let Some((device_id, user_id, last_seen, created)) = row else {
                    return Ok((Err(ServerError::Unauthorized), None));
                };
                // Una chiave non usata da troppo tempo non vale più: il dispositivo si
                // revoca e si accede di nuovo.
                if inactive_days > 0 {
                    let since = last_seen
                        .as_deref()
                        .and_then(parse_timestamp)
                        .or_else(|| parse_timestamp(&created));
                    let limit = chrono::TimeDelta::days(i64::from(inactive_days));
                    if since.is_none_or(|since| now - since >= limit) {
                        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                        revoke_device_rows(&tx, &user_id, &device_id, &timestamp(now))?;
                        tx.commit()?;
                        return Ok((Err(ServerError::Unauthorized), Some(device_id)));
                    }
                }
                let stale = last_seen
                    .as_deref()
                    .and_then(parse_timestamp)
                    .is_none_or(|seen| now - seen >= LAST_SEEN_INTERVAL);
                if stale {
                    conn.execute(
                        "UPDATE devices SET last_seen_at = ?2 WHERE id = ?1",
                        params![device_id, timestamp(now)],
                    )?;
                }
                Ok((
                    Ok(AuthContext {
                        user_id,
                        device_id,
                        key_sha256: hash,
                    }),
                    None,
                ))
            })
            .await?;
        if let Some(device_id) = expired {
            tracing::info!(target: "audit", event = "device_revoked", device_id = %device_id, by = "inactivity");
            self.notify_devices_revoked(&[device_id]);
        }
        result
    }

    /// Verifica la password attuale per una rotta con chiave. Ogni tentativo
    /// prenota un posto nella serie della chiave **prima** di argon2, con un solo
    /// `UPDATE` condizionato: una raffica non fa più di dieci verifiche per chiave.
    /// Un errore conta anche nel limite per origine; al decimo di fila la chiave (e
    /// il suo dispositivo) si revoca. Un successo non azzera la serie: lo fa la
    /// transazione che applica l'operazione, dopo aver ricontrollato la chiave
    /// ([`Accounts::lock_key`]).
    async fn verify_current_password(
        &self,
        origin: IpAddr,
        auth: &AuthContext,
        password: &str,
    ) -> Result<(String, StreakSlot), ServerError> {
        let attempt = self.login_limiter.begin(origin, Instant::now())?;
        let slot = StreakSlot::new(self.db.clone(), auth.key_sha256.clone());
        if !slot.reserve().await? {
            // Chiave revocata o dieci verifiche già in corso o fallite: nessuna
            // verifica in più. Il tentativo non conta come fallimento dell'origine.
            slot.keep();
            drop(attempt);
            return Err(ServerError::InvalidCredentials);
        }

        let user_id = auth.user_id.clone();
        let checked = async {
            let user = self
                .db
                .blocking(move |conn| Ok(find_by_id(conn, &user_id)?))
                .await?;
            let hash = user.as_ref().and_then(|u| u.password_hash.clone());
            let matches = self.argon2_verify(hash.clone(), password).await?;
            Ok::<_, ServerError>((user, hash, matches))
        }
        .await;
        let (user, hash, matches) = match checked {
            Ok(checked) => checked,
            Err(error) => {
                // Nessuna verifica conclusa (server occupato, errore interno): il
                // posto nella serie torna libero.
                slot.release().await;
                return Err(error);
            }
        };
        let now = self.now();
        if matches && user.as_ref().is_some_and(|u| u.usable(now)) {
            attempt.succeeded();
            return Ok((hash.unwrap_or_default(), slot));
        }
        // Password sbagliata: il posto resta occupato, è un errore della serie.
        slot.keep();
        attempt.failed(Instant::now());
        tracing::warn!(
            target: "audit", event = "password_check_failed", user_id = %auth.user_id, device_id = %auth.device_id, origin = %origin
        );
        let key_hash = auth.key_sha256.clone();
        let device_id = auth.device_id.clone();
        let user_id = auth.user_id.clone();
        let auth_user = auth.user_id.clone();
        let now_text = timestamp(now);
        let revoked_device = self
            .db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut revoked = None;
                let streak: i64 = tx
                    .query_row(
                        "SELECT wrong_password_streak FROM device_keys WHERE key_sha256 = ?1",
                        [&key_hash],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                if streak >= WRONG_PASSWORD_STREAK_LIMIT {
                    tx.execute(
                        "UPDATE device_keys SET revoked_at = ?2 WHERE key_sha256 = ?1 AND revoked_at IS NULL",
                        params![key_hash, now_text],
                    )?;
                    if revoke_device_rows(&tx, &user_id, &device_id, &now_text)? {
                        revoked = Some(device_id);
                    }
                }
                tx.commit()?;
                Ok(revoked)
            })
            .await?;
        // Dopo il commit, come ogni altra revoca.
        if let Some(device_id) = revoked_device {
            tracing::warn!(
                target: "audit", event = "device_revoked", user_id = %auth_user, device_id = %device_id, by = "wrong_password_streak"
            );
            self.notify_devices_revoked(&[device_id]);
        }
        Err(ServerError::InvalidCredentials)
    }

    /// Dentro la transazione che applica un'operazione con password: la chiave e il
    /// dispositivo non sono stati revocati nel frattempo (per esempio da una serie
    /// di password sbagliate in parallelo) e la serie è sotto il limite. Poi la
    /// serie si azzera.
    fn lock_key(conn: &Connection, auth: &AuthContext) -> Result<(), ServerError> {
        let streak: Option<i64> = conn
            .query_row(
                "SELECT k.wrong_password_streak FROM device_keys k
                 JOIN devices d ON d.id = k.device_id
                 WHERE k.key_sha256 = ?1 AND k.revoked_at IS NULL AND d.revoked_at IS NULL",
                [&auth.key_sha256],
                |r| r.get(0),
            )
            .optional()?;
        match streak {
            // La serie comprende il posto prenotato da questa richiesta.
            Some(streak) if streak <= WRONG_PASSWORD_STREAK_LIMIT => {
                conn.execute(
                    "UPDATE device_keys SET wrong_password_streak = 0 WHERE key_sha256 = ?1",
                    [&auth.key_sha256],
                )?;
                Ok(())
            }
            _ => Err(ServerError::Unauthorized),
        }
    }

    /// `PUT /v1/account/password`.
    pub async fn change_password(
        &self,
        origin: IpAddr,
        auth: &AuthContext,
        current_password: &str,
        new_password: &str,
        sign_out_other_devices: bool,
    ) -> Result<(), ServerError> {
        validate_new_password(new_password)?;
        let (old_hash, slot) = self
            .verify_current_password(origin, auth, current_password)
            .await?;
        let applied = self
            .apply_password_change(auth, &old_hash, new_password, sign_out_other_devices)
            .await;
        slot.settle(&applied).await;
        applied
    }

    async fn apply_password_change(
        &self,
        auth: &AuthContext,
        old_hash: &str,
        new_password: &str,
        sign_out_other_devices: bool,
    ) -> Result<(), ServerError> {
        let new_hash = self.argon2_hash(new_password).await?;
        let (audit_user, audit_device) = (auth.user_id.clone(), auth.device_id.clone());
        let auth = auth.clone();
        let old_hash = old_hash.to_owned();
        let now = timestamp(self.now());
        let revoked = self
            .db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                Self::lock_user(&tx, &auth.user_id, &old_hash)?;
                Self::lock_key(&tx, &auth)?;
                tx.execute(
                    "UPDATE users SET password_hash = ?2, password_temporary = 0,
                     temporary_expires_at = NULL WHERE id = ?1",
                    params![auth.user_id, new_hash],
                )?;
                let mut revoked = Vec::new();
                if sign_out_other_devices {
                    let others: Vec<String> = {
                        let mut stmt = tx.prepare(
                            "SELECT id FROM devices WHERE user_id = ?1 AND id <> ?2 AND revoked_at IS NULL",
                        )?;
                        stmt.query_map(params![auth.user_id, auth.device_id], |r| r.get(0))?
                            .collect::<rusqlite::Result<_>>()?
                    };
                    for device_id in others {
                        if revoke_device_rows(&tx, &auth.user_id, &device_id, &now)? {
                            revoked.push(device_id);
                        }
                    }
                }
                tx.commit()?;
                Ok(revoked)
            })
            .await?;
        tracing::info!(
            target: "audit", event = "password_changed", user_id = %audit_user, device_id = %audit_device, others_revoked = revoked.len()
        );
        self.notify_devices_revoked(&revoked);
        Ok(())
    }

    /// `DELETE /v1/devices/self`.
    pub async fn revoke_own_device(&self, auth: &AuthContext) -> Result<(), ServerError> {
        self.revoke_device(auth, &auth.device_id.clone()).await
    }

    /// `DELETE /v1/account/devices/{id}`: un dispositivo dello stesso utente.
    pub async fn revoke_device(
        &self,
        auth: &AuthContext,
        device_id: &str,
    ) -> Result<(), ServerError> {
        self.revoke_user_device(&auth.user_id, device_id, "user").await
    }

    async fn revoke_user_device(
        &self,
        user_id: &str,
        device_id: &str,
        by: &'static str,
    ) -> Result<(), ServerError> {
        let owned_user_id = user_id.to_owned();
        let owned_device_id = device_id.to_owned();
        let now = timestamp(self.now());
        self.db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if !revoke_device_rows(&tx, &owned_user_id, &owned_device_id, &now)? {
                    return Err(device_not_found());
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        tracing::info!(target: "audit", event = "device_revoked", user_id = %user_id, device_id = %device_id, by = by);
        self.notify(|o| o.device_revoked(device_id));
        Ok(())
    }

    /// Dispositivi attivi dell'utente.
    pub async fn list_devices(&self, user_id: &str) -> Result<Vec<DeviceInfo>, ServerError> {
        let user_id = user_id.to_owned();
        self.db
            .blocking(move |conn| {
                if find_by_id(conn, &user_id)?.is_none() {
                    return Err(user_not_found());
                }
                Ok(active_devices(conn, &user_id)?)
            })
            .await
    }

    /// `DELETE /v1/account`: cancella utente, dispositivi e chiavi.
    pub async fn delete_account(
        &self,
        origin: IpAddr,
        auth: &AuthContext,
        password: &str,
    ) -> Result<(), ServerError> {
        let (hash, slot) = self.verify_current_password(origin, auth, password).await?;
        if let Err(refused) = self.check_deletion(&auth.user_id).await {
            // Niente è stato applicato: il posto nella serie della chiave torna libero.
            slot.release().await;
            return Err(refused);
        }
        let auth = auth.clone();
        let user_id = auth.user_id.clone();
        let applied = self
            .db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                Self::lock_user(&tx, &auth.user_id, &hash)?;
                Self::lock_key(&tx, &auth)?;
                tx.execute("DELETE FROM users WHERE id = ?1", [&auth.user_id])?;
                tx.commit()?;
                Ok(())
            })
            .await;
        slot.settle(&applied).await;
        if applied.is_ok() {
            tracing::info!(target: "audit", event = "user_deleted", user_id = %user_id, by = "user");
            self.notify(|o| o.user_deleted(&user_id));
        }
        applied
    }

    // --- Amministrazione -------------------------------------------------------

    /// Crea un utente con una password provvisoria.
    pub async fn create_user(
        &self,
        username: &str,
        email: Option<&str>,
        max_devices: Option<u32>,
    ) -> Result<CreatedUser, ServerError> {
        let username = validate_username(username)?;
        let email = email
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(validate_email)
            .transpose()?;
        let temporary = password::temporary();
        let hash = self.argon2_hash(&temporary).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = timestamp(self.now());
        let expires = self.temporary_expiry();
        let created = CreatedUser {
            id: id.clone(),
            username: username.clone(),
            email: email.clone(),
            temporary_password: temporary,
            temporary_expires_at: expires.clone(),
        };
        self.db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let username_taken: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE username = ?1 COLLATE NOCASE)",
                    [&username],
                    |r| r.get(0),
                )?;
                if username_taken {
                    return Err(ServerError::UsernameTaken);
                }
                if let Some(email) = &email {
                    let email_taken: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM users WHERE email = ?1 COLLATE NOCASE)",
                        [email],
                        |r| r.get(0),
                    )?;
                    if email_taken {
                        return Err(ServerError::EmailTaken);
                    }
                }
                tx.execute(
                    "INSERT INTO users (id, username, email, password_hash, password_temporary,
                     temporary_expires_at, max_devices, created_at)
                     VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7)",
                    params![id, username, email, hash, expires, max_devices, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        tracing::info!(target: "audit", event = "user_created", user_id = %created.id);
        Ok(created)
    }

    /// Nuova provvisoria. Revoca tutti i dispositivi dell'utente e le loro chiavi, nella
    /// stessa transazione: chi ha rubato una chiave non la tiene dopo il recupero della
    /// password. Dopo il commit avvisa gli osservatori.
    pub async fn reset_password(&self, user_id: &str) -> Result<TemporaryPassword, ServerError> {
        let temporary = password::temporary();
        let hash = self.argon2_hash(&temporary).await?;
        let expires = self.temporary_expiry();
        let user_id = user_id.to_owned();
        let stored_expires = expires.clone();
        let user_id_for_log = user_id.clone();
        let now = timestamp(self.now());
        let revoked = self
            .db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let changed = tx.execute(
                    "UPDATE users SET password_hash = ?2, password_temporary = 1,
                     temporary_expires_at = ?3 WHERE id = ?1",
                    params![user_id, hash, stored_expires],
                )?;
                if changed == 0 {
                    return Err(user_not_found());
                }
                let revoked = Self::revoke_all_devices(&tx, &user_id, &now)?;
                tx.commit()?;
                Ok(revoked)
            })
            .await?;
        tracing::info!(
            target: "audit", event = "admin_password_reset", user_id = %user_id_for_log, devices_revoked = revoked.len()
        );
        self.notify_devices_revoked(&revoked);
        Ok(TemporaryPassword {
            temporary_password: temporary,
            temporary_expires_at: expires,
        })
    }

    /// Disattiva l'utente e revoca tutti i suoi dispositivi e le loro chiavi, nella stessa
    /// transazione: riattivarlo non fa rivivere le chiavi di prima (chi le aveva rubate non le
    /// ritrova), si accede di nuovo.
    pub async fn disable_user(&self, user_id: &str) -> Result<(), ServerError> {
        let now = timestamp(self.now());
        let user_id_for_log = user_id.to_owned();
        let user_id = user_id.to_owned();
        let revoked = self
            .db
            .blocking(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let changed = tx.execute(
                    "UPDATE users SET disabled_at = COALESCE(disabled_at, ?2) WHERE id = ?1",
                    params![user_id, now],
                )?;
                if changed == 0 {
                    return Err(user_not_found());
                }
                let revoked = Self::revoke_all_devices(&tx, &user_id, &now)?;
                tx.commit()?;
                Ok(revoked)
            })
            .await?;
        tracing::info!(target: "audit", event = "user_disabled", user_id = %user_id_for_log, devices_revoked = revoked.len());
        self.notify_devices_revoked(&revoked);
        Ok(())
    }

    pub async fn enable_user(&self, user_id: &str) -> Result<(), ServerError> {
        self.update_user(
            user_id,
            "UPDATE users SET disabled_at = NULL WHERE id = ?1",
            None,
        )
        .await?;
        tracing::info!(target: "audit", event = "user_enabled", user_id = %user_id);
        Ok(())
    }

    pub async fn delete_user(&self, user_id: &str) -> Result<(), ServerError> {
        // Un utente che non c'è è `not_found`, come prima; uno che c'è può essere protetto.
        if self.db.blocking({
            let id = user_id.to_owned();
            move |conn| Ok(find_by_id(conn, &id)?.is_some())
        }).await? {
            self.check_deletion(user_id).await?;
        }
        self.update_user(user_id, "DELETE FROM users WHERE id = ?1", None)
            .await?;
        tracing::info!(target: "audit", event = "user_deleted", user_id = %user_id, by = "admin");
        self.notify(|o| o.user_deleted(user_id));
        Ok(())
    }

    /// `None`: torna a valere `default_max_devices`. 0 = nessun limite.
    pub async fn set_max_devices(
        &self,
        user_id: &str,
        max_devices: Option<u32>,
    ) -> Result<(), ServerError> {
        let user_id = user_id.to_owned();
        self.db
            .blocking(move |conn| {
                let changed = conn.execute(
                    "UPDATE users SET max_devices = ?2 WHERE id = ?1",
                    params![user_id, max_devices],
                )?;
                if changed == 0 {
                    return Err(user_not_found());
                }
                Ok(())
            })
            .await
    }

    pub async fn revoke_device_admin(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> Result<(), ServerError> {
        let exists_id = user_id.to_owned();
        let exists = self
            .db
            .blocking(move |conn| Ok(find_by_id(conn, &exists_id)?.is_some()))
            .await?;
        if !exists {
            return Err(user_not_found());
        }
        self.revoke_user_device(user_id, device_id, "admin").await
    }

    async fn update_user(
        &self,
        user_id: &str,
        sql: &'static str,
        value: Option<String>,
    ) -> Result<(), ServerError> {
        let user_id = user_id.to_owned();
        self.db
            .blocking(move |conn| {
                let changed = match value {
                    Some(v) => conn.execute(sql, params![user_id, v])?,
                    None => conn.execute(sql, params![user_id])?,
                };
                if changed == 0 {
                    return Err(user_not_found());
                }
                Ok(())
            })
            .await
    }

    /// Aggiunge ai contatori di calcolo di `user_id` (D5). Un utente cancellato nel
    /// frattempo non lascia righe. Le somme si fermano a `i64::MAX` (come un `MIN`): SQLite
    /// altrimenti trasforma l'overflow in REAL e rompe la lettura dei contatori.
    pub async fn record_usage(&self, user_id: &str, usage: ComputeUsage) -> Result<(), ServerError> {
        let user_id = user_id.to_owned();
        let at = self.now();
        let now = timestamp(at);
        let day = at.format("%Y-%m-%d").to_string();
        let oldest = (at - chrono::TimeDelta::days(7)).format("%Y-%m-%d").to_string();
        self.db
            .blocking(move |conn| {
                let clamp = |n: u64| n.min(i64::MAX as u64) as i64;
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO compute_usage (user_id, requests, audio_seconds, embedded_tokens, updated_at,
                                                llm_prompt_tokens, llm_completion_tokens)
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM users WHERE id = ?1)
                     ON CONFLICT(user_id) DO UPDATE SET
                        requests = CASE WHEN excluded.requests > 9223372036854775807 - requests
                            THEN 9223372036854775807 ELSE requests + excluded.requests END,
                        audio_seconds = audio_seconds + excluded.audio_seconds,
                        embedded_tokens = CASE WHEN excluded.embedded_tokens > 9223372036854775807 - embedded_tokens
                            THEN 9223372036854775807 ELSE embedded_tokens + excluded.embedded_tokens END,
                        llm_prompt_tokens = CASE WHEN excluded.llm_prompt_tokens > 9223372036854775807 - llm_prompt_tokens
                            THEN 9223372036854775807 ELSE llm_prompt_tokens + excluded.llm_prompt_tokens END,
                        llm_completion_tokens = CASE WHEN excluded.llm_completion_tokens > 9223372036854775807 - llm_completion_tokens
                            THEN 9223372036854775807 ELSE llm_completion_tokens + excluded.llm_completion_tokens END,
                        updated_at = excluded.updated_at",
                    params![
                        user_id,
                        clamp(usage.requests),
                        usage.audio_seconds,
                        clamp(usage.embedded_tokens),
                        now,
                        clamp(usage.llm_prompt_tokens),
                        clamp(usage.llm_completion_tokens)
                    ],
                )?;
                let tokens = usage.llm_prompt_tokens.saturating_add(usage.llm_completion_tokens);
                if tokens > 0 {
                    tx.execute(
                        "INSERT INTO llm_usage_day (user_id, day, tokens)
                         SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM users WHERE id = ?1)
                         ON CONFLICT(user_id, day) DO UPDATE SET tokens = CASE
                            WHEN excluded.tokens > 9223372036854775807 - tokens THEN 9223372036854775807
                            ELSE tokens + excluded.tokens END",
                        params![user_id, day, clamp(tokens)],
                    )?;
                    tx.execute(
                        "DELETE FROM llm_usage_day WHERE user_id = ?1 AND day < ?2",
                        params![user_id, oldest],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// I contatori di calcolo di un utente (zero se non ha ancora fatto richieste).
    pub async fn usage(&self, user_id: &str) -> Result<ComputeUsage, ServerError> {
        let user_id = user_id.to_owned();
        self.db
            .blocking(move |conn| {
                let row = conn
                    .query_row(
                        "SELECT requests, audio_seconds, embedded_tokens, llm_prompt_tokens, llm_completion_tokens
                         FROM compute_usage WHERE user_id = ?1",
                        [&user_id],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, f64>(1)?,
                                r.get::<_, i64>(2)?,
                                r.get::<_, i64>(3)?,
                                r.get::<_, i64>(4)?,
                            ))
                        },
                    )
                    .optional()?;
                Ok(row.map_or_else(ComputeUsage::default, |(requests, audio, tokens, prompt, completion)| {
                    ComputeUsage {
                        requests: requests.max(0) as u64,
                        audio_seconds: audio.max(0.0),
                        embedded_tokens: tokens.max(0) as u64,
                        llm_prompt_tokens: prompt.max(0) as u64,
                        llm_completion_tokens: completion.max(0) as u64,
                    }
                }))
            })
            .await
    }

    /// I token dell'assistente di un utente in un giorno UTC (`YYYY-MM-DD`); zero senza riga.
    pub async fn llm_tokens_on(&self, user_id: &str, day: &str) -> Result<u64, ServerError> {
        let (user_id, day) = (user_id.to_owned(), day.to_owned());
        self.db
            .blocking(move |conn| {
                let tokens: Option<i64> = conn
                    .query_row(
                        "SELECT tokens FROM llm_usage_day WHERE user_id = ?1 AND day = ?2",
                        params![user_id, day],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(tokens.unwrap_or(0).max(0) as u64)
            })
            .await
    }

    pub async fn list_users(&self) -> Result<Vec<UserSummary>, ServerError> {
        self.db
            .blocking(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT u.id, u.username, u.email, u.password_temporary, u.temporary_expires_at,
                            u.max_devices, u.disabled_at, u.created_at,
                            (SELECT COUNT(*) FROM devices d WHERE d.user_id = u.id AND d.revoked_at IS NULL),
                            COALESCE(c.requests, 0), COALESCE(c.audio_seconds, 0), COALESCE(c.embedded_tokens, 0),
                            COALESCE(c.llm_prompt_tokens, 0), COALESCE(c.llm_completion_tokens, 0)
                     FROM users u LEFT JOIN compute_usage c ON c.user_id = u.id
                     ORDER BY u.username COLLATE NOCASE",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(UserSummary {
                            id: r.get(0)?,
                            username: r.get(1)?,
                            email: r.get(2)?,
                            password_temporary: r.get::<_, i64>(3)? != 0,
                            temporary_expires_at: r.get(4)?,
                            max_devices: r
                                .get::<_, Option<i64>>(5)?
                                .map(|m| m.clamp(0, i64::from(u32::MAX)) as u32),
                            disabled_at: r.get(6)?,
                            created_at: r.get(7)?,
                            active_devices: r.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
                            usage: ComputeUsage {
                                requests: r.get::<_, i64>(9)?.max(0) as u64,
                                audio_seconds: r.get::<_, f64>(10)?.max(0.0),
                                embedded_tokens: r.get::<_, i64>(11)?.max(0) as u64,
                                llm_prompt_tokens: r.get::<_, i64>(12)?.max(0) as u64,
                                llm_completion_tokens: r.get::<_, i64>(13)?.max(0) as u64,
                            },
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// `GET /v1/account`.
    pub async fn profile(&self, user_id: &str) -> Result<Profile, ServerError> {
        let user_id = user_id.to_owned();
        self.db
            .blocking(move |conn| {
                let row: Option<(String, Option<String>, bool)> = conn
                    .query_row(
                        "SELECT username, email, password_hash IS NOT NULL FROM users WHERE id = ?1",
                        [&user_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((username, email, has_password)) = row else {
                    return Err(user_not_found());
                };
                Ok(Profile { user_id, username, email, has_password, identities: Vec::new() })
            })
            .await
    }

    /// Id dell'utente con questo username (maiuscole e spazi ai lati non contano).
    pub async fn user_id_by_username(&self, username: &str) -> Result<String, ServerError> {
        let username = username.trim().to_owned();
        self.db
            .blocking(move |conn| {
                conn.query_row(
                    "SELECT id FROM users WHERE username = ?1 COLLATE NOCASE",
                    [&username],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(user_not_found)
            })
            .await
    }

    pub fn default_max_devices(&self) -> u32 {
        self.default_max_devices
    }
}

fn user_not_found() -> ServerError {
    ServerError::NotFound("Utente non trovato.".to_owned())
}

fn device_not_found() -> ServerError {
    ServerError::NotFound("Dispositivo non trovato.".to_owned())
}
