//! L'archivio degli elementi: SQL e regole di versione, conflitto, limiti e segnaposto.
//!
//! Sono funzioni sincrone su una connessione (le chiama `Db::blocking`). Ogni query ha lo
//! `user_id` di chi chiama; ogni lettura e ogni scrittura rifà, **dentro la transazione**, il
//! controllo che il dispositivo e l'utente siano ancora attivi (INV-37). Nessun errore riporta
//! un id o un contenuto (INV-38).

use std::fmt;

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use serde_json::value::RawValue;

use super::TOMBSTONE_DAYS;
use super::crypto::SyncKey;
use crate::accounts::timestamp;
use crate::error::ServerError;

/// Chi chiama: l'utente e il dispositivo di una chiave già verificata.
pub struct Who<'a> {
    pub user_id: &'a str,
    pub device_id: &'a str,
    /// Lo SHA-256 della chiave con cui la richiesta si è autenticata: la revoca di **quella** chiave
    /// ferma la richiesta anche se è già in corso.
    pub key_sha256: &'a str,
}

/// I limiti per utente.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_items: u32,
    pub max_item_bytes: usize,
}

/// Un elemento come lo vedono i dispositivi. Il `Debug` non mostra l'id né il contenuto.
#[derive(Serialize)]
pub struct StoredItem {
    pub id: String,
    pub version: u64,
    pub deleted: bool,
    pub updated_at: String,
    pub updated_by_device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Box<RawValue>>,
}

impl fmt::Debug for StoredItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredItem")
            .field("version", &self.version)
            .field("deleted", &self.deleted)
            .finish_non_exhaustive()
    }
}

/// La risposta di `GET /v1/sync/items`.
#[derive(Debug, Serialize)]
pub struct Listing {
    /// Il contatore delle scritture dell'utente: il prossimo `since`.
    pub version: u64,
    /// Cambia quando il server è ripartito da zero (`sync reset`).
    pub epoch: u64,
    pub items: Vec<StoredItem>,
}

#[derive(Debug)]
pub enum PutOutcome {
    Written { version: u64, updated_at: String },
    /// `None`: la versione dichiarata è di un elemento che non c'è più.
    Conflict(Option<StoredItem>),
}

#[derive(Debug)]
pub enum DeleteOutcome {
    Deleted { version: u64 },
    AlreadyDeleted { version: u64 },
    Conflict(StoredItem),
    Missing,
}

#[derive(Debug)]
pub enum StoreError {
    /// Dispositivo o utente non più attivi.
    Inactive,
    /// Troppi elementi non cancellati.
    LimitReached,
    /// L'elemento supera `sync_max_item_bytes`.
    TooLarge,
    /// L'elemento non è un oggetto JSON.
    NotObject,
    /// Un elemento non si apre con la chiave attuale (nessun id nell'errore).
    Unreadable,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        StoreError::Db(error)
    }
}

impl From<StoreError> for ServerError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Inactive => ServerError::Unauthorized,
            StoreError::LimitReached => ServerError::Extension {
                status: axum::http::StatusCode::CONFLICT,
                code: "sync_limit",
                message: "Hai raggiunto il numero massimo di caselle sincronizzate.".to_owned(),
                retry_after_s: None,
                extra: None,
            },
            StoreError::TooLarge => ServerError::PayloadTooLarge,
            StoreError::NotObject => ServerError::InvalidRequest(
                "L'elemento deve essere un oggetto JSON.".to_owned(),
            ),
            StoreError::Unreadable => ServerError::internal(
                "sync: un elemento non si apre con la chiave attuale (QAMILLE_SYNC_KEY)",
            ),
            StoreError::Db(error) => ServerError::internal(format!("database: {error}")),
        }
    }
}

/// 64 cifre esadecimali minuscole.
pub fn valid_item_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

struct Row {
    item_id: String,
    version: i64,
    nonce: Option<Vec<u8>>,
    ciphertext: Option<Vec<u8>>,
    deleted: bool,
    updated_at: String,
    updated_by_device: String,
}

const COLUMNS: &str = "item_id, version, nonce, ciphertext, deleted, updated_at, updated_by_device";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        item_id: r.get(0)?,
        version: r.get(1)?,
        nonce: r.get(2)?,
        ciphertext: r.get(3)?,
        deleted: r.get::<_, i64>(4)? != 0,
        updated_at: r.get(5)?,
        updated_by_device: r.get(6)?,
    })
}

