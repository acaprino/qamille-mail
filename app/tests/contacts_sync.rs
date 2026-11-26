#[path = "support/contacts.rs"]
mod support;

use qamille::{
    application::{contact_merge::MergeAction, contact_ports::ContactStore, contact_sync::*},
    domain::contacts::*,
};
use std::sync::{Arc, atomic::Ordering};
use support::*;

fn keys(items: &[&str]) -> Vec<String> {
    items.iter().map(|k| k.to_string()).collect()
}
fn state(fx: &Fixture) -> ContactsState {
    fx.contacts.read().unwrap()
}
fn person(id: &str, etag: &str, name: &str, email: &str) -> qamille::application::contact_ports::RemoteContact {
    FakeContactGateway::remote(id, etag, fields(name, &[email]))
}
fn online() -> (Fixture, Arc<FakeContactGateway>) {
    let gateway = Arc::new(FakeContactGateway::default());
    let fx = fixture_with(base_store(), gateway.clone());
    grant(&fx, &["google:1", "google:2"]);
    (fx, gateway)
}
fn source_of<'a>(st: &'a ContactsState, remote: &str) -> &'a SourceContact {
    st.sources.iter().find(|s| s.remote_id.as_deref() == Some(remote)).unwrap_or_else(|| panic!("no source for {remote}"))
}

#[test]
fn first_sync_pages_until_exhausted_and_stores_the_token() {
    let (fx, gateway) = online();
    gateway.put(1, (1..=5).map(|n| person(&format!("people/{n}"), "e1", &format!("Persona {n}"), &format!("p{n}@x.it"))).collect());
    gateway.page_size.store(2, Ordering::SeqCst);
    let report = fx.service.sync_book("google:1").unwrap();
    assert_eq!((report.added, report.full, report.error.clone()), (5, true, None));
    assert_eq!(gateway.calls("list 1 other=false"), 3);
    let st = state(&fx);
    assert_eq!(st.sources.len(), 5);
    assert_eq!(st.sync_tokens.get("google:1").map(String::as_str), Some("T1"));
    assert!(st.synced_at.contains_key("google:1"));
    let first = source_of(&st, "people/1");
    assert_eq!((first.sync.clone(), first.base.clone()), (SyncState::Synced, Some(first.fields.clone())));
    assert_eq!(first.etag.as_deref(), Some("e1"));
}

#[test]
fn incremental_sync_applies_changes_and_removals() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it"), person("people/2", "e1", "Bea", "bea@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let mut gone = person("people/2", "e2", "Bea", "bea@x.it");
    gone.deleted = true;
    gateway.deltas.lock().unwrap().insert(1, vec![person("people/1", "e9", "Ada Lovelace", "ada@x.it"), gone, person("people/3", "e1", "Carla", "carla@x.it")]);
    let report = fx.service.sync_book("google:1").unwrap();
    assert_eq!((report.added, report.updated, report.removed, report.full), (1, 1, 1, false));
    assert!(gateway.calls("list 1 other=false token=Some(\"T1\")") == 1);
    let st = state(&fx);
    assert_eq!(source_of(&st, "people/1").fields.name, "Ada Lovelace");
    assert_eq!(source_of(&st, "people/1").etag.as_deref(), Some("e9"));
    assert!(st.sources.iter().all(|s| s.remote_id.as_deref() != Some("people/2")));
    assert_eq!(st.sources.len(), 2);
}

#[test]
fn expired_token_triggers_a_full_resync_without_an_error() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it"), person("people/2", "e1", "Bea", "bea@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    gateway.expire.store(true, Ordering::SeqCst);
    let report = fx.service.sync_book("google:1").unwrap();
    assert_eq!((report.full, report.removed, report.error), (true, 1, None));
    assert_eq!(state(&fx).sources.len(), 1);
}

