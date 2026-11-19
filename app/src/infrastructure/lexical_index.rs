//! Account-local BM25 index: terms/positions only, no duplicate source bodies.
use crate::{
    application::{AppError, AppResult},
    domain::search::ParsedQuery,
    model::{Email, Store},
};
use rusqlite::{Connection, params};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

fn path(root: &Path, account: u32) -> PathBuf {
    root.join("search-lexical")
        .join(format!("{account}.sqlite"))
}
fn error(e: rusqlite::Error) -> AppError {
    if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull) {
        AppError::new(
            "storage_full",
            "Account storage limit reached while writing the word index",
        )
    } else {
        AppError::storage(format!("Word index: {e}"))
    }
}
fn rebuildable(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
    ) || matches!(
        error,
        rusqlite::Error::InvalidColumnType(..) | rusqlite::Error::FromSqlConversionFailure(..)
    ) || matches!(error,rusqlite::Error::SqliteFailure(_,Some(message)) if message.starts_with("no such table: documents") || message.starts_with("no such table: terms") || message.starts_with("no such column: hash"))
}
fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.prepare("SELECT id,hash FROM documents LIMIT 0")?;
    db.prepare("SELECT rowid FROM terms LIMIT 0")?;
    Ok(())
}
pub struct Inspection {
    pub rebuild_ids: Vec<u64>,
    pub quarantined: bool,
}
/// Caller holds the repository lock. This scans only document IDs/hashes, not
/// text/vector pages. Deep FTS damage is signalled by a failing reader.
pub fn inspect(root: &Path, account: u32, emails: &[&Email]) -> AppResult<Inspection> {
    let file = path(root, account);
    let expected: HashMap<_, _> = emails
        .iter()
        .filter(|email| {
            email.search_index.version >= 3 && !email.search_index.partial && !email.storage_evicted
        })
        .map(|email| (email.id, *email))
        .collect();
    if !file.exists() {
        return Ok(Inspection {
            rebuild_ids: expected.keys().copied().collect(),
            quarantined: false,
        });
    }
    let checked = (|| -> rusqlite::Result<Vec<u64>> {
        let db = Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        schema(&db)?;
        if file.with_extension("repair").exists() {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                None,
            ));
        }
        let mut pending: std::collections::HashSet<_> = expected.keys().copied().collect();
        let mut statement = db.prepare("SELECT id,hash FROM documents")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id = row.get::<_, i64>(0)? as u64;
            if let Some(email) = expected.get(&id) {
                if row.get::<_, String>(1)? == email.remote_content_hash {
                    pending.remove(&id);
                }
            }
        }
        Ok(pending.into_iter().collect())
    })();
    match checked {
        Ok(ids) => Ok(Inspection {
            rebuild_ids: ids,
            quarantined: false,
        }),
        Err(failure) if rebuildable(&failure) => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let quarantine = file.with_file_name(format!(
                "{account}.corrupt-{}-{stamp}.sqlite",
                std::process::id()
            ));
            match std::fs::remove_file(file.with_extension("repair")) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let journal = PathBuf::from(format!("{}-journal", file.display()));
            let quarantined_journal = PathBuf::from(format!("{}-journal", quarantine.display()));
            if journal.exists() {
                std::fs::rename(&journal, &quarantined_journal)?;
            }
            if let Err(e) = std::fs::rename(&file, &quarantine) {
                if quarantined_journal.exists() {
                    let _ = std::fs::rename(quarantined_journal, journal);
                }
                return Err(e.into());
            }
            Ok(Inspection {
                rebuild_ids: emails
                    .iter()
                    .filter(|email| email.search_index.version >= 3 && !email.storage_evicted)
                    .map(|email| email.id)
                    .collect(),
                quarantined: true,
            })
        }
        Err(failure) => Err(error(failure)),
    }
}
/// Staged corrupt indexes are generated caches. Call only after invalidation
/// checkpoints are durable, or for accounts absent from the durable store.
pub fn discard_quarantines(root: &Path) -> std::io::Result<()> {
    let dir = root.join("search-lexical");
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.split_once(".corrupt-").is_some_and(|(id, suffix)| {
            id.parse::<u32>().is_ok()
                && (suffix.ends_with(".sqlite") || suffix.ends_with(".sqlite-journal"))
        }) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
fn key(id: u64) -> AppResult<i64> {
    i64::try_from(id).map_err(|_| AppError::validation("Message identifier exceeds SQLite range"))
}
fn configure(db: &Connection) -> AppResult<()> {
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(error)?;
    db.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA journal_mode=DELETE;")
        .map_err(error)
}
pub fn write(root: &Path, email: &Email, remaining: Option<u64>) -> AppResult<()> {
    let target = path(root, email.account_id);
    let fresh = !target.exists();
    let file = if fresh {
        target.with_extension("new")
    } else {
        target.clone()
    };
    struct Temporary(Option<PathBuf>);
    impl Drop for Temporary {
        fn drop(&mut self) {
            if let Some(path) = &self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _temporary = Temporary(fresh.then(|| file.clone()));
    if fresh && remaining.is_some_and(|n| n < 131072) {
        return Err(AppError::new(
            "storage_full",
            "Account needs at least 128 KiB for its word index and transaction reserve",
        ));
    }
    std::fs::create_dir_all(file.parent().unwrap())?;
    if fresh {
        // A crash before publication leaves only disposable, unpublished data.
        // Never reuse its pages as an unaccounted part of a new transaction.
        for orphan in [
            file.clone(),
            PathBuf::from(format!("{}-journal", file.display())),
        ] {
            match std::fs::remove_file(orphan) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    let attachment_text = email
        .attachments
        .iter()
        .map(|a| format!("{}\n{}", a.name, a.inline_text.as_deref().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n");
    let working =
        (email.body.len() + attachment_text.len() + email.subject.len()) as u64 * 8 + 65536;
    let journal_budget = std::fs::metadata(&target)
        .map_or(0, |m| m.len())
        .saturating_mul(101)
        / 100
        + 65536;
    if remaining.is_some_and(|bytes| bytes < journal_budget) {
        return Err(AppError::new(
            "storage_full",
            "Account needs more transaction space to update its word index",
        ));
    }
    let _space = super::storage_fs::reserve(root, working.saturating_add(journal_budget))?;
    let mut db = Connection::open(&file).map_err(error)?;
    configure(&db)?;
    if fresh {
        db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")
            .map_err(error)?;
    }
    let old_size = std::fs::metadata(&file)?.len();
    let max_bytes = remaining.map(|n| old_size.saturating_add(n.saturating_sub(journal_budget)));
    if let Some(bytes) = max_bytes {
        db.pragma_update(
            None,
            "max_page_count",
            (bytes / 4096).max(1).min(i64::MAX as u64) as i64,
        )
        .map_err(error)?;
    }
    db.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS terms USING fts5(subject, sender, recipients, body, attachments, content='', contentless_delete=1, tokenize='unicode61', detail=full, columnsize=1);
        CREATE TABLE IF NOT EXISTS documents(id INTEGER PRIMARY KEY, hash TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS maintenance(key TEXT PRIMARY KEY,value INTEGER NOT NULL);").map_err(error)?;
    let tx = db.transaction().map_err(error)?;
    tx.execute("DELETE FROM terms WHERE rowid=?1", [key(email.id)?])
        .map_err(error)?;
    tx.execute("INSERT INTO terms(rowid,subject,sender,recipients,body,attachments) VALUES(?1,?2,?3,?4,?5,?6)",params![key(email.id)?,email.subject,format!("{} {}",email.from_name,email.from_addr),email.to.iter().chain(&email.cc).cloned().collect::<Vec<_>>().join(" "),email.body,attachment_text]).map_err(error)?;
    tx.execute(
        "INSERT OR REPLACE INTO documents(id,hash) VALUES(?1,?2)",
        params![key(email.id)?, email.remote_content_hash],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    drop(db);
    if fresh {
        std::fs::rename(file, target)?;
    }
    Ok(())
}
pub struct Reader {
    db: Connection,
    account: u32,
    file: PathBuf,
    /// Every document's content hash, read once per reader (W2.3): one query
    /// instead of one per message.
    hashes: RefCell<Option<HashMap<i64, String>>>,
}
/// Regular mail search uses BM25 too, with filters before sorting/pagination.
pub fn search(
    root: &Path,
    store: &Store,
    query: &str,
    account: Option<u32>,
    now: i64,
    sort: &str,
) -> AppResult<Option<Vec<u64>>> {
    let parsed =
        ParsedQuery::parse_scoped(store, query, now, account).map_err(AppError::validation)?;
    let mut readers = HashMap::new();
    let mut ranked = Vec::new();
    for owner in store
        .accounts
        .iter()
        .filter(|owner| account.is_none_or(|id| id == owner.id))
    {
        if let Some(reader) = Reader::open(root, owner.id)? {
            ranked.extend(reader.rank(store, &parsed, store.emails.len())?);
            readers.insert(owner.id, reader);
        }
    }
    if readers.is_empty() {
        return Ok(None);
    }
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
    let mut ids: Vec<_> = ranked.into_iter().map(|hit| hit.0).collect();
    let mut seen: std::collections::HashSet<_> = ids.iter().copied().collect();
    for email in store
        .emails
        .iter()
        .filter(|email| account.is_none_or(|id| id == email.account_id))
    {
        let matched = match readers.get(&email.account_id) {
            Some(reader) if reader.current(email)? => reader.matches(email, &parsed, false)?,
            _ => parsed.matches(email, store),
        };
        if matched && seen.insert(email.id) {
            ids.push(email.id);
        }
    }
    let known: HashMap<_, _> = store.emails.iter().map(|email| (email.id, email)).collect();
    let nearest = matches!(sort, "nearest" | "closest")
        || (sort.is_empty() && parsed.words().is_empty() && parsed.around_center().is_some());
    if nearest {
        ids.sort_by_key(|id| {
            (
                parsed.date_distance(known[id].date),
                std::cmp::Reverse(known[id].date),
                std::cmp::Reverse(*id),
            )
        });
    } else if sort == "oldest" {
        ids.sort_by_key(|id| (known[id].date, std::cmp::Reverse(*id)));
    } else if matches!(sort, "date" | "newest") || parsed.words().is_empty() {
        ids.sort_by_key(|id| std::cmp::Reverse((known[id].date, *id)));
    }
    Ok(Some(ids))
}
impl Reader {
    pub fn open(root: &Path, account: u32) -> AppResult<Option<Self>> {
        let file = path(root, account);
        if !file.exists() {
            return Ok(None);
        }
        let db =
            match Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
                Ok(db) => db,
                Err(failure) if rebuildable(&failure) => return Ok(None),
                Err(failure) => return Err(error(failure)),
            };
        match schema(&db) {
            Ok(()) => {}
            Err(failure) if rebuildable(&failure) => return Ok(None),
            Err(failure) => return Err(error(failure)),
        }
        if file.with_extension("repair").exists() {
            return Ok(None);
        }
        db.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE;")
            .map_err(error)?;
        Ok(Some(Self { db, account, file, hashes: RefCell::new(None) }))
    }
    fn failure(&self, failure: rusqlite::Error) -> AppError {
        if rebuildable(&failure) {
            let _ = std::fs::write(self.file.with_extension("repair"), b"");
        }
        error(failure)
    }
    pub fn current(&self, email: &Email) -> AppResult<bool> {
        if self.hashes.borrow().is_none() {
            let mut statement = self
                .db
                .prepare("SELECT id, hash FROM documents")
                .map_err(|failure| self.failure(failure))?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)))
                .map_err(|failure| self.failure(failure))?
                .filter_map(|row| row.map(|(id, hash)| hash.map(|hash| (id, hash))).transpose())
                .collect::<Result<HashMap<_, _>, _>>()
                .map_err(|failure| self.failure(failure))?;
            *self.hashes.borrow_mut() = Some(rows);
        }
        let id = key(email.id)?;
        let hashes = self.hashes.borrow();
        let hash = hashes.as_ref().and_then(|hashes| hashes.get(&id));
        Ok(email.account_id == self.account
            && !email.storage_evicted
            && email.search_index.version >= 3
            && email.search_index.content_hash == email.remote_content_hash
            && hash.map(String::as_str) == Some(email.remote_content_hash.as_str()))
    }
    fn contains(&self, id: u64, field: Option<&str>, text: &str) -> AppResult<bool> {
        let phrase = format!(
            "{}\"{}\"",
            if field == Some("body") { "body : " } else { "" },
            text.replace('"', "\"\"")
        );
        self.db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM terms WHERE terms MATCH ?1 AND rowid=?2)",
                params![phrase, key(id)?],
                |row| row.get(0),
            )
            .map_err(|failure| self.failure(failure))
    }
    pub fn matches(&self, email: &Email, parsed: &ParsedQuery, semantic: bool) -> AppResult<bool> {
        let failure = RefCell::new(None);
        let matched = parsed.indexed_matches(email, semantic, &|field, text| match self
            .contains(email.id, field, text)
        {
            Ok(value) => value,
            Err(e) => {
                *failure.borrow_mut() = Some(e);
                false
            }
        });
        if let Some(error) = failure.into_inner() {
            Err(error)
        } else {
            Ok(matched)
        }
    }
    pub fn rank(
        &self,
        store: &Store,
        parsed: &ParsedQuery,
        limit: usize,
    ) -> AppResult<Vec<(u64, f64)>> {
        let expression = parsed
            .words()
            .iter()
            .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if expression.is_empty() {
            return Ok(vec![]);
        }
        let mut statement=self.db.prepare("SELECT rowid,bm25(terms,5.0,3.0,2.0,1.0,1.0) AS relevance FROM terms WHERE terms MATCH ?1 ORDER BY relevance, rowid DESC").map_err(|failure|self.failure(failure))?;
        let mut rows = statement
            .query([expression])
            .map_err(|failure| self.failure(failure))?;
        let known: HashMap<_, _> = store.emails.iter().map(|email| (email.id, email)).collect();
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(|failure| self.failure(failure))? {
            let id = row
                .get::<_, i64>(0)
                .map_err(|failure| self.failure(failure))? as u64;
            let Some(email) = known.get(&id) else {
                continue;
            };
            if self.current(email)? && self.matches(email, parsed, false)? {
                result.push((id, row.get(1).map_err(|failure| self.failure(failure))?));
            }
            if result.len() >= limit {
                break;
            }
        }
        Ok(result)
    }
}
pub fn message_bytes(root: &Path, store: &Store) -> std::io::Result<HashMap<u64, u64>> {
    let mut account_sizes: HashMap<u32, u64> = HashMap::new();
    let dir = root.join("search-lexical");
    if dir.exists() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let account = name
                .strip_suffix(".sqlite")
                .and_then(|id| id.parse::<u32>().ok())
                .or_else(|| {
                    name.split_once(".corrupt-").and_then(|(id, suffix)| {
                        (suffix.ends_with(".sqlite") || suffix.ends_with(".sqlite-journal"))
                            .then(|| id.parse::<u32>().ok())
                            .flatten()
                    })
                });
            if let Some(id) = account {
                *account_sizes.entry(id).or_default() += entry.metadata()?.len();
            }
        }
    }
    let mut result = HashMap::new();
    for account in &store.accounts {
        let Some(&size) = account_sizes.get(&account.id) else {
            continue;
        };
        let emails: Vec<_> = store
            .emails
            .iter()
            .filter(|email| email.account_id == account.id && !email.storage_evicted)
            .collect();
        for (index, email) in emails.iter().enumerate() {
            result.insert(
                email.id,
                size / emails.len() as u64 + u64::from(index == 0) * (size % emails.len() as u64),
            );
        }
    }
    Ok(result)
}
pub fn reconcile(root: &Path, store: &Store) -> std::io::Result<()> {
    reconcile_with_reservation(root, store, &mut |root, bytes| {
        super::storage_fs::reserve(root, bytes).map_err(|e| std::io::Error::other(e.message))
    })
}
fn reconcile_with_reservation(
    root: &Path,
    store: &Store,
    reserve: &mut dyn FnMut(&Path, u64) -> std::io::Result<super::storage_fs::DiskReservation>,
) -> std::io::Result<()> {
    let known: HashMap<_, _> = store.emails.iter().map(|email| (email.id, email)).collect();
    let dir = root.join("search-lexical");
    if !dir.exists() {
        return Ok(());
    }
    for file in std::fs::read_dir(dir)? {
        let file = file?;
        if !file.file_type()?.is_file() {
            continue;
        }
        let name = file.file_name();
        let name = name.to_string_lossy();
        if name
            .strip_suffix(".new")
            .or_else(|| name.strip_suffix(".new-journal"))
            .is_some_and(|id| id.parse::<u32>().is_ok())
        {
            std::fs::remove_file(file.path())?;
            continue;
        }
        let Some(account) = file
            .path()
            .file_stem()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if file.path().extension().and_then(|s| s.to_str()) != Some("sqlite") {
            continue;
        }
        if store.account(account).is_none() {
            std::fs::remove_file(file.path())?;
            continue;
        }
        let mut db = Connection::open(file.path()).map_err(std::io::Error::other)?;
        configure(&db).map_err(|e| std::io::Error::other(e.message))?;
        let had_maintenance: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='maintenance')",
                [],
                |row| row.get(0),
            )
            .map_err(std::io::Error::other)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS maintenance(key TEXT PRIMARY KEY,value INTEGER NOT NULL);",
        )
        .map_err(std::io::Error::other)?;
        let ids: Vec<i64> = db
            .prepare("SELECT id FROM documents")
            .and_then(|mut statement| statement.query_map([], |row| row.get(0))?.collect())
            .map_err(std::io::Error::other)?;
        let tx = db.transaction().map_err(std::io::Error::other)?;
        let mut removed = false;
        for id in ids {
            if !known
                .get(&(id as u64))
                .is_some_and(|mail| mail.account_id == account && !mail.storage_evicted)
            {
                removed = true;
                tx.execute("DELETE FROM terms WHERE rowid=?1", [id])
                    .map_err(std::io::Error::other)?;
                tx.execute("DELETE FROM documents WHERE id=?1", [id])
                    .map_err(std::io::Error::other)?;
            }
        }
        if removed || !had_maintenance {
            tx.execute(
                "INSERT OR REPLACE INTO maintenance(key,value) VALUES('compact',1)",
                [],
            )
            .map_err(std::io::Error::other)?;
        }
        tx.commit().map_err(std::io::Error::other)?;
        let count: i64 = db
            .query_row("SELECT count(*) FROM documents", [], |row| row.get(0))
            .map_err(std::io::Error::other)?;
        if count == 0 {
            drop(db);
            std::fs::remove_file(file.path())?;
        } else if db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM maintenance WHERE key='compact' AND value=1)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(std::io::Error::other)?
        {
            let _space = reserve(root, std::fs::metadata(file.path())?.len())?;
            db.execute_batch("INSERT INTO terms(terms) VALUES('optimize');")
                .map_err(std::io::Error::other)?;
            {
                let mut statement = db
                    .prepare("PRAGMA incremental_vacuum")
                    .map_err(std::io::Error::other)?;
                let mut rows = statement.query([]).map_err(std::io::Error::other)?;
                while rows.next().map_err(std::io::Error::other)?.is_some() {}
            }
            db.execute("DELETE FROM maintenance WHERE key='compact'", [])
                .map_err(std::io::Error::other)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{indexing::MessageIndexState, model::Account};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "qamille-bm25-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn email(id: u64, subject: &str, body: &str) -> Email {
        Email {
            id,
            account_id: 1,
            subject: subject.into(),
            body: body.into(),
            from_addr: "alice@example.com".into(),
            folder: "Inbox".into(),
            remote_only: true,
            remote_content_hash: format!("hash-{id}"),
            search_index: MessageIndexState {
                version: 3,
                content_hash: format!("hash-{id}"),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn store(emails: Vec<Email>) -> Store {
        Store {
            accounts: vec![Account {
                id: 1,
                email: "owner@example.com".into(),
                ..Default::default()
            }],
            emails: emails
                .into_iter()
                .map(|mut e| {
                    e.body.clear();
                    e
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn bm25_ranks_subjects_and_finds_online_only_bodies_with_filters() {
        let root = Fixture::new();
        let a = email(1, "mercury invoice", "alpha beta payment");
        let b = email(2, "ordinary invoice", "mercury alpha middle beta");
        write(&root.0, &a, None).unwrap();
        write(&root.0, &b, None).unwrap();
        let store = store(vec![a, b]);
        let reader = Reader::open(&root.0, 1).unwrap().unwrap();
        let query = ParsedQuery::parse(&store, "mercury from:alice@example.com", 0).unwrap();
        let ranked = reader.rank(&store, &query, 10).unwrap();
        assert_eq!(ranked[0].0, 1);
        assert_eq!(ranked.len(), 2);
        let phrase = ParsedQuery::parse(&store, "\"alpha beta\"", 0).unwrap();
        assert_eq!(
            reader
                .rank(&store, &phrase, 10)
                .unwrap()
                .iter()
                .map(|h| h.0)
                .collect::<Vec<_>>(),
            [1]
        );
        let wrong = ParsedQuery::parse(&store, "mercury from:bob@example.com", 0).unwrap();
        assert!(reader.rank(&store, &wrong, 10).unwrap().is_empty());
        let negative = ParsedQuery::parse(&store, "mercury -payment", 0).unwrap();
        assert_eq!(reader.rank(&store, &negative, 10).unwrap()[0].0, 2);
        let body = ParsedQuery::parse(&store, "body:payment", 0).unwrap();
        assert!(reader.matches(&store.emails[0], &body, false).unwrap());
    }
    #[test]
    fn replacement_and_hash_checks_never_return_obsolete_terms_or_other_accounts() {
        let root = Fixture::new();
        let mut e = email(1, "oldterm", "old body");
        write(&root.0, &e, None).unwrap();
        e.subject = "newterm".into();
        e.remote_content_hash = "new".into();
        e.search_index.content_hash = "new".into();
        write(&root.0, &e, None).unwrap();
        let store = store(vec![e]);
        let reader = Reader::open(&root.0, 1).unwrap().unwrap();
        assert!(
            reader
                .rank(
                    &store,
                    &ParsedQuery::parse(&store, "oldterm", 0).unwrap(),
                    10
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reader
                .rank(
                    &store,
                    &ParsedQuery::parse(&store, "newterm", 0).unwrap(),
                    10
                )
                .unwrap()
                .len(),
            1
        );
        let mut changed = store.emails[0].clone();
        changed.remote_content_hash = "changed".into();
        assert!(!reader.current(&changed).unwrap());
        changed = store.emails[0].clone();
        changed.account_id = 2;
        assert!(!reader.current(&changed).unwrap());
    }
    #[test]
    fn contentless_index_accounts_for_disk_and_removes_evicted_messages() {
        let root = Fixture::new();
        let marker = "Private source paragraph with original spacing and sentence ordering that must never be stored as a duplicate mail body.";
        let e = email(1, "invoice", marker);
        write(&root.0, &e, None).unwrap();
        let bytes = std::fs::read(path(&root.0, 1)).unwrap();
        assert!(
            !bytes
                .windows(marker.len())
                .any(|part| part == marker.as_bytes())
        );
        let second = email(2, "retainedterm", "retained body");
        write(&root.0, &second, None).unwrap();
        let mut store = store(vec![e, second]);
        assert_eq!(
            message_bytes(&root.0, &store)
                .unwrap()
                .values()
                .sum::<u64>(),
            std::fs::metadata(path(&root.0, 1)).unwrap().len()
        );
        store.emails[0].storage_evicted = true;
        reconcile(&root.0, &store).unwrap();
        let reader = Reader::open(&root.0, 1).unwrap().unwrap();
        assert_eq!(
            reader
                .rank(
                    &store,
                    &ParsedQuery::parse(&store, "retainedterm", 0).unwrap(),
                    10
                )
                .unwrap()
                .len(),
            1
        );
        drop(reader);
        store.emails[1].storage_evicted = true;
        reconcile(&root.0, &store).unwrap();
        assert!(!path(&root.0, 1).exists());
    }
    #[test]
    fn failed_admission_does_not_publish_an_empty_or_partial_database() {
        let root = Fixture::new();
        let e = email(1, "huge", &"expanding tokens index ".repeat(100_000));
        assert_eq!(
            write(&root.0, &e, Some(131072)).unwrap_err().code,
            "storage_full"
        );
        assert!(!path(&root.0, 1).exists());
        assert!(!path(&root.0, 1).with_extension("new").exists());
        write(&root.0, &email(1, "small", "body"), Some(131072)).unwrap();
    }
    #[test]
    fn compaction_is_retried_after_failure_and_drains_every_vacuum_row() {
        let root = Fixture::new();
        let mut first = email(1, "old", "");
        first.body = (0..30000).map(|n| format!("uniqueterm{n} ")).collect();
        let second = email(2, "retained", "body");
        write(&root.0, &first, None).unwrap();
        write(&root.0, &second, None).unwrap();
        let store = store(vec![second]);
        let before = std::fs::metadata(path(&root.0, 1)).unwrap().len();
        let failure = reconcile_with_reservation(&root.0, &store, &mut |_, _| {
            Err(std::io::Error::other("simulated disk pressure"))
        });
        assert!(failure.is_err());
        {
            let db = Connection::open(path(&root.0, 1)).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT value FROM maintenance WHERE key='compact'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        // No more documents are removed in this invocation: pending maintenance
        // must still run, and PRAGMA rows must all be consumed.
        reconcile(&root.0, &store).unwrap();
        let after = std::fs::metadata(path(&root.0, 1)).unwrap().len();
        let db = Connection::open(path(&root.0, 1)).unwrap();
        assert_eq!(
            db.query_row("PRAGMA freelist_count", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(after < before / 2, "before={before}, after={after}");
        assert!(
            !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM maintenance WHERE key='compact')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
        );
    }
    #[test]
    fn corrupt_optional_index_falls_back_but_genuine_open_errors_remain_visible() {
        let root = Fixture::new();
        std::fs::create_dir_all(root.0.join("search-lexical")).unwrap();
        std::fs::write(path(&root.0, 1), b"corrupt sqlite bytes").unwrap();
        assert!(Reader::open(&root.0, 1).unwrap().is_none());
        std::fs::remove_file(path(&root.0, 1)).unwrap();
        std::fs::create_dir(path(&root.0, 1)).unwrap();
        assert!(Reader::open(&root.0, 1).is_err());
    }
}
