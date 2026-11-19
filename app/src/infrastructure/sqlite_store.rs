//! SQLite (WAL) adapter for the mail aggregate, ADR-0007 decision 1.
//!
//! The in-memory `Store` stays the model: `read` rebuilds it from the tables and
//! `transaction` hands it to the operation, then writes back only the records
//! that changed (diff by id, one UPSERT per table) inside one `BEGIN IMMEDIATE`.
//!
//! * INV-002: readers run in a deferred read transaction on their own pooled
//!   connection. In WAL mode they never wait for the writer, in this process or in
//!   another one (desktop and MCP share the file); no file lock guards a read.
//! * Writers are serialised by SQLite's single writer slot. The operation runs
//!   while it is held, so it always sees the previous writer's committed state and
//!   no update is lost; `busy_timeout` makes a second writer wait instead of fail.
//!   A collision that still fails immediately — WAL reports some `SQLITE_BUSY` /
//!   `SQLITE_LOCKED` without consulting the busy handler — repeats the whole
//!   attempt, up to five times with pauses of 50–800 ms, before it is reported
//!   (Review Focus 4: two processes writing the same rows must retry, never fail).
//! * Order of the in-memory vectors is part of the data (undo actions apply in
//!   order, accounts are listed in creation order): each table has `seq`, and the
//!   diff re-inserts the tail of a table when records were moved or inserted
//!   before existing ones. Plain appends, edits and removals touch only their rows.
//! * Persistence rules are those of the JSON adapter: managed files are stored as
//!   portable `workspace:/` references, received HTML (reader-only) is never
//!   written, and secrets (`#[serde(skip)]`) never reach the data file.
//! * Every record is built and read with exhaustive struct patterns and literals,
//!   so adding a field to the model fails to compile here instead of silently
//!   dropping it.
//! * Quota admission, cleanup intents and file reconciliation reuse the JSON
//!   adapter's path-derived helpers unchanged; only the persistence differs.
use super::repository::JsonStoreRepository;
use crate::{
    application::{
        AppError, AppResult,
        ports::{ChangeFeedRow, StoreRepository},
    },
    domain::{
        outbox::Delivery,
        undo::UndoAction,
    },
    model::{
        Account, Attachment, Draft, Email, Event, Meeting, RemoteLocation, Settings, Signature,
        Store,
    },
};
use rusqlite::{
    Connection, OptionalExtension, Row, TransactionBehavior, params_from_iter,
    types::{FromSql, Type, Value},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    thread,
    time::Duration,
};

/// How long a derived quota usage is trusted before the files are measured again.
const USAGE_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const SCHEMA: &str = include_str!("sqlite_schema.sql");
/// The shape recorded in `meta.schema_version`:
/// 1 — initial tables (Task 4.1). Development databases of the tail of that
///     period already carry the undo retry columns, earlier ones do not: the
///     columns reached the schema script while the recorded version stayed 1.
/// 2 — `undo_actions.retry_at_ms` / `attempts` guaranteed (ADR-0007 decision 8).
///     Version 1 databases migrate in place (see `migrate`), rows are kept.
/// 3 — `change_feed` (Task 4.3): the cross-process announcement table, created
///     purely by the idempotent schema script; version 2 databases gain the
///     table on the next open and record 3.
/// 5 — `accounts.history` (W3.1), added in place with an empty JSON object.
const SCHEMA_VERSION: u32 = 5;
/// Idle connections kept for reuse; more are opened on demand and dropped on return.
const POOL_LIMIT: usize = 8;
/// How long a write waits for a writer in *another process* (the MCP server).
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);
/// Whole-attempt retries when SQLite reports `SQLITE_BUSY` / `SQLITE_LOCKED`
/// despite the busy timeout: WAL answers some collisions immediately, without
/// consulting the handler (Review Focus 4 — a cross-process collision must
/// never reach the user).
const BUSY_RETRIES: u32 = 5;
/// First retry pause; doubles per attempt up to `BUSY_RETRY_MAX` (50 … 800 ms).
const BUSY_RETRY_MIN: Duration = Duration::from_millis(50);
const BUSY_RETRY_MAX: Duration = Duration::from_millis(800);
/// Safety valve only: a commit folds the log into the database file by itself once
/// the log passes this many pages (64 MiB). Normal operation never gets there,
/// because `checkpoint` runs from the maintenance job, not from a write.
const WAL_SAFETY_PAGES: u32 = 16_384;
/// The log file is truncated back to this size after a checkpoint.
const WAL_FILE_LIMIT: u64 = 64 * 1024 * 1024;

pub struct SqliteStoreRepository {
    path: PathBuf,
    pool: Mutex<Vec<Connection>>,
    /// Writers of this process queue here, so only another process is ever held to
    /// the busy timeout, and SQLite never polls on behalf of an in-process writer.
    writer: Mutex<()>,
    busy_timeout: Duration,
    /// Quota admission, cleanup intents and file reconciliation depend only on the
    /// data directory, which is `path`'s parent. The JSON file it names is never
    /// opened: this value is used for its helpers alone.
    files: JsonStoreRepository,
    /// The history lives in `activity.sqlite` beside the store (centro attività), not in the
    /// store database: the same file whichever engine runs, so the MCP process and a JSON-era
    /// workspace share it.
    activity: super::activity_db::ActivityDb,
    /// The last committed store this process knows, by store generation (W2.2).
    /// Reads share it until any process commits again; writes start from it.
    snapshot: Mutex<SnapshotSlot>,
    /// Signalled when an in-place write gives the snapshot back (`Lent`).
    snapshot_returned: Condvar,
    /// `used_bytes` per account at a store generation, for the quota check of
    /// the next write (measured, then derived from each change), with the instant
    /// of the measure: index files grow outside the transactions, so a derived
    /// value is trusted for `USAGE_TTL` at most.
    usage: Mutex<Option<(i64, std::time::Instant, HashMap<u32, u64>)>>,
}

/// Where the snapshot is kept. While `lent`, a write of this process is
/// changing it in place (`transaction_on_emails`, no other reference exists):
/// readers on other threads wait for it to come back, a few milliseconds,
/// instead of reading the whole database again.
#[derive(Default)]
struct SnapshotSlot {
    kept: Option<Snapshot>,
    lent: bool,
}

/// A committed store and the `generation` it was read or written at. Every
/// commit, by any process, moves the generation (`meta.generation`).
struct Snapshot {
    generation: i64,
    store: Arc<Store>,
    /// Disk form of everything but the emails, as committed: the base the small
    /// tables are compared with (memory-only changes made on read, such as an
    /// adopted old signature, are then still written by the next write).
    rest: Arc<Store>,
}

/// What a write starts from: the committed store and its small tables on disk.
#[derive(Clone)]
struct Base {
    store: Arc<Store>,
    rest: Arc<Store>,
}

/// Everything but the emails, moved out of a disk-form store.
fn split_rest(disk: &mut Store) -> (Store, Vec<Email>) {
    let emails = std::mem::take(&mut disk.emails);
    (disk.clone(), emails)
}

/// The store generation: incremented by every write transaction that changed
/// something (any process).
fn generation(connection: &Connection) -> AppResult<i64> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE name = 'generation'", [], |row| row.get(0))
        .optional()
        .map_err(db)?;
    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

fn db(error: rusqlite::Error) -> AppError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull) => AppError::new(
            "disk_full",
            "Not enough free disk space. The operation is paused; no mail was removed.",
        ),
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            AppError::storage(format!(
                "Mail data cannot be read. The database file is unchanged: {error}"
            ))
        }
        _ => AppError::storage(format!("Mail database: {error}")),
    }
}

fn poisoned<T>(_: T) -> AppError {
    AppError::storage("Repository lock failed")
}

/// `SQLITE_BUSY` / `SQLITE_LOCKED`: a writer of *another process* holds the
/// write lock (or WAL reported the collision without consulting the busy
/// handler). Nothing was committed; the attempt may be repeated as a whole.
fn busy(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Why one write attempt stopped. `Busy` carries, when the attempt had already
/// evicted cached content physically before its commit was refused, the emails
/// whose indexes a *final* failure must mark for rebuilding; a retry instead
/// re-runs the operation on a fresh read and needs no recovery.
enum Refused {
    Busy { error: AppError, evicted: Option<HashSet<u64>> },
    Failed(AppError),
}
impl From<AppError> for Refused {
    fn from(error: AppError) -> Self {
        Self::Failed(error)
    }
}

thread_local! {
    /// Write slots held by this thread, by repository: a write started inside a
    /// write of the same repository can only deadlock, so it is refused instead.
    static WRITING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct WriteSlot<'a> {
    id: usize,
    _guard: MutexGuard<'a, ()>,
}
impl Drop for WriteSlot<'_> {
    fn drop(&mut self) {
        WRITING.with(|held| {
            let mut held = held.borrow_mut();
            if let Some(at) = held.iter().rposition(|id| *id == self.id) {
                held.remove(at);
            }
        });
    }
}

/// Outcome of a passive checkpoint: frames in the log and frames folded into the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    pub busy: bool,
    pub log_frames: i64,
    pub checkpointed_frames: i64,
}

/// A pooled connection, returned on drop unless it was left inside a transaction.
struct Lease<'a> {
    pool: &'a Mutex<Vec<Connection>>,
    connection: Option<Connection>,
}
impl Deref for Lease<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().expect("a lease holds its connection until dropped")
    }
}
impl DerefMut for Lease<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection.as_mut().expect("a lease holds its connection until dropped")
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            if connection.is_autocommit() {
                if let Ok(mut pool) = self.pool.lock() {
                    if pool.len() < POOL_LIMIT {
                        pool.push(connection);
                    }
                }
            }
        }
    }
}

fn stored_version(connection: &Connection) -> AppResult<Option<u32>> {
    let has_meta: bool = connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    if !has_meta {
        return Ok(None);
    }
    let value: Option<String> = connection
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .optional()
        .map_err(db)?;
    value
        .map(|value| {
            value
                .parse()
                .map_err(|_| AppError::storage("Mail database has an unreadable schema version"))
        })
        .transpose()
}

/// Creates the tables on first use and migrates older databases in place. The
/// common case (schema already current) takes no write lock, so opening never
/// queues behind a running writer.
fn apply_schema(connection: &mut Connection) -> AppResult<()> {
    let check = |version: Option<u32>| match version {
        Some(version) if version > SCHEMA_VERSION => Err(AppError::storage(format!(
            "This mail database was written by a newer Qamille (schema {version}); \
             update the app to open it. Nothing was changed."
        ))),
        other => Ok(other == Some(SCHEMA_VERSION)),
    };
    if check(stored_version(connection)?)? {
        return Ok(());
    }
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db)?;
    // Another process may have created or migrated the schema while this one
    // waited for the lock.
    if check(stored_version(&tx)?)? {
        return Ok(());
    }
    tx.execute_batch(SCHEMA).map_err(db)?;
    migrate(&tx)?;
    put_meta(&tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    tx.commit().map_err(db)
}

