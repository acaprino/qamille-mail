//! The activity log port (centro attività, Task 1): the in-memory adapter keeps a
//! newest-first log, pages it, filters it, corrects an action's outcome by the
//! undo id, and counts the errors the user has not seen; the JSON store has no
//! log at all and answers with the empty defaults.

use qamille::{
    application::ports::StoreRepository,
    domain::activity::*,
    infrastructure::repository::MemoryRepository,
    model::Store,
};

fn memory() -> MemoryRepository {
    MemoryRepository::new(Store::default())
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

fn event(code: &str) -> NewActivity {
    NewActivity { kind: ActivityKind::Event, code: code.into(), ..Default::default() }
}

fn error(code: &str) -> NewActivity {
    NewActivity { kind: ActivityKind::Error, code: code.into(), ..Default::default() }
}

fn seqs(entries: &[ActivityEntry]) -> Vec<i64> {
    entries.iter().map(|entry| entry.seq).collect()
}

#[test]
fn memory_repository_appends_pages_and_filters() {
    let repository = memory();
    assert!(repository.activity_supported());
    let first = repository.append_activity(&action(code::MARK_AS_DONE, None), 10).unwrap();
    let second = repository.append_activity(&event(code::SYNC_COMPLETE), 20).unwrap();
    let third = repository.append_activity(&error(code::SYNC_FAILED), 30).unwrap();
    assert_eq!((first, second, third), (Some(1), Some(2), Some(3)));

    let newest = repository.activity_page(None, 2, ActivityFilter::All).unwrap();
    assert_eq!(seqs(&newest), vec![3, 2], "newest first, limited");
    assert_eq!(newest[0].at_ms, 30);
    let rest = repository.activity_page(Some(2), 10, ActivityFilter::All).unwrap();
    assert_eq!(seqs(&rest), vec![1], "before_seq is exclusive");

    let errors = repository.activity_page(None, 10, ActivityFilter::Error).unwrap();
    assert_eq!(seqs(&errors), vec![3]);
    let actions = repository.activity_page(None, 10, ActivityFilter::Action).unwrap();
    assert_eq!(seqs(&actions), vec![1]);
    let events = repository.activity_page(None, 10, ActivityFilter::Event).unwrap();
    assert_eq!(seqs(&events), vec![2]);
}

#[test]
fn memory_repository_updates_outcome_by_undo_id() {
    let repository = memory();
    repository.append_activity(&action(code::ARCHIVE, Some(77)), 10).unwrap();
    assert_eq!(
        repository.set_activity_outcome(77, ActivityOutcome::Undone, None).unwrap(),
        Some(1)
    );
    let page = repository.activity_page(None, 10, ActivityFilter::All).unwrap();
    assert_eq!(page[0].outcome, ActivityOutcome::Undone);
    assert_eq!(page[0].detail, None);
    assert_eq!(
        repository.set_activity_outcome(999, ActivityOutcome::Failed, None).unwrap(),
        None,
        "an unknown undo id changes nothing"
    );
}

#[test]
fn a_failed_action_counts_as_an_error_for_the_filter_and_for_unseen() {
    let repository = memory();
    repository.append_activity(&error(code::SYNC_FAILED), 30).unwrap();
    repository.append_activity(&action(code::SEND, Some(5)), 40).unwrap();
    repository.append_activity(&action(code::ARCHIVE, Some(6)), 50).unwrap();
    repository
        .set_activity_outcome(5, ActivityOutcome::Failed, Some("Server unreachable"))
        .unwrap();

    let errors = repository.activity_page(None, 10, ActivityFilter::Error).unwrap();
    assert_eq!(seqs(&errors), vec![2, 1], "the failed send is an error, the done archive is not");
    assert_eq!(errors[0].detail.as_deref(), Some("Server unreachable"));
    assert_eq!(repository.unseen_activity_errors().unwrap(), 2);
}

#[test]
fn unseen_errors_follow_the_seen_marker() {
    let repository = memory();
    repository.append_activity(&error(code::SYNC_FAILED), 30).unwrap();
    repository.append_activity(&action(code::SEND, Some(5)), 40).unwrap();
    repository.set_activity_outcome(5, ActivityOutcome::Failed, None).unwrap();
    repository.append_activity(&action(code::ARCHIVE, Some(6)), 50).unwrap();

    assert_eq!(repository.activity_seen_ms().unwrap(), 0);
    assert_eq!(repository.unseen_activity_errors().unwrap(), 2);
    repository.set_activity_seen_ms(35).unwrap();
    assert_eq!(repository.activity_seen_ms().unwrap(), 35);
    assert_eq!(repository.unseen_activity_errors().unwrap(), 1, "only the failure after the marker");
    repository.set_activity_seen_ms(60).unwrap();
    assert_eq!(repository.unseen_activity_errors().unwrap(), 0);
}

#[test]
fn memory_repository_finds_the_last_row_of_a_family() {
    let repository = memory();
    let mut first = error(code::INDEX_FAILED);
    first.account_id = Some(1);
    let mut second = event(code::INDEX_COMPLETE);
    second.account_id = Some(2);
    let mut model = error(code::MODEL_FAILED);
    model.account_id = None;
    repository.append_activity(&first, 10).unwrap();
    repository.append_activity(&second, 20).unwrap();
    repository.append_activity(&model, 30).unwrap();

    let family = [code::INDEX_COMPLETE, code::INDEX_FAILED];
    let found = repository.last_activity(&family, Some(1)).unwrap().unwrap();
    assert_eq!(found.code, code::INDEX_FAILED);
    assert!(repository.last_activity(&family, Some(3)).unwrap().is_none(), "other account");
    let global = repository
        .last_activity(&[code::MODEL_INSTALLED, code::MODEL_FAILED], None)
        .unwrap()
        .unwrap();
    assert_eq!(global.code, code::MODEL_FAILED, "None matches the rows without an account");
}

#[test]
fn only_four_mail_actions_are_logged() {
    for logged in ["markAsDone", "archive", "snooze", "moveToTrash"] {
        assert!(LOGGED_MAIL_ACTIONS.contains(&logged), "{logged}");
    }
    assert_eq!(LOGGED_MAIL_ACTIONS.len(), 4);
    assert_eq!(ACTIVITY_RETENTION_MS, 90 * 86_400_000);
}
