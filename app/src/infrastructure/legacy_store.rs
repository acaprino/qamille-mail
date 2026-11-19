//! Compatibility entry points for the native UI and CLI.
use crate::domain::model::{Store, AccountKind};
use std::path::PathBuf;

impl Store {
    /// Brings in the workspace of the same copy installed before the rebranding.
    ///
    /// The data lives next to the executable, and the app's folder took the new name
    /// (`…\Sparklone` and `…\Qemille` became `…\Qamille`, `Qemille-Professional` became
    /// `Qamille-Professional`): without this the updated app would open empty. Runs once
    /// at startup, before anything reads the data folder. Returns the folder moved.
    pub fn adopt_previous_install() -> Option<PathBuf> {
        let explicit = ["QAMILLE_DATA", "QEMILLE_DATA", "SPARKLONE_DATA"]
            .iter()
            .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()));
        if explicit {
            return None;
        }
        let exe = std::env::current_exe().ok()?;
        adopt_previous_install_at(exe.parent()?)
    }

    pub fn data_dir() -> PathBuf {
        // QEMILLE_DATA and SPARKLONE_DATA: names used before the rebrandings, still set
        // by assistant integrations installed with older versions.
        let base = ["QAMILLE_DATA", "QEMILLE_DATA", "SPARKLONE_DATA"]
            .iter()
            .find_map(|name| std::env::var_os(name).filter(|path| !path.is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .expect("The executable location is required for portable storage")
                    .parent()
                    .expect("The executable must have a parent directory")
                    .join("data")
            });
        // Backend files and browser profiles must share the same path base.
        let base = std::path::absolute(base)
            .expect("The workspace location is required for portable storage");
        let _ = std::fs::create_dir_all(&base);
        base
    }
    pub fn path() -> PathBuf {
        Self::data_dir().join("store.json")
    }
    pub fn load_or_default() -> Store {
        let data_dir = Self::data_dir();
        if super::store_migration::sqlite_active(&data_dir) {
            // The SQLite engine is active: `store.json` is the pre-migration
            // backup and must not be served as current data. Any failure
            // falls back to an empty store, exactly like the damaged-JSON
            // path below — never to the stale JSON copy.
            let loaded = super::sqlite_store::SqliteStoreRepository::open(
                super::store_migration::sqlite_path(&data_dir),
            )
            .and_then(|repository| {
                use crate::application::ports::StoreRepository;
                repository.read()
            });
            let mut store = loaded.unwrap_or_default();
            store.load_secrets();
            return store;
        }
        let mut store = match std::fs::read_to_string(Self::path()) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(_) => {
                    // Keep the unreadable file before starting an empty workspace.
                    let _ = std::fs::rename(Self::path(), Self::path().with_extension("damaged.json"));
                    Store::default()
                }
            },
            Err(_) => Store::default(),
        };
        super::portable_files::resolve_store_paths(&mut store, &Self::data_dir())
            .expect("Portable data paths could not be read");
        store.load_secrets();
        store.adopt_old_signature();
        store
    }
    pub(crate) fn load_secrets(&mut self) {
        self.settings.ai_key = crate::secret::get(crate::secret::AI_KEY);
        self.settings.google_client_secret = crate::secret::get(crate::oauth::CLIENT_SECRET_KEY);
        for a in &mut self.accounts {
            if a.kind == AccountKind::Gmail {
                a.password = crate::secret::get(&crate::secret::account_key(&a.email));
            }
        }
    }
    pub fn save(&self) -> std::io::Result<()> {
        use crate::application::ports::StoreRepository;
        let data_dir = Self::data_dir();
        if super::store_migration::sqlite_active(&data_dir) {
            let repository = super::sqlite_store::SqliteStoreRepository::open(
                super::store_migration::sqlite_path(&data_dir),
            )
            .map_err(std::io::Error::other)?;
            let store = self.clone();
            return repository
                .transaction(&mut |current| {
                    // The closure may run again on a BUSY retry: it clones.
                    *current = store.clone();
                    Ok(())
                })
                .map_err(std::io::Error::other);
        }
        super::repository::JsonStoreRepository::new(Self::path())
            .transaction(&mut |current| { *current = self.clone(); Ok(()) })
            .map_err(std::io::Error::other)
    }
    pub fn mtime() -> Option<std::time::SystemTime> {
        std::fs::metadata(Self::path()).and_then(|m| m.modified()).ok()
    }
}