/// Brings a database recorded below `SCHEMA_VERSION` to the current shape.
/// Every step is idempotent and keyed on the actual columns, not on the
/// recorded version: development databases exist with version 1 both without
/// and with the undo retry columns (they reached the schema script while the
/// version stayed 1 — gate G1). No step drops data; the store is not activated
/// in production yet, and every migration so far is a pure `ALTER TABLE ADD
/// COLUMN`, which keeps all rows. 2 → 3 needs no ALTER at all: the schema
/// script itself creates `change_feed` inside the same transaction that
/// records the new version.
fn migrate(connection: &Connection) -> AppResult<()> {
    // 4 → 5: the history backfill cursor (W3.1); `{}` reads as not started.
    if !has_column(connection, "accounts", "history")? {
        connection
            .execute("ALTER TABLE accounts ADD COLUMN history TEXT NOT NULL DEFAULT '{}'", [])
            .map_err(db)?;
    }
    // 3 → 4: the direct parent of a message (ADR-0010). Received mail already
    // carries it in `thread_id` (the In-Reply-To header, or its own Message-ID
    // for a root), so only received mail is filled. A reply Qamille sent
    // copied its parent's thread, one level too high, so its parent stays
    // empty rather than a false link. Filling sits inside the branch that adds
    // the column, so it never runs twice.
    if !has_column(connection, "emails", "in_reply_to")? {
        connection
            .execute("ALTER TABLE emails ADD COLUMN in_reply_to TEXT NOT NULL DEFAULT ''", [])
            .map_err(db)?;
        connection
            .execute(
                "UPDATE emails SET in_reply_to = trim(thread_id, '<>') \
                 WHERE folder <> 'Sent' AND local_sent = 0 AND thread_id <> '' \
                 AND trim(thread_id, '<>') <> trim(message_id, '<>')",
                [],
            )
            .map_err(db)?;
    }
    // 1 → 2: the retry queue of ADR-0007 decision 8 persists its next attempt
    // and its attempt count on the undo action. The added columns default to
    // exactly what the model defaults to: no retry scheduled, no attempt made.
    for (column, alter) in [
        ("retry_at_ms", "ALTER TABLE undo_actions ADD COLUMN retry_at_ms INTEGER"),
        (
            "attempts",
            "ALTER TABLE undo_actions ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !has_column(connection, "undo_actions", column)? {
            connection.execute(alter, []).map_err(db)?;
        }
    }
    Ok(())
}

fn has_column(connection: &Connection, table: &str, column: &str) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info (?) WHERE name = ?",
            [table, column],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(db)
}

impl SqliteStoreRepository {
    /// Opens (creating if needed) the database at `path`: WAL journal, a 5 s busy
    /// timeout, foreign keys on, and the schema applied. The file's directory is
    /// the data directory that managed attachments and indexes live in.
    pub fn open(path: impl Into<PathBuf>) -> AppResult<Self> {
        Self::open_with_busy_timeout(path, BUSY_TIMEOUT)
    }

    pub fn open_with_busy_timeout(path: impl Into<PathBuf>, busy_timeout: Duration) -> AppResult<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let repository = Self {
            files: JsonStoreRepository::new(path.clone()),
            activity: super::activity_db::ActivityDb::in_dir(
                path.parent().unwrap_or_else(|| Path::new("")),
            ),
            path,
            pool: Mutex::new(Vec::new()),
            writer: Mutex::new(()),
            busy_timeout,
            snapshot: Mutex::new(SnapshotSlot::default()),
            snapshot_returned: Condvar::new(),
            usage: Mutex::new(None),
        };
        let mut connection = repository.connect()?;
        // auto_vacuum only takes effect on a database that has no tables yet, so it is
        // set here, before the journal mode writes the header: freed pages then wait
        // on the free list instead of shrinking the file inside a write
        // (`reclaim_space` returns them). On an existing database it is a no-op.
        // Never issued against an existing database: it would need a lock and so
        // queue behind a running writer.
        let tables: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .map_err(db)?;
        if tables == 0 {
            connection.execute_batch("PRAGMA auto_vacuum=INCREMENTAL").map_err(db)?;
        }
        // The journal mode is stored in the file, so one connection sets it for all.
        let mode: String = connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .map_err(db)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(AppError::storage(format!(
                "This folder cannot hold the mail database: write-ahead logging is unavailable (journal mode {mode})"
            )));
        }
        apply_schema(&mut connection)?;
        repository.pool.lock().map_err(poisoned)?.push(connection);
        Ok(repository)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> AppResult<Connection> {
        let connection = Connection::open(&self.path).map_err(db)?;
        connection
            .execute_batch(&format!(
                "PRAGMA busy_timeout={}; PRAGMA foreign_keys=ON; \
                 PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint={WAL_SAFETY_PAGES}; \
                 PRAGMA journal_size_limit={WAL_FILE_LIMIT};",
                self.busy_timeout.as_millis()
            ))
            .map_err(db)?;
        connection.set_prepared_statement_cache_capacity(64);
        Ok(connection)
    }

