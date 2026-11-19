//! The activity log's own database (centro attività): one file, `activity.sqlite`, in the data
//! directory, shared by the desktop and the MCP process and independent of the engine the mail store
//! runs on. The app ships on the JSON engine, so these tests are the contract that matters.

use rusqlite::Connection;
use qamille::{
    application::ports::{ActivityStore, StoreRepository},
    domain::activity::*,
    infrastructure::{
        activity_db::{ACTIVITY_FILE, ActivityDb},
        repository::JsonStoreRepository,
        sqlite_store::SqliteStoreRepository,
    },
};
use std::path::PathBuf;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qamille-activity-db-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn file(&self) -> PathBuf {
        self.0.join(ACTIVITY_FILE)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
fn action(code: &str, undo_id: Option<u64>) -> NewActivity {
    NewActivity {
        kind: ActivityKind::Action,
        code: code.into(),
        source: ActivitySource::User,
        undo_id,
        ..Default::default()
    }
}
fn error(code: &str) -> NewActivity {
    NewActivity { kind: ActivityKind::Error, code: code.into(), ..Default::default() }
}
fn seqs(rows: &[ActivityEntry]) -> Vec<i64> {
    rows.iter().map(|row| row.seq).collect()
}

#[test]
fn rows_round_trip_with_every_field_and_page_newest_first() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    let entry = NewActivity {
        kind: ActivityKind::Action,
        code: code::ARCHIVE.into(),
        source: ActivitySource::Assistant,
        account_id: Some(2),
        subject: Some("Cyber Resilience Act".into()),
        detail: Some("Server unreachable".into()),
        count: Some(80),
        ref_kind: Some("email".into()),
        ref_id: Some(41),
        undo_id: Some(7),
        outcome: ActivityOutcome::Failed,
    };
    let first = log.append(&entry, 1_000).unwrap();
    let second = log.append(&action(code::SNOOZE, None), 2_000).unwrap();
    assert!(second > first);

    let page = log.page(None, 10, ActivityFilter::All).unwrap();
    assert_eq!(seqs(&page), vec![second, first], "newest first");
    let row = &page[1];
    assert_eq!((row.seq, row.at_ms, row.code.as_str()), (first, 1_000, "archive"));
    assert_eq!((row.source, row.kind, row.outcome), (ActivitySource::Assistant, ActivityKind::Action, ActivityOutcome::Failed));
    assert_eq!((row.account_id, row.count, row.ref_id, row.undo_id), (Some(2), Some(80), Some(41), Some(7)));
    assert_eq!(row.subject.as_deref(), Some("Cyber Resilience Act"));
    assert_eq!(row.detail.as_deref(), Some("Server unreachable"));
    assert_eq!(row.ref_kind.as_deref(), Some("email"));

    assert_eq!(seqs(&log.page(None, 1, ActivityFilter::All).unwrap()), vec![second]);
    assert_eq!(seqs(&log.page(Some(second), 10, ActivityFilter::All).unwrap()), vec![first], "before_seq is exclusive");
    // The Errors filter lists a failed action as well as an error row.
    assert_eq!(seqs(&log.page(None, 10, ActivityFilter::Error).unwrap()), vec![first]);
    assert_eq!(seqs(&log.page(None, 10, ActivityFilter::Action).unwrap()), vec![second, first]);
    assert!(log.page(None, 10, ActivityFilter::Event).unwrap().is_empty());
}

#[test]
fn the_file_is_not_created_until_something_is_recorded() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    assert!(!scratch.file().exists(), "constructing the log touches no disk");
    log.append(&action(code::ARCHIVE, None), 1).unwrap();
    assert!(scratch.file().exists());
}

#[test]
fn an_outcome_update_survives_a_reopen_and_keeps_the_detail_when_none_is_given() {
    let scratch = Scratch::new();
    ActivityDb::in_dir(&scratch.0).append(&action(code::SEND, Some(5)), 10).unwrap();
    let reopened = ActivityDb::in_dir(&scratch.0);
    assert_eq!(reopened.set_outcome(5, ActivityOutcome::Failed, Some("Server down")).unwrap(), Some(1));
    assert_eq!(reopened.set_outcome(404, ActivityOutcome::Undone, None).unwrap(), None, "unknown undo id");
    assert_eq!(reopened.set_outcome(5, ActivityOutcome::Undone, None).unwrap(), Some(1));
    let page = reopened.page(None, 10, ActivityFilter::All).unwrap();
    assert_eq!(page[0].outcome, ActivityOutcome::Undone);
    assert_eq!(page[0].detail.as_deref(), Some("Server down"), "no detail given keeps the old one");
}