#[test]
fn full_resync_keeps_cards_with_unsent_local_edits() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.org = "Acme".into();
    fx.service.update(&id, edited).unwrap();
    gateway.put(1, vec![]);
    gateway.expire.store(true, Ordering::SeqCst);
    fx.service.sync_book("google:1").unwrap();
    assert_eq!(state(&fx).sources.len(), 1, "a card with pending local edits is not dropped by a resync");
}

#[test]
fn unmanaged_fields_survive_a_read_write_cycle() {
    let (fx, gateway) = online();
    let mut remote = person("people/1", "e1", "Ada", "ada@x.it");
    remote.unmanaged = serde_json::json!({"memberships": [{"contactGroupMembership": {"contactGroupId": "family"}}], "photos": [{"url": "https://x.test/p.png"}]});
    gateway.put(1, vec![remote.clone()]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.title = "Analista".into();
    fx.service.update(&id, edited).unwrap();
    let outcome = fx.service.flush_queue().unwrap();
    assert!(outcome.iter().all(|o| o.ok), "{outcome:?}");
    assert_eq!(gateway.last_update_unmanaged.lock().unwrap().clone(), Some(remote.unmanaged.clone()));
    assert_eq!(source_of(&state(&fx), "people/1").unmanaged, remote.unmanaged);
}

#[test]
fn writes_to_one_account_never_overlap() {
    let (fx, gateway) = online();
    for n in 0..4 {
        fx.service.create("google:1", fields(&format!("Nuovo {n}"), &[&format!("n{n}@x.it")])).unwrap();
    }
    assert_eq!(state(&fx).queue.len(), 4);
    let outcome = fx.service.flush_queue().unwrap();
    assert!(outcome.iter().all(|o| o.ok), "{outcome:?}");
    assert_eq!(gateway.max_active.load(Ordering::SeqCst), 1);
    assert!(state(&fx).queue.is_empty());
    assert!(state(&fx).sources.iter().all(|s| s.remote_id.is_some() && s.sync == SyncState::Synced));
}

#[test]
fn update_uses_the_stored_etag_and_applies_the_response_locally() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.org = "Acme".into();
    fx.service.update(&id, edited.clone()).unwrap();
    assert_eq!(all(&fx)[0].origins[0].sync, SyncState::Pending);
    fx.service.flush_queue().unwrap();
    assert!(gateway.log.lock().unwrap().iter().any(|l| l == "update 1 people/1 etag=e1"));
    let lists_before = gateway.calls("list");
    let st = state(&fx);
    let card = source_of(&st, "people/1");
    assert_ne!(card.etag.as_deref(), Some("e1"));
    assert_eq!((card.sync.clone(), card.base.clone(), card.fields.org.clone()), (SyncState::Synced, Some(card.fields.clone()), "Acme".to_string()));
    assert!(st.queue.is_empty());
    assert_eq!(gateway.calls("list"), lists_before, "no sync is used to read back a write");
}

#[test]
fn stale_etag_with_one_sided_change_merges_cleanly() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    // The server changes the organisation behind our back.
    {
        let mut server = gateway.server.lock().unwrap();
        let card = &mut server.get_mut(&1).unwrap()[0];
        card.fields.org = "Acme".into();
        card.etag = "e5".into();
    }
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.title = "Analista".into();
    fx.service.update(&id, edited).unwrap();
    fx.service.flush_queue().unwrap();
    let server = gateway.server.lock().unwrap();
    let card = &server[&1][0];
    assert_eq!((card.fields.org.as_str(), card.fields.title.as_str()), ("Acme", "Analista"));
    drop(server);
    assert_eq!(state(&fx).sources[0].sync, SyncState::Synced);
}

