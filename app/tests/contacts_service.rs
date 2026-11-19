#[path = "support/contacts.rs"]
mod support;

use qamille::{
    application::{contact_ports::ContactStore, contacts::*, mail::MailService},
    domain::contacts::*,
};
use support::*;

#[test]
fn same_person_in_google_local_and_mail_is_one_card_with_three_origins() {
    let mut store = base_store();
    store.emails.push(mail_from(2, "ada", "ada@x.it"));
    let fx = fixture(store);
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), fields("Ada Lovelace", &["ada@x.it"]), 200),
        source("c2", BookId::local(), None, fields("Ada L.", &["ADA@x.it"]), 100),
    ]);
    let cards = all(&fx);
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0].fields.name, "Ada Lovelace");
    let labels: Vec<&str> = cards[0].origins.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, vec!["Google · alex@work.test", "Locale", "Posta · alex@home.test"]);
    assert_eq!(fx.service.get(&cards[0].id).unwrap(), cards[0]);
}

#[test]
fn create_requires_a_book() {
    let fx = fixture(base_store());
    assert_eq!(fx.service.create("", fields("Ada", &["a@x.it"])).unwrap_err().code, "book_required");
    assert_eq!(fx.service.create("  ", fields("Ada", &["a@x.it"])).unwrap_err().code, "book_required");
    assert!(fx.contacts.read().unwrap().sources.is_empty());
}

#[test]
fn create_in_read_only_or_missing_books_is_rejected() {
    let fx = fixture(base_store());
    for book in ["mail:1", "google-other:1", "google:99", "google:abc"] {
        assert!(fx.service.create(book, fields("Ada", &["a@x.it"])).is_err(), "{book}");
    }
    assert!(fx.service.create("local", ContactFields::default()).is_err(), "an empty card is not a contact");
}

#[test]
fn create_in_local_remembers_the_book_and_google_queues_a_create() {
    let fx = fixture(base_store());
    grant(&fx, &["google:1"]);
    fx.service.create("local", fields("Ada", &["a@x.it"])).unwrap();
    let state = fx.contacts.read().unwrap();
    assert_eq!(state.settings.last_book.as_deref(), Some("local"));
    assert!(state.queue.is_empty());
    drop(state);
    let view = fx.service.create("google:1", fields("Bea", &["b@x.it"])).unwrap();
    let state = fx.contacts.read().unwrap();
    assert_eq!(state.settings.last_book.as_deref(), Some("google:1"));
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.queue[0].op, WriteOp::Create);
    assert_eq!(view.origins[0].sync, SyncState::Pending);
}

#[test]
fn mail_only_contact_is_read_only_and_prefill_returns_its_fields() {
    let mut store = base_store();
    store.emails.push(mail_from(1, "Bea Verdi", "bea@y.it"));
    let fx = fixture(store);
    let card = all(&fx).remove(0);
    assert!(card.origins.iter().all(|o| !o.writable));
    assert!(fx.service.update(&card.id, fields("Bea", &["bea@y.it"])).is_err());
    let prefill = fx.service.prefill(&card.id).unwrap();
    assert_eq!((prefill.name.as_str(), prefill.emails[0].value.as_str()), ("Bea Verdi", "bea@y.it"));
}

#[test]
fn update_writes_every_writable_origin_and_queues_google_writes() {
    let fx = fixture(base_store());
    grant(&fx, &["google:1"]);
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), fields("Ada", &["ada@x.it", "old@x.it"]), 100),
        source("c2", BookId::local(), None, fields("Ada", &["ada@x.it"]), 100),
    ]);
    let id = all(&fx)[0].id.clone();
    let mut edited = all(&fx)[0].fields.clone();
    edited.org = "Acme".into();
    edited.emails.retain(|e| e.value != "old@x.it");
    edited.emails.push(Labeled { label: "work".into(), value: "new@x.it".into() });
    fx.service.update(&id, edited).unwrap();
    let state = fx.contacts.read().unwrap();
    for s in &state.sources {
        assert_eq!(s.fields.org, "Acme", "{}", s.key);
        assert_eq!(s.modified, 2_000_000_000);
        assert!(s.fields.emails.iter().any(|e| e.value == "new@x.it"));
        assert!(!s.fields.emails.iter().any(|e| e.value == "old@x.it"));
    }
    assert_eq!(state.queue, vec![QueuedWrite { key: "c1".into(), op: WriteOp::Update }]);
    assert_eq!(state.sources[0].sync, SyncState::Pending);
    assert_eq!(state.sources[1].sync, SyncState::Synced);
    drop(state);
    fx.service.update(&id, all(&fx)[0].fields.clone()).unwrap();
    assert_eq!(fx.contacts.read().unwrap().queue.len(), 1, "a repeated update is not queued twice");
}

