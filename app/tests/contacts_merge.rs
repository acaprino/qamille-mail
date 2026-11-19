#[path = "support/contacts.rs"]
mod support;

use qamille::{
    application::{contact_ports::ContactStore, contact_merge::*},
    domain::contacts::*,
};
use support::*;

fn grant(fx: &Fixture, books: &[&str]) {
    fx.contacts
        .transaction(&mut |s| {
            s.settings.write_granted = books.iter().map(|b| b.to_string()).collect();
            Ok(())
        })
        .unwrap();
}
fn state(fx: &Fixture) -> ContactsState {
    fx.contacts.read().unwrap()
}
fn keys(items: &[&str]) -> Vec<String> {
    items.iter().map(|k| k.to_string()).collect()
}
/// Mario Rossi in Google 1, Local and Google 2, sharing one email.
fn trio(fx: &Fixture) {
    let mario = || fields("Mario Rossi", &["mario@x.it"]);
    seed(fx, vec![
        source("c1", BookId::google(1), Some("people/1"), mario(), 100),
        source("c2", BookId::local(), None, mario(), 100),
        source("c3", BookId::google(2), Some("people/3"), mario(), 100),
    ]);
    grant(fx, &["google:1", "google:2"]);
}
fn merged_fields() -> ContactFields {
    let mut f = fields("Mario Rossi", &["mario@x.it", "m.rossi@acme.it"]);
    f.org = "Acme".into();
    f
}

#[test]
fn same_person_writes_nothing_remote_and_is_undone_by_unlink() {
    let fx = fixture(base_store());
    let mut a = fields("Mario Rossi", &["a@x.it"]);
    a.org = "Acme".into();
    let mut b = fields("Mario Rossi", &["b@y.it"]);
    b.org = "Acme".into();
    seed(&fx, vec![source("c1", BookId::local(), None, a, 1), source("c2", BookId::google(1), Some("people/2"), b, 1)]);
    assert_eq!(all(&fx).len(), 2);
    assert_eq!(fx.service.duplicates(false).unwrap().len(), 1);
    let before = state(&fx).sources;
    fx.service.same_person(keys(&["c1", "c2"])).unwrap();
    assert_eq!(all(&fx).len(), 1);
    assert_eq!(all(&fx)[0].origins.len(), 2);
    assert_eq!(state(&fx).sources, before, "no card is written");
    assert!(state(&fx).queue.is_empty());
    assert!(fx.service.duplicates(true).unwrap().is_empty(), "a confirmed link silences the signal");
    fx.service.unlink(keys(&["c1", "c2"])).unwrap();
    assert_eq!(all(&fx).len(), 2);
    assert_eq!(fx.service.duplicates(false).unwrap().len(), 1);
}

#[test]
fn different_people_is_remembered_and_not_proposed_again() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::local(), None, fields("Ada", &["same@x.it"]), 1),
        source("c2", BookId::google(1), Some("people/2"), fields("Bea", &["same@x.it"]), 1),
    ]);
    let groups = fx.service.duplicates(false).unwrap();
    assert_eq!((groups.len(), groups[0].level, groups[0].members.len()), (1, Level::Certain, 2));
    assert_eq!(all(&fx).len(), 1, "a certain match shows as one card");
    fx.service.different_people(keys(&["c1", "c2"])).unwrap();
    assert!(fx.service.duplicates(true).unwrap().is_empty());
    assert_eq!(all(&fx).len(), 2);
}

#[test]
fn merge_with_in_all_creates_missing_copies_and_updates_the_rest() {
    let gateway = std::sync::Arc::new(FakeContactGateway::default());
    gateway.put(1, vec![FakeContactGateway::remote("people/1", "e1", fields("Mario Rossi", &["mario@x.it"]))]);
    let fx = fixture_with(base_store(), gateway.clone());
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), fields("Mario Rossi", &["mario@x.it"]), 100),
        source("c2", BookId::local(), None, fields("M. Rossi", &["mario@x.it"]), 100),
    ]);
    grant(&fx, &["google:1", "google:2"]);
    let keep = keys(&["local", "google:1", "google:2"]);
    let preview = fx.service.merge_preview(keys(&["c1", "c2"]), keep.clone()).unwrap();
    let kinds: Vec<(&str, &MergeAction)> = preview.actions.iter().map(|a| (a.book.as_str(), &a.action)).collect();
    assert_eq!(kinds, vec![("google:1", &MergeAction::Update), ("google:2", &MergeAction::Create), ("local", &MergeAction::Update)]);
    let outcome = fx.service.merge_apply(keys(&["c1", "c2"]), keep, merged_fields()).unwrap();
    assert!(outcome.per_book.iter().all(|o| o.ok), "{:?}", outcome.per_book);
    let st = state(&fx);
    assert_eq!(st.sources.len(), 3);
    assert!(st.sources.iter().all(|s| s.fields == merged_fields()));
    assert!(st.queue.is_empty(), "the Google writes were sent: {:?}", st.queue);
    assert_eq!((gateway.calls("update 1 people/1"), gateway.calls("create 2")), (1, 1));
    assert_eq!(all(&fx).len(), 1);
    assert_eq!(all(&fx)[0].origins.len(), 3);
}