#[test]
fn a_failure_after_the_panel_was_seen_is_unseen_again() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    let long_ago = now_ms() - 3_600_000;
    log.append(&error(code::SYNC_FAILED), long_ago).unwrap();
    log.append(&action(code::SEND, Some(3)), long_ago + 10).unwrap();
    log.append(&action(code::ARCHIVE, Some(4)), long_ago + 20).unwrap();
    assert_eq!(log.seen_ms().unwrap(), 0);
    assert_eq!(log.unseen_errors().unwrap(), 1, "the error row; nothing failed yet");

    // The user opens the panel: everything so far is seen, and it stays seen across a reopen.
    log.set_seen_ms(now_ms()).unwrap();
    assert_eq!(log.unseen_errors().unwrap(), 0);
    assert_eq!(ActivityDb::in_dir(&scratch.0).unseen_errors().unwrap(), 0);

    // The send queued an hour ago fails now: the dot lights, though the row is old.
    log.set_outcome(3, ActivityOutcome::Failed, Some("Server unreachable")).unwrap();
    assert_eq!(log.unseen_errors().unwrap(), 1);
    // An undo is not news.
    log.set_outcome(4, ActivityOutcome::Undone, None).unwrap();
    assert_eq!(log.unseen_errors().unwrap(), 1);
}

#[test]
fn last_matches_the_family_and_the_null_account() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    let with_account = |kind, code: &str, account| NewActivity {
        kind,
        code: code.into(),
        account_id: account,
        ..Default::default()
    };
    log.append(&with_account(ActivityKind::Error, code::INDEX_FAILED, Some(1)), 10).unwrap();
    log.append(&with_account(ActivityKind::Event, code::INDEX_COMPLETE, Some(2)), 20).unwrap();
    log.append(&with_account(ActivityKind::Error, code::MODEL_FAILED, None), 30).unwrap();
    let family = [code::INDEX_COMPLETE, code::INDEX_FAILED];
    assert_eq!(log.last(&family, Some(1)).unwrap().unwrap().code, "indexFailed");
    assert!(log.last(&family, Some(3)).unwrap().is_none());
    assert!(log.last(&family, None).unwrap().is_none(), "None never matches an account");
    assert_eq!(log.last(&[code::MODEL_INSTALLED, code::MODEL_FAILED], None).unwrap().unwrap().code, "modelFailed");
    assert!(log.last(&[], Some(1)).unwrap().is_none());
    log.append(&with_account(ActivityKind::Event, code::INDEX_COMPLETE, Some(1)), 40).unwrap();
    assert_eq!(log.last(&family, Some(1)).unwrap().unwrap().code, "indexComplete", "the newest of the family");
}

#[test]
fn unknown_text_from_a_newer_release_reads_as_a_plain_event() {
    let scratch = Scratch::new();
    drop(ActivityDb::in_dir(&scratch.0).append(&action(code::ARCHIVE, None), 1).unwrap());
    let raw = Connection::open(scratch.file()).unwrap();
    raw.execute(
        "INSERT INTO activity_log (at_ms, changed_ms, kind, code, source, outcome) \
         VALUES (5, 5, 'milestone', 'futureThing', 'robot', 'pending')",
        [],
    )
    .unwrap();
    drop(raw);
    let page = ActivityDb::in_dir(&scratch.0).page(None, 10, ActivityFilter::All).unwrap();
    assert_eq!(page[0].code, "futureThing", "a row from a newer release is not an error");
    assert_eq!((page[0].kind, page[0].source, page[0].outcome), (ActivityKind::Event, ActivitySource::System, ActivityOutcome::Done));
}