fn write_slot(&self) -> AppResult<WriteSlot<'_>> {
        let id = &self.writer as *const _ as usize;
        if WRITING.with(|held| held.borrow().contains(&id)) {
            return Err(AppError::storage(
                "A write was started while another write of the same store is running on this thread",
            ));
        }
        let guard = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        WRITING.with(|held| held.borrow_mut().push(id));
        Ok(WriteSlot { id, _guard: guard })
    }

    /// Folds the write-ahead log into the database file. Passive: it takes neither
    /// the write slot nor a reader's snapshot, so it neither waits for nor delays
    /// a writer or a reader. For the maintenance job (Task 4.2), never for a write.
    pub fn checkpoint(&self) -> AppResult<Checkpoint> {
        let lease = self.lease()?;
        lease
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| {
                Ok(Checkpoint {
                    busy: row.get::<_, i64>(0)? != 0,
                    log_frames: row.get(1)?,
                    checkpointed_frames: row.get(2)?,
                })
            })
            .map_err(db)
    }

    /// Returns free pages to the file system (`None` = all). A write like any other,
    /// so it queues with the writers; for the maintenance job, never for a write.
    pub fn reclaim_space(&self, max_pages: Option<u32>) -> AppResult<u32> {
        let _slot = self.write_slot()?;
        let lease = self.lease()?;
        let free = |connection: &Connection| -> AppResult<u32> {
            connection.query_row("PRAGMA freelist_count", [], |row| row.get(0)).map_err(db)
        };
        let before = free(&lease)?;
        {
            // Each step of the pragma frees one page: it must be stepped to the end.
            let mut statement = lease
                .prepare(&format!("PRAGMA incremental_vacuum({})", max_pages.unwrap_or(0)))
                .map_err(db)?;
            let mut rows = statement.query([]).map_err(db)?;
            while rows.next().map_err(db)?.is_some() {}
        }
        Ok(before.saturating_sub(free(&lease)?))
    }

    /// Records the import provenance in `meta` (Task 4.3): a later open can
    /// tell a database migrated from `store.json` from a natively created one.
    pub fn record_migration_source(&self) -> AppResult<()> {
        let mut lease = self.lease()?;
        let tx = lease
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        put_meta(&tx, "migrated_from", "store.json")?;
        tx.commit().map_err(db)
    }

    /// Committed change-feed rows with `seq > since`, excluding the rows
    /// written by `exclude_pid` (the desktop passes its own pid: its writers
    /// already announced those changes themselves). Bounded per call, so a
    /// poller that was idle for long cannot pull the whole table at once.
    pub fn poll_change_feed_from(
        &self,
        since: i64,
        exclude_pid: u32,
    ) -> AppResult<Vec<ChangeFeedRow>> {
        let lease = self.lease()?;
        let mut statement = lease
            .prepare_cached(
                "SELECT seq, kind, account_id, ids FROM change_feed \
                 WHERE seq > ?1 AND pid != ?2 ORDER BY seq LIMIT 500",
            )
            .map_err(db)?;
        let rows = statement
            .query_map((since, i64::from(exclude_pid)), |row| {
                let ids: String = row.get(3)?;
                Ok(ChangeFeedRow {
                    seq: row.get(0)?,
                    kind: row.get(1)?,
                    account_id: row.get::<_, Option<i64>>(2)?.map(|id| id as u32),
                    ids: serde_json::from_str(&ids).unwrap_or_default(),
                })
            })
            .map_err(db)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db)
    }

    fn lease(&self) -> AppResult<Lease<'_>> {
        let idle = self.pool.lock().map_err(poisoned)?.pop();
        let connection = match idle {
            Some(connection) => connection,
            None => self.connect()?,
        };
        Ok(Lease { pool: &self.pool, connection: Some(connection) })
    }

    /// Disk form (portable references, no reader-only HTML) to the in-memory form
    /// every caller of the JSON adapter already receives.
    fn runtime(&self, mut store: Store) -> AppResult<Store> {
        if let Some(root) = self.path.parent() {
            super::portable_files::resolve_store_paths(&mut store, root)?;
        }
        store.adopt_old_signature();
        Ok(store)
    }

    fn slot(&self) -> MutexGuard<'_, SnapshotSlot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// Whether this thread holds the write slot of this repository.
    fn writing_here(&self) -> bool {
        let id = &self.writer as *const _ as usize;
        WRITING.with(|held| held.borrow().contains(&id))
    }
    /// Keeps `base` as the snapshot of `generation`, unless a newer one is kept.
    fn keep_snapshot(&self, generation: i64, base: Base) {
        let mut slot = self.slot();
        if slot.kept.as_ref().is_none_or(|kept| kept.generation <= generation) {
            slot.kept = Some(Snapshot { generation, store: base.store, rest: base.rest });
        }
    }
    /// The snapshot of `generation`, if this process holds it. A snapshot lent
    /// to an in-place write is waited for, except by the writing thread itself
    /// (an operation that reads while it writes), which reads the database.
    fn snapshot_at(&self, generation: i64) -> Option<Base> {
        let mut slot = self.slot();
        if slot.lent && !self.writing_here() {
            slot = self
                .snapshot_returned
                .wait_while(slot, |slot| slot.lent)
                .unwrap_or_else(PoisonError::into_inner);
        }
        slot.kept
            .as_ref()
            .filter(|kept| kept.generation == generation)
            .map(|kept| Base { store: kept.store.clone(), rest: kept.rest.clone() })
    }
    /// Lends the snapshot of `generation` to an in-place write, when nothing
    /// else in this process holds a reference to it (no reader keeps an
    /// `Arc<Store>` of it): the write may then change it without a copy.
    fn lend_snapshot(&self, generation: i64) -> Option<Lent<'_>> {
        let mut slot = self.slot();
        let kept = slot.kept.as_mut().filter(|kept| kept.generation == generation)?;
        Arc::get_mut(&mut kept.store)?;
        let snapshot = slot.kept.take()?;
        slot.lent = true;
        Some(Lent { repository: self, snapshot: Some(snapshot) })
    }
    /// The committed store of `generation`: the snapshot, or a read in `connection`.
    fn base(&self, connection: &Connection, generation: i64) -> AppResult<Base> {
        if let Some(base) = self.snapshot_at(generation) {
            return Ok(base);
        }
        let mut disk = read_store(connection)?;
        let (rest, emails) = split_rest(&mut disk);
        disk.emails = emails;
        let base = Base { store: Arc::new(self.runtime(disk)?), rest: Arc::new(rest) };
        self.keep_snapshot(generation, base.clone());
        Ok(base)
    }
    fn forget_snapshot(&self) {
        self.slot().kept = None;
    }

    /// After an aborted write: mark the indexes of the affected mail for rebuilding
    /// (physical cleanup may already have run) and consume the cleanup intent.
    fn recover(&self, affected: &HashSet<u64>) -> AppResult<()> {
        let mut lease = self.lease()?;
        let tx = lease
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        self.forget_snapshot();
        let generation = generation(&tx)?;
        let base = self.base(&tx, generation)?;
        let before = base.store.clone();
        let mut store = (*before).clone();
        let pending = self.files.read_cleanup_intent(&store)?;
        let mut changed = JsonStoreRepository::invalidate_indexes(&mut store, affected);
        changed |= JsonStoreRepository::invalidate_indexes(&mut store, &pending);
        let result = self.files.recover_lexical(&mut store, &mut changed);
        if changed {
            write_store(&tx, &before, &base.rest, &store, self.path.parent())?;
            tx.commit().map_err(db)?;
            self.forget_snapshot();
        } else {
            drop(tx);
        }
        self.files.clear_cleanup_intent()?;
        if result.is_ok() {
            if let Some(root) = self.path.parent() {
                super::lexical_index::discard_quarantines(root)?;
            }
        }
        result
    }

    /// The quota usage known for `generation` (derived by this process's last
    /// write, younger than `USAGE_TTL`), else measured on `before`, with the
    /// instant it was measured.
    fn known_usage(&self, generation: i64, before: &Store) -> (std::time::Instant, Option<HashMap<u32, u64>>) {
        let known = self
            .usage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(at, measured, _)| *at == generation && measured.elapsed() < USAGE_TTL)
            .map(|(_, measured, usage)| (*measured, usage.clone()));
        match known {
            Some((measured, usage)) => (measured, Some(usage)),
            // Measured again for a generation this process did not write, or when old.
            None => (std::time::Instant::now(), self.files.usage_by_account(before).ok()),
        }
    }

    /// Writes under the write slot, repeating a refused attempt (see `transaction`).
    fn write(&self, listed: Option<&[u64]>, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        let _slot = self.write_slot()?;
        // Review Focus 4: two processes writing the same rows must retry on
        // SQLITE_BUSY, never fail the user. `busy_timeout` covers the writer
        // that waits for another process's lock, but WAL reports some
        // collisions immediately, without consulting the handler: the whole
        // attempt is then repeated — it re-reads the committed state first, so
        // no update is lost — up to five times, pausing 50 to 800 ms.
        let mut pause = BUSY_RETRY_MIN;
        let mut refused: Option<(AppError, Option<HashSet<u64>>)> = None;
        for attempt in 0..=BUSY_RETRIES {
            match self.write_attempt(listed, op) {
                Ok(()) => return Ok(()),
                Err(Refused::Busy { error, evicted }) => {
                    refused = Some((error, evicted));
                    if attempt < BUSY_RETRIES {
                        thread::sleep(pause);
                        pause = (pause * 2).min(BUSY_RETRY_MAX);
                    }
                }
                Err(Refused::Failed(error)) => return Err(error),
            }
        }
        let (error, evicted) = refused.expect("the loop only ends on a refused attempt");
        if let Some(evicted) = evicted {
            let _ = self.recover(&evicted);
        }
        Err(error)
    }

    /// One attempt at the write transaction. A `Busy` outcome leaves nothing
    /// committed, so the caller may repeat the attempt — which re-reads the
    /// committed state first, and no update is lost. With `listed` (the
    /// promise of `transaction_on_emails`) the snapshot itself is changed when
    /// no reader of this process holds it (`write_in_place`); otherwise the
    /// operation changes a copy, compared whole with the snapshot.
    fn write_attempt(&self, listed: Option<&[u64]>, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> Result<(), Refused> {
        let mut lease = self.lease()?;
        let tx = match lease.transaction_with_behavior(TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(error) if busy(&error) => {
                return Err(Refused::Busy { error: db(error), evicted: None });
            }
            Err(error) => return Err(Refused::Failed(db(error))),
        };
        // INV-8: the snapshot is the base only when no process committed since
        // it was taken (same generation, read under this write lock).
        let generation = generation(&tx)?;
        let base = self.base(&tx, generation)?;
        let (measured_at, known) = self.known_usage(generation, &base.store);
        let base = match listed {
            Some(listed) => {
                // The base's own reference would keep the snapshot from being lent.
                drop(base);
                match self.lend_snapshot(generation) {
                    Some(lent) => {
                        return self.write_in_place(tx, generation, lent, listed, measured_at, known, op);
                    }
                    None => self.base(&tx, generation)?,
                }
            }
            None => base,
        };
        let before = base.store.clone();
        let mut store = writable_copy(&before);
        // An Err leaves `tx` to roll back: nothing of the operation is stored.
        op(&mut store)?;
        // HTML is reader-only: never stored, never in the snapshot either.
        for email in &mut store.emails {
            if !email.body_html.is_empty() {
                email.body_html.clear();
            }
        }
        if let Some(rest) = self.finish(tx, generation, &before, &base.rest, &mut store, measured_at, known, None)? {
            self.keep_snapshot(generation + 1, Base { store: Arc::new(store), rest: Arc::new(rest) });
            drop((before, base));
        }
        Ok(())
    }

    /// `write_attempt` on the lent snapshot itself (`transaction_on_emails`):
    /// the mailbox is neither copied nor compared, only the listed messages
    /// are (`Undo`). The snapshot is given back changed once the write
    /// committed, as it was otherwise. Whenever it cannot be trusted any more
    /// (the operation broke its promise, a write failed half-way) it is
    /// dropped instead, and the next read loads the database.
    #[allow(clippy::too_many_arguments)]
    fn write_in_place(
        &self,
        tx: rusqlite::Transaction<'_>,
        generation: i64,
        mut lent: Lent<'_>,
        listed: &[u64],
        measured_at: std::time::Instant,
        known: Option<HashMap<u32, u64>>,
        op: &mut dyn FnMut(&mut Store) -> AppResult<()>,
    ) -> Result<(), Refused> {
        let old_rest = lent.rest();
        let store = lent.store();
        let undo = Undo::record(store, listed);
        #[cfg(debug_assertions)]
        let whole = store.emails.clone();
        if let Err(error) = op(store) {
            // `tx` rolls back: nothing of the operation is stored, and the
            // snapshot returns as it was.
            if undo.restore(store) {
                lent.give_back(generation, None);
            }
            return Err(Refused::Failed(error));
        }
        #[cfg(debug_assertions)]
        assert!(
            undo.materialize(store).is_some_and(|before| before.emails == whole),
            "transaction_on_emails({listed:?}): the operation changed or removed a message it did not list"
        );
        let fast = undo.compare(store).and_then(|diff| {
            let previous = known.as_ref()?;
            let usage = crate::domain::storage::derived_usage_from_touched(&undo.before, store, &diff, previous)
                .filter(|usage| JsonStoreRepository::within_limits(store, usage))?;
            Some((diff, usage))
        });
        let fast_path = fast.is_some();
        let result = match fast {
            Some(fast) => self.finish(tx, generation, &undo.before, &old_rest, store, measured_at, None, Some(fast)),
            None => {
                // Not local enough (a removal, a new message or order, a quota
                // to enforce): the whole previous store, as any write sees it.
                let Some(before) = undo.materialize(store) else {
                    return Err(Refused::Failed(AppError::storage(
                        "A change removed a message it did not declare; nothing was saved",
                    )));
                };
                for email in &mut store.emails {
                    if !email.body_html.is_empty() {
                        email.body_html.clear();
                    }
                }
                self.finish(tx, generation, &before, &old_rest, store, measured_at, known, None)
            }
        };
        match result {
            Ok(Some(rest)) => {
                lent.give_back(generation + 1, Some(rest));
                Ok(())
            }
            // Nothing was written: the snapshot returns as it was.
            Ok(None) => {
                if undo.restore(store) {
                    lent.give_back(generation, None);
                }
                Ok(())
            }
            // Refused at commit, nothing stored. The fast path changed only
            // the listed messages and the small tables; the other may have
            // evicted any message, so its store is not given back.
            Err(Refused::Busy { error, evicted }) => {
                if fast_path && undo.restore(store) {
                    lent.give_back(generation, None);
                }
                Err(Refused::Busy { error, evicted })
            }
            Err(failed) => Err(failed),
        }
    }

    /// The end of a write, once the operation ran on `store`: quota (derived
    /// from the touched messages when possible, else enforced on the files),
    /// rows, commit. `before` is the committed store; with `fast` (the
    /// difference and the usage derived from it), only the previous versions
    /// of the touched messages, which `fast.0` indexes. Returns the small
    /// tables' disk form when the generation moved, `None` when nothing changed.
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        tx: rusqlite::Transaction<'_>,
        generation: i64,
        before: &Store,
        old_rest: &Store,
        store: &mut Store,
        measured_at: std::time::Instant,
        known: Option<HashMap<u32, u64>>,
        fast: Option<(crate::domain::storage::EmailDiff, HashMap<u32, u64>)>,
    ) -> Result<Option<Store>, Refused> {
        // Fast path (W2.3): one comparison finds the touched emails; when the
        // quota usage derived from them needs no eviction, nothing is measured.
        let (diff, usage) = match fast {
            Some((diff, usage)) => (Some(diff), Some(usage)),
            None => known
                .as_ref()
                .and_then(|previous| {
                    let found = crate::domain::storage::diff_emails(&before.emails, &store.emails);
                    crate::domain::storage::derived_usage(before, store, &found, previous)
                        .filter(|derived| JsonStoreRepository::within_limits(store, derived))
                        .map(|derived| (found, derived))
                })
                .map_or((None, None), |(diff, usage)| (Some(diff), Some(usage))),
        };
        let mut affected: HashSet<u64>;
        let removed_sources;
        let mut evicted_any = false;
        if let Some(found) = &diff {
            affected = found
                .changed
                .iter()
                .filter(|(new, old)| old.is_some_and(|old| store.emails[*new].storage_evicted && !before.emails[old].storage_evicted))
                .map(|(new, _)| store.emails[*new].id)
                .collect();
            removed_sources = !affected.is_empty();
        } else {
            let remaining: HashMap<u64, bool> = store
                .emails
                .iter()
                .map(|email| (email.id, email.storage_evicted))
                .collect();
            affected = before
                .emails
                .iter()
                .filter(|email| match remaining.get(&email.id) {
                    None => true,
                    Some(evicted) => *evicted && !email.storage_evicted,
                })
                .map(|email| email.id)
                .collect();
            let reports = match self.files.enforce_physical(before, store, &mut affected) {
                Ok(reports) => reports,
                Err(error) => {
                    drop(tx);
                    let _ = self.recover(&affected);
                    return Err(Refused::Failed(error));
                }
            };
            evicted_any = reports.iter().any(|report| !report.evicted_ids.is_empty());
            let remaining: HashMap<u64, bool> = store
                .emails
                .iter()
                .map(|email| (email.id, email.storage_evicted))
                .collect();
            removed_sources = before.emails.iter().any(|email| match remaining.get(&email.id) {
                None => true,
                Some(evicted) => *evicted && !email.storage_evicted,
            });
        }
        if removed_sources || evicted_any || !store.cache_removals.is_empty() {
            store.storage_maintenance_pending = true;
        }
        let written = match write_store_with(&tx, before, old_rest, store, self.path.parent(), diff.as_ref()) {
            Ok(written) => written,
            Err(error) => {
                drop(tx);
                let _ = self.recover(&affected);
                return Err(Refused::Failed(error));
            }
        };
        drop(diff);
        if let Err(error) = tx.commit() {
            if busy(&error) {
                // The commit was refused with nothing stored; only when every
                // retry is refused too does the eviction this attempt already
                // performed physically have to be recovered.
                return Err(Refused::Busy { error: db(error), evicted: Some(affected) });
            }
            let _ = self.recover(&affected);
            return Err(Refused::Failed(db(error)));
        }
        if written.moved {
            *self.usage.lock().unwrap_or_else(PoisonError::into_inner) = usage.map(|usage| (generation + 1, measured_at, usage));
        }
        // The operation committed. Failure to remove a derived maintenance marker
        // is retried; it must not report a rollback to the caller.
        let _ = self.files.clear_cleanup_intent();
        Ok(written.moved.then_some(written.rest))
    }
}