#[test]
fn delete_removes_only_the_chosen_books_and_never_a_mail_source() {
    let mut store = base_store();
    store.emails.push(mail_from(2, "ada", "ada@x.it"));
    let fx = fixture(store);
    grant(&fx, &["google:1"]);
    seed(&fx, vec![
        source("c1", BookId::google(1), Some("people/1"), fields("Ada", &["ada@x.it"]), 100),
        source("c2", BookId::local(), None, fields("Ada", &["ada@x.it"]), 100),
    ]);
    let id = all(&fx)[0].id.clone();
    assert!(fx.service.delete(&id, vec!["mail:2".into()]).is_err());
    fx.service.delete(&id, vec!["local".into()]).unwrap();
    assert_eq!(fx.contacts.read().unwrap().sources.len(), 1);
    fx.service.delete(&id, vec!["google:1".into()]).unwrap();
    let state = fx.contacts.read().unwrap();
    assert!(state.sources[0].deleted);
    assert_eq!(state.queue, vec![QueuedWrite { key: "c1".into(), op: WriteOp::Delete }]);
}

#[test]
fn deleting_a_card_never_sent_to_google_drops_its_queued_create() {
    let fx = fixture(base_store());
    grant(&fx, &["google:1"]);
    let view = fx.service.create("google:1", fields("Bea", &["b@x.it"])).unwrap();
    fx.service.delete(&view.id, vec!["google:1".into()]).unwrap();
    let state = fx.contacts.read().unwrap();
    assert!(state.sources.is_empty() && state.queue.is_empty());
}

#[test]
fn deleting_the_last_writable_origin_leaves_the_mail_card_visible() {
    let mut store = base_store();
    store.emails.push(mail_from(1, "ada", "ada@x.it"));
    let fx = fixture(store);
    seed(&fx, vec![source("c2", BookId::local(), None, fields("Ada Lovelace", &["ada@x.it"]), 100)]);
    let id = all(&fx)[0].id.clone();
    fx.service.delete(&id, vec!["local".into()]).unwrap();
    let cards = all(&fx);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].origins.len(), 1);
    assert_eq!(cards[0].origins[0].book, "mail:1");
}

#[test]
fn hidden_cards_are_excluded_unless_requested() {
    let mut store = base_store();
    store.emails.push(mail_from(1, "Bea", "bea@y.it"));
    let fx = fixture(store);
    let id = all(&fx)[0].id.clone();
    fx.service.hide(&id, true).unwrap();
    assert!(all(&fx).is_empty());
    let shown = fx.service.list(ContactQuery { include_hidden: true, limit: 10, ..Default::default() }).unwrap();
    assert_eq!(shown.items.len(), 1);
    assert!(shown.items[0].hidden);
    fx.service.hide(&id, false).unwrap();
    assert_eq!(all(&fx).len(), 1);
}

#[test]
fn sources_of_a_removed_account_leave_the_view_but_stay_in_the_file() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::google(9), Some("people/9"), fields("Fantasma", &["f@x.it"]), 1),
        source("c2", BookId::local(), None, fields("Presente", &["p@x.it"]), 1),
    ]);
    let names: Vec<String> = all(&fx).into_iter().map(|c| c.fields.name).collect();
    assert_eq!(names, vec!["Presente".to_string()]);
    assert_eq!(fx.contacts.read().unwrap().sources.len(), 2);
}