#[test]
fn pruning_drops_rows_older_than_the_cutoff_and_keeps_the_rest() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    let now = now_ms();
    log.append(&action(code::ARCHIVE, None), now - ACTIVITY_RETENTION_MS - 86_400_000).unwrap();
    let recent = log.append(&action(code::SNOOZE, None), now - 86_400_000).unwrap();
    log.prune(now - ACTIVITY_RETENTION_MS).unwrap();
    assert_eq!(seqs(&log.page(None, 10, ActivityFilter::All).unwrap()), vec![recent]);
    // A row never reuses a number, even after the table was emptied of its newest rows.
    log.prune(now + 1).unwrap();
    assert!(log.page(None, 10, ActivityFilter::All).unwrap().is_empty());
    assert!(log.append(&action(code::ARCHIVE, None), now).unwrap() > recent);
}

#[test]
fn the_feed_announces_what_another_process_wrote_and_never_what_this_one_did() {
    let scratch = Scratch::new();
    let log = ActivityDb::in_dir(&scratch.0);
    let seq = log.append(&action(code::ARCHIVE, Some(9)), 10).unwrap();
    log.set_outcome(9, ActivityOutcome::Undone, None).unwrap();
    let head = log.feed_head().unwrap();
    assert_eq!(head, 2, "the append and the outcome each add a feed row");

    let own = std::process::id();
    let (cursor, ids) = log.feed_since(0, own).unwrap();
    assert_eq!((cursor, ids), (head, vec![]), "this process's rows advance the cursor and announce nothing");
    // Seen from another process (a different pid), the same rows are news, once per row.
    let (cursor, ids) = log.feed_since(0, own + 1).unwrap();
    assert_eq!((cursor, ids), (head, vec![seq as u64]));
    // Nothing new: the cursor stays and nothing is returned.
    assert_eq!(log.feed_since(head, own + 1).unwrap(), (head, vec![]));
}

#[test]
fn two_handles_on_one_directory_share_the_log_like_the_two_processes_do() {
    let scratch = Scratch::new();
    let desktop = ActivityDb::in_dir(&scratch.0);
    let mcp = ActivityDb::in_dir(&scratch.0);
    let seq = mcp.append(&action(code::ARCHIVE, None), 1).unwrap();
    assert_eq!(seqs(&desktop.page(None, 10, ActivityFilter::All).unwrap()), vec![seq]);
}