/// [`Store::adopt_previous_install`] for the app in `app_dir`.
pub fn adopt_previous_install_at(app_dir: &std::path::Path) -> Option<PathBuf> {
    let name = app_dir.file_name()?.to_str()?;
    let at = name.to_ascii_lowercase().find("qamille")?;
    let target = app_dir.join("data");
    // A workspace already in use is never replaced.
    if std::fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_some()) {
        return None;
    }
    // Most recent name first: Sparklone became Qemille, Qemille became Qamille.
    let (old_brand, old_dir) = ["Qemille", "Sparklone"].into_iter().find_map(|brand| {
        let old_dir = app_dir.parent()?.join(format!("{}{brand}{}", &name[..at], &name[at + "qamille".len()..]));
        old_dir.join("data").is_dir().then_some((brand, old_dir))
    })?;
    if target.exists() {
        std::fs::remove_dir(&target).ok()?;
    }
    // One rename on the same disk: all or nothing. If the old app is still open its
    // files are locked, the rename fails and the next start tries again.
    std::fs::rename(old_dir.join("data"), &target).ok()?;
    let _ = std::fs::write(
        old_dir.join("DATA-MOVED-TO-QAMILLE.txt"),
        format!(
            "{old_brand} is now called Qamille. This folder's data was moved to:
{}

Open Qamille from there; this folder can be removed.
",
            target.display()
        ),
    );
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::adopt_previous_install_at;
    use std::fs;

    fn root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("qamille-adopt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_workspace_of_the_old_folder_moves_into_the_renamed_app() {
        let root = root("move");
        fs::create_dir_all(root.join("Sparklone-Professional/data/webview")).unwrap();
        fs::write(root.join("Sparklone-Professional/data/store.json"), b"{}").unwrap();
        let app = root.join("Qamille-Professional");
        fs::create_dir_all(app.join("data")).unwrap();
        assert_eq!(adopt_previous_install_at(&app), Some(app.join("data")));
        assert_eq!(fs::read(app.join("data/store.json")).unwrap(), b"{}");
        assert!(app.join("data/webview").is_dir());
        assert!(!root.join("Sparklone-Professional/data").exists());
        assert!(root.join("Sparklone-Professional/DATA-MOVED-TO-QAMILLE.txt").exists());
        // Once done there is nothing left to bring in.
        assert_eq!(adopt_previous_install_at(&app), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_workspace_of_a_qemille_folder_moves_too() {
        let root = root("qemille");
        fs::create_dir_all(root.join("Qemille/data")).unwrap();
        fs::write(root.join("Qemille/data/store.json"), b"{}").unwrap();
        let app = root.join("Qamille");
        fs::create_dir_all(&app).unwrap();
        assert_eq!(adopt_previous_install_at(&app), Some(app.join("data")));
        assert_eq!(fs::read(app.join("data/store.json")).unwrap(), b"{}");
        assert!(fs::read_to_string(root.join("Qemille/DATA-MOVED-TO-QAMILLE.txt")).unwrap().starts_with("Qemille is now called Qamille."));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_workspace_in_use_or_an_unrelated_folder_is_left_alone() {
        let root = root("keep");
        fs::create_dir_all(root.join("Sparklone/data")).unwrap();
        fs::write(root.join("Sparklone/data/store.json"), b"old").unwrap();
        let app = root.join("Qamille");
        fs::create_dir_all(app.join("data")).unwrap();
        fs::write(app.join("data/store.json"), b"new").unwrap();
        assert_eq!(adopt_previous_install_at(&app), None);
        assert_eq!(fs::read(root.join("Sparklone/data/store.json")).unwrap(), b"old");
        // A development build (target\release) has no old sibling to adopt.
        assert_eq!(adopt_previous_install_at(&root.join("release")), None);
        fs::remove_dir_all(root).unwrap();
    }
}