#[test]
fn stale_etag_with_the_same_field_changed_marks_a_conflict_and_keeps_all_three_versions() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    {
        let mut server = gateway.server.lock().unwrap();
        let card = &mut server.get_mut(&1).unwrap()[0];
        card.fields.name = "Ada B.".into();
        card.etag = "e5".into();
    }
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.name = "Ada L.".into();
    fx.service.update(&id, edited).unwrap();
    fx.service.flush_queue().unwrap();
    let st = state(&fx);
    let card = source_of(&st, "people/1");
    assert_eq!(card.sync, SyncState::Conflict);
    assert_eq!(card.fields.name, "Ada L.");
    assert_eq!(card.theirs.as_ref().map(|t| t.name.as_str()), Some("Ada B."));
    assert_eq!(card.base.as_ref().map(|b| b.name.as_str()), Some("Ada"));
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.name, "Ada B.", "the server copy is untouched");
    let updates = gateway.calls("update");
    fx.service.flush_queue().unwrap();
    assert_eq!(gateway.calls("update"), updates, "a conflicted card is not retried");
}

#[test]
fn resolving_a_conflict_with_mine_rewrites_with_the_new_etag_and_theirs_adopts_the_server() {
    for keep in [Keep::Mine, Keep::Theirs] {
        let (fx, gateway) = online();
        gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
        fx.service.sync_book("google:1").unwrap();
        {
            let mut server = gateway.server.lock().unwrap();
            let card = &mut server.get_mut(&1).unwrap()[0];
            card.fields.name = "Ada B.".into();
            card.etag = "e5".into();
        }
        let id = all(&fx)[0].id.clone();
        let mut edited = all(&fx)[0].fields.clone();
        edited.name = "Ada L.".into();
        fx.service.update(&id, edited).unwrap();
        fx.service.flush_queue().unwrap();
        let key = state(&fx).sources[0].key.clone();
        fx.service.resolve_conflict(&key, keep).unwrap();
        fx.service.flush_queue().unwrap();
        let st = state(&fx);
        let card = &st.sources[0];
        assert_eq!((card.sync.clone(), card.theirs.clone()), (SyncState::Synced, None));
        let expected = if keep == Keep::Mine { "Ada L." } else { "Ada B." };
        assert_eq!(card.fields.name, expected);
        assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.name, expected);
        assert!(st.queue.is_empty());
    }
}

#[test]
fn first_write_without_consent_asks_for_authorization_and_fails_saving_if_refused() {
    let gateway = Arc::new(FakeContactGateway::default());
    let fx = fixture_with(base_store(), gateway.clone());
    *gateway.refuse_consent.lock().unwrap() = Some("Consent refused".into());
    let refused = fx.service.create("google:1", fields("Ada", &["a@x.it"]));
    assert_eq!(refused.unwrap_err().code, "consent_required");
    let st = state(&fx);
    assert!(st.sources.is_empty() && st.queue.is_empty(), "nothing is saved or queued without consent");
    assert!(st.settings.write_granted.is_empty());
    *gateway.refuse_consent.lock().unwrap() = None;
    fx.service.create("google:1", fields("Ada", &["a@x.it"])).unwrap();
    fx.service.create("google:1", fields("Bea", &["b@x.it"])).unwrap();
    assert_eq!(state(&fx).settings.write_granted, vec!["google:1".to_string()]);
    assert_eq!(gateway.calls("authorize 1 write=true"), 2, "one refused attempt and one that succeeded; later writes do not ask again");
}

#[test]
fn other_contacts_are_read_only_and_promote_creates_a_google_card() {
    let (fx, gateway) = online();
    gateway.other.lock().unwrap().insert(1, vec![person("otherContacts/9", "e1", "Dan Grigi", "dan@x.it")]);
    fx.service.sync_book("google-other:1").unwrap();
    let st = state(&fx);
    assert_eq!(st.sources.len(), 1);
    assert_eq!(st.sources[0].book, BookId::google_other(1));
    let card = all(&fx).remove(0);
    assert!(card.origins.iter().all(|o| !o.writable));
    let promoted = fx.service.promote_other(&card.id).unwrap();
    assert!(promoted.origins.iter().any(|o| o.book == "google:1"));
    assert_eq!(gateway.calls("promote 1"), 1);
}