fn load_row(conn: &Connection, user_id: &str, item_id: &str) -> rusqlite::Result<Option<Row>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM sync_items WHERE user_id = ?1 AND item_id = ?2"),
        params![user_id, item_id],
        row_of,
    )
    .optional()
}

/// Decifra una riga per `user_id`: i dati associati sono utente e id della riga.
fn open_row(key: &SyncKey, user_id: &str, row: Row) -> Result<StoredItem, StoreError> {
    let item = if row.deleted {
        None
    } else {
        let (Some(nonce), Some(ciphertext)) = (&row.nonce, &row.ciphertext) else {
            return Err(StoreError::Unreadable);
        };
        let plain = key
            .open(user_id, &row.item_id, nonce, ciphertext)
            .map_err(|_| StoreError::Unreadable)?;
        let text = String::from_utf8(plain).map_err(|_| StoreError::Unreadable)?;
        Some(RawValue::from_string(text).map_err(|_| StoreError::Unreadable)?)
    };
    Ok(StoredItem {
        id: row.item_id,
        version: row.version.max(0) as u64,
        deleted: row.deleted,
        updated_at: row.updated_at,
        updated_by_device: row.updated_by_device,
        item,
    })
}

/// Il dispositivo e l'utente sono ancora attivi, con una chiave non revocata.
fn require_active(conn: &Connection, who: &Who<'_>) -> Result<(), StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM devices d JOIN users u ON u.id = d.user_id
             WHERE d.id = ?1 AND d.user_id = ?2 AND d.revoked_at IS NULL AND u.disabled_at IS NULL
               AND EXISTS (SELECT 1 FROM device_keys k WHERE k.device_id = d.id AND k.key_sha256 = ?3
                            AND k.revoked_at IS NULL)",
            params![who.device_id, who.user_id, who.key_sha256],
            |r| r.get(0),
        )
        .optional()?;
    found.map(|_| ()).ok_or(StoreError::Inactive)
}

/// Contatore ed epoca dell'utente (0 e 0 se non ha mai scritto).
fn state(conn: &Connection, user_id: &str) -> rusqlite::Result<(i64, i64)> {
    Ok(conn
        .query_row("SELECT version, epoch FROM sync_state WHERE user_id = ?1", [user_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .unwrap_or((0, 0)))
}

fn live_count(conn: &Connection, user_id: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM sync_items WHERE user_id = ?1 AND deleted = 0",
        [user_id],
        |r| r.get(0),
    )
}

/// Incrementa il contatore delle scritture dell'utente e restituisce il nuovo valore.
fn bump(tx: &Transaction<'_>, user_id: &str) -> rusqlite::Result<i64> {
    tx.query_row(
        "INSERT INTO sync_state (user_id, version) VALUES (?1, 1)
         ON CONFLICT(user_id) DO UPDATE SET version = version + 1
         RETURNING version",
        [user_id],
        |r| r.get(0),
    )
}

/// Toglie i segnaposto dell'utente più vecchi di 90 giorni e, oltre `cap`, i più vecchi per
/// versione: chi non li ha visti trova l'elemento «mancante», che vale come rimosso.
fn prune(tx: &Transaction<'_>, user_id: &str, now: DateTime<Utc>, cap: u32) -> rusqlite::Result<()> {
    let cutoff = timestamp(now - TimeDelta::days(TOMBSTONE_DAYS));
    tx.execute(
        "DELETE FROM sync_items WHERE user_id = ?1 AND deleted = 1 AND updated_at < ?2",
        params![user_id, cutoff],
    )?;
    tx.execute(
        "DELETE FROM sync_items WHERE user_id = ?1 AND deleted = 1 AND item_id NOT IN
           (SELECT item_id FROM sync_items WHERE user_id = ?1 AND deleted = 1
            ORDER BY version DESC LIMIT ?2)",
        params![user_id, cap],
    )?;
    Ok(())
}

/// Gli elementi dell'utente cambiati dopo la versione `since`, segnaposto compresi.
pub fn list(
    conn: &mut Connection,
    key: &SyncKey,
    who: &Who<'_>,
    since: u64,
) -> Result<Listing, StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
    require_active(&tx, who)?;
    let (version, epoch) = state(&tx, who.user_id)?;
    let since = i64::try_from(since).unwrap_or(i64::MAX);
    let rows: Vec<Row> = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {COLUMNS} FROM sync_items WHERE user_id = ?1 AND version > ?2 ORDER BY version"
        ))?;
        stmt.query_map(params![who.user_id, since], row_of)?
            .collect::<Result<_, _>>()?
    };
    let items = rows
        .into_iter()
        .map(|row| open_row(key, who.user_id, row))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Listing { version: version.max(0) as u64, epoch: epoch.max(0) as u64, items })
}