/// Everything but the emails, copied.
fn without_emails(store: &Store) -> Store {
    // Exhaustive: a new `Store` field must be copied here.
    let Store {
        outbox, cache_removals, storage_maintenance_pending, storage_index_rebuild_required,
        undo_actions, accounts, emails: _, drafts, events, meetings, settings, next_id, signatures,
    } = store;
    Store {
        outbox: outbox.clone(),
        cache_removals: cache_removals.clone(),
        storage_maintenance_pending: *storage_maintenance_pending,
        storage_index_rebuild_required: *storage_index_rebuild_required,
        undo_actions: undo_actions.clone(),
        accounts: accounts.clone(),
        emails: Vec::new(),
        drafts: drafts.clone(),
        events: events.clone(),
        meetings: meetings.clone(),
        settings: settings.clone(),
        next_id: *next_id,
        signatures: signatures.clone(),
    }
}

/// A copy of `store` for an operation to change, with room to append rows
/// (a history batch, a sync) without moving the whole message list again.
fn writable_copy(store: &Store) -> Store {
    let mut emails = Vec::with_capacity(store.emails.len() + store.emails.len() / 64 + 64);
    emails.extend_from_slice(&store.emails);
    Store { emails, ..without_emails(store) }
}

/// The snapshot, lent to one in-place write (`lend_snapshot`). Dropped
/// without `give_back` (an error or a panic mid-write), the store it changed
/// is discarded, and the next read loads the database again.
struct Lent<'a> {
    repository: &'a SqliteStoreRepository,
    snapshot: Option<Snapshot>,
}
impl Lent<'_> {
    fn store(&mut self) -> &mut Store {
        let snapshot = self.snapshot.as_mut().expect("a loan holds its snapshot until given back");
        Arc::get_mut(&mut snapshot.store).expect("a lent snapshot has no other reference")
    }
    fn rest(&self) -> Arc<Store> {
        self.snapshot.as_ref().expect("a loan holds its snapshot until given back").rest.clone()
    }
    /// Keeps the store as the snapshot of `generation` again, with the small
    /// tables' disk form `rest` when the write changed them.
    fn give_back(mut self, generation: i64, rest: Option<Store>) {
        let mut snapshot = self.snapshot.take().expect("a loan holds its snapshot until given back");
        snapshot.generation = generation;
        if let Some(rest) = rest {
            snapshot.rest = Arc::new(rest);
        }
        self.release(Some(snapshot));
    }
    fn release(&self, snapshot: Option<Snapshot>) {
        let mut slot = self.repository.slot();
        slot.lent = false;
        if let Some(snapshot) = snapshot {
            if slot.kept.as_ref().is_none_or(|kept| kept.generation <= snapshot.generation) {
                slot.kept = Some(snapshot);
            }
        }
        drop(slot);
        self.repository.snapshot_returned.notify_all();
    }
}
impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if let Some(snapshot) = self.snapshot.take() {
            self.release(None);
            drop(snapshot);
        }
    }
}

/// What an in-place write compares and rolls back with: the order of the ids,
/// the positions of the listed messages, and the store as it was before the
/// operation except for the unlisted messages, which the operation promised
/// not to change.
struct Undo {
    order: Vec<u64>,
    positions: Vec<usize>,
    /// The small tables, and the listed messages in list order.
    before: Store,
}
impl Undo {
    fn record(store: &Store, listed: &[u64]) -> Self {
        let set: Option<HashSet<u64>> = (listed.len() > 8).then(|| listed.iter().copied().collect());
        let is_listed = |id: &u64| set.as_ref().map_or_else(|| listed.contains(id), |set| set.contains(id));
        let order: Vec<u64> = store.emails.iter().map(|email| email.id).collect();
        let positions: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, id)| is_listed(id))
            .map(|(at, _)| at)
            .collect();
        let mut before = without_emails(store);
        before.emails = positions.iter().map(|at| store.emails[*at].clone()).collect();
        Self { order, positions, before }
    }
    fn same_order(&self, emails: &[Email]) -> bool {
        emails.len() == self.order.len() && emails.iter().zip(&self.order).all(|(email, id)| email.id == *id)
    }
    /// The difference, indexing `self.before.emails` for previous versions,
    /// with the reader-only HTML of the listed messages cleared. `None` unless
    /// every unlisted message is still in its place and none was added (a
    /// listed one may be gone).
    fn compare(&self, store: &mut Store) -> Option<crate::domain::storage::EmailDiff> {
        let emails = &mut store.emails;
        let (mut changed, mut removed) = (Vec::new(), Vec::new());
        // Positions in `emails`, and in `positions` / `before.emails`.
        let (mut at, mut next) = (0usize, 0usize);
        for (position, id) in self.order.iter().enumerate() {
            let listed = self.positions.get(next) == Some(&position);
            if emails.get(at).is_some_and(|email| email.id == *id) {
                if listed {
                    let email = &mut emails[at];
                    if !email.body_html.is_empty() {
                        email.body_html.clear();
                    }
                    if *email != self.before.emails[next] {
                        changed.push((at, Some(next)));
                    }
                }
                at += 1;
            } else if listed {
                removed.push(next);
            } else {
                return None;
            }
            next += usize::from(listed);
        }
        (at == emails.len()).then_some(crate::domain::storage::EmailDiff { changed, removed, order_kept: true })
    }
    /// The whole store as it was before the operation, copied; `None` when an
    /// unlisted message is gone.
    fn materialize(&self, store: &Store) -> Option<Store> {
        let saved = &self.before.emails;
        let emails = if self.same_order(&store.emails) {
            let mut emails = store.emails.clone();
            for (at, email) in self.positions.iter().zip(saved) {
                emails[*at] = email.clone();
            }
            emails
        } else {
            let by_id: HashMap<u64, &Email> = store.emails.iter().map(|email| (email.id, email)).collect();
            let mut saved = self.positions.iter().copied().zip(saved).peekable();
            self.order
                .iter()
                .enumerate()
                .map(|(at, id)| match saved.next_if(|(position, _)| *position == at) {
                    Some((_, email)) => Some(email.clone()),
                    None => by_id.get(id).map(|email| (*email).clone()),
                })
                .collect::<Option<Vec<_>>>()?
        };
        Some(Store { emails, ..without_emails(&self.before) })
    }
    /// Puts `store` back as it was before the operation; `false` when an
    /// unlisted message is gone and `store` can no longer be trusted.
    fn restore(self, store: &mut Store) -> bool {
        let same_order = self.same_order(&store.emails);
        let Undo { order, positions, mut before } = self;
        let saved = std::mem::take(&mut before.emails);
        let mut emails = std::mem::take(&mut store.emails);
        *store = before;
        if same_order {
            for (at, email) in positions.iter().zip(saved) {
                emails[*at] = email;
            }
            store.emails = emails;
            return true;
        }
        let mut by_id: HashMap<u64, Email> = emails.drain(..).map(|email| (email.id, email)).collect();
        let mut saved = positions.iter().copied().zip(saved).peekable();
        let restored: Option<Vec<Email>> = order
            .iter()
            .enumerate()
            .map(|(at, id)| match saved.next_if(|(position, _)| *position == at) {
                Some((_, email)) => Some(email),
                None => by_id.remove(id),
            })
            .collect();
        match restored {
            Some(restored) => {
                store.emails = restored;
                true
            }
            None => false,
        }
    }
}

impl StoreRepository for SqliteStoreRepository {
    fn activity_log(&self) -> Option<&dyn crate::application::ports::ActivityStore> {
        Some(&self.activity)
    }
    fn read(&self) -> AppResult<Store> {
        Ok((*self.read_shared()?).clone())
    }

