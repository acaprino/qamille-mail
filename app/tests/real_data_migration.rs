//! The 3 -> 4 migration (ADR-0010, `in_reply_to`) and the conversation list, on a COPY of a real
//! mailbox. Opt-in: the test is `#[ignore]` and reads the copy's path from the environment.
//!
//! ```text
//! QAMILLE_REAL_DB=<copy of store.sqlite> cargo test --manifest-path app/Cargo.toml -j 2 \
//!   --target-dir <dir> --test real_data_migration -- --ignored --nocapture
//! ```
//!
//! The copy must be a schema 3 database (take a fresh copy for every run: the migration changes it)
//! and must live outside any repository. The test never opens the original, never starts the app and
//! never reaches a mail server: the mail gateway is a stub that is not called by listing.
//!
//! PRIVACY: this file prints and asserts on counts, versions and timings only. No subject, name,
//! address or body is ever read into a message, and no message id is printed.
use rusqlite::{Connection, OpenFlags, types::Value};
use qamille::{
    application::{
        AppResult, ServiceContext,
        mail::{MailPage, MailQuery, MailService},
        ports::*,
    },
    infrastructure::{
        repository::JsonStoreRepository, sqlite_store::SqliteStoreRepository, store_migration,
    },
    model::*,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

// ---- Test doubles: listing reads the store and nothing else ----

struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
    }
}
struct NoSecrets;
impl SecretStore for NoSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct NoAttachments;
impl AttachmentGateway for NoAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
/// Listing must never reach a provider: every call here would be a bug, so each one fails loudly.
struct NoNetwork;
impl MailGateway for NoNetwork {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        panic!("listing must not sync")
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        panic!("listing must not act on a server")
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        panic!("listing must not send")
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        panic!("listing must not log in")
    }
}

// ---- Plain rusqlite reads: the oracle does not go through the code under test ----

