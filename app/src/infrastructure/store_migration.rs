//! One-time import from `store.json` into the SQLite store, and the engine
//! activation marker (Task 4.3, ADR-0007 decision 1).
//!
//! The migration never deletes the JSON: it stays the pre-migration backup and
//! the demo mode's store. Activation is a separate, deliberate step — a marker
//! file in the data directory that BOTH the desktop and the MCP process read
//! before choosing their repository, so the two can never diverge onto two
//! engines (a split across `store.json` and `store.sqlite` would lose mail).
//! The marker is not written by default: this plan merges the engine dark
//! (controller addition 4 to the 4.3 brief).
use crate::application::{AppError, AppResult, ports::StoreRepository};
use std::path::{Path, PathBuf};

/// What one startup call of [`migrate_if_needed`] decided and found.
#[derive(Clone, Debug)]
pub struct MigrationReport {
    /// `store.json` was imported into a fresh `store.sqlite` by this call.
    pub migrated: bool,
    /// `store.json` existed but was unreadable: preserved as
    /// `store.damaged.json`, and an empty database was created (Review
    /// Focus 5).
    pub damaged: bool,
    pub emails: usize,
    pub accounts: usize,
    /// The SQLite engine is the active store of both processes (marker file).
    pub activated: bool,
}

const ENGINE_MARKER: &str = "store-engine.txt";
const SQLITE_FILE: &str = "store.sqlite";
const DAMAGED_FILE: &str = "store.damaged.json";

pub fn sqlite_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SQLITE_FILE)
}
fn marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(ENGINE_MARKER)
}

/// Whether the SQLite engine is active: the marker names it. A missing or
/// unreadable marker means "not active" — the safe JSON side.
pub fn sqlite_active(data_dir: &Path) -> bool {
    std::fs::read_to_string(marker_path(data_dir))
        .map(|content| content.trim() == "sqlite")
        .unwrap_or(false)
}

/// Activates the SQLite engine for both processes: migrates if needed, proves
/// the database opens and reads, and only then writes the marker. Fails
/// without writing anything when the database cannot serve the store.
pub fn activate_sqlite(data_dir: &Path) -> AppResult<MigrationReport> {
    let mut report = migrate_if_needed(data_dir)?;
    let repository = super::sqlite_store::SqliteStoreRepository::open(sqlite_path(data_dir))?;
    repository.read()?;
    std::fs::write(marker_path(data_dir), "sqlite\n").map_err(|error| {
        AppError::storage(format!(
            "The SQLite engine marker could not be written: {error}"
        ))
    })?;
    report.activated = true;
    Ok(report)
}

/// Reads the store of the ACTIVE engine from a data directory whose
/// `store.lock` the caller already holds (Task 4.3): the semantic index
/// commits must never be served the stale JSON backup while the SQLite
/// engine is active. No file locking happens here — the SQLite side reads
/// through WAL, and the JSON side is parsed directly.
pub fn read_active_store(data_dir: &Path) -> AppResult<crate::model::Store> {
    if sqlite_active(data_dir) {
        let repository =
            super::sqlite_store::SqliteStoreRepository::open(sqlite_path(data_dir))?;
        return repository.read();
    }
    let mut store: crate::model::Store =
        serde_json::from_slice(&std::fs::read(data_dir.join("store.json"))?)?;
    super::portable_files::resolve_store_paths(&mut store, data_dir)?;
    Ok(store)
}

/// Imports `store.json` into `store.sqlite` the first time (keeping the JSON
/// untouched); every later call reports the current counts without changing
/// anything. An unreadable JSON is preserved as `store.damaged.json` and an
/// empty database is created (Review Focus 5). With neither file present the
/// workspace is fresh and nothing is created: the engine choice at open time
/// decides the first store.
pub fn migrate_if_needed(data_dir: &Path) -> AppResult<MigrationReport> {
    let activated = sqlite_active(data_dir);
    let sqlite = sqlite_path(data_dir);
    let json = data_dir.join("store.json");
    if !sqlite.exists() && json.exists() {
        // Both processes may race here on first start: the schema open and the
        // import transaction are idempotent under SQLite's writer lock, and
        // importing the same JSON snapshot twice writes the same rows.
        return match super::repository::JsonStoreRepository::new(&json).read() {
            Ok(store) => {
                let repository = super::sqlite_store::SqliteStoreRepository::open(&sqlite)?;
                let (emails, accounts) = (store.emails.len(), store.accounts.len());
                repository.transaction(&mut |current| {
                    // The closure may run again on a BUSY retry: it clones.
                    *current = store.clone();
                    Ok(())
                })?;
                repository.record_migration_source()?;
                Ok(MigrationReport {
                    migrated: true,
                    damaged: false,
                    emails,
                    accounts,
                    activated,
                })
            }
            Err(_) => {
                let _ = std::fs::rename(&json, data_dir.join(DAMAGED_FILE));
                super::sqlite_store::SqliteStoreRepository::open(&sqlite)?;
                Ok(MigrationReport {
                    migrated: true,
                    damaged: true,
                    emails: 0,
                    accounts: 0,
                    activated,
                })
            }
        };
    }
    if !sqlite.exists() {
        return Ok(MigrationReport {
            migrated: false,
            damaged: false,
            emails: 0,
            accounts: 0,
            activated,
        });
    }
    let repository = super::sqlite_store::SqliteStoreRepository::open(&sqlite)?;
    let store = repository.read()?;
    Ok(MigrationReport {
        migrated: false,
        damaged: false,
        emails: store.emails.len(),
        accounts: store.accounts.len(),
        activated,
    })
}