    fn read_shared(&self) -> AppResult<Arc<Store>> {
        let mut lease = self.lease()?;
        // One deferred transaction = one consistent snapshot across all tables, and
        // in WAL mode it neither blocks nor is blocked by a writer.
        let tx = lease
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(db)?;
        let generation = generation(&tx)?;
        Ok(self.base(&tx, generation)?.store)
    }

    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.write(None, op)
    }

    fn transaction_on_emails(&self, emails: &[u64], op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.write(Some(emails), op)
    }

    fn message_storage_bytes(&self, store: &Store) -> AppResult<HashMap<u64, u64>> {
        self.files.message_storage_bytes(store)
    }

    fn draft_storage_bytes(
        &self,
        store: &Store,
    ) -> AppResult<HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
        self.files.draft_storage_bytes(store)
    }

    fn reconcile_storage(&self) -> AppResult<()> {
        let _slot = self.write_slot()?;
        let mut lease = self.lease()?;
        let tx = lease
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let generation = generation(&tx)?;
        let base = self.base(&tx, generation)?;
        let before = base.store.clone();
        let mut store = (*before).clone();
        let before_pending = store.storage_maintenance_pending;
        let before_rebuild = store.storage_index_rebuild_required;
        let had_removals = !store.cache_removals.is_empty();
        let mut recovered = false;
        let mut intent_loaded = false;
        let result = (|| {
            let pending = self.files.read_cleanup_intent(&store)?;
            intent_loaded = true;
            recovered |= JsonStoreRepository::invalidate_indexes(&mut store, &pending);
            self.files.recover_lexical(&mut store, &mut recovered)?;
            self.files.reconcile_locked(&mut store)
        })();
        store.storage_maintenance_pending = result.is_err();
        if result.is_ok() {
            store.cache_removals.clear();
        }
        if recovered
            || before_pending != store.storage_maintenance_pending
            || before_rebuild != store.storage_index_rebuild_required
            || (had_removals && result.is_ok())
        {
            let written = write_store(&tx, &before, &base.rest, &store, self.path.parent())?;
            tx.commit().map_err(db)?;
            let committed = generation + i64::from(written.moved);
            self.keep_snapshot(committed, Base { store: Arc::new(store), rest: Arc::new(written.rest) });
        } else {
            drop(tx);
        }
        if intent_loaded {
            self.files.clear_cleanup_intent()?;
        }
        if result.is_ok() {
            if let Some(root) = self.path.parent() {
                super::lexical_index::discard_quarantines(root)?;
            }
        }
        result
    }

    fn poll_change_feed(&self, since: i64) -> AppResult<Vec<ChangeFeedRow>> {
        self.poll_change_feed_from(since, std::process::id())
    }

    fn change_feed_head(&self) -> AppResult<i64> {
        let lease = self.lease()?;
        lease
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM change_feed", [], |row| {
                row.get(0)
            })
            .map_err(db)
    }

    fn storage_checkpoint(&self) -> AppResult<()> {
        // Maintenance cadence only, never a user write: fold the WAL, return
        // the free pages, then prune the feed — 10 000 rows is far more than a
        // 2 s poller can fall behind, and the rows are pure announcements.
        self.checkpoint()?;
        self.reclaim_space(None)?;
        let lease = self.lease()?;
        lease
            .execute(
                "DELETE FROM change_feed WHERE seq <= \
                 (SELECT COALESCE(MAX(seq), 0) FROM change_feed) - 10000",
                [],
            )
            .map_err(db)?;
        drop(lease);
        // The activity history keeps 90 days (centro attività): the same pass, never a user write.
        self.prune_activity(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as i64)
                .unwrap_or(0),
        )
    }
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/// A record table: `cols[0]` is the unique `id`; `seq` (insertion order) is implicit.
struct Table {
    name: &'static str,
    cols: &'static [&'static str],
    select: OnceLock<String>,
    upsert: OnceLock<String>,
    delete: OnceLock<String>,
}
impl Table {
    const fn new(name: &'static str, cols: &'static [&'static str]) -> Self {
        Self {
            name,
            cols,
            select: OnceLock::new(),
            upsert: OnceLock::new(),
            delete: OnceLock::new(),
        }
    }
    fn select(&self) -> &str {
        self.select.get_or_init(|| {
            format!("SELECT {} FROM {} ORDER BY seq", self.cols.join(", "), self.name)
        })
    }
    fn upsert(&self) -> &str {
        self.upsert.get_or_init(|| {
            let marks: Vec<_> = (1..=self.cols.len()).map(|n| format!("?{n}")).collect();
            let updates: Vec<_> = self.cols[1..]
                .iter()
                .map(|col| format!("{col} = excluded.{col}"))
                .collect();
            format!(
                "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT (id) DO UPDATE SET {}",
                self.name,
                self.cols.join(", "),
                marks.join(", "),
                updates.join(", ")
            )
        })
    }
    fn delete(&self) -> &str {
        self.delete
            .get_or_init(|| format!("DELETE FROM {} WHERE id = ?1", self.name))
    }
}

static ACCOUNTS: Table = Table::new(
    "accounts",
    &[
        "id", "provider_id", "search_index", "name", "email", "color", "kind", "imap_host",
        "imap_port", "smtp_host", "smtp_port", "username", "oauth_client_id", "signature",
        "last_signature", "archive_folder", "trash_folder", "last_sync", "fetch_count",
        "storage_policy", "offline_sync_complete", "offline_sync_error", "history",
    ],
);
static EMAILS: Table = Table::new(
    "emails",
    &[
        "id", "account_id", "folder", "thread_id", "message_id", "local_sent", "remote_deleted",
        "outgoing_fingerprint", "uid", "remote_folder", "remote_uid_validity",
        "remote_content_hash", "remote_global_id", "from_name", "from_addr", "recipients_to",
        "recipients_cc", "subject", "body", "remote_only", "storage_evicted", "date",
        "date_missing", "read", "starred", "pinned", "done", "muted", "set_aside", "replied",
        "priority_thread", "category", "snoozed_until", "reminder_at", "due", "assigned_to",
        "classified", "returned_at", "bulk", "auto_submitted", "calendar", "search_index",
        "in_reply_to",
    ],
);
static DRAFTS: Table = Table::new(
    "drafts",
    &[
        "id", "account_id", "to_addrs", "cc_addrs", "bcc_addrs", "request_id", "remote_id",
        "remote_revision", "cloud_content_hash", "cloud_sync_key", "cloud_sync_pending",
        "subject", "body", "reply_to", "send_at", "attachments", "updated", "format",
        "signature_id", "quote",
    ],
);
static OUTBOX: Table = Table::new(
    "outbox",
    &[
        "id", "delivery_key", "request_hash", "draft", "plain", "html", "sender_name",
        "attachment_bytes", "status", "created_at", "updated_at", "send_at",
        "requested_send_at", "error", "owner_pid", "requires_mcp_send", "source_profile",
    ],
);
static UNDO_ACTIONS: Table = Table::new(
    "undo_actions",
    &["id", "operation", "subject", "created_at_ms", "expires_at_ms", "status", "error", "retry_at_ms", "attempts"],
);
static EVENTS: Table = Table::new(
    "events",
    &[
        "id", "title", "starts_at", "ends_at", "location", "attendees", "meeting_link",
        "conference", "rsvp", "notes", "color",
    ],
);
static MEETINGS: Table = Table::new(
    "meetings",
    &[
        "id", "title", "date", "duration_secs", "audio_path", "transcript", "notes", "summary",
        "key_points", "action_items", "event_id",
    ],
);
static SIGNATURES: Table = Table::new("signatures", &["id", "name", "body", "modified", "html"]);

// ---------------------------------------------------------------------------
// Value conversion (u64 ids and sizes keep their 64 bits in a signed INTEGER)
// ---------------------------------------------------------------------------

fn int(value: impl Into<i64>) -> Value {
    Value::Integer(value.into())
}
fn opt_int<T: Into<i64>>(value: Option<T>) -> Value {
    value.map_or(Value::Null, int)
}
fn big(value: u64) -> Value {
    Value::Integer(value as i64)
}
fn opt_big(value: Option<u64>) -> Value {
    value.map_or(Value::Null, big)
}
fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}
fn opt_text(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, text)
}
fn json<T: Serialize + ?Sized>(value: &T) -> AppResult<Value> {
    Ok(Value::Text(serde_json::to_string(value)?))
}
/// A unit enum stored under its serde name (`"Gmail"`, `"needs_review"`).
fn name<T: Serialize>(value: &T) -> AppResult<Value> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(name) => Ok(Value::Text(name)),
        other => Err(AppError::storage(format!("Expected a plain enum value, got {other}"))),
    }
}

/// Reads columns left to right, in the order of the table's `cols`.
struct Cur<'r, 's> {
    row: &'r Row<'s>,
    at: usize,
}
impl<'r, 's> Cur<'r, 's> {
    fn new(row: &'r Row<'s>) -> Self {
        Self { row, at: 0 }
    }
    fn get<T: FromSql>(&mut self) -> rusqlite::Result<T> {
        let value = self.row.get(self.at)?;
        self.at += 1;
        Ok(value)
    }
    fn big(&mut self) -> rusqlite::Result<u64> {
        Ok(self.get::<i64>()? as u64)
    }
    fn opt_big(&mut self) -> rusqlite::Result<Option<u64>> {
        Ok(self.get::<Option<i64>>()?.map(|value| value as u64))
    }
    fn json<T: DeserializeOwned>(&mut self) -> rusqlite::Result<T> {
        let at = self.at;
        let text: String = self.get()?;
        serde_json::from_str(&text)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(at, Type::Text, Box::new(e)))
    }
    fn name<T: DeserializeOwned>(&mut self) -> rusqlite::Result<T> {
        let at = self.at;
        let text: String = self.get()?;
        serde_json::from_value(serde_json::Value::String(text))
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(at, Type::Text, Box::new(e)))
    }
}

// ---------------------------------------------------------------------------
// Rows: one `*_row` (write) and one `*_from` (read) per record kind
// ---------------------------------------------------------------------------