#[test]
fn a_disabled_book_is_skipped_by_sync() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.set_sync("google:1", false).unwrap();
    let report = fx.service.sync_book("google:1").unwrap();
    assert!(report.skipped);
    assert_eq!(gateway.calls("list"), 0);
    let all_books = fx.service.sync_all().unwrap();
    assert!(all_books.books.iter().all(|b| b.book != "google:1" || b.skipped));
    fx.service.set_sync("google:1", true).unwrap();
    assert!(!fx.service.sync_book("google:1").unwrap().skipped);
}

#[test]
fn partial_failure_is_reported_per_book_and_failed_writes_stay_queued() {
    let (fx, gateway) = online();
    let mario = || fields("Mario Rossi", &["mario@x.it"]);
    gateway.put(1, vec![FakeContactGateway::remote("people/1", "e1", mario())]);
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), mario(), 1),
        source("c2", BookId::local(), None, mario(), 1),
    ]);
    gateway.fail_writes.lock().unwrap().insert(2, "Account 2 is unreachable".into());
    let outcome = fx.service.merge_apply(keys(&["c1", "c2"]), keys(&["local", "google:1", "google:2"]), fields("Mario R.", &["mario@x.it"])).unwrap();
    let by_book = |book: &str| outcome.per_book.iter().find(|o| o.book == book).unwrap().clone();
    assert!(by_book("local").ok && by_book("google:1").ok);
    assert!(!by_book("google:2").ok);
    assert!(by_book("google:2").error.unwrap().contains("unreachable"));
    let st = state(&fx);
    assert_eq!(st.queue.len(), 1, "{:?}", st.queue);
    assert_eq!(st.queue[0].op, WriteOp::Create);
    assert_eq!(gateway.calls("create 1"), 0);
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.name, "Mario R.");
    gateway.fail_writes.lock().unwrap().clear();
    let retry = fx.service.flush_queue().unwrap();
    assert!(retry.iter().all(|o| o.ok), "{retry:?}");
    assert!(state(&fx).queue.is_empty());
    assert_eq!(gateway.calls("update 1 people/1"), 1, "the part that already succeeded is not repeated");
}

#[test]
fn undo_after_a_later_remote_change_becomes_a_conflict_not_a_blind_restore() {
    let (fx, gateway) = online();
    let mario = || fields("Mario Rossi", &["mario@x.it"]);
    gateway.put(1, vec![FakeContactGateway::remote("people/1", "e1", mario())]);
    seed(&fx, vec![source("c1", BookId::google(1), Some("people/1"), mario(), 1), source("c2", BookId::local(), None, mario(), 1)]);
    let mut merged = mario();
    merged.org = "Acme".into();
    let outcome = fx.service.merge_apply(keys(&["c1", "c2"]), keys(&["local", "google:1"]), merged).unwrap();
    {
        let mut server = gateway.server.lock().unwrap();
        let card = &mut server.get_mut(&1).unwrap()[0];
        card.fields.org = "Beta Srl".into();
        card.etag = "e77".into();
    }
    fx.service.merge_undo(outcome.journal_id).unwrap();
    fx.service.flush_queue().unwrap();
    let st = state(&fx);
    let card = source_of(&st, "people/1");
    assert_eq!(card.sync, SyncState::Conflict);
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.org, "Beta Srl", "the later remote change is not overwritten");
}

#[test]
fn sync_all_reads_every_enabled_google_book_and_flushes_the_queue() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    gateway.put(2, vec![person("people/2", "e1", "Bea", "bea@y.it")]);
    fx.service.create("google:1", fields("Nuovo", &["n@x.it"])).unwrap();
    let report = fx.service.sync_all().unwrap();
    let synced: Vec<&str> = report.books.iter().map(|b| b.book.as_str()).collect();
    for book in ["google:1", "google-other:1", "google:2", "google-other:2"] {
        assert!(synced.contains(&book), "{book} in {synced:?}");
    }
    assert!(report.writes.iter().all(|w| w.ok));
    assert_eq!(gateway.calls("create 1"), 1);
}