#[test]
fn merge_with_only_in_one_deletes_the_others_after_preview() {
    let fx = fixture(base_store());
    trio(&fx);
    let preview = fx.service.merge_preview(keys(&["c1", "c2", "c3"]), keys(&["local"])).unwrap();
    assert_eq!(preview.actions.iter().filter(|a| a.action == MergeAction::Delete).count(), 2);
    assert_eq!(preview.actions.iter().filter(|a| a.action == MergeAction::Update).count(), 1);
    assert_eq!(state(&fx).sources.len(), 3, "a preview changes nothing");
    fx.service.merge_apply(keys(&["c1", "c2", "c3"]), keys(&["local"]), merged_fields()).unwrap();
    let st = state(&fx);
    assert!(st.sources.iter().find(|s| s.key == "c1").unwrap().deleted);
    assert!(st.sources.iter().find(|s| s.key == "c3").unwrap().deleted);
    assert_eq!(st.sources.iter().find(|s| s.key == "c2").unwrap().fields, merged_fields());
    assert_eq!(st.queue.iter().filter(|q| q.op == WriteOp::Delete).count(), 2);
    assert_eq!(all(&fx).len(), 1);
}

#[test]
fn merge_with_no_book_checked_is_rejected() {
    let fx = fixture(base_store());
    trio(&fx);
    assert!(fx.service.merge_preview(keys(&["c1", "c2"]), vec![]).is_err());
    assert!(fx.service.merge_apply(keys(&["c1", "c2"]), vec![], merged_fields()).is_err());
    assert!(fx.service.merge_preview(keys(&["c1"]), keys(&["local"])).is_err(), "one card is not a group");
    assert!(fx.service.merge_preview(keys(&["c1", "nope"]), keys(&["local"])).is_err());
    assert!(fx.service.merge_preview(keys(&["c1", "c2"]), keys(&["mail:1"])).is_err(), "mail is not a destination");
    assert_eq!(state(&fx).sources.len(), 3);
}

#[test]
fn merge_never_deletes_mail_or_google_other_sources() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::local(), None, fields("Mario", &["mario@x.it"]), 1),
        source("c2", BookId::google_other(1), Some("otherContacts/9"), fields("Mario", &["mario@x.it"]), 1),
        source("c3", BookId::google(1), Some("people/3"), fields("Mario", &["mario@x.it"]), 1),
    ]);
    grant(&fx, &["google:1"]);
    let preview = fx.service.merge_preview(keys(&["c1", "c2", "c3"]), keys(&["local"])).unwrap();
    assert!(preview.actions.iter().all(|a| a.book != "google-other:1"));
    fx.service.merge_apply(keys(&["c1", "c2", "c3"]), keys(&["local"]), merged_fields()).unwrap();
    let st = state(&fx);
    let other = st.sources.iter().find(|s| s.key == "c2").unwrap();
    assert!(!other.deleted);
    assert_eq!(other.fields, fields("Mario", &["mario@x.it"]));
    assert_eq!(all(&fx).len(), 1, "the read-only card stays grouped with the others");
}

#[test]
fn read_only_book_is_unavailable_and_the_others_proceed() {
    let fx = fixture(base_store());
    trio(&fx);
    grant(&fx, &["google:1"]);
    let keep = keys(&["local", "google:1", "google:2"]);
    let preview = fx.service.merge_preview(keys(&["c1", "c2", "c3"]), keep.clone()).unwrap();
    let blocked = preview.actions.iter().find(|a| a.book == "google:2").unwrap();
    assert!(matches!(blocked.action, MergeAction::Unavailable(_)), "{:?}", blocked.action);
    let outcome = fx.service.merge_apply(keys(&["c1", "c2", "c3"]), keep, merged_fields()).unwrap();
    assert!(!outcome.per_book.iter().find(|o| o.book == "google:2").unwrap().ok);
    assert!(outcome.per_book.iter().find(|o| o.book == "local").unwrap().ok);
    let st = state(&fx);
    assert_eq!(st.sources.iter().find(|s| s.key == "c3").unwrap().fields, fields("Mario Rossi", &["mario@x.it"]));
    assert_eq!(st.sources.iter().find(|s| s.key == "c2").unwrap().fields, merged_fields());
}