/// Crea o aggiorna un elemento. `base_version` è la versione da cui parte il chiamante (0 per
/// un elemento nuovo).
#[allow(clippy::too_many_arguments)]
pub fn put(
    conn: &mut Connection,
    key: &SyncKey,
    who: &Who<'_>,
    item_id: &str,
    base_version: u64,
    item: &RawValue,
    limits: Limits,
    now: DateTime<Utc>,
) -> Result<PutOutcome, StoreError> {
    let text = item.get();
    if !text.trim_start().starts_with('{') {
        return Err(StoreError::NotObject);
    }
    if text.len() > limits.max_item_bytes {
        return Err(StoreError::TooLarge);
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active(&tx, who)?;
    let current = load_row(&tx, who.user_id, item_id)?;
    let current_version = current.as_ref().map_or(0, |row| row.version.max(0) as u64);
    if base_version != current_version {
        let current = current.map(|row| open_row(key, who.user_id, row)).transpose()?;
        return Ok(PutOutcome::Conflict(current));
    }
    let becomes_live = current.as_ref().is_none_or(|row| row.deleted);
    if becomes_live && live_count(&tx, who.user_id)? >= i64::from(limits.max_items) {
        return Err(StoreError::LimitReached);
    }
    let version = bump(&tx, who.user_id)?;
    let sealed = key.seal(who.user_id, item_id, text.as_bytes());
    let updated_at = timestamp(now);
    tx.execute(
        "INSERT INTO sync_items
           (user_id, item_id, version, nonce, ciphertext, deleted, updated_at, updated_by_device)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)
         ON CONFLICT(user_id, item_id) DO UPDATE SET
           version = excluded.version, nonce = excluded.nonce, ciphertext = excluded.ciphertext,
           deleted = 0, updated_at = excluded.updated_at,
           updated_by_device = excluded.updated_by_device",
        params![
            who.user_id,
            item_id,
            version,
            &sealed.nonce[..],
            &sealed.ciphertext,
            updated_at,
            who.device_id
        ],
    )?;
    prune(&tx, who.user_id, now, limits.max_items)?;
    tx.commit()?;
    Ok(PutOutcome::Written { version: version.max(0) as u64, updated_at })
}

/// Cancella un elemento lasciando un segnaposto. Con `base_version` la cancellazione è
/// condizionata, senza è incondizionata; una cancellazione già fatta non scrive di nuovo.
pub fn delete(
    conn: &mut Connection,
    key: &SyncKey,
    who: &Who<'_>,
    item_id: &str,
    base_version: Option<u64>,
    limits: Limits,
    now: DateTime<Utc>,
) -> Result<DeleteOutcome, StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    require_active(&tx, who)?;
    let Some(row) = load_row(&tx, who.user_id, item_id)? else {
        return Ok(DeleteOutcome::Missing);
    };
    if row.deleted {
        return Ok(DeleteOutcome::AlreadyDeleted { version: row.version.max(0) as u64 });
    }
    if base_version.is_some_and(|base| base != row.version.max(0) as u64) {
        return Ok(DeleteOutcome::Conflict(open_row(key, who.user_id, row)?));
    }
    let version = bump(&tx, who.user_id)?;
    tx.execute(
        "UPDATE sync_items SET version = ?3, nonce = NULL, ciphertext = NULL, deleted = 1,
           updated_at = ?4, updated_by_device = ?5
         WHERE user_id = ?1 AND item_id = ?2",
        params![who.user_id, item_id, version, timestamp(now), who.device_id],
    )?;
    prune(&tx, who.user_id, now, limits.max_items)?;
    tx.commit()?;
    Ok(DeleteOutcome::Deleted { version: version.max(0) as u64 })
}

/// Cancella i segnaposto di tutti gli utenti più vecchi di 90 giorni. Restituisce quanti.
pub fn purge_expired(conn: &mut Connection, now: DateTime<Utc>) -> Result<usize, StoreError> {
    let cutoff = timestamp(now - TimeDelta::days(TOMBSTONE_DAYS));
    Ok(conn.execute(
        "DELETE FROM sync_items WHERE deleted = 1 AND updated_at < ?1",
        [cutoff],
    )?)
}

