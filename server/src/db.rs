//! `server.db`: apertura, migrazioni con nome e accesso dalla parte asincrona.
//!
//! Una sola connessione dietro un mutex; ogni uso passa da [`Db::blocking`], che
//! la presta a una chiusura eseguita su `spawn_blocking`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::ServerError;

/// Valore scritto in `PRAGMA user_version` dalle migrazioni con nome. Un binario che
/// conosce solo le versioni 1-5 rifiuta un file con questo valore.
pub const SCHEMA_FORMAT: i64 = 6;

/// Una migrazione con nome: applicata una volta, registrata in `schema_migrations`.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub name: &'static str,
    pub sql: &'static str,
}

/// Le migrazioni del core: utenti, dispositivi, contatori di calcolo e di assistente, elementi di
/// sincronizzazione.
pub const CORE_MIGRATIONS: &[Migration] = &[
    Migration { name: "core-001-users", sql: SCHEMA_V1 },
    Migration { name: "core-002-usage", sql: SCHEMA_V4 },
    Migration { name: "core-003-llm-usage", sql: SCHEMA_V6_LLM },
    Migration { name: "core-004-sync", sql: SCHEMA_V7_SYNC },
];

/// I nomi delle versioni 1..5 dei file di prima delle migrazioni con nome, in ordine.
const LEGACY_ORDER: [&str; 5] = [
    "core-001-users",
    "ent-001-email-codes",
    "ent-002-code-probe",
    "core-002-usage",
    "ent-003-identities-billing",
];

/// Nomi enterprise che un file può contenere: il core li tollera, così lo stesso file si apre a turno con il core e con l'enterprise.
const TOLERATED: [&str; 7] = [
    "ent-001-email-codes",
    "ent-002-code-probe",
    "ent-003-identities-billing",
    "ent-004-billing-outbox",
    "ent-005-usage-periods",
    "ent-006-terms",
    "ent-007-alert-kinds",
];

/// Schema v1 (wave 1). Pubblico per le prove di migrazione.
#[doc(hidden)]
pub const SCHEMA_V1: &str = "
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    email TEXT,
    password_hash TEXT,
    password_temporary INTEGER NOT NULL DEFAULT 0,
    temporary_expires_at TEXT,
    max_devices INTEGER,
    disabled_at TEXT,
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX users_username ON users(username COLLATE NOCASE);
CREATE UNIQUE INDEX users_email ON users(email COLLATE NOCASE) WHERE email IS NOT NULL;

CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at TEXT
);
CREATE INDEX devices_user_revoked ON devices(user_id, revoked_at);