#[test]
fn concurrent_writers_each_get_their_own_row() {
    let scratch = Scratch::new();
    let log = std::sync::Arc::new(ActivityDb::in_dir(&scratch.0));
    let handles: Vec<_> = (0..4)
        .map(|writer| {
            let log = log.clone();
            std::thread::spawn(move || {
                for step in 0..10 {
                    log.append(&action(code::ARCHIVE, Some(writer * 100 + step)), 1).unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(log.page(None, 100, ActivityFilter::All).unwrap().len(), 40);
}

// ---- through the two store adapters ----

#[test]
fn the_json_store_keeps_a_history_in_its_own_file() {
    let scratch = Scratch::new();
    let repository = JsonStoreRepository::new(scratch.0.join("store.json"));
    assert!(repository.activity_supported(), "the app ships on the JSON engine and must have a history");
    assert!(!scratch.file().exists(), "reading the store creates no activity file");
    repository.read().unwrap();
    assert!(!scratch.file().exists());
    let seq = repository.append_activity(&action(code::ARCHIVE, Some(1)), 10).unwrap();
    assert_eq!(seq, Some(1));
    assert_eq!(repository.activity_page(None, 10, ActivityFilter::All).unwrap().len(), 1);
    assert_eq!(repository.set_activity_outcome(1, ActivityOutcome::Undone, None).unwrap(), Some(1));
    assert!(scratch.file().exists());
}

#[test]
fn a_demo_workspace_keeps_no_history() {
    let scratch = Scratch::new();
    let repository = JsonStoreRepository::demo(scratch.0.join("store.json"));
    assert!(!repository.activity_supported());
    assert_eq!(repository.append_activity(&action(code::ARCHIVE, None), 10).unwrap(), None);
    assert!(repository.activity_page(None, 10, ActivityFilter::All).unwrap().is_empty());
    assert_eq!(repository.unseen_activity_errors().unwrap(), 0);
    assert!(!scratch.file().exists(), "a demo never writes into the data directory");
}

#[test]
fn the_sqlite_engine_and_the_json_engine_share_one_history() {
    let scratch = Scratch::new();
    let json = JsonStoreRepository::new(scratch.0.join("store.json"));
    json.append_activity(&action(code::ARCHIVE, None), 10).unwrap();
    let sqlite = SqliteStoreRepository::open(scratch.0.join("store.sqlite")).unwrap();
    assert!(sqlite.activity_supported());
    assert_eq!(sqlite.activity_page(None, 10, ActivityFilter::All).unwrap().len(), 1, "switching engines loses no history");
    sqlite.append_activity(&action(code::SNOOZE, None), 20).unwrap();
    assert_eq!(json.activity_page(None, 10, ActivityFilter::All).unwrap().len(), 2);
}

#[test]
fn the_maintenance_checkpoint_prunes_the_history_on_both_engines() {
    let now = now_ms();
    let old = now - ACTIVITY_RETENTION_MS - 86_400_000;
    for engine in ["json", "sqlite"] {
        let scratch = Scratch::new();
        let repository: Box<dyn StoreRepository> = if engine == "json" {
            Box::new(JsonStoreRepository::new(scratch.0.join("store.json")))
        } else {
            Box::new(SqliteStoreRepository::open(scratch.0.join("store.sqlite")).unwrap())
        };
        repository.append_activity(&action(code::ARCHIVE, None), old).unwrap();
        let recent = repository.append_activity(&action(code::SNOOZE, None), now - 1000).unwrap().unwrap();
        repository.storage_checkpoint().unwrap();
        let rows = repository.activity_page(None, 10, ActivityFilter::All).unwrap();
        assert_eq!(seqs(&rows), vec![recent], "{engine}: only the old row is pruned");
    }
}

#[test]
fn a_repository_poller_sees_what_the_other_process_wrote() {
    let scratch = Scratch::new();
    let desktop = JsonStoreRepository::new(scratch.0.join("store.json"));
    let mut cursor = desktop.activity_feed_head().unwrap();
    // The MCP process writes through its own handle; to the desktop it has another pid, which the
    // feed records at write time. Here both handles share this test's pid, so the foreign write is
    // planted with another pid directly in the file.
    desktop.append_activity(&action(code::ARCHIVE, None), 1).unwrap();
    let raw = Connection::open(scratch.file()).unwrap();
    raw.execute(
        "INSERT INTO activity_log (at_ms, changed_ms, kind, code, source, outcome) VALUES (2, 2, 'action', 'archive', 'assistant', 'done')",
        [],
    )
    .unwrap();
    let planted = raw.last_insert_rowid();
    raw.execute(
        "INSERT INTO activity_feed (activity_seq, pid) VALUES (?1, ?2)",
        rusqlite::params![planted, i64::from(std::process::id()) + 1],
    )
    .unwrap();
    drop(raw);
    let (next, ids) = desktop.poll_activity_feed(cursor).unwrap();
    assert!(next > cursor);
    assert_eq!(ids, vec![planted as u64], "only the foreign write is announced");
    cursor = next;
    assert_eq!(desktop.poll_activity_feed(cursor).unwrap(), (cursor, vec![]));
}

#[test]
fn a_store_without_a_data_directory_log_answers_with_the_empty_defaults() {
    struct Bare;
    impl StoreRepository for Bare {
        fn read(&self) -> qamille::application::AppResult<qamille::model::Store> {
            Ok(Default::default())
        }
        fn transaction(
            &self,
            _: &mut dyn FnMut(&mut qamille::model::Store) -> qamille::application::AppResult<()>,
        ) -> qamille::application::AppResult<()> {
            Ok(())
        }
    }
    let bare = Bare;
    assert!(!bare.activity_supported());
    assert_eq!(bare.append_activity(&action(code::ARCHIVE, None), 1).unwrap(), None);
    assert!(bare.activity_page(None, 10, ActivityFilter::All).unwrap().is_empty());
    assert_eq!(bare.poll_activity_feed(7).unwrap(), (7, vec![]));
    assert_eq!(bare.activity_feed_head().unwrap(), 0);
    bare.prune_activity(now_ms()).unwrap();
    let _: &dyn ActivityStore = &ActivityDb::in_dir(&std::env::temp_dir());
}