#[test]
fn undo_restores_only_what_was_written() {
    let fx = fixture(base_store());
    trio(&fx);
    let outcome = fx.service.merge_apply(keys(&["c1", "c2", "c3"]), keys(&["local", "google:2"]), merged_fields()).unwrap();
    assert_eq!(state(&fx).journal.len(), 1);
    fx.service.merge_undo(outcome.journal_id).unwrap();
    let st = state(&fx);
    assert!(st.journal.is_empty());
    assert!(st.sources.iter().all(|s| !s.deleted));
    assert!(st.sources.iter().all(|s| s.fields == fields("Mario Rossi", &["mario@x.it"])), "{:?}", st.sources);
    assert!(!st.queue.iter().any(|q| q.op == WriteOp::Delete), "{:?}", st.queue);
    assert!(fx.service.merge_undo(outcome.journal_id).is_err(), "a journal entry is used once");
}

#[test]
fn undo_removes_cards_the_merge_created() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), fields("Mario Rossi", &["mario@x.it"]), 1),
        source("c2", BookId::local(), None, fields("Mario Rossi", &["mario@x.it"]), 1),
    ]);
    grant(&fx, &["google:1", "google:2"]);
    let outcome = fx.service.merge_apply(keys(&["c1", "c2"]), keys(&["local", "google:1", "google:2"]), merged_fields()).unwrap();
    assert_eq!(state(&fx).sources.len(), 3);
    fx.service.merge_undo(outcome.journal_id).unwrap();
    let st = state(&fx);
    assert_eq!(st.sources.len(), 2);
    assert!(!st.queue.iter().any(|q| q.op == WriteOp::Create));
}

#[test]
fn undo_recreates_a_remote_card_with_a_new_remote_id() {
    let fx = fixture(base_store());
    trio(&fx);
    let outcome = fx.service.merge_apply(keys(&["c1", "c2", "c3"]), keys(&["local"]), merged_fields()).unwrap();
    // The deletion reached Google: the tombstone and its queue entry are gone.
    fx.contacts
        .transaction(&mut |s| {
            s.sources.retain(|x| x.key != "c1");
            s.queue.retain(|q| q.key != "c1");
            Ok(())
        })
        .unwrap();
    let undone = fx.service.merge_undo(outcome.journal_id).unwrap();
    assert_eq!(undone.recreated, vec!["Mario Rossi".to_string()], "the app can tell the user which Google cards are new copies");
    assert!(undone.per_book.iter().all(|o| o.ok) && undone.per_book.iter().any(|o| o.book == "google:1"));
    let st = state(&fx);
    let back = st.sources.iter().find(|s| s.key == "c1").expect("the card is back");
    assert_eq!((back.remote_id.clone(), back.etag.clone()), (None, None));
    assert_eq!(back.sync, SyncState::Pending);
    assert!(st.queue.contains(&QueuedWrite { key: "c1".into(), op: WriteOp::Create }));
}

#[test]
fn journal_older_than_30_days_is_dropped() {
    let fx = fixture(base_store());
    trio(&fx);
    fx.contacts
        .transaction(&mut |s| {
            s.journal.push(MergeJournal { id: 900, at: 1_000_000, before: vec![], created: vec![], link_keys: vec![], links_before: vec![], outcome: vec![] });
            Ok(())
        })
        .unwrap();
    fx.service.merge_apply(keys(&["c1", "c2"]), keys(&["local"]), merged_fields()).unwrap();
    let ids: Vec<u64> = state(&fx).journal.iter().map(|j| j.id).collect();
    assert!(!ids.contains(&900), "{ids:?}");
    assert_eq!(ids.len(), 1);
}

#[test]
fn undo_keeps_a_link_the_user_made_before_the_merge() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::local(), None, fields("Mario Rossi", &["a@x.it"]), 1),
        source("c2", BookId::google(1), Some("people/2"), fields("M. Rossi", &["b@y.it"]), 1),
        source("c3", BookId::local(), None, fields("Mario Rossi", &["c@z.it"]), 1),
    ]);
    grant(&fx, &["google:1"]);
    fx.service.same_person(keys(&["c1", "c2"])).unwrap();
    assert_eq!(all(&fx).len(), 2, "c1+c2 are one card, c3 another");
    let outcome = fx.service.merge_apply(keys(&["c1", "c3"]), keys(&["local"]), merged_fields()).unwrap();
    fx.service.merge_undo(outcome.journal_id).unwrap();
    let cards = all(&fx);
    assert_eq!(cards.len(), 2, "the earlier link survives the undo: {:?}", state(&fx).links);
    assert!(cards.iter().any(|c| c.origins.len() == 2), "{cards:?}");
}
