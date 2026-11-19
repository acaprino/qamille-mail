//! Sincronizzazione delle caselle tra i computer dello stesso utente (spec S1–S8): il server
//! conserva gli elementi cifrati con la sua chiave e li consegna ai dispositivi attivi
//! dell'utente. Non interpreta il contenuto e non usa le credenziali.

pub mod crypto;
pub mod routes;
pub mod store;

pub use crypto::{KeyError, OpenError, Sealed, SyncKey};

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::value::RawValue;

use crate::accounts::{Accounts, AuthContext};
use crate::config::{Config, ConfigError};
use crate::error::ServerError;
use crate::limiter::FailureLimiter;
use store::{DeleteOutcome, Limits, Listing, Probe, PutOutcome, Who};

/// Elementi non cancellati per utente (predefinito) e tetto del limite configurabile.
pub const DEFAULT_MAX_ITEMS: u32 = 200;
pub const MAX_ITEMS_CEILING: u32 = 10_000;
/// Dimensione massima di un elemento (predefinita), minimo e tetto del limite configurabile.
pub const DEFAULT_MAX_ITEM_BYTES: usize = 64 * 1024;
pub const MIN_ITEM_BYTES: usize = 1024;
pub const MAX_ITEM_BYTES_CEILING: usize = 1024 * 1024;
/// Tetto di `sync_max_items * sync_max_item_bytes`: `GET` apre e risponde con tutti gli elementi di
/// un utente in memoria, quindi la somma ha un limite (il predefinito, 12,5 MiB, ci sta largo).
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Margine del corpo di `PUT` sopra `sync_max_item_bytes`: l'involucro `{base_version, item}`.
pub const BODY_MARGIN: usize = 1024;
/// Giorni per cui il server tiene il segnaposto di una cancellazione.
pub const TOMBSTONE_DAYS: i64 = 90;
/// Richieste di sincronizzazione insieme per utente, su tutti i suoi dispositivi.
pub const MAX_IN_FLIGHT_PER_USER: usize = 2;
/// `retry_after_s` di un rifiuto per troppe richieste insieme.
pub const BUSY_RETRY_S: u64 = 2;

/// La configurazione della sincronizzazione. Senza `key` è spenta. Il `Debug` non stampa le
/// chiavi (lo fa già `SyncKey`).
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// `QAMILLE_SYNC_KEY`, solo dall'ambiente.
    pub key: Option<SyncKey>,
    /// `QAMILLE_SYNC_KEY_OLD`, solo per `sync rekey`.
    pub old_key: Option<SyncKey>,
    /// Elementi non cancellati per utente (`sync_max_items`).
    pub max_items: u32,
    /// Byte di un elemento (`sync_max_item_bytes`).
    pub max_item_bytes: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            key: None,
            old_key: None,
            max_items: DEFAULT_MAX_ITEMS,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
        }
    }
}

impl SyncConfig {
    /// La sincronizzazione è accesa solo con la chiave del server.
    pub fn enabled(&self) -> bool {
        self.key.is_some()
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=MAX_ITEMS_CEILING).contains(&self.max_items) {
            return Err(ConfigError::Invalid {
                key: "sync_max_items",
                reason: "deve stare tra 1 e 10000",
            });
        }
        if !(MIN_ITEM_BYTES..=MAX_ITEM_BYTES_CEILING).contains(&self.max_item_bytes) {
            return Err(ConfigError::Invalid {
                key: "sync_max_item_bytes",
                reason: "deve stare tra 1024 e 1048576",
            });
        }
        if u64::from(self.max_items) * self.max_item_bytes as u64 > MAX_TOTAL_BYTES {
            return Err(ConfigError::Invalid {
                key: "sync_max_items",
                reason: "sync_max_items per sync_max_item_bytes non può superare 64 MiB",
            });
        }
        Ok(())
    }
}

/// Ciò che `GET /v1/info` dichiara quando la sincronizzazione è accesa.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SyncInfo {
    pub available: bool,
}

