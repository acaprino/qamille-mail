//! The activity log's own database (centro attività): one small SQLite file next to the store,
//! `activity.sqlite`, opened by both processes (the desktop and the MCP server) whatever engine
//! the mail store runs on. It is deliberately not a table of the store database: the history
//! must work on the JSON engine the app ships with today, and it joins nothing. A write is one
//! short transaction; a read never waits for a writer (WAL).
//!
//! Besides the rows (`activity_log`) it keeps the "seen" marker (`activity_meta`) and a small
//! feed (`activity_feed`): every write appends the row's `seq` and the writer's pid, and the
//! desktop polls the feed to announce what the *other* process wrote (`ActivityChanged`).
use crate::application::{AppError, AppResult, ports::ActivityStore};
use crate::domain::activity::{
    ActivityEntry, ActivityFilter, ActivityKind, ActivityOutcome, ActivitySource, NewActivity,
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::Duration,
};

/// The file name inside the data directory.
pub const ACTIVITY_FILE: &str = "activity.sqlite";
/// How long a write waits for the other process's write. A history row is never worth more
/// than this on the path of a user's action.
const BUSY_TIMEOUT: Duration = Duration::from_millis(2000);
/// Feed rows kept: far more than a 2 s poller can fall behind.
const FEED_KEEP: i64 = 1000;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS activity_log (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms      INTEGER NOT NULL,
    changed_ms INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    code       TEXT NOT NULL,
    source     TEXT NOT NULL,
    account_id INTEGER,
    subject    TEXT,
    detail     TEXT,
    count      INTEGER,
    ref_kind   TEXT,
    ref_id     INTEGER,
    undo_id    INTEGER,
    outcome    TEXT NOT NULL DEFAULT 'done'
);
CREATE INDEX IF NOT EXISTS activity_log_at ON activity_log(at_ms);
CREATE INDEX IF NOT EXISTS activity_log_undo ON activity_log(undo_id) WHERE undo_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS activity_log_code ON activity_log(code, account_id, seq);
CREATE TABLE IF NOT EXISTS activity_feed (
    seq          INTEGER PRIMARY KEY AUTOINCREMENT,
    activity_seq INTEGER NOT NULL,
    pid          INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS activity_meta (
    name  TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

const COLUMNS: &str = "seq, at_ms, kind, code, source, account_id, subject, detail, \
     count, ref_kind, ref_id, undo_id, outcome";

fn db(error: rusqlite::Error) -> AppError {
    AppError::storage(format!("Activity history: {error}"))
}

pub struct ActivityDb {
    path: PathBuf,
    /// Opened on the first use, so a data directory that never records anything never gets the file.
    connection: Mutex<Option<Connection>>,
}

impl ActivityDb {
    /// The log of the data directory `dir`. Nothing is opened or created yet.
    pub fn in_dir(dir: &Path) -> Self {
        Self { path: dir.join(ACTIVITY_FILE), connection: Mutex::new(None) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&self) -> AppResult<Connection> {
        if let Some(parent) = self.path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&self.path).map_err(db)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(db)?;
        // The journal mode is stored in the file; NORMAL is enough for a history (a crash can lose
        // the last row, never corrupt the file).
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(db)?;
        connection.execute_batch(SCHEMA).map_err(db)?;
        Ok(connection)
    }

    /// Runs `work` on the one connection of this process, opening it first when needed. After an
    /// error the connection is dropped, so the next call starts from a fresh one.
    fn with<T>(&self, work: impl FnOnce(&mut Connection) -> AppResult<T>) -> AppResult<T> {
        let mut guard = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(self.open()?);
        }
        let result = work(guard.as_mut().expect("opened above"));
        if result.is_err() {
            *guard = None;
        }
        result
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn entry(row: &Row<'_>) -> rusqlite::Result<ActivityEntry> {
    let text = |at: usize| row.get::<_, String>(at);
    Ok(ActivityEntry {
        seq: row.get(0)?,
        at_ms: row.get(1)?,
        kind: ActivityKind::parse(&text(2)?),
        code: text(3)?,
        source: ActivitySource::parse(&text(4)?),
        account_id: row.get::<_, Option<i64>>(5)?.map(|id| id as u32),
        subject: row.get(6)?,
        detail: row.get(7)?,
        count: row.get::<_, Option<i64>>(8)?.map(|count| count as u32),
        ref_kind: row.get(9)?,
        ref_id: row.get::<_, Option<i64>>(10)?.map(|id| id as u64),
        undo_id: row.get::<_, Option<i64>>(11)?.map(|id| id as u64),
        outcome: ActivityOutcome::parse(&text(12)?),
    })
}

fn feed(connection: &Connection, activity_seq: i64) -> AppResult<()> {
    connection
        .execute(
            "INSERT INTO activity_feed (activity_seq, pid) VALUES (?1, ?2)",
            rusqlite::params![activity_seq, i64::from(std::process::id())],
        )
        .map_err(db)?;
    Ok(())
}

fn seen(connection: &Connection) -> AppResult<i64> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM activity_meta WHERE name = 'seen_ms'", [], |row| row.get(0))
        .optional()
        .map_err(db)?;
    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

impl ActivityStore for ActivityDb {
    fn append(&self, new: &NewActivity, at_ms: i64) -> AppResult<i64> {
        self.with(|connection| {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            tx.execute(
                "INSERT INTO activity_log (at_ms, changed_ms, kind, code, source, account_id, \
                 subject, detail, count, ref_kind, ref_id, undo_id, outcome) \
                 VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    at_ms,
                    new.kind.as_str(),
                    new.code,
                    new.source.as_str(),
                    new.account_id.map(i64::from),
                    new.subject,
                    new.detail,
                    new.count.map(i64::from),
                    new.ref_kind,
                    new.ref_id.map(|id| id as i64),
                    new.undo_id.map(|id| id as i64),
                    new.outcome.as_str(),
                ],
            )
            .map_err(db)?;
            let seq = tx.last_insert_rowid();
            feed(&tx, seq)?;
            tx.commit().map_err(db)?;
            Ok(seq)
        })
    }

    fn set_outcome(
        &self,
        undo_id: u64,
        outcome: ActivityOutcome,
        detail: Option<&str>,
    ) -> AppResult<Option<i64>> {
        self.with(|connection| {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db)?;
            let seq: Option<i64> = tx
                .query_row(
                    "SELECT MAX(seq) FROM activity_log WHERE undo_id = ?1",
                    [undo_id as i64],
                    |row| row.get(0),
                )
                .map_err(db)?;
            let Some(seq) = seq else {
                return Ok(None);
            };
            // `changed_ms` is when the row last changed: a failure that lands after the user
            // opened the panel is still news, whatever time the action was queued at.
            tx.execute(
                "UPDATE activity_log SET outcome = ?2, detail = COALESCE(?3, detail), \
                 changed_ms = ?4 WHERE seq = ?1",
                rusqlite::params![seq, outcome.as_str(), detail, now_ms()],
            )
            .map_err(db)?;
            feed(&tx, seq)?;
            tx.commit().map_err(db)?;
            Ok(Some(seq))
        })
    }

    fn page(
        &self,
        before_seq: Option<i64>,
        limit: u32,
        filter: ActivityFilter,
    ) -> AppResult<Vec<ActivityEntry>> {
        // A closed enum, so the condition is a literal, never user text.
        let condition = match filter {
            ActivityFilter::All => "1",
            ActivityFilter::Action => "kind = 'action'",
            ActivityFilter::Event => "kind = 'event'",
            ActivityFilter::Error => "(kind = 'error' OR outcome = 'failed')",
        };
        self.with(|connection| {
            let mut statement = connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM activity_log \
                     WHERE (?1 IS NULL OR seq < ?1) AND {condition} ORDER BY seq DESC LIMIT ?2"
                ))
                .map_err(db)?;
            let rows = statement
                .query_map((before_seq, i64::from(limit)), entry)
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    fn last(&self, codes: &[&str], account_id: Option<u32>) -> AppResult<Option<ActivityEntry>> {
        if codes.is_empty() {
            return Ok(None);
        }
        let placeholders = (0..codes.len())
            .map(|at| format!("?{}", at + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let account = account_id.map(i64::from);
        self.with(|connection| {
            let mut values: Vec<&dyn rusqlite::ToSql> = vec![&account];
            values.extend(codes.iter().map(|code| code as &dyn rusqlite::ToSql));
            connection
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM activity_log \
                     WHERE account_id IS ?1 AND code IN ({placeholders}) ORDER BY seq DESC LIMIT 1"
                ))
                .and_then(|mut statement| {
                    statement.query_row(rusqlite::params_from_iter(values), entry).optional()
                })
                .map_err(db)
        })
    }

    fn seen_ms(&self) -> AppResult<i64> {
        self.with(|connection| seen(connection))
    }

    fn set_seen_ms(&self, at_ms: i64) -> AppResult<()> {
        self.with(|connection| {
            connection
                .execute(
                    "INSERT INTO activity_meta (name, value) VALUES ('seen_ms', ?1) \
                     ON CONFLICT (name) DO UPDATE SET value = excluded.value",
                    [at_ms.to_string()],
                )
                .map_err(db)?;
            Ok(())
        })
    }

    fn unseen_errors(&self) -> AppResult<u32> {
        self.with(|connection| {
            let seen = seen(connection)?;
            connection
                .query_row(
                    "SELECT COUNT(*) FROM activity_log \
                     WHERE (kind = 'error' OR outcome = 'failed') AND changed_ms > ?1",
                    [seen],
                    |row| row.get(0),
                )
                .map_err(db)
        })
    }

    fn feed_head(&self) -> AppResult<i64> {
        self.with(|connection| {
            connection
                .query_row("SELECT COALESCE(MAX(seq), 0) FROM activity_feed", [], |row| row.get(0))
                .map_err(db)
        })
    }

    fn feed_since(&self, since: i64, exclude_pid: u32) -> AppResult<(i64, Vec<u64>)> {
        self.with(|connection| {
            let head: i64 = connection
                .query_row("SELECT COALESCE(MAX(seq), 0) FROM activity_feed", [], |row| row.get(0))
                .map_err(db)?;
            if head <= since {
                return Ok((since, Vec::new()));
            }
            let mut statement = connection
                .prepare_cached(
                    "SELECT DISTINCT activity_seq FROM activity_feed \
                     WHERE seq > ?1 AND seq <= ?2 AND pid != ?3 ORDER BY activity_seq",
                )
                .map_err(db)?;
            let ids = statement
                .query_map((since, head, i64::from(exclude_pid)), |row| row.get::<_, i64>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            Ok((head, ids.into_iter().map(|id| id as u64).collect()))
        })
    }

    fn prune(&self, before_ms: i64) -> AppResult<()> {
        self.with(|connection| {
            connection
                .execute("DELETE FROM activity_log WHERE at_ms < ?1", [before_ms])
                .map_err(db)?;
            connection
                .execute(
                    "DELETE FROM activity_feed WHERE seq <= \
                     (SELECT COALESCE(MAX(seq), 0) FROM activity_feed) - ?1",
                    [FEED_KEEP],
                )
                .map_err(db)?;
            Ok(())
        })
    }
}
