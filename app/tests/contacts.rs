use qamille::{
    application::{AppError, contact_ports::ContactStore},
    domain::contacts::*,
    infrastructure::contact_repository::{FileContactStore, MemoryContactStore},
};
use std::sync::Arc;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("qamille-contacts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn card(key: &str) -> SourceContact {
    SourceContact { key: key.into(), book: BookId::local(), fields: ContactFields { name: key.into(), ..Default::default() }, ..Default::default() }
}

#[test]
fn file_store_round_trips_state() {
    let dir = temp_dir("roundtrip");
    let store = FileContactStore::new(dir.clone());
    assert_eq!(store.read().unwrap(), ContactsState::default());
    store
        .transaction(&mut |s| {
            s.sources.push(card("a"));
            s.hidden.push("a".into());
            s.settings.last_book = Some("local".into());
            let id = s.new_id();
            assert_eq!(id, 1);
            Ok(())
        })
        .unwrap();
    let reopened = FileContactStore::new(dir.clone()).read().unwrap();
    assert_eq!(reopened.sources.len(), 1);
    assert_eq!(reopened.hidden, vec!["a".to_string()]);
    assert_eq!(reopened.settings.last_book.as_deref(), Some("local"));
    assert_eq!(reopened.next_id, 1);
    assert!(dir.join("contacts.json").exists());
}

#[test]
fn failed_transaction_changes_nothing() {
    let dir = temp_dir("rollback");
    let stores: Vec<Box<dyn ContactStore>> = vec![Box::new(FileContactStore::new(dir)), Box::new(MemoryContactStore::default())];
    for store in stores {
        store
            .transaction(&mut |s| {
                s.sources.push(card("kept"));
                Ok(())
            })
            .unwrap();
        let failed = store.transaction(&mut |s| {
            s.sources.push(card("lost"));
            Err(AppError::validation("no"))
        });
        assert!(failed.is_err());
        let keys: Vec<String> = store.read().unwrap().sources.into_iter().map(|c| c.key).collect();
        assert_eq!(keys, vec!["kept".to_string()]);
    }
}

#[test]
fn damaged_file_reports_contacts_damaged_and_is_left_untouched() {
    let dir = temp_dir("damaged");
    let path = dir.join("contacts.json");
    std::fs::write(&path, b"{ not json").unwrap();
    let store = FileContactStore::new(dir);
    assert_eq!(store.read().unwrap_err().code, "contacts_damaged");
    let write = store.transaction(&mut |s| {
        s.sources.push(card("x"));
        Ok(())
    });
    assert_eq!(write.unwrap_err().code, "contacts_damaged");
    assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
}

#[test]
fn two_threads_serialize_through_the_lock() {
    let dir = temp_dir("threads");
    let store = Arc::new(FileContactStore::new(dir));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    store
                        .transaction(&mut |s| {
                            s.next_id += 1;
                            Ok(())
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(store.read().unwrap().next_id, 200);
}

#[test]
fn state_without_new_fields_loads_with_defaults() {
    let dir = temp_dir("defaults");
    std::fs::write(dir.join("contacts.json"), br#"{"sources":[{"key":"k","book":"local","fields":{"name":"Ada"}}]}"#).unwrap();
    let state = FileContactStore::new(dir).read().unwrap();
    assert_eq!(state.sources.len(), 1);
    assert!(state.links.is_empty() && state.queue.is_empty() && state.journal.is_empty());
    assert_eq!(state.settings, ContactSettings::default());
}

#[test]
fn the_flush_lock_serializes_two_holders_even_across_service_instances() {
    let dir = temp_dir("flushlock");
    let store = Arc::new(FileContactStore::new(dir));
    let first = store.lock_flush().unwrap();
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (other, flag) = (store.clone(), entered.clone());
    let worker = std::thread::spawn(move || {
        let _second = other.lock_flush().unwrap();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!entered.load(std::sync::atomic::Ordering::SeqCst), "the second holder must wait");
    drop(first);
    worker.join().unwrap();
    assert!(entered.load(std::sync::atomic::Ordering::SeqCst));
    assert!(MemoryContactStore::default().lock_flush().is_ok());
}

