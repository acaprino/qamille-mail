//! Task 4.3 (ADR-0007 decision 1): the one-time import from `store.json` into
//! the SQLite store, the engine marker both processes read, and the
//! cross-process change feed the desktop poller turns into local events
//! (EX-005, EX-006, Review Focus 5, decision D-crossproc).

use rusqlite::Connection;
use qamille::{
    application::ports::StoreRepository,
    domain::model::{Store, sample_store},
    infrastructure::{
        repository::JsonStoreRepository, sqlite_store::SqliteStoreRepository, store_migration,
    },
};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "qamille-migration-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        Self(root)
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
    fn json(&self) -> PathBuf {
        self.data().join("store.json")
    }
    fn db(&self) -> PathBuf {
        self.data().join("store.sqlite")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The EX-006 store shape: two accounts, 67 messages.
fn ex006_store() -> Store {
    let mut store = sample_store();
    let template = store.emails[0].clone();
    while store.emails.len() < 67 {
        let index = store.emails.len();
        let mut email = template.clone();
        email.id = store.new_id();
        email.message_id = format!("<padded-{index}@example.com>");
        email.subject = format!("Padded {index}");
        store.emails.push(email);
    }
    store
}

fn write_json(path: &Path, store: &Store) {
    JsonStoreRepository::new(path)
        .transaction(&mut |current| {
            *current = store.clone();
            Ok(())
        })
        .unwrap();
}

/// `Store` has no `PartialEq`: the persistence contract is the serialized
/// document (the plan's fallback for the EX-006 comparison).
fn serde_equal(left: &Store, right: &Store) {
    assert_eq!(
        serde_json::to_value(left).unwrap(),
        serde_json::to_value(right).unwrap()
    );
}

#[test]
fn ex006_json_store_migrates_with_its_counts_and_stays_readable() {
    let scratch = Scratch::new("ex006");
    write_json(&scratch.json(), &ex006_store());
    let bytes_before = std::fs::read(scratch.json()).unwrap();

    let report = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(report.migrated, "the first call imports");
    assert!(!report.damaged);
    assert_eq!(report.emails, 67);
    assert_eq!(report.accounts, 2);
    assert!(!report.activated, "the engine stays dark until activation");

    // The JSON original is preserved untouched, and both stores now answer
    // with the same content.
    assert_eq!(std::fs::read(scratch.json()).unwrap(), bytes_before);
    let json_read = JsonStoreRepository::new(scratch.json()).read().unwrap();
    let sqlite_read = SqliteStoreRepository::open(scratch.db())
        .unwrap()
        .read()
        .unwrap();
    serde_equal(&json_read, &sqlite_read);
    assert_eq!(sqlite_read.emails.len(), 67);

    // The import records its provenance.
    let source: String = Connection::open(scratch.db())
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE name = 'migrated_from'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(source, "store.json");

    // The second call migrates nothing and changes nothing.
    let again = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(!again.migrated);
    assert!(!again.damaged);
    assert_eq!(again.emails, 67);
    assert_eq!(again.accounts, 2);
    assert_eq!(std::fs::read(scratch.json()).unwrap(), bytes_before);
}

#[test]
fn an_unreadable_json_is_preserved_and_an_empty_database_created() {
    let scratch = Scratch::new("damaged");
    std::fs::write(scratch.json(), b"{ this is not json").unwrap();

    let report = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(report.migrated);
    assert!(report.damaged, "Review Focus 5: the damage is reported");
    assert_eq!(report.emails, 0);

    let damaged = scratch.data().join("store.damaged.json");
    assert_eq!(std::fs::read(&damaged).unwrap(), b"{ this is not json");
    assert!(
        !scratch.json().exists(),
        "the unreadable file was renamed aside"
    );
    let store = SqliteStoreRepository::open(scratch.db())
        .unwrap()
        .read()
        .unwrap();
    assert!(store.emails.is_empty());

    let again = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(!again.migrated && !again.damaged);
}

#[test]
fn a_fresh_directory_migrates_nothing_and_creates_nothing() {
    let scratch = Scratch::new("fresh");
    let report = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(!report.migrated);
    assert!(!report.damaged);
    assert!(
        !scratch.db().exists(),
        "the engine choice at open time decides the first store"
    );
}

#[test]
fn activation_proves_the_store_and_writes_the_marker_both_processes_read() {
    let scratch = Scratch::new("activate");
    write_json(&scratch.json(), &ex006_store());
    assert!(!store_migration::sqlite_active(&scratch.data()));

    let report = store_migration::activate_sqlite(&scratch.data()).unwrap();
    assert!(report.activated);
    assert!(report.migrated, "activation imports whatever is still pending");
    assert!(store_migration::sqlite_active(&scratch.data()));

    // Every later startup call sees the active engine.
    let again = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(again.activated);
    assert!(!again.migrated);
    assert_eq!(again.emails, 67);
}

#[test]
fn the_feed_announces_committed_changes_to_the_other_process() {
    let scratch = Scratch::new("feed");
    let writer = SqliteStoreRepository::open(scratch.db()).unwrap();
    let seeded = sample_store();
    let (email_id, draft_id) = (seeded.emails[0].id, seeded.drafts[0].id);
    let seed = seeded.clone();
    writer
        .transaction(&mut |current| {
            *current = seed.clone();
            Ok(())
        })
        .unwrap();
    // The poller starts from the tail: the seeding's own rows are history.
    let reader = SqliteStoreRepository::open(scratch.db()).unwrap();
    let head = reader.change_feed_head().unwrap();

    writer
        .transaction(&mut |store| {
            let email = store.email_mut(email_id).unwrap();
            email.done = true;
            email.labels.push("Feed".into());
            let draft = store
                .drafts
                .iter_mut()
                .find(|d| d.id == draft_id)
                .unwrap();
            draft.subject = "Changed".into();
            Ok(())
        })
        .unwrap();

    // What the desktop poller sees: the rows of a foreign pid.
    let foreign = reader.poll_change_feed_from(head, 999_999).unwrap();
    let email_row = foreign
        .iter()
        .find(|row| row.kind == "email-changed")
        .expect("the email change is announced");
    assert_eq!(email_row.account_id, Some(1));
    assert_eq!(email_row.ids, vec![email_id]);
    let draft_row = foreign
        .iter()
        .find(|row| row.kind == "drafts-changed")
        .expect("the draft change is announced");
    assert_eq!(draft_row.ids, vec![draft_id]);
    assert!(foreign.iter().all(|row| row.seq > head));
    assert!(reader.change_feed_head().unwrap() >= foreign.last().unwrap().seq);

    // The events the poller synthesizes from the rows.
    assert_eq!(email_row.events().len(), 2, "the message and its counts");
    assert_eq!(draft_row.events().len(), 1);

    // A process's own rows are skipped: its writers already announced them.
    assert!(
        reader.poll_change_feed(head).unwrap().is_empty(),
        "same pid: the feed answers nothing"
    );
}

#[test]
fn a_v2_database_gains_the_feed_table_and_records_the_current_version() {
    let scratch = Scratch::new("v2");
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let seeded = sample_store();
    let email_id = seeded.emails[0].id;
    let seed = seeded.clone();
    repo.transaction(&mut |current| {
        *current = seed.clone();
        Ok(())
    })
    .unwrap();
    drop(repo);
    // A database as version 2 left it: recorded 2, no feed table.
    let raw = Connection::open(scratch.db()).unwrap();
    raw.execute_batch(
        "UPDATE meta SET value = '2' WHERE name = 'schema_version';
         DROP TABLE change_feed;",
    )
    .unwrap();
    drop(raw);

    let again = SqliteStoreRepository::open(scratch.db()).unwrap();
    let version: String = Connection::open(scratch.db())
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "5", "the recorded version moves to the current one");
    // The data survived and the feed works on the upgraded database.
    assert!(again.read().unwrap().emails.iter().any(|e| e.id == email_id));
    let head = again.change_feed_head().unwrap();
    again
        .transaction(&mut |store| {
            store.email_mut(email_id).unwrap().done = true;
            Ok(())
        })
        .unwrap();
    assert!(again.change_feed_head().unwrap() > head);
}