#[test]
fn a_failing_book_reports_its_error_and_does_not_stop_the_others() {
    let (fx, gateway) = online();
    gateway.put(2, vec![person("people/2", "e1", "Bea", "bea@y.it")]);
    gateway.fail_reads.lock().unwrap().insert(1, "Account 1 is unreachable".into());
    let report = fx.service.sync_all().unwrap();
    let first = report.books.iter().find(|b| b.book == "google:1").unwrap();
    assert!(first.error.as_deref().unwrap().contains("unreachable"));
    assert!(report.books.iter().any(|b| b.book == "google:2" && b.error.is_none() && b.added == 1));
}

#[test]
fn settings_round_trip_priority_and_roles() {
    let (fx, _) = online();
    fx.service.save_settings(keys(&["google:2", "local"]), keys(&["ciao"])).unwrap();
    let settings = fx.service.settings().unwrap();
    assert_eq!(settings.priority, vec![BookId::google(2), BookId::local()]);
    assert_eq!(settings.roles, vec!["ciao".to_string()]);
}

/// A Google card whose `name` was changed on both sides: it is "in conflict" with all three versions kept.
fn conflicted() -> (Fixture, Arc<FakeContactGateway>, String) {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    {
        let mut server = gateway.server.lock().unwrap();
        let card = &mut server.get_mut(&1).unwrap()[0];
        card.fields.name = "Ada B.".into();
        card.etag = "e5".into();
    }
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.name = "Ada L.".into();
    fx.service.update(&id, edited).unwrap();
    fx.service.flush_queue().unwrap();
    let key = state(&fx).sources[0].key.clone();
    assert_eq!(state(&fx).sources[0].sync, SyncState::Conflict);
    (fx, gateway, key)
}

#[test]
fn editing_a_card_in_conflict_is_refused_and_nothing_changes() {
    let (fx, gateway, key) = conflicted();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.phones = vec![Labeled { label: String::new(), value: "333 1234567".into() }];
    let refused = fx.service.update(&id, edited).unwrap_err();
    assert_eq!(refused.code, "conflict_pending");
    let before = state(&fx);
    let card = before.sources.iter().find(|s| s.key == key).unwrap();
    assert_eq!((card.sync.clone(), card.fields.phones.len()), (SyncState::Conflict, 0));
    assert_eq!(card.theirs.as_ref().map(|t| t.name.as_str()), Some("Ada B."));
    fx.service.flush_queue().unwrap();
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.name, "Ada B.", "Google's version is untouched");
}

#[test]
fn merge_will_not_overwrite_a_conflicted_copy_and_the_preview_says_so() {
    let (fx, gateway, key) = conflicted();
    seed(&fx, vec![source("loc", BookId::local(), None, fields("Ada L.", &["ada@x.it"]), 1)]);
    let keys = vec![key.clone(), "loc".to_string()];
    let keep = vec!["local".to_string(), "google:1".to_string()];
    let preview = fx.service.merge_preview(keys.clone(), keep.clone()).unwrap();
    let google = preview.actions.iter().find(|a| a.book == "google:1").unwrap();
    assert!(matches!(&google.action, MergeAction::Unavailable(reason) if reason.contains("conflict")), "{:?}", google.action);
    let outcome = fx.service.merge_apply(keys, keep, fields("Ada Lovelace", &["ada@x.it"])).unwrap();
    assert!(!outcome.per_book.iter().find(|o| o.book == "google:1").unwrap().ok);
    let st = state(&fx);
    assert_eq!(st.sources.iter().find(|s| s.key == key).unwrap().fields.name, "Ada L.", "the conflicted card keeps its local version");
    assert_eq!(st.sources.iter().find(|s| s.key == key).unwrap().sync, SyncState::Conflict);
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.name, "Ada B.");
}