fn plain(path: &PathBuf) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("the copy opens read-only")
}
fn version(connection: &Connection) -> String {
    connection
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .expect("schema_version is recorded")
}
fn count(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).expect("a count")
}
fn columns(connection: &Connection) -> Vec<String> {
    let mut statement = connection.prepare("SELECT name FROM pragma_table_info('emails') ORDER BY cid").unwrap();
    statement.query_map([], |row| row.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}
/// id -> fingerprint of every column except `in_reply_to`: proves the migration touched nothing else.
fn fingerprints(connection: &Connection, skip: &[&str]) -> BTreeMap<i64, u64> {
    let names: Vec<String> = columns(connection).into_iter().filter(|c| !skip.contains(&c.as_str())).collect();
    let quoted: Vec<String> = names.iter().map(|c| format!("\"{c}\"")).collect();
    let mut statement = connection.prepare(&format!("SELECT {} FROM emails", quoted.join(", "))).unwrap();
    let id_at = names.iter().position(|c| c == "id").expect("emails.id");
    statement
        .query_map([], |row| {
            let mut hasher = DefaultHasher::new();
            let mut id = 0;
            for (at, _) in names.iter().enumerate() {
                let value: Value = row.get(at)?;
                if at == id_at {
                    if let Value::Integer(n) = value {
                        id = n;
                    }
                }
                format!("{value:?}").hash(&mut hasher);
            }
            Ok((id, hasher.finish()))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
struct Row {
    folder: String,
    local_sent: bool,
    message_id: String,
    thread_id: String,
    in_reply_to: String,
}
fn rows(connection: &Connection) -> BTreeMap<i64, Row> {
    let mut statement =
        connection.prepare("SELECT id, folder, local_sent, message_id, thread_id, in_reply_to FROM emails").unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                Row {
                    folder: row.get(1)?,
                    local_sent: row.get::<_, i64>(2)? != 0,
                    message_id: row.get(3)?,
                    thread_id: row.get(4)?,
                    in_reply_to: row.get(5)?,
                },
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
/// What ADR-0010 says the migration fills, derived here from the rule and not from the SQL:
/// received mail only, and only where the thread is a parent rather than the message itself.
fn expected_parent(row: &Row) -> String {
    let trim = |s: &str| s.trim_matches(|c| c == '<' || c == '>').to_string();
    if row.folder != "Sent" && !row.local_sent && !row.thread_id.is_empty() && trim(&row.thread_id) != trim(&row.message_id) {
        trim(&row.thread_id)
    } else {
        String::new()
    }
}

fn service(repository: Arc<dyn StoreRepository>) -> MailService {
    MailService::new(Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository,
        mail: Arc::new(NoNetwork),
        secrets: Arc::new(NoSecrets),
        clock: Arc::new(RealClock),
        attachments: Arc::new(NoAttachments),
    }))
}

/// How many listed replies name a message of their own row, and how many of those hang somewhere
/// else. A reply is named by `in_reply_to`, or by `thread_id` when none is stored (received mail
/// only); a name that matches no message of the row, a self reference and anything sent are skipped.
fn parent_check(store: &Store, page: &MailPage) -> (usize, usize) {
    let trim = |s: &str| s.trim_matches(|c| c == '<' || c == '>').to_string();
    let by_id: HashMap<u64, &Email> = store.emails.iter().map(|e| (e.id, e)).collect();
    let (mut checked, mut wrong) = (0, 0);
    for row in &page.items {
        let Some(main) = by_id.get(&row.id) else { continue };
        for reply in &row.replies {
            let Some(email) = by_id.get(&reply.id) else { continue };
            if email.folder == "Sent" || email.local_sent {
                continue;
            }
            let named = if email.in_reply_to.is_empty() { trim(&email.thread_id) } else { trim(&email.in_reply_to) };
            if named.is_empty() || named == trim(&email.message_id) {
                continue;
            }
            let expected = if trim(&main.message_id) == named {
                Some(None)
            } else {
                // The tree keeps one copy per Message-ID, the lowest id.
                row.replies
                    .iter()
                    .filter(|r| by_id.get(&r.id).is_some_and(|e| trim(&e.message_id) == named))
                    .map(|r| r.id)
                    .min()
                    .map(Some)
            };
            if let Some(expected) = expected {
                checked += 1;
                if reply.parent_id != expected {
                    wrong += 1;
                }
            }
        }
    }
    (checked, wrong)
}

fn report(label: &str, page: &MailPage, millis: u128) {
    let with_replies = page.items.iter().filter(|r| !r.replies.is_empty()).count();
    let replies: usize = page.items.iter().map(|r| r.replies.len()).sum();
    let longest = page.items.iter().map(|r| r.replies.len() + r.replies_more).max().unwrap_or(0);
    println!(
        "{label}: {millis} ms, {} rows (total {}), {with_replies} with replies, {replies} replies listed, longest conversation {} replies",
        page.items.len(),
        page.total,
        longest
    );
}

#[test]
#[ignore = "needs QAMILLE_REAL_DB: the path of a COPY of a schema 3 store.sqlite, outside any repository"]
fn a_real_schema_3_mailbox_migrates_to_4_and_lists_conversations() {
    let path: PathBuf = std::env::var_os("QAMILLE_REAL_DB").expect("set QAMILLE_REAL_DB to a copy of store.sqlite").into();
    assert!(path.is_file(), "QAMILLE_REAL_DB must name an existing file");

    // ---- before: plain rusqlite, nothing of Qamille ----
    let before = plain(&path);
    assert_eq!(version(&before), "3", "the copy must be a schema 3 database: take a fresh copy");
    assert!(!columns(&before).iter().any(|c| c == "in_reply_to"), "schema 3 has no in_reply_to column yet");
    let count_before = count(&before, "SELECT count(*) FROM emails");
    let sent_before = count(&before, "SELECT count(*) FROM emails WHERE folder = 'Sent' OR local_sent <> 0");
    let others_before = fingerprints(&before, &[]);
    let attachments_before = count(&before, "SELECT count(*) FROM attachments");
    let accounts_before = count(&before, "SELECT count(*) FROM accounts");
    drop(before);
    println!("before: schema 3, {count_before} messages ({sent_before} sent), {attachments_before} attachments, {accounts_before} accounts");
    assert!(count_before > 0, "an empty copy proves nothing");

    // ---- migrate: the code under test ----
    let started = Instant::now();
    let repository = Arc::new(SqliteStoreRepository::open(path.clone()).expect("the schema 3 database opens and migrates"));
    println!("open + migrate: {} ms", started.elapsed().as_millis());

    let after = plain(&path);
    assert_eq!(version(&after), "6", "the version is recorded as 6");
    let count_after = count(&after, "SELECT count(*) FROM emails");
    assert_eq!(count_after, count_before, "no message lost or added");
    assert_eq!(count(&after, "SELECT count(*) FROM attachments"), attachments_before, "no attachment lost");
    assert_eq!(count(&after, "SELECT count(*) FROM accounts"), accounts_before, "no account lost");
    // Nothing but the new column changed: every other column of every row is byte for byte what it was.
    let others_after = fingerprints(&after, &["in_reply_to", "reply_to_addresses"]);
    assert_eq!(others_after.len(), others_before.len());
    let changed = others_before.iter().filter(|(id, print)| others_after.get(id) != Some(print)).count();
    assert_eq!(changed, 0, "{changed} messages changed in a column the migration must not touch");

    let table = rows(&after);
    let in_sent = |r: &Row| r.folder == "Sent" || r.local_sent;
    let sent_filled = table.values().filter(|r| in_sent(r) && !r.in_reply_to.is_empty()).count();
    assert_eq!(sent_filled, 0, "in_reply_to is empty for everything in Sent: the migration never deduces it");
    let filled_on_received = table.values().filter(|r| !in_sent(r) && !r.in_reply_to.is_empty()).count();
    let wrong = table.values().filter(|r| r.in_reply_to != expected_parent(r)).count();
    assert_eq!(wrong, 0, "{wrong} messages carry a parent the rule does not give them");
    let roots = table.values().filter(|r| !in_sent(r) && r.in_reply_to.is_empty()).count();
    println!(
        "after: schema 4, {count_after} messages; in_reply_to filled on {filled_on_received} received, empty on {roots} received roots/unthreaded and on all {} sent",
        table.values().filter(|r| in_sent(r)).count()
    );
    assert_eq!(table.values().filter(|r| in_sent(r)).count() as i64, sent_before);

    // The same data as the application reads it (the model, not the table).
    let store = repository.read().expect("the repository reads the migrated store");
    assert_eq!(store.emails.len() as i64, count_after, "the repository sees every message");
    let model_wrong = store.emails.iter().filter(|e| table.get(&(e.id as i64)).map(|r| r.in_reply_to.as_str()) != Some(e.in_reply_to.as_str())).count();
    assert_eq!(model_wrong, 0, "the model carries the stored parent of every message");
    let by_id: HashMap<u64, String> = store.emails.iter().map(|e| (e.id, e.folder.clone())).collect();

    // ---- reopen: a second open changes nothing ----
    let snapshot = |connection: &Connection| (count(connection, "SELECT count(*) FROM emails"), fingerprints(connection, &[]), version(connection));
    let first = snapshot(&after);
    drop(after);
    drop(store);
    let list_repository = repository.clone();
    drop(repository);
    // Another open of the same file while the first handle lives, then a fresh connection to compare.
    let reopened = SqliteStoreRepository::open(path.clone()).expect("a migrated database reopens");
    let again = plain(&path);
    assert_eq!(snapshot(&again), first, "reopening changes nothing: rows, parents and version are the same");
    assert_eq!(count(&again, "SELECT count(*) FROM pragma_table_info('emails') WHERE name = 'in_reply_to'"), 1, "the column is added once");
    drop(reopened);
    drop(again);
    println!("reopen: identical rows, parents and version");

    // ---- Conversations mode on the real data ----
    let service = service(list_repository);
    let mut views: Vec<(&str, MailQuery)> = vec![
        ("Inbox, Conversations, first page", MailQuery { folder: "Inbox".into(), threads: true, limit: 60, ..Default::default() }),
        ("Inbox, Conversations, limit 100", MailQuery { folder: "Inbox".into(), threads: true, limit: 100, ..Default::default() }),
        ("Inbox, Email mode, limit 100", MailQuery { folder: "Inbox".into(), threads: false, limit: 100, ..Default::default() }),
        ("Sent, Conversations", MailQuery { folder: "Sent".into(), threads: true, limit: 100, ..Default::default() }),
        ("Archive, Conversations", MailQuery { folder: "Archive".into(), threads: true, limit: 100, ..Default::default() }),
    ];
    for account in 1..=accounts_before as u32 {
        views.push(("Inbox of one account, Conversations", MailQuery { folder: "Inbox".into(), threads: true, limit: 60, account_id: Some(account), ..Default::default() }));
    }
    let mut slowest = 0;
    for (label, query) in views {
        let threads = query.threads;
        let folder = query.folder.clone();
        let started = Instant::now();
        let page = service.list(query).expect("listing succeeds on the migrated real data");
        let millis = started.elapsed().as_millis();
        slowest = slowest.max(millis);
        report(label, &page, millis);
        if threads {
            let mut seen = HashSet::new();
            for row in &page.items {
                assert_eq!(by_id.get(&row.id).map(String::as_str), Some(folder.as_str()), "a main row belongs to the folder shown");
                assert!(seen.insert(row.id), "a conversation is listed once");
                let mut among = HashSet::new();
                for reply in &row.replies {
                    assert!(among.insert(reply.id), "a message appears once in its conversation");
                    assert_ne!(reply.id, row.id, "the main row is not repeated among its replies");
                    let kept = by_id.get(&reply.id).map(String::as_str);
                    assert!(kept.is_some_and(|f| f != "Trash" && f != "Spam") || folder == "Trash" || folder == "Spam", "INV-004: no Trash or Spam among the replies");
                }
            }
        }
    }
    println!("slowest list call: {slowest} ms");
}

/// ADR-0010 / spec §5.2: for mail already stored, "the parent of RECEIVED mail is derived from `thread_id`
/// when it differs from `message_id`". The migration does it for a schema 3 SQLite database (test above).
/// The user's real workspace, though, runs on `store.json` (no `store-engine.txt`), and the import into a
/// fresh SQLite store writes the model as it is. This test counts what each of those two paths gives.
///
/// D-DATA (found on a real copy, counts only): neither the JSON engine nor the import into SQLite
/// stores `in_reply_to` for the received mail written before this change, so the list used to hang
/// every such reply under the oldest message. The list now reads the parent from `thread_id` when
/// none is stored (`domain::conversation::parent_header`), so what is asserted is what the list
/// shows: every reply that names a message of its own row hangs under it, on both engines.
///
/// QAMILLE_REAL_JSON=<copy of store.json> (outside any repository; its folder gets an `import-<pid>`
/// subfolder that the test creates and removes).
#[test]
#[ignore = "needs QAMILLE_REAL_JSON: the path of a COPY of store.json, outside any repository"]
fn a_real_json_store_gets_parents_for_received_mail() {
    let source: PathBuf = std::env::var_os("QAMILLE_REAL_JSON").expect("set QAMILLE_REAL_JSON to a copy of store.json").into();
    assert!(source.is_file(), "QAMILLE_REAL_JSON must name an existing file");
    let dir = source.parent().expect("a folder").join(format!("import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(&source, dir.join("store.json")).unwrap();
    let derivable = |e: &Email| {
        let trim = |s: &str| s.trim_matches(|c| c == '<' || c == '>').to_string();
        e.folder != "Sent" && !e.local_sent && !e.thread_id.is_empty() && trim(&e.thread_id) != trim(&e.message_id)
    };

    // ---- 1. the JSON engine, as the app runs it today ----
    let json = Arc::new(JsonStoreRepository::new(dir.join("store.json")));
    let store = json.read().expect("the JSON store reads");
    let total = store.emails.len();
    let can_derive = store.emails.iter().filter(|e| derivable(e)).count();
    let json_filled = store.emails.iter().filter(|e| derivable(e) && !e.in_reply_to.is_empty()).count();
    let any_filled = store.emails.iter().filter(|e| !e.in_reply_to.is_empty()).count();
    println!("JSON engine: {total} messages, {can_derive} received with a parent in thread_id, {json_filled} of them with in_reply_to ({any_filled} messages have any)");
    let started = Instant::now();
    let page = service(json.clone())
        .list(MailQuery { folder: "Inbox".into(), threads: true, limit: 100, ..Default::default() })
        .expect("listing works on the JSON engine");
    report("JSON engine, Inbox, Conversations", &page, started.elapsed().as_millis());
    let (json_checked, json_wrong) = parent_check(&store, &page);
    println!("JSON engine: {json_checked} replies name a message of their own row, {json_wrong} of them hang elsewhere");
    drop(store);

    // ---- 2. the import into a fresh SQLite store (the way to the SQLite engine) ----
    let imported = store_migration::migrate_if_needed(&dir).expect("the JSON store imports");
    assert!(imported.migrated && imported.emails == total, "the import carried every message");
    let table = rows(&plain(&dir.join("store.sqlite")));
    assert_eq!(table.len(), total, "no message lost by the import");
    let sqlite_expected = table.values().filter(|r| !expected_parent(r).is_empty()).count();
    let sqlite_filled = table.values().filter(|r| !r.in_reply_to.is_empty() && expected_parent(r) == r.in_reply_to).count();
    println!("SQLite after the import: {sqlite_expected} received messages have a parent in thread_id, {sqlite_filled} of them carry it in in_reply_to");
    drop(table);
    let sqlite = Arc::new(SqliteStoreRepository::open(dir.join("store.sqlite")).expect("the imported store opens"));
    let started = Instant::now();
    let sqlite_page = service(sqlite.clone())
        .list(MailQuery { folder: "Inbox".into(), threads: true, limit: 100, ..Default::default() })
        .expect("listing works on the imported SQLite store");
    report("SQLite after the import, Inbox, Conversations", &sqlite_page, started.elapsed().as_millis());
    let (sqlite_checked, sqlite_wrong) = parent_check(&sqlite.read().expect("the imported store reads"), &sqlite_page);
    println!("SQLite after the import: {sqlite_checked} replies name a message of their own row, {sqlite_wrong} of them hang elsewhere");
    drop(sqlite);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(json_checked > 0, "the copy has conversations whose replies name a message of their row: an empty check proves nothing");
    assert_eq!(json_wrong, 0, "JSON engine: {json_wrong} of {json_checked} replies hang under a message they do not answer");
    assert_eq!(sqlite_wrong, 0, "import into SQLite: {sqlite_wrong} of {sqlite_checked} replies hang under a message they do not answer");
}
