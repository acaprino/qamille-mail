#[path = "support/sync_fakes.rs"]
mod fakes;

use fakes::{App, FakeSync};
use qamille::application::ports::SecretStore;
use qamille::domain::model::Store;
use std::sync::Arc;

#[test]
fn new_mailboxes_do_not_inherit_the_removed_mailbox_or_its_google_access() {
    let app = App::new(&Arc::new(FakeSync::default()));
    app.account_service.connect_password("A", "a@example.com", "pw-a").unwrap();
    let id = app.store().accounts[0].id;
    for scope in ["contacts", "calendar"] {
        app.vault.set(&format!("{scope}:{id}"), "old-grant").unwrap();
    }
    app.account_service.remove_account(id).unwrap();
    let serialized = serde_json::to_string(&app.store()).unwrap();
    let mut restarted: Store = serde_json::from_str(&serialized).unwrap();
    assert!(restarted.allocate_mailbox_id().unwrap() > id);
    app.account_service.connect_password("B", "b@example.com", "pw-b").unwrap();
    assert!(app.store().accounts[0].id > id);
    assert!(app.token("a@example.com").is_empty());
    for scope in ["contacts", "calendar"] {
        assert!(app.vault.get(&format!("{scope}:{id}")).unwrap().is_empty());
    }
}

#[test]
fn legacy_workspaces_stay_clear_of_the_old_grant_namespace() {
    let mut stored = serde_json::to_value(Store::default()).unwrap();
    stored["settings"].as_object_mut().unwrap().remove("mailbox_id_high_water");
    let mut legacy: Store = serde_json::from_value(stored).unwrap();
    let id = legacy.allocate_mailbox_id().unwrap();
    assert!(id > i32::MAX as u32);
    let mut restarted: Store = serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(restarted.allocate_mailbox_id(), id.checked_add(1));
    restarted.settings.mailbox_id_high_water = Some(u32::MAX - 1);
    assert!(restarted.allocate_mailbox_id().is_none());
    assert_eq!(restarted.settings.mailbox_id_high_water, Some(u32::MAX - 1));
    restarted.settings.mailbox_id_high_water = Some(u32::MAX);
    assert!(restarted.allocate_mailbox_id().is_none());
}

#[test]
fn cleanup_resumes_after_reconnection_without_removing_the_new_credential() {
    let app = App::new(&Arc::new(FakeSync::default()));
    app.account_service.connect_password("Original", "same@example.com", "old").unwrap();
    let id = app.store().accounts[0].id;
    let interrupted = app.account_service.clone().with_grant_cleanup(|_, detach| {
        detach()?;
        Err(qamille::application::AppError::storage("injected sidecar write failure"))
    });
    interrupted.remove_account(id).unwrap();
    assert!(app.store().settings.pending_mailbox_cleanup.contains_key(&id));
    app.account_service.connect_password("Reconnected", "same@example.com", "new").unwrap();
    assert_ne!(app.store().accounts[0].id, id);
    app.account_service.retry_mailbox_cleanup().unwrap();
    assert!(app.store().settings.pending_mailbox_cleanup.is_empty());
    assert_eq!(app.token("same@example.com"), "new");
}