/// Righe per volta di `rekey`: tiene in memoria poche credenziali alla volta.
const REKEY_BATCH: usize = 100;

/// Ricifra tutti gli elementi non cancellati con `new`, aprendoli con `old`, in una sola
/// transazione: al primo elemento che non si apre non cambia nulla. Versioni, date e
/// dispositivo restano come sono; i nonce sono nuovi.
pub fn rekey(conn: &mut Connection, old: &SyncKey, new: &SyncKey) -> Result<usize, StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut done = 0;
    let mut after: (String, String) = (String::new(), String::new());
    loop {
        let batch: Vec<(String, String, Vec<u8>, Vec<u8>)> = {
            let mut stmt = tx.prepare(
                "SELECT user_id, item_id, nonce, ciphertext FROM sync_items
                 WHERE deleted = 0 AND (user_id, item_id) > (?1, ?2)
                 ORDER BY user_id, item_id LIMIT ?3",
            )?;
            stmt.query_map(params![after.0, after.1, REKEY_BATCH as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<Result<_, _>>()?
        };
        let Some(last) = batch.last() else { break };
        after = (last.0.clone(), last.1.clone());
        for (user_id, item_id, nonce, ciphertext) in &batch {
            let mut plain = old
                .open(user_id, item_id, nonce, ciphertext)
                .map_err(|_| StoreError::Unreadable)?;
            let sealed = new.seal(user_id, item_id, &plain);
            zeroize::Zeroize::zeroize(&mut plain);
            tx.execute(
                "UPDATE sync_items SET nonce = ?3, ciphertext = ?4 WHERE user_id = ?1 AND item_id = ?2",
                params![user_id, item_id, &sealed.nonce[..], &sealed.ciphertext],
            )?;
            done += 1;
        }
    }
    tx.commit()?;
    Ok(done)
}

/// Cancella tutti gli elementi e i segnaposto di tutti gli utenti e incrementa l'epoca di chi
/// ne aveva: i computer lo vedono e ricaricano le loro caselle invece di crederle rimosse. Il
/// contatore delle versioni non si azzera mai. Restituisce le righe cancellate.
pub fn reset(conn: &mut Connection) -> Result<usize, StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = tx.execute("DELETE FROM sync_items", [])?;
    tx.execute("UPDATE sync_state SET epoch = epoch + 1", [])?;
    tx.commit()?;
    Ok(removed)
}

/// Cancella tutti gli elementi e i segnaposto di un utente (un'estensione che decide di non
/// conservare più le sue caselle). Se ha tolto qualcosa incrementa l'epoca dell'utente, come
/// `reset`: i suoi computer ricaricano le loro caselle invece di crederle rimosse; il contatore
/// delle versioni non torna indietro. Senza righe non cambia nulla, quindi si può ripetere.
/// Restituisce le righe cancellate.
pub fn purge_user(conn: &mut Connection, user_id: &str) -> Result<usize, StoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = tx.execute("DELETE FROM sync_items WHERE user_id = ?1", [user_id])?;
    if removed > 0 {
        tx.execute("UPDATE sync_state SET epoch = epoch + 1 WHERE user_id = ?1", [user_id])?;
    }
    tx.commit()?;
    Ok(removed)
}

/// Che cosa dice la prova di aprire un elemento già salvato con la chiave attuale.
#[derive(Debug, PartialEq, Eq)]
pub enum Probe {
    Empty,
    Opens,
    DoesNotOpen,
}

/// Prova ad aprire il primo elemento non cancellato: una chiave diversa da quella con cui
/// sono stati salvati si vede subito, all'avvio.
pub fn probe(conn: &mut Connection, key: &SyncKey) -> Result<Probe, StoreError> {
    let row: Option<(String, String, Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT user_id, item_id, nonce, ciphertext FROM sync_items WHERE deleted = 0
             ORDER BY user_id, item_id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(match row {
        None => Probe::Empty,
        Some((user_id, item_id, nonce, ciphertext)) => {
            if key.open(&user_id, &item_id, &nonce, &ciphertext).is_ok() {
                Probe::Opens
            } else {
                Probe::DoesNotOpen
            }
        }
    })
}