fn account_row(account: &Account) -> AppResult<Vec<Value>> {
    let Account {
        provider_id, search_index, id, name: account_name, email, color, kind, imap_host,
        imap_port, smtp_host, smtp_port, username, password: _, oauth_client_id, signature,
        last_signature, archive_folder, trash_folder, last_sync, fetch_count, storage_policy,
        offline_sync_complete, offline_sync_error, history,
    } = account;
    Ok(vec![
        int(*id),
        text(provider_id),
        json(search_index)?,
        text(account_name),
        text(email),
        json(color)?,
        name(kind)?,
        text(imap_host),
        int(*imap_port),
        text(smtp_host),
        int(*smtp_port),
        text(username),
        text(oauth_client_id),
        json(signature)?,
        opt_big(*last_signature),
        text(archive_folder),
        text(trash_folder),
        opt_int(*last_sync),
        int(*fetch_count),
        json(storage_policy)?,
        int(*offline_sync_complete),
        opt_text(offline_sync_error),
        json(history)?,
    ])
}
fn account_from(c: &mut Cur) -> rusqlite::Result<Account> {
    Ok(Account {
        id: c.get()?,
        provider_id: c.get()?,
        search_index: c.json()?,
        name: c.get()?,
        email: c.get()?,
        color: c.json()?,
        kind: c.name()?,
        imap_host: c.get()?,
        imap_port: c.get()?,
        smtp_host: c.get()?,
        smtp_port: c.get()?,
        username: c.get()?,
        password: String::new(),
        oauth_client_id: c.get()?,
        signature: c.json()?,
        last_signature: c.opt_big()?,
        archive_folder: c.get()?,
        trash_folder: c.get()?,
        last_sync: c.get()?,
        fetch_count: c.get()?,
        storage_policy: c.json()?,
        offline_sync_complete: c.get()?,
        offline_sync_error: c.get()?,
        history: c.json()?,
    })
}

/// Scalar columns of an email. Labels, remote locations and attachments live in
/// their own tables; the reader-only HTML is deliberately not stored.
fn email_row(email: &Email) -> AppResult<Vec<Value>> {
    let Email {
        search_index, id, account_id, folder, thread_id, message_id, local_sent, remote_deleted,
        outgoing_fingerprint, uid, remote_folder, remote_uid_validity, remote_content_hash,
        remote_global_id, remote_locations: _, from_name, from_addr, to, cc, subject, body,
        body_html: _, remote_only, storage_evicted, date, date_missing, read, starred, pinned,
        done, muted, set_aside, replied, priority_thread, category, labels: _, snoozed_until,
        reminder_at, due, assigned_to, attachments: _, classified, returned_at, bulk,
        auto_submitted, calendar, in_reply_to,
    } = email;
    Ok(vec![
        big(*id),
        int(*account_id),
        text(folder),
        text(thread_id),
        text(message_id),
        int(*local_sent),
        int(*remote_deleted),
        text(outgoing_fingerprint),
        opt_int(*uid),
        opt_text(remote_folder),
        opt_int(*remote_uid_validity),
        text(remote_content_hash),
        opt_big(*remote_global_id),
        text(from_name),
        text(from_addr),
        json(to)?,
        json(cc)?,
        text(subject),
        text(body),
        int(*remote_only),
        int(*storage_evicted),
        int(*date),
        int(*date_missing),
        int(*read),
        int(*starred),
        int(*pinned),
        int(*done),
        int(*muted),
        int(*set_aside),
        int(*replied),
        int(*priority_thread),
        name(category)?,
        opt_int(*snoozed_until),
        opt_int(*reminder_at),
        opt_int(*due),
        opt_text(assigned_to),
        int(*classified),
        opt_int(*returned_at),
        int(*bulk),
        int(*auto_submitted),
        int(*calendar),
        json(search_index)?,
        text(in_reply_to),
    ])
}
fn email_from(c: &mut Cur) -> rusqlite::Result<Email> {
    Ok(Email {
        id: c.big()?,
        account_id: c.get()?,
        folder: c.get()?,
        thread_id: c.get()?,
        message_id: c.get()?,
        local_sent: c.get()?,
        remote_deleted: c.get()?,
        outgoing_fingerprint: c.get()?,
        uid: c.get()?,
        remote_folder: c.get()?,
        remote_uid_validity: c.get()?,
        remote_content_hash: c.get()?,
        remote_global_id: c.opt_big()?,
        remote_locations: Vec::new(),
        from_name: c.get()?,
        from_addr: c.get()?,
        to: c.json()?,
        cc: c.json()?,
        subject: c.get()?,
        body: c.get()?,
        body_html: String::new(),
        remote_only: c.get()?,
        storage_evicted: c.get()?,
        date: c.get()?,
        date_missing: c.get()?,
        read: c.get()?,
        starred: c.get()?,
        pinned: c.get()?,
        done: c.get()?,
        muted: c.get()?,
        set_aside: c.get()?,
        replied: c.get()?,
        priority_thread: c.get()?,
        category: c.name()?,
        labels: Vec::new(),
        snoozed_until: c.get()?,
        reminder_at: c.get()?,
        due: c.get()?,
        assigned_to: c.get()?,
        attachments: Vec::new(),
        classified: c.get()?,
        returned_at: c.get()?,
        bulk: c.get()?,
        auto_submitted: c.get()?,
        calendar: c.get()?,
        search_index: c.json()?,
        in_reply_to: c.get()?,
    })
}

fn attachment_row(attachment: &Attachment) -> Vec<Value> {
    let Attachment { id, name, size, mime, path, inline_text } = attachment;
    vec![big(*id), text(name), big(*size), text(mime), opt_text(path), opt_text(inline_text)]
}

fn draft_row(draft: &Draft) -> AppResult<Vec<Value>> {
    let Draft {
        id, account_id, to, cc, bcc, request_id, remote_id, remote_revision, cloud_content_hash,
        cloud_sync_key, cloud_sync_pending, subject, body, reply_to, send_at, attachments,
        updated, format, signature_id, quote,
    } = draft;
    Ok(vec![
        big(*id),
        int(*account_id),
        text(to),
        text(cc),
        text(bcc),
        text(request_id),
        opt_text(remote_id),
        opt_text(remote_revision),
        text(cloud_content_hash),
        text(cloud_sync_key),
        int(*cloud_sync_pending),
        text(subject),
        text(body),
        opt_big(*reply_to),
        opt_int(*send_at),
        json(attachments)?,
        int(*updated),
        name(format)?,
        opt_big(*signature_id),
        text(quote),
    ])
}
fn draft_from(c: &mut Cur) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: c.big()?,
        account_id: c.get()?,
        to: c.get()?,
        cc: c.get()?,
        bcc: c.get()?,
        request_id: c.get()?,
        remote_id: c.get()?,
        remote_revision: c.get()?,
        cloud_content_hash: c.get()?,
        cloud_sync_key: c.get()?,
        cloud_sync_pending: c.get()?,
        subject: c.get()?,
        body: c.get()?,
        reply_to: c.opt_big()?,
        send_at: c.get()?,
        attachments: c.json()?,
        updated: c.get()?,
        format: c.name()?,
        signature_id: c.opt_big()?,
        quote: c.get()?,
    })
}

fn delivery_row(delivery: &Delivery) -> AppResult<Vec<Value>> {
    let Delivery {
        id, key, request_hash, draft, plain, html, sender_name, attachment_bytes, status,
        created_at, updated_at, send_at, requested_send_at, error, owner_pid,
        requires_mcp_send, source_profile,
    } = delivery;
    Ok(vec![
        big(*id),
        text(key),
        text(request_hash),
        json(draft)?,
        text(plain),
        opt_text(html),
        text(sender_name),
        big(*attachment_bytes),
        name(status)?,
        int(*created_at),
        int(*updated_at),
        int(*send_at),
        opt_int(*requested_send_at),
        opt_text(error),
        opt_int(*owner_pid),
        int(*requires_mcp_send),
        opt_text(source_profile),
    ])
}
fn delivery_from(c: &mut Cur) -> rusqlite::Result<Delivery> {
    Ok(Delivery {
        id: c.big()?,
        key: c.get()?,
        request_hash: c.get()?,
        draft: c.json()?,
        plain: c.get()?,
        html: c.get()?,
        sender_name: c.get()?,
        attachment_bytes: c.big()?,
        status: c.name()?,
        created_at: c.get()?,
        updated_at: c.get()?,
        send_at: c.get()?,
        requested_send_at: c.get()?,
        error: c.get()?,
        owner_pid: c.get()?,
        requires_mcp_send: c.get()?,
        source_profile: c.get()?,
    })
}

fn undo_row(action: &UndoAction) -> AppResult<Vec<Value>> {
    let UndoAction { id, operation, subject, created_at_ms, expires_at_ms, status, error, retry_at_ms, attempts } = action;
    Ok(vec![
        big(*id),
        json(operation)?,
        text(subject),
        int(*created_at_ms),
        int(*expires_at_ms),
        name(status)?,
        opt_text(error),
        opt_int(*retry_at_ms),
        int(*attempts),
    ])
}
fn undo_from(c: &mut Cur) -> rusqlite::Result<UndoAction> {
    Ok(UndoAction {
        id: c.big()?,
        operation: c.json()?,
        subject: c.get()?,
        created_at_ms: c.get()?,
        expires_at_ms: c.get()?,
        status: c.name()?,
        error: c.get()?,
        retry_at_ms: c.get()?,
        attempts: c.get()?,
    })
}

fn event_row(event: &Event) -> AppResult<Vec<Value>> {
    let Event {
        id, title, start, end, location, attendees, meeting_link, conference, rsvp, notes, color,
    } = event;
    Ok(vec![
        big(*id),
        text(title),
        int(*start),
        int(*end),
        text(location),
        json(attendees)?,
        opt_text(meeting_link),
        text(conference),
        name(rsvp)?,
        text(notes),
        json(color)?,
    ])
}
fn event_from(c: &mut Cur) -> rusqlite::Result<Event> {
    Ok(Event {
        id: c.big()?,
        title: c.get()?,
        start: c.get()?,
        end: c.get()?,
        location: c.get()?,
        attendees: c.json()?,
        meeting_link: c.get()?,
        conference: c.get()?,
        rsvp: c.name()?,
        notes: c.get()?,
        color: c.json()?,
    })
}

fn meeting_row(meeting: &Meeting) -> AppResult<Vec<Value>> {
    let Meeting {
        id, title, date, duration_secs, audio_path, transcript, notes, summary, key_points,
        action_items, event_id,
    } = meeting;
    Ok(vec![
        big(*id),
        text(title),
        int(*date),
        big(*duration_secs),
        opt_text(audio_path),
        text(transcript),
        text(notes),
        text(summary),
        json(key_points)?,
        json(action_items)?,
        opt_big(*event_id),
    ])
}
fn meeting_from(c: &mut Cur) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: c.big()?,
        title: c.get()?,
        date: c.get()?,
        duration_secs: c.big()?,
        audio_path: c.get()?,
        transcript: c.get()?,
        notes: c.get()?,
        summary: c.get()?,
        key_points: c.json()?,
        action_items: c.json()?,
        event_id: c.opt_big()?,
    })
}