#[test]
fn undo_refuses_when_a_card_it_would_restore_is_in_conflict() {
    let (fx, gateway) = online();
    let mario = || fields("Mario Rossi", &["mario@x.it"]);
    gateway.put(1, vec![FakeContactGateway::remote("people/1", "e1", mario())]);
    seed(&fx, vec![source("c1", BookId::google(1), Some("people/1"), mario(), 1), source("c2", BookId::local(), None, mario(), 1)]);
    let mut merged = mario();
    merged.org = "Acme".into();
    let outcome = fx.service.merge_apply(keys(&["c1", "c2"]), keys(&["local", "google:1"]), merged).unwrap();
    // Put the Google card in conflict, as a later sync would.
    fx.contacts
        .transaction(&mut |s| {
            let card = s.sources.iter_mut().find(|c| c.key == "c1").unwrap();
            card.sync = SyncState::Conflict;
            card.theirs = Some(fields("Mario R.", &[]));
            Ok(())
        })
        .unwrap();
    let refused = fx.service.merge_undo(outcome.journal_id).unwrap_err();
    assert_eq!(refused.code, "conflict_pending");
    assert_eq!(state(&fx).journal.len(), 1, "the history entry is kept so the undo can be retried");
}

#[test]
fn an_edit_made_while_a_write_is_in_flight_is_kept_and_sent_next() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut v1 = all(&fx)[0].fields.clone();
    v1.org = "Acme".into();
    fx.service.update(&id, v1).unwrap();
    let (service, anchor) = (fx.service.clone(), id.clone());
    *gateway.on_write.lock().unwrap() = Some(Box::new(move || {
        let mut v2 = service.get(&anchor).unwrap().fields;
        v2.title = "CTO".into();
        service.update(&anchor, v2).unwrap();
    }));
    fx.service.flush_queue().unwrap();
    let st = state(&fx);
    let card = source_of(&st, "people/1");
    assert_eq!((card.fields.org.as_str(), card.fields.title.as_str()), ("Acme", "CTO"), "the newer edit is not rolled back");
    assert_eq!(card.sync, SyncState::Pending);
    assert_eq!(st.queue, vec![QueuedWrite { key: card.key.clone(), op: WriteOp::Update }]);
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.title, "", "the server only has the first version so far");
    fx.service.flush_queue().unwrap();
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.title, "CTO");
    let st = state(&fx);
    assert!(st.queue.is_empty());
    assert_eq!(source_of(&st, "people/1").sync, SyncState::Synced);
}

#[test]
fn an_edit_made_while_a_create_is_in_flight_becomes_an_update() {
    let (fx, gateway) = online();
    let created = fx.service.create("google:1", fields("Bea", &["bea@x.it"])).unwrap();
    let (service, anchor) = (fx.service.clone(), created.id.clone());
    *gateway.on_write.lock().unwrap() = Some(Box::new(move || {
        let mut v2 = service.get(&anchor).unwrap().fields;
        v2.org = "Beta Srl".into();
        service.update(&anchor, v2).unwrap();
    }));
    fx.service.flush_queue().unwrap();
    let st = state(&fx);
    let card = &st.sources[0];
    assert!(card.remote_id.is_some(), "the create reached Google");
    assert_eq!((card.fields.org.as_str(), card.sync.clone()), ("Beta Srl", SyncState::Pending));
    assert_eq!(st.queue, vec![QueuedWrite { key: card.key.clone(), op: WriteOp::Update }]);
    assert_eq!(gateway.calls("create 1"), 1);
    fx.service.flush_queue().unwrap();
    assert_eq!(gateway.calls("create 1"), 1, "the card is not created a second time");
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.org, "Beta Srl");
}