/// I cancelli delle rotte: al più [`MAX_IN_FLIGHT_PER_USER`] richieste insieme per **utente**,
/// qualunque dispositivo, e un limite per origine sulle chiavi sbagliate.
pub struct SyncGate {
    max_in_flight: usize,
    in_flight: Mutex<HashMap<String, usize>>,
    pub(crate) failures: FailureLimiter,
}

/// Un posto tra le richieste in volo di un utente; si libera quando la risposta è pronta (o il
/// client se n'è andato).
pub struct Slot {
    gate: Arc<SyncGate>,
    user_id: String,
}

impl fmt::Debug for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Slot")
    }
}

impl SyncGate {
    pub fn new(max_failures: u32, window: std::time::Duration) -> Self {
        SyncGate {
            max_in_flight: MAX_IN_FLIGHT_PER_USER,
            in_flight: Mutex::new(HashMap::new()),
            failures: FailureLimiter::new(max_failures, window),
        }
    }

    /// Un posto per `user_id`, o `None` oltre il limite.
    pub fn enter(self: &Arc<Self>, user_id: &str) -> Option<Slot> {
        let mut map = self.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        let count = map.entry(user_id.to_owned()).or_insert(0);
        if *count >= self.max_in_flight {
            return None;
        }
        *count += 1;
        Some(Slot { gate: Arc::clone(self), user_id: user_id.to_owned() })
    }

    /// Richieste di `user_id` in volo adesso (per i test).
    pub fn in_flight(&self, user_id: &str) -> usize {
        self.in_flight
            .lock()
            .map(|map| map.get(user_id).copied().unwrap_or(0))
            .unwrap_or(0)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut map = self.gate.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(count) = map.get_mut(&self.user_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.user_id);
            }
        }
    }
}

/// Il servizio: chiave, limiti e accesso al database. Il contenuto degli elementi passa di qui
/// solo cifrato verso il disco e in chiaro verso il dispositivo dell'utente.
pub struct SyncService {
    accounts: Arc<Accounts>,
    key: Arc<SyncKey>,
    limits: Limits,
    gate: Arc<SyncGate>,
}

impl fmt::Debug for SyncService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncService").field("limits", &self.limits).finish_non_exhaustive()
    }
}

fn invalid_id() -> ServerError {
    ServerError::InvalidRequest("Identificativo dell'elemento non valido.".to_owned())
}