#[test]
fn books_lists_local_google_and_mail_books_per_account() {
    let fx = fixture(base_store());
    let books = fx.service.books().unwrap();
    let ids: Vec<String> = books.iter().map(|b| b.id.clone()).collect();
    for expected in ["local", "google:1", "google-other:1", "mail:1", "google:2", "mail:2", "mail:3"] {
        assert!(ids.contains(&expected.to_string()), "{expected} missing in {ids:?}");
    }
    assert!(!ids.contains(&"google:3".to_string()), "a sample account has no Google book");
    let local = books.iter().find(|b| b.id == "local").unwrap();
    assert!(local.writable && local.write_granted && local.sync_enabled);
    assert!(!books.iter().find(|b| b.id == "google-other:1").unwrap().writable);
}

#[test]
fn list_filters_by_book_and_text_and_paginates() {
    let fx = fixture(base_store());
    let mut sources = Vec::new();
    for n in 0..5 {
        sources.push(source(&format!("c{n}"), BookId::local(), None, fields(&format!("Persona {n}"), &[&format!("p{n}@x.it")]), 1));
    }
    sources.push(source("g", BookId::google(1), Some("people/g"), fields("Unico Google", &["u@g.it"]), 1));
    seed(&fx, sources);
    let page = fx.service.list(ContactQuery { limit: 2, ..Default::default() }).unwrap();
    assert_eq!((page.items.len(), page.total, page.next_offset), (2, 6, Some(2)));
    let google = fx.service.list(ContactQuery { book: Some("google:1".into()), limit: 10, ..Default::default() }).unwrap();
    assert_eq!(google.items.len(), 1);
    let text = fx.service.list(ContactQuery { text: "p3@X".into(), limit: 10, ..Default::default() }).unwrap();
    assert_eq!(text.items[0].fields.name, "Persona 3");
    assert_eq!(fx.service.list(ContactQuery { text: "zzz".into(), limit: 10, ..Default::default() }).unwrap().total, 0);
}

#[test]
fn similar_to_finds_cards_by_shared_email_or_same_name() {
    let fx = fixture(base_store());
    seed(&fx, vec![
        source("c1", BookId::local(), None, fields("Ada Lovelace", &["ada@x.it"]), 1),
        source("c2", BookId::local(), None, fields("Luca Bianchi", &["luca@x.it"]), 1),
    ]);
    assert_eq!(fx.service.similar_to(&fields("Qualcuno", &["ADA@x.it"])).unwrap().len(), 1);
    assert_eq!(fx.service.similar_to(&fields("lovelace ada", &[])).unwrap().len(), 1);
    assert!(fx.service.similar_to(&fields("Nessuno", &["n@x.it"])).unwrap().is_empty());
}

#[test]
fn suggest_matches_name_and_address_and_merges_origins() {
    let mut store = base_store();
    store.emails.push(mail_from(2, "ada", "ada@x.it"));
    let fx = fixture(store);
    seed(&fx, vec![source("c1", BookId::local(), None, fields("Ada Lovelace", &["ada@x.it", "ada@work.it"]), 1)]);
    let found = fx.service.suggest("lovel", 5).unwrap();
    assert!(
        found.iter().any(|s| s.address == "ada@x.it" && s.name == "Ada Lovelace" && s.books == vec!["local".to_string(), "mail:2".to_string()]),
        "{found:?}"
    );
    assert_eq!(fx.service.suggest("ada@w", 5).unwrap().len(), 1);
    assert!(fx.service.suggest("", 5).unwrap().is_empty());
    assert_eq!(fx.service.suggest("ada", 1).unwrap().len(), 1);
}

#[test]
fn search_suggestions_output_is_unchanged() {
    let mut store = base_store();
    let mut first = mail_from(1, "Maria Rossi", "maria@client.test");
    first.to = vec!["Alex <alex@work.test>".into(), "Zed <ZED@x.it>".into()];
    first.cc = vec!["plain@x.it".into()];
    store.emails.push(first);
    let fx = fixture(store);
    let people = MailService::new(fx.context.clone()).search_suggestions(None).unwrap();
    let seen: Vec<(String, String)> = people.into_iter().map(|p| (p.name, p.address)).collect();
    assert_eq!(
        seen,
        vec![
            ("Alex".to_string(), "alex@work.test".to_string()),
            ("Maria Rossi".to_string(), "maria@client.test".to_string()),
            (String::new(), "plain@x.it".to_string()),
            ("Zed".to_string(), "zed@x.it".to_string()),
        ]
    );
}