fn signature_row(signature: &Signature) -> Vec<Value> {
    let Signature { id, name, body, modified, html } = signature;
    vec![big(*id), text(name), text(body), int(*modified), int(*html)]
}
fn signature_from(c: &mut Cur) -> rusqlite::Result<Signature> {
    Ok(Signature {
        id: c.big()?,
        name: c.get()?,
        body: c.get()?,
        modified: c.get()?,
        html: c.get()?,
    })
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn select<T>(
    connection: &Connection,
    table: &Table,
    read: fn(&mut Cur) -> rusqlite::Result<T>,
) -> AppResult<Vec<T>> {
    let mut statement = connection.prepare_cached(table.select()).map_err(db)?;
    let rows = statement
        .query_map([], |row| read(&mut Cur::new(row)))
        .map_err(db)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db)
}

/// Child rows of every email, keyed by the email id, in list order.
fn children<T>(
    connection: &Connection,
    sql: &str,
    read: fn(&mut Cur) -> rusqlite::Result<T>,
) -> AppResult<HashMap<u64, Vec<T>>> {
    let mut statement = connection.prepare_cached(sql).map_err(db)?;
    let mut rows = statement.query([]).map_err(db)?;
    let mut grouped: HashMap<u64, Vec<T>> = HashMap::new();
    while let Some(row) = rows.next().map_err(db)? {
        let mut cursor = Cur::new(row);
        let owner = cursor.big().map_err(db)?;
        grouped.entry(owner).or_default().push(read(&mut cursor).map_err(db)?);
    }
    Ok(grouped)
}