impl SyncService {
    /// `None` senza la chiave del server: la sincronizzazione è spenta.
    pub fn new(accounts: Arc<Accounts>, config: &Config) -> Option<SyncService> {
        let key = config.sync.key.clone()?;
        Some(SyncService {
            accounts,
            key: Arc::new(key),
            limits: Limits {
                max_items: config.sync.max_items,
                max_item_bytes: config.sync.max_item_bytes,
            },
            gate: Arc::new(SyncGate::new(
                config.compute_max_auth_failures_per_origin,
                crate::compute::DEVICE_AUTH_WINDOW,
            )),
        })
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn gate(&self) -> &Arc<SyncGate> {
        &self.gate
    }

    pub async fn list(&self, who: &AuthContext, since: u64) -> Result<Listing, ServerError> {
        let (key, user, device, dk) = (Arc::clone(&self.key), who.user_id.clone(), who.device_id.clone(), who.key_sha256().to_owned());
        self.accounts
            .db()
            .blocking(move |conn| {
                store::list(conn, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, since)
                    .map_err(ServerError::from)
            })
            .await
    }

    pub async fn put(
        &self,
        who: &AuthContext,
        item_id: &str,
        base_version: u64,
        item: Box<RawValue>,
    ) -> Result<PutOutcome, ServerError> {
        if !store::valid_item_id(item_id) {
            return Err(invalid_id());
        }
        let (key, user, device, dk) = (Arc::clone(&self.key), who.user_id.clone(), who.device_id.clone(), who.key_sha256().to_owned());
        let (item_id, limits, now) = (item_id.to_owned(), self.limits, self.accounts.now());
        self.accounts
            .db()
            .blocking(move |conn| {
                store::put(
                    conn,
                    &key,
                    &Who { user_id: &user, device_id: &device, key_sha256: &dk },
                    &item_id,
                    base_version,
                    &item,
                    limits,
                    now,
                )
                .map_err(ServerError::from)
            })
            .await
    }

    pub async fn delete(
        &self,
        who: &AuthContext,
        item_id: &str,
        base_version: Option<u64>,
    ) -> Result<DeleteOutcome, ServerError> {
        if !store::valid_item_id(item_id) {
            return Err(invalid_id());
        }
        let (key, user, device, dk) = (Arc::clone(&self.key), who.user_id.clone(), who.device_id.clone(), who.key_sha256().to_owned());
        let (item_id, limits, now) = (item_id.to_owned(), self.limits, self.accounts.now());
        self.accounts
            .db()
            .blocking(move |conn| {
                store::delete(
                    conn,
                    &key,
                    &Who { user_id: &user, device_id: &device, key_sha256: &dk },
                    &item_id,
                    base_version,
                    limits,
                    now,
                )
                .map_err(ServerError::from)
            })
            .await
    }

    /// Toglie i segnaposto scaduti di tutti gli utenti.
    pub async fn purge_expired(&self) -> Result<usize, ServerError> {
        let now = self.accounts.now();
        self.accounts
            .db()
            .blocking(move |conn| store::purge_expired(conn, now).map_err(ServerError::from))
            .await
    }

    /// Prova ad aprire un elemento già salvato con la chiave attuale.
    pub async fn probe(&self) -> Result<Probe, ServerError> {
        let key = Arc::clone(&self.key);
        self.accounts
            .db()
            .blocking(move |conn| store::probe(conn, &key).map_err(ServerError::from))
            .await
    }
}

/// Cancella dal server tutti gli elementi di sincronizzazione di un utente (per le estensioni
/// che fissano quanto conservare le caselle di chi non ha più diritto alla sincronizzazione).
/// Non serve la chiave del server: non apre nulla. Restituisce le righe cancellate; su un
/// utente senza elementi non fa nulla.
pub async fn purge_user(db: &crate::db::Db, user_id: &str) -> Result<u64, ServerError> {
    let user_id = user_id.to_owned();
    db.blocking(move |conn| store::purge_user(conn, &user_id).map(|n| n as u64).map_err(ServerError::from))
        .await
}

/// Ogni quanto si tolgono i segnaposto scaduti.
pub const PURGE_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

impl SyncService {
    /// Un'attività che ogni ora toglie i segnaposto scaduti. Non scrive id né contenuti.
    pub fn spawn_purge(self: &Arc<Self>) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PURGE_EVERY).await;
                match service.purge_expired().await {
                    Ok(0) => {}
                    Ok(removed) => {
                        tracing::info!(removed, "segnaposto della sincronizzazione scaduti rimossi");
                    }
                    Err(error) => tracing::warn!(%error, "pulizia dei segnaposto non riuscita"),
                }
            }
        });
    }

    /// Le righe di log all'avvio. Una chiave che non apre gli elementi già salvati **non** ferma
    /// il server (l'amministratore deve poter entrare): la sincronizzazione risponde `500`.
    pub async fn log_startup(&self, old_key_set: bool) {
        tracing::info!("sincronizzazione delle caselle accesa");
        match self.probe().await {
            Ok(Probe::DoesNotOpen) => tracing::error!(
                "QAMILLE_SYNC_KEY non apre gli elementi già salvati: il server resta acceso, ma la sincronizzazione risponde 500 finché non si rimette la chiave giusta o non si fa `sync reset --yes`"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "prova della chiave di sincronizzazione non riuscita"),
        }
        if old_key_set {
            tracing::warn!("QAMILLE_SYNC_KEY_OLD è impostata: togli la chiave vecchia dopo `sync rekey`");
        }
    }
}