/// The activation gate of the 4.3 brief (controller addition 2): the real
/// `store.json` of the release workspace migrates losslessly into the SQLite
/// store and the JSON original survives byte-identical. Skipped on a machine
/// without the release data; no activation without this test green.
#[test]
fn real_store_json_migrates_losslessly() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../release/Qamille/data/store.json");
    if !source.exists() {
        return;
    }
    let scratch = Scratch::new("real");
    std::fs::copy(&source, scratch.json()).unwrap();
    let bytes_before = std::fs::read(scratch.json()).unwrap();

    let report = store_migration::migrate_if_needed(&scratch.data()).unwrap();
    assert!(report.migrated && !report.damaged, "{report:?}");
    assert!(report.emails > 0, "the real store has mail: {report:?}");
    assert!(report.accounts > 0);

    let json_read = JsonStoreRepository::new(scratch.json()).read().unwrap();
    let sqlite_read = SqliteStoreRepository::open(scratch.db())
        .unwrap()
        .read()
        .unwrap();
    serde_equal(&json_read, &sqlite_read);
    assert_eq!(report.emails, json_read.emails.len());
    assert_eq!(report.accounts, json_read.accounts.len());

    // The gate names drafts, undo rows and labels explicitly.
    assert_eq!(sqlite_read.drafts.len(), json_read.drafts.len());
    assert_eq!(sqlite_read.undo_actions.len(), json_read.undo_actions.len());
    let labels = |store: &Store| store.emails.iter().map(|email| email.labels.len()).sum::<usize>();
    assert_eq!(labels(&sqlite_read), labels(&json_read));
    assert_eq!(
        std::fs::read(scratch.json()).unwrap(),
        bytes_before,
        "the original is untouched"
    );
}