#[test]
fn delete_during_creation_survives_a_restart_even_when_the_create_reply_is_lost() {
    use qamille::{application::contacts::ContactService, infrastructure::contact_repository::FileContactStore};

    let (fx, gateway) = online();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("qamille-contact-create-delete-{}-{stamp}", std::process::id()));
    let store = Arc::new(FileContactStore::new(path.clone()));
    let open = || ContactService::new(fx.context.clone(), Arc::new(FileContactStore::new(path.clone())), gateway.clone());
    let service = open();
    let created = service.create("google:1", fields("Bea", &["bea@x.it"])).unwrap();

    // Save the exact durable state at the crash point, before Google returns its resource name.
    let pending = Arc::new(std::sync::Mutex::new(None));
    let snapshot = pending.clone();
    let writer = open();
    let reader = store.clone();
    *gateway.on_write.lock().unwrap() = Some(Box::new(move || {
        writer.delete(&created.id, vec!["google:1".into()]).unwrap();
        *snapshot.lock().unwrap() = Some(reader.read().unwrap());
    }));
    assert!(service.flush_queue().unwrap().iter().all(|o| o.ok));
    let acknowledged = store.read().unwrap();
    assert!(acknowledged.sources[0].deleted && acknowledged.sources[0].remote_id.is_some());
    assert_eq!(acknowledged.queue[0].op, WriteOp::Delete);

    let interrupted = pending.lock().unwrap().take().unwrap();
    assert!(interrupted.sources[0].deleted && interrupted.sources[0].remote_id.is_none());
    assert!(interrupted.sources[0].create_request.is_some());
    store.transaction(&mut |state| { *state = interrupted.clone(); Ok(()) }).unwrap();
    drop(service);
    let reopened = open();
    // An empty read while the create is still propagating must keep the deletion retryable.
    let provider_copy = gateway.server.lock().unwrap().remove(&1).unwrap();
    assert!(reopened.flush_queue().unwrap().iter().all(|o| !o.ok));
    assert_eq!(store.read().unwrap().queue[0].op, WriteOp::Delete);
    gateway.put(1, provider_copy);
    assert_eq!(reopened.sync_book("google:1").unwrap().added, 0);
    assert_eq!(store.read().unwrap().sources.len(), 1);
    assert!(reopened.flush_queue().unwrap().iter().all(|o| o.ok));
    assert!(gateway.server.lock().unwrap()[&1].is_empty());
    assert_eq!(gateway.calls("create 1"), 1);
    assert_eq!(gateway.calls("delete 1"), 1);
    assert!(store.read().unwrap().queue.is_empty());
    assert!(store.read().unwrap().sources.is_empty());
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn a_conflict_elsewhere_does_not_spoil_the_result_of_a_merge() {
    let (fx, gateway, _) = conflicted();
    gateway.server.lock().unwrap().get_mut(&1).unwrap().push(person("people/9", "e1", "Mario Rossi", "mario@x.it"));
    seed(&fx, vec![
        source("g9", BookId::google(1), Some("people/9"), fields("Mario Rossi", &["mario@x.it"]), 1),
        source("l9", BookId::local(), None, fields("Mario R.", &["mario@x.it"]), 1),
    ]);
    let outcome = fx.service.merge_apply(keys(&["g9", "l9"]), keys(&["local", "google:1"]), fields("Mario Rossi", &["mario@x.it", "m@acme.it"])).unwrap();
    assert!(outcome.per_book.iter().all(|o| o.ok), "the unrelated conflict on Ada is not this merge's failure: {:?}", outcome.per_book);
    assert_eq!(gateway.server.lock().unwrap()[&1].iter().find(|c| c.remote_id == "people/9").unwrap().fields.emails.len(), 2);
}

#[test]
fn queued_writes_of_a_removed_account_are_skipped_without_noise() {
    let (fx, _) = online();
    seed(&fx, vec![source("ghost", BookId::google(9), Some("people/9"), fields("Fantasma", &["f@x.it"]), 1)]);
    fx.contacts.transaction(&mut |s| { s.queue.push(QueuedWrite { key: "ghost".into(), op: WriteOp::Update }); Ok(()) }).unwrap();
    let outcome = fx.service.flush_queue().unwrap();
    assert!(outcome.is_empty(), "{outcome:?}");
    let st = state(&fx);
    assert_eq!(st.sources[0].sync, SyncState::Synced, "the card is not marked as failed");
    assert_eq!(st.queue.len(), 1, "the write waits in case the account comes back");
}