CREATE TABLE device_keys (
    key_sha256 TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    revoked_at TEXT,
    wrong_password_streak INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX device_keys_device ON device_keys(device_id);
";

/// v4: contatori di calcolo per utente (D5): richieste, secondi di audio, token
/// embeddati. Solo numeri, mai contenuti.
#[doc(hidden)]
pub const SCHEMA_V4: &str = "
CREATE TABLE compute_usage (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    requests INTEGER NOT NULL DEFAULT 0,
    audio_seconds REAL NOT NULL DEFAULT 0,
    embedded_tokens INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
";

/// Contatori dell'assistente: token in entrata e in uscita per utente, e i token di ogni giorno
/// (UTC) per il tetto giornaliero. Solo numeri, mai contenuti.
#[doc(hidden)]
pub const SCHEMA_V6_LLM: &str = "
ALTER TABLE compute_usage ADD COLUMN llm_prompt_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE compute_usage ADD COLUMN llm_completion_tokens INTEGER NOT NULL DEFAULT 0;
CREATE TABLE llm_usage_day (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    tokens INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, day)
);
";

/// Sincronizzazione delle caselle: il contatore e l'epoca per utente, gli elementi cifrati. Un
/// segnaposto di cancellazione non conserva nulla del contenuto. `updated_by_device` è un testo
/// senza chiave esterna. Solo cifrato, mai in chiaro: vedi `sync::crypto`.
#[doc(hidden)]
pub const SCHEMA_V7_SYNC: &str = "
CREATE TABLE sync_state (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    version INTEGER NOT NULL DEFAULT 0,
    epoch INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE sync_items (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    nonce BLOB,
    ciphertext BLOB,
    deleted INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    updated_by_device TEXT NOT NULL,
    PRIMARY KEY (user_id, item_id),
    CHECK (length(item_id) = 64 AND item_id NOT GLOB '*[^0-9a-f]*'),
    CHECK ((deleted = 0 AND length(nonce) = 12 AND ciphertext IS NOT NULL)
        OR (deleted = 1 AND nonce IS NULL AND ciphertext IS NULL))
) WITHOUT ROWID;
CREATE INDEX sync_items_user_version ON sync_items(user_id, version);
";

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("non riesco ad aprire il database {path}: {reason}")]
    Open { path: String, reason: String },
    #[error(
        "il database {path} è della versione {found}, più recente di quella che questo programma conosce ({SCHEMA_FORMAT}): aggiorna qamille-server"
    )]
    NewerDatabase { path: String, found: i64 },
    #[error(
        "il database {path} contiene la migrazione «{name}», che questo programma non conosce: usa il programma che l'ha creata"
    )]
    UnknownMigration { path: String, name: String },
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Apre (o crea) il database e applica le migrazioni con nome che mancano.
    pub fn open(path: &Path, migrations: &[Migration]) -> Result<Self, DbError> {
        let shown = path.display().to_string();
        let fail = |e: rusqlite::Error| DbError::Open {
            path: shown.clone(),
            reason: e.to_string(),
        };
        let mut conn = Connection::open(path).map_err(fail)?;
        conn.busy_timeout(Duration::from_secs(30)).map_err(fail)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(fail)?;

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(fail)?;
        if version > SCHEMA_FORMAT {
            return Err(DbError::NewerDatabase {
                path: shown,
                found: version,
            });
        }
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(fail)?;
        // Tutto nella stessa transazione: un file si aggiorna tutto o resta com'era.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(fail)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                name TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL
            );",
        )
        .map_err(fail)?;
        let recorded: i64 = tx
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .map_err(fail)?;
        if recorded == 0 && (1..=5).contains(&version) {
            // File di prima delle migrazioni con nome: le versioni 1..N diventano i primi N nomi.
            for name in LEGACY_ORDER.iter().take(version as usize) {
                tx.execute(
                    "INSERT INTO schema_migrations (name, applied_at)
                     VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    [name],
                )
                .map_err(fail)?;
            }
        }
        let applied: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT name FROM schema_migrations")
                .map_err(fail)?;
            stmt.query_map([], |r| r.get::<_, String>(0))
                .map_err(fail)?
                .collect::<Result<_, _>>()
                .map_err(fail)?
        };
        for name in &applied {
            let known = migrations.iter().any(|m| m.name == name)
                || TOLERATED.contains(&name.as_str());
            if !known {
                return Err(DbError::UnknownMigration {
                    path: shown,
                    name: name.clone(),
                });
            }
        }
        for migration in migrations {
            if applied.iter().any(|n| n == migration.name) {
                continue;
            }
            tx.execute_batch(migration.sql).map_err(fail)?;
            tx.execute(
                "INSERT INTO schema_migrations (name, applied_at)
                 VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                [migration.name],
            )
            .map_err(fail)?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_FORMAT)
            .map_err(fail)?;
        tx.commit().map_err(fail)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Esegue `f` con la connessione su un thread bloccante.
    pub async fn blocking<F, T>(&self, f: F) -> Result<T, ServerError>
    where
        F: FnOnce(&mut Connection) -> Result<T, ServerError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            // Una chiusura andata in panico ha già annullato la sua transazione
            // (rollback al drop): la connessione resta utilizzabile.
            let mut guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(|e| ServerError::internal(format!("attività del database interrotta: {e}")))?
    }
}
