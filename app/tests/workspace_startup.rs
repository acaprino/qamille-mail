use qamille::{
    application::ports::StoreRepository,
    infrastructure::repository::JsonStoreRepository,
    model::{Store, sample_store},
};
use std::path::PathBuf;

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "qamille-workspace-startup-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0.join("store.json")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.starts_with(temp));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qamille-workspace-startup-")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn assert_empty(store: &Store) {
    assert!(
        store.accounts.is_empty(),
        "Normal startup must not create demo accounts"
    );
    assert!(store.emails.is_empty());
    assert!(store.drafts.is_empty());
    assert!(store.events.is_empty());
    assert!(store.meetings.is_empty());
    assert!(store.signatures.is_empty());
    assert_eq!(store.settings.user_name, "You");
}

#[test]
fn first_start_persists_an_empty_workspace_across_restarts() {
    let workspace = Workspace::new();
    let repository = JsonStoreRepository::new(workspace.path());
    assert_empty(&repository.read().unwrap());
    assert!(workspace.path().is_file());
    assert_empty(&JsonStoreRepository::new(workspace.path()).read().unwrap());
}

#[test]
fn first_transaction_does_not_inject_demo_data() {
    let workspace = Workspace::new();
    let repository = JsonStoreRepository::new(workspace.path());
    repository
        .transaction(&mut |store| {
            assert_empty(store);
            store.settings.user_name = "Workspace owner".into();
            Ok(())
        })
        .unwrap();
    let saved = repository.read().unwrap();
    assert!(saved.accounts.is_empty());
    assert!(saved.emails.is_empty());
    assert_eq!(saved.settings.user_name, "Workspace owner");
}

#[test]
fn existing_saved_workspaces_are_preserved_including_old_demo_data() {
    for store in [Store::default(), sample_store()] {
        let workspace = Workspace::new();
        let bytes = serde_json::to_vec(&store).unwrap();
        std::fs::write(workspace.path(), &bytes).unwrap();
        let loaded = JsonStoreRepository::new(workspace.path()).read().unwrap();
        assert_eq!(
            serde_json::to_value(loaded).unwrap(),
            serde_json::to_value(store).unwrap()
        );
        assert_eq!(std::fs::read(workspace.path()).unwrap(), bytes);
    }
}

#[test]
fn unreadable_data_returns_an_error_without_injecting_demo_data() {
    let workspace = Workspace::new();
    let bytes = b"{broken original data";
    std::fs::write(workspace.path(), bytes).unwrap();
    assert!(JsonStoreRepository::new(workspace.path()).read().is_err());
    assert_eq!(std::fs::read(workspace.path()).unwrap(), bytes);
}

#[test]
fn explicit_demo_seeds_only_new_workspaces() {
    let workspace = Workspace::new();
    let demo = JsonStoreRepository::demo(workspace.path());
    let store = demo.read().unwrap();
    assert_eq!(store.accounts.len(), 2);
    assert!(!store.emails.is_empty());
    let persisted = std::fs::read(workspace.path()).unwrap();
    let reopened = JsonStoreRepository::new(workspace.path()).read().unwrap();
    assert_eq!(
        serde_json::to_value(reopened).unwrap(),
        serde_json::to_value(store).unwrap()
    );
    assert_eq!(std::fs::read(workspace.path()).unwrap(), persisted);

    let empty_workspace = Workspace::new();
    JsonStoreRepository::new(empty_workspace.path())
        .read()
        .unwrap();
    assert_empty(
        &JsonStoreRepository::demo(empty_workspace.path())
            .read()
            .unwrap(),
    );
}

#[test]
fn legacy_startup_does_not_create_demo_data() {
    let workspace = Workspace::new();
    let previous = std::env::var_os("QAMILLE_DATA");
    // This test binary has one environment-dependent test; all others use explicit paths.
    unsafe { std::env::set_var("QAMILLE_DATA", &workspace.0) };
    let store = Store::load_or_default();
    unsafe {
        match previous {
            Some(value) => std::env::set_var("QAMILLE_DATA", value),
            None => std::env::remove_var("QAMILLE_DATA"),
        }
    }
    assert_empty(&store);
}