fn read_store(connection: &Connection) -> AppResult<Store> {
    let meta: HashMap<String, String> = {
        let mut statement = connection.prepare_cached("SELECT name, value FROM meta").map_err(db)?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?
    };
    let flag = |key: &str| meta.get(key).is_some_and(|value| value == "1");
    let next_id = match meta.get("next_id") {
        Some(value) => value
            .parse()
            .map_err(|_| AppError::storage("Mail database has an unreadable id counter"))?,
        None => 0,
    };
    let settings: Option<String> = connection
        .query_row("SELECT json FROM settings WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map_err(db)?;
    let settings = match settings {
        Some(document) => serde_json::from_str::<Settings>(&document)
            .map_err(|e| AppError::storage(format!("Mail settings cannot be read: {e}")))?,
        None => Settings::default(),
    };

    let mut emails = select(connection, &EMAILS, email_from)?;
    let mut labels = children(
        connection,
        "SELECT email_id, label FROM email_labels ORDER BY email_id, position",
        |c| c.get::<String>(),
    )?;
    let mut locations = children(
        connection,
        "SELECT email_id, folder, uid, uid_validity FROM remote_locations ORDER BY email_id, position",
        |c| Ok(RemoteLocation { folder: c.get()?, uid: c.get()?, uid_validity: c.get()? }),
    )?;
    let mut attachments = children(
        connection,
        "SELECT email_id, id, name, size, mime, path, inline_text FROM attachments ORDER BY email_id, position",
        |c| {
            Ok(Attachment {
                id: c.big()?,
                name: c.get()?,
                size: c.big()?,
                mime: c.get()?,
                path: c.get()?,
                inline_text: c.get()?,
            })
        },
    )?;
    for email in &mut emails {
        email.labels = labels.remove(&email.id).unwrap_or_default();
        email.remote_locations = locations.remove(&email.id).unwrap_or_default();
        email.attachments = attachments.remove(&email.id).unwrap_or_default();
    }

    let cache_removals = {
        let mut statement = connection
            .prepare_cached("SELECT path FROM cache_removals ORDER BY position")
            .map_err(db)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?
    };

    Ok(Store {
        outbox: select(connection, &OUTBOX, delivery_from)?,
        cache_removals,
        storage_maintenance_pending: flag("storage_maintenance_pending"),
        storage_index_rebuild_required: flag("storage_index_rebuild_required"),
        undo_actions: select(connection, &UNDO_ACTIONS, undo_from)?,
        accounts: select(connection, &ACCOUNTS, account_from)?,
        emails,
        drafts: select(connection, &DRAFTS, draft_from)?,
        events: select(connection, &EVENTS, event_from)?,
        meetings: select(connection, &MEETINGS, meeting_from)?,
        settings,
        next_id,
        signatures: select(connection, &SIGNATURES, signature_from)?,
    })
}

// ---------------------------------------------------------------------------
// Writing: diff by id
// ---------------------------------------------------------------------------

/// Brings one table from `old` (what the transaction read) to `new`. Writes the
/// rows that changed, deletes those that disappeared, and returns the ids whose
/// row was inserted from scratch, so that dependent child rows are rewritten too.
/// Writes the difference of one table and reports the ids it inserted or
/// re-sequenced (`fresh`). `touched` additionally collects every id whose row
/// changed shape — inserted, updated, re-sequenced or deleted — for the
/// cross-process change feed (Task 4.3).
fn sync<T>(
    connection: &Connection,
    table: &Table,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> i64,
    row: impl Fn(&T) -> AppResult<Vec<Value>>,
    touched: &mut HashSet<i64>,
) -> AppResult<HashSet<i64>> {
    let new_ids: Vec<i64> = new.iter().map(&key).collect();
    let mut present = HashSet::with_capacity(new_ids.len());
    for id in &new_ids {
        if !present.insert(*id) {
            return Err(AppError::storage(format!(
                "Mail database: two {} share the id {}",
                table.name, *id as u64
            )));
        }
    }
    let old_ids: Vec<i64> = old.iter().map(&key).collect();
    // The surviving old records must stay a prefix of the new order; from the first
    // difference on (a move, or an insert before existing records) every record is
    // re-inserted so `seq`, and with it the vector order, is rebuilt.
    let kept: Vec<i64> = old_ids.iter().copied().filter(|id| present.contains(id)).collect();
    let matching = kept.iter().zip(&new_ids).take_while(|(a, b)| a == b).count();
    let resequence_from = if matching == kept.len() { new_ids.len() } else { matching };
    let previous: HashMap<i64, &T> = old.iter().map(|record| (key(record), record)).collect();

    let mut delete = connection.prepare_cached(table.delete()).map_err(db)?;
    for id in old_ids.iter().filter(|id| !present.contains(id)) {
        delete.execute([id]).map_err(db)?;
        touched.insert(*id);
    }
    let mut fresh = HashSet::new();
    let mut upsert = connection.prepare_cached(table.upsert()).map_err(db)?;
    for (index, record) in new.iter().enumerate() {
        let id = new_ids[index];
        let values = row(record)?;
        debug_assert_eq!(values.len(), table.cols.len(), "{} row shape", table.name);
        match previous.get(&id) {
            None => {
                fresh.insert(id);
            }
            Some(_) if index >= resequence_from => {
                delete.execute([id]).map_err(db)?;
                fresh.insert(id);
            }
            Some(existing) => {
                if row(existing)? == values {
                    continue;
                }
            }
        }
        touched.insert(id);
        upsert.execute(params_from_iter(values.iter())).map_err(db)?;
    }
    Ok(fresh)
}

/// The `emails` table difference: deletes the vanished rows, upserts the new
/// and changed ones, compared as values (no row is built for an unchanged
/// email). Returns the written emails with `true` for those (re)inserted from
/// scratch, whose child rows must be written whole. Keeps `seq` like `sync`.
fn sync_emails<'a, 'b>(
    connection: &Connection,
    old: &'b [Email],
    new: &'a [Email],
    root: Option<&Path>,
    diff: Option<&crate::domain::storage::EmailDiff>,
    touched: &mut HashSet<i64>,
) -> AppResult<(Vec<(&'a Email, Option<&'b Email>)>, Vec<&'b Email>)> {
    let mut deleted = Vec::new();
    let mut delete = connection.prepare_cached(EMAILS.delete()).map_err(db)?;
    // The difference found the ids unique and the order kept (`seq`): only the
    // touched records are written, and `old` may hold just their previous
    // versions (an in-place write).
    if let Some(diff) = diff.filter(|diff| diff.order_kept) {
        let mut upsert = connection.prepare_cached(EMAILS.upsert()).map_err(db)?;
        for index in &diff.removed {
            let email = &old[*index];
            delete.execute([email.id as i64]).map_err(db)?;
            touched.insert(email.id as i64);
            deleted.push(email);
        }
        let mut written = Vec::new();
        for (index, previous) in &diff.changed {
            let record = &new[*index];
            let previous = previous.map(|previous| &old[previous]);
            let values = email_row(&portable_email(record, root)?)?;
            if let Some(old) = previous {
                if email_row(&portable_email(old, root)?)? == values {
                    written.push((record, previous));
                    continue;
                }
            }
            touched.insert(record.id as i64);
            upsert.execute(params_from_iter(values.iter())).map_err(db)?;
            written.push((record, previous));
        }
        return Ok((written, deleted));
    }
    let mut present = HashSet::with_capacity(new.len());
    for email in new {
        if !present.insert(email.id) {
            return Err(AppError::storage(format!("Mail database: two emails share the id {}", email.id)));
        }
    }
    // The surviving old records must stay a prefix of the new order (`seq`).
    let kept = old.iter().filter(|e| present.contains(&e.id));
    let matching = kept.clone().zip(new).take_while(|(a, b)| a.id == b.id).count();
    let resequence_from = if matching == kept.count() { new.len() } else { matching };
    // Records usually keep their position: compare by index, and look a record
    // up by id only when the positions disagree.
    let mut by_id: Option<HashMap<u64, &'b Email>> = None;
    for email in old.iter().filter(|e| !present.contains(&e.id)) {
        delete.execute([email.id as i64]).map_err(db)?;
        touched.insert(email.id as i64);
        deleted.push(email);
    }
    let mut written = Vec::new();
    let mut upsert = connection.prepare_cached(EMAILS.upsert()).map_err(db)?;
    for (index, record) in new.iter().enumerate() {
        let id = record.id as i64;
        let existing = match old.get(index) {
            Some(candidate) if candidate.id == record.id => Some(candidate),
            _ => by_id
                .get_or_insert_with(|| old.iter().map(|e| (e.id, e)).collect())
                .get(&record.id)
                .copied(),
        };
        let previous = match existing {
            None => None,
            Some(_) if index >= resequence_from => {
                delete.execute([id]).map_err(db)?;
                None
            }
            Some(existing) => {
                if existing == record {
                    continue;
                }
                Some(existing)
            }
        };
        let values = email_row(&portable_email(record, root)?)?;
        if let Some(old) = previous {
            // Same disk row (e.g. only reader HTML differed): nothing to write.
            if email_row(&portable_email(old, root)?)? == values {
                written.push((record, previous));
                continue;
            }
        }
        touched.insert(id);
        upsert.execute(params_from_iter(values.iter())).map_err(db)?;
        written.push((record, previous));
    }
    Ok((written, deleted))
}

fn write_children(
    connection: &Connection,
    id: i64,
    previous: Option<&Email>,
    email: &Email,
) -> AppResult<()> {
    let existing = previous.is_some();
    if previous.is_none_or(|old| old.labels != email.labels) {
        if existing {
            connection
                .prepare_cached("DELETE FROM email_labels WHERE email_id = ?1")
                .and_then(|mut s| s.execute([id]))
                .map_err(db)?;
        }
        let mut insert = connection
            .prepare_cached("INSERT INTO email_labels (email_id, position, label) VALUES (?1, ?2, ?3)")
            .map_err(db)?;
        for (position, label) in email.labels.iter().enumerate() {
            insert.execute((id, position as i64, label)).map_err(db)?;
        }
    }
    if previous.is_none_or(|old| old.remote_locations != email.remote_locations) {
        if existing {
            connection
                .prepare_cached("DELETE FROM remote_locations WHERE email_id = ?1")
                .and_then(|mut s| s.execute([id]))
                .map_err(db)?;
        }
        let mut insert = connection
            .prepare_cached("INSERT INTO remote_locations (email_id, position, folder, uid, uid_validity) VALUES (?1, ?2, ?3, ?4, ?5)")
            .map_err(db)?;
        for (position, location) in email.remote_locations.iter().enumerate() {
            let RemoteLocation { folder, uid, uid_validity } = location;
            insert
                .execute((id, position as i64, folder, uid, uid_validity))
                .map_err(db)?;
        }
    }
    let rows = |list: &[Attachment]| list.iter().map(attachment_row).collect::<Vec<_>>();
    if previous.is_none_or(|old| rows(&old.attachments) != rows(&email.attachments)) {
        if existing {
            connection
                .prepare_cached("DELETE FROM attachments WHERE email_id = ?1")
                .and_then(|mut s| s.execute([id]))
                .map_err(db)?;
        }
        let mut insert = connection
            .prepare_cached("INSERT INTO attachments (email_id, position, id, name, size, mime, path, inline_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
            .map_err(db)?;
        for (position, attachment) in email.attachments.iter().enumerate() {
            let values = attachment_row(attachment);
            insert
                .execute(params_from_iter(
                    [int(id), int(position as i64)].iter().chain(values.iter()),
                ))
                .map_err(db)?;
        }
    }
    Ok(())
}

fn put_meta(connection: &Connection, name: &str, value: &str) -> AppResult<()> {
    connection
        .prepare_cached(
            "INSERT INTO meta (name, value) VALUES (?1, ?2) \
             ON CONFLICT (name) DO UPDATE SET value = excluded.value",
        )
        .and_then(|mut s| s.execute((name, value)))
        .map_err(db)?;
    Ok(())
}

/// Disk form of one email: portable file references, no reader-only HTML.
fn portable_email(email: &Email, root: Option<&Path>) -> AppResult<Email> {
    let mut email = email.clone();
    email.body_html.clear();
    if let Some(root) = root {
        let mut single = Store { emails: vec![email], ..Default::default() };
        super::portable_files::persist_store_paths(&mut single, root)?;
        email = single.emails.pop().unwrap_or_default();
    }
    Ok(email)
}

/// Disk form of everything but the emails (cheap: no emails are copied).
fn portable_rest(store: &Store, root: Option<&Path>) -> AppResult<Store> {
    let mut rest = without_emails(store);
    if let Some(root) = root {
        super::portable_files::persist_store_paths(&mut rest, root)?;
    }
    Ok(rest)
}

/// Writes the difference between `old` (the committed store this transaction
/// started from) and `new` (the store after the operation), both in memory
/// form. Emails are compared as values and only the changed ones are turned
/// into rows (W2.3); the small tables are compared in disk form as before.
/// Every write moves the store generation that snapshots are keyed by.
/// Returns the disk form of the small tables it wrote, the next base.
fn write_store(connection: &Connection, old_store: &Store, old_rest: &Store, new_store: &Store, root: Option<&Path>) -> AppResult<Written> {
    write_store_with(connection, old_store, old_rest, new_store, root, None)
}

/// `write_store` with the emails' difference already computed, when known.
fn write_store_with(
    connection: &Connection,
    old_store: &Store,
    old_rest: &Store,
    new_store: &Store,
    root: Option<&Path>,
    diff: Option<&crate::domain::storage::EmailDiff>,
) -> AppResult<Written> {
    let changes_before = connection.total_changes();
    let old = old_rest;
    let new_rest = portable_rest(new_store, root)?;
    let new = &new_rest;
    // Exhaustive: a new `Store` field must be given a home here.
    let Store {
        outbox, cache_removals, storage_maintenance_pending, storage_index_rebuild_required,
        undo_actions, accounts, emails: _, drafts, events, meetings, settings, next_id, signatures,
    } = new;
    let emails = &new_store.emails;

    // The per-table write sets double as the cross-process change feed's
    // payload (Task 4.3): what this transaction touched is what the other
    // process's poller must announce.
    let mut email_rows: HashSet<i64> = HashSet::new();
    let mut account_rows: HashSet<i64> = HashSet::new();
    let mut draft_rows: HashSet<i64> = HashSet::new();
    let mut outbox_rows: HashSet<i64> = HashSet::new();
    let mut undo_rows: HashSet<i64> = HashSet::new();

    // Emails first: deleting one cascades to its child rows.
    let (changed, deleted) = sync_emails(connection, &old_store.emails, emails, root, diff, &mut email_rows)?;
    for (email, before) in &changed {
        write_children(connection, email.id as i64, before.map(|old| portable_email(old, root)).transpose()?.as_ref(), &portable_email(email, root)?)?;
    }

    sync(connection, &ACCOUNTS, &old.accounts, accounts, |a| i64::from(a.id), account_row, &mut account_rows)?;
    sync(connection, &DRAFTS, &old.drafts, drafts, |d| d.id as i64, draft_row, &mut draft_rows)?;
    sync(connection, &OUTBOX, &old.outbox, outbox, |d| d.id as i64, delivery_row, &mut outbox_rows)?;
    sync(connection, &UNDO_ACTIONS, &old.undo_actions, undo_actions, |a| a.id as i64, undo_row, &mut undo_rows)?;
    // Meetings, calendar events and signatures refresh through their own
    // surfaces; the feed carries only what the poller can announce as typed
    // mail events or what can move a scheduler deadline.
    sync(connection, &EVENTS, &old.events, events, |e| e.id as i64, event_row, &mut HashSet::new())?;
    sync(connection, &MEETINGS, &old.meetings, meetings, |m| m.id as i64, meeting_row, &mut HashSet::new())?;
    sync(connection, &SIGNATURES, &old.signatures, signatures, |s| s.id as i64, |s| Ok(signature_row(s)), &mut HashSet::new())?;

    let document = serde_json::to_string(settings)?;
    if document != serde_json::to_string(&old.settings)? {
        connection
            .prepare_cached(
                "INSERT INTO settings (id, json) VALUES (1, ?1) \
                 ON CONFLICT (id) DO UPDATE SET json = excluded.json",
            )
            .and_then(|mut s| s.execute([&document]))
            .map_err(db)?;
    }
    if *next_id != old.next_id {
        put_meta(connection, "next_id", &next_id.to_string())?;
    }
    for (key, value, was) in [
        (
            "storage_maintenance_pending",
            *storage_maintenance_pending,
            old.storage_maintenance_pending,
        ),
        (
            "storage_index_rebuild_required",
            *storage_index_rebuild_required,
            old.storage_index_rebuild_required,
        ),
    ] {
        if value != was {
            put_meta(connection, key, if value { "1" } else { "0" })?;
        }
    }
    if *cache_removals != old.cache_removals {
        connection.execute("DELETE FROM cache_removals", []).map_err(db)?;
        let mut insert = connection
            .prepare_cached("INSERT INTO cache_removals (position, path) VALUES (?1, ?2)")
            .map_err(db)?;
        for (position, path) in cache_removals.iter().enumerate() {
            insert.execute((position as i64, path)).map_err(db)?;
        }
    }

    // Cross-process change feed (Task 4.3, EX-005 / decision D-crossproc):
    // appended inside the same IMMEDIATE transaction, so a poller can never
    // see a feed row without the data it announces. `pid` is the writing
    // process: the desktop poller skips its own rows, whose writers already
    // emitted the events themselves.
    let pid = std::process::id();
    let created_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    // Only a transaction that wrote something moves the generation.
    let moved = connection.total_changes() != changes_before;
    if moved {
        let generation = generation(connection)?;
        put_meta(connection, "generation", &(generation + 1).to_string())?;
    }
    if !email_rows.is_empty() {
        // One row per account with the ids it touched; a deleted email takes
        // its account from `old` (the UI drops rows that read_rows omits).
        let mut per_account: HashMap<u32, Vec<u64>> = HashMap::new();
        let moved = changed.iter().flat_map(|(new, old)| std::iter::once(*new).chain(*old));
        for email in moved.chain(deleted.iter().copied()) {
            if email_rows.contains(&(email.id as i64)) {
                per_account
                    .entry(email.account_id)
                    .or_default()
                    .push(email.id);
            }
        }
        for (account_id, mut ids) in per_account {
            ids.sort_unstable();
            ids.dedup();
            append_feed(connection, "email-changed", Some(account_id), &ids, pid, created_ms)?;
        }
    }
    let sorted = |rows: &HashSet<i64>| -> Vec<u64> {
        let mut ids: Vec<u64> = rows.iter().map(|id| *id as u64).collect();
        ids.sort_unstable();
        ids
    };
    append_feed(connection, "accounts-changed", None, &sorted(&account_rows), pid, created_ms)?;
    append_feed(connection, "drafts-changed", None, &sorted(&draft_rows), pid, created_ms)?;
    append_feed(connection, "outbox-changed", None, &sorted(&outbox_rows), pid, created_ms)?;
    append_feed(connection, "undo-changed", None, &sorted(&undo_rows), pid, created_ms)?;
    Ok(Written { rest: new_rest, moved })
}

/// What `write_store` committed: the small tables' disk form and whether the
/// store generation moved (it does only when a row changed).
struct Written {
    rest: Store,
    moved: bool,
}

/// One change-feed row. An empty id list announces nothing (INV-004) and is
/// skipped; called inside the writing transaction only.
fn append_feed(
    connection: &Connection,
    kind: &str,
    account_id: Option<u32>,
    ids: &[u64],
    pid: u32,
    created_ms: i64,
) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let document = serde_json::to_string(ids)?;
    connection
        .prepare_cached(
            "INSERT INTO change_feed (kind, account_id, ids, pid, created_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut statement| {
            statement.execute((
                kind,
                account_id.map(i64::from),
                &document,
                i64::from(pid),
                created_ms,
            ))
        })
        .map_err(db)?;
    Ok(())
}