#[test]
fn a_remote_deletion_does_not_throw_away_unsent_local_edits() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.org = "Acme".into();
    fx.service.update(&id, edited).unwrap();
    let mut gone = person("people/1", "e2", "Ada", "ada@x.it");
    gone.deleted = true;
    gateway.deltas.lock().unwrap().insert(1, vec![gone]);
    gateway.server.lock().unwrap().insert(1, vec![]);
    fx.service.sync_book("google:1").unwrap();
    let st = state(&fx);
    assert_eq!(st.sources.len(), 1, "the edited card survives");
    let card = &st.sources[0];
    assert_eq!((card.fields.org.as_str(), card.remote_id.clone(), card.etag.clone()), ("Acme", None, None));
    assert_eq!(st.queue, vec![QueuedWrite { key: card.key.clone(), op: WriteOp::Create }], "it is created again on Google");
    fx.service.flush_queue().unwrap();
    assert_eq!(gateway.server.lock().unwrap()[&1].len(), 1);
    assert_eq!(gateway.server.lock().unwrap()[&1][0].fields.org, "Acme");
}

#[test]
fn a_remote_deletion_of_an_untouched_card_still_removes_it() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let mut gone = person("people/1", "e2", "Ada", "ada@x.it");
    gone.deleted = true;
    gateway.deltas.lock().unwrap().insert(1, vec![gone]);
    fx.service.sync_book("google:1").unwrap();
    assert!(state(&fx).sources.is_empty());
}

#[test]
fn a_book_without_read_consent_is_flagged_so_the_app_can_ask_for_it() {
    let (fx, gateway) = online();
    gateway.put(2, vec![person("people/2", "e1", "Bea", "bea@y.it")]);
    gateway.no_read_consent.lock().unwrap().insert(1);
    let report = fx.service.sync_all().unwrap();
    let first = report.books.iter().find(|b| b.book == "google:1").unwrap();
    assert!(first.consent_required, "{first:?}");
    assert!(first.error.is_some());
    let second = report.books.iter().find(|b| b.book == "google:2").unwrap();
    assert!(!second.consent_required && second.error.is_none() && second.added == 1);
    // After the user allows reading, the same book syncs.
    gateway.no_read_consent.lock().unwrap().clear();
    fx.service.authorize("google:1", false).unwrap();
    assert_eq!(gateway.calls("authorize 1 write=false"), 1);
    assert!(fx.service.sync_all().unwrap().books.iter().all(|b| !b.consent_required));
}

#[test]
fn a_write_refused_for_missing_consent_forgets_the_grant_and_stays_pending() {
    let (fx, gateway) = online();
    gateway.put(1, vec![person("people/1", "e1", "Ada", "ada@x.it")]);
    fx.service.sync_book("google:1").unwrap();
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.org = "Acme".into();
    fx.service.update(&id, edited.clone()).unwrap();
    // The user revoked access in their Google account after granting it here.
    gateway.write_without_consent.lock().unwrap().insert(1);
    let outcome = fx.service.flush_queue().unwrap();
    assert!(outcome.iter().any(|o| o.book == "google:1" && !o.ok), "{outcome:?}");
    let st = state(&fx);
    assert!(!st.settings.write_granted.contains(&"google:1".to_string()), "the remembered grant is dropped");
    assert!(st.settings.write_granted.contains(&"google:2".to_string()), "other books keep theirs");
    let card = source_of(&st, "people/1");
    assert_eq!(card.sync, SyncState::Pending, "waiting for permission is not an error of the card");
    assert_eq!(st.queue.len(), 1, "the write stays queued");
    // The next edit asks for permission again instead of silently failing.
    gateway.write_without_consent.lock().unwrap().clear();
    let before = gateway.calls("authorize 1 write=true");
    edited.org = "Acme Ltd".into();
    fx.service.update(&id, edited).unwrap();
    assert_eq!(gateway.calls("authorize 1 write=true"), before + 1);
    assert!(state(&fx).settings.write_granted.contains(&"google:1".to_string()));
}
