//! Local MCP grants. Only trusted desktop settings may write this file.
use crate::{
    application::{AppError, AppResult},
    model::{Access, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub access: Access,
    pub account_ids: Option<Vec<u32>>,
    pub calendar_ids: Option<Vec<String>>,
    pub ai_execute: bool,
    pub permanent_delete: bool,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            access: Access::ReadOnly,
            account_ids: None,
            calendar_ids: None,
            ai_execute: false,
            permanent_delete: false,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub profiles: BTreeMap<String, Profile>,
    pub worker_autostart: bool,
}
/// Authorization reads the same engine as desktop and the standalone worker.
/// The JSON backup remains in place after SQLite activation.
pub(crate) fn active_workspace(root: &std::path::Path) -> AppResult<Store> {
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(root.join("store.lock"))?;
    lock.lock_shared()?;
    super::store_migration::read_active_store(root)
}

pub fn rank(access: Access) -> u8 {
    match access {
        Access::ReadOnly => 0,
        Access::Triage => 1,
        Access::Send => 2,
    }
}
pub fn load() -> AppResult<Config> {
    load_at(&Store::data_dir())
}
fn load_at(root: &std::path::Path) -> AppResult<Config> {
    match std::fs::read(root.join("mcp-config.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            AppError::storage("MCP permissions file is invalid; repair it in Settings")
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn profile(name: &str, maximum: Access) -> AppResult<Profile> {
    let config = load()?;
    let mut profile = match config.profiles.get(name) {
        Some(profile) => profile.clone(),
        None if name == "default" => Profile {
            access: maximum,
            ..Profile::default()
        },
        None => {
            return Err(AppError::new(
                "permission_denied",
                "Unknown MCP profile; configure it in Settings",
            ));
        }
    };
    if rank(profile.access) > rank(maximum) {
        profile.access = maximum;
    }
    Ok(profile)
}
pub fn save(config: &Config) -> AppResult<()> {
    save_at(config, &Store::data_dir())
}

fn validate(config: &Config) -> AppResult<()> {
    for (name, profile) in &config.profiles {
        if name.is_empty()
            || name.len() > 80
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(AppError::validation(
                "Profile names use letters, digits, underscore or hyphen",
            ));
        }
        if profile
            .account_ids
            .as_ref()
            .is_some_and(|ids| ids.len() > 100)
        {
            return Err(AppError::validation("Too many account grants"));
        }
    }
    Ok(())
}

/// The settings of Settings → Assistants. Nothing else changes with them: start at sign-in is
/// Qamille itself in the tray (`background.rs` in the desktop app), and `worker_autostart` is only
/// read so that a settings file of an earlier version still loads. Rejected settings write nothing.
pub fn save_with_autostart(config: &Config) -> AppResult<()> {
    validate(config)?;
    save(config)
}

pub(super) fn revoke_mailbox(root: &std::path::Path, id: u32) -> AppResult<()> {
    let _lock = config_lock(root)?;
    let mut config = load_at(root)?;
    let mut changed = false;
    for profile in config.profiles.values_mut() {
        if let Some(ids) = &mut profile.account_ids {
            let count = ids.len();
            ids.retain(|granted| *granted != id);
            changed |= count != ids.len();
        }
    }
    if changed { publish(&config, root)?; }
    Ok(())
}

fn save_at(config: &Config, root: &std::path::Path) -> AppResult<()> {
    validate(config)?;
    let _lock = config_lock(root)?;
    publish(config, root)
}
fn config_lock(root: &std::path::Path) -> AppResult<File> {
    std::fs::create_dir_all(&root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("mcp-config.lock"))?;
    lock.lock()?;
    Ok(lock)
}
fn publish(config: &Config, root: &std::path::Path) -> AppResult<()> {
    let temp = root.join(format!("mcp-config.{}.tmp", std::process::id()));
    let result = (|| -> AppResult<()> {
        let mut file = File::create(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(config)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, root.join("mcp-config.json"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Where earlier versions wrote the file that started the background worker at sign-in (the
/// installer's uninstall hook deletes the same path). Only read now, to migrate it.
#[cfg(windows)]
fn startup_launcher() -> AppResult<std::path::PathBuf> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| AppError::storage("Windows startup folder is unavailable"))?;
    Ok(std::path::PathBuf::from(appdata)
        .join("Microsoft/Windows/Start Menu/Programs/Startup/Qamille-worker.vbs"))
}
/// The worker's startup file of an earlier version, with its text, if there is one. Bytes that are
/// not text still count: the file is Qamille's, and it can only be removed.
pub fn legacy_launcher() -> AppResult<Option<(std::path::PathBuf, String)>> {
    #[cfg(windows)]
    {
        let path = startup_launcher()?;
        return Ok(legacy_launcher_at(&path)?.map(|content| (path, content)));
    }
    #[cfg(not(windows))]
    Ok(None)
}
/// Deletes the worker's startup file of an earlier version; one already gone is not an error.
pub fn remove_legacy_launcher() -> AppResult<()> {
    #[cfg(windows)]
    remove_legacy_launcher_at(&startup_launcher()?)?;
    Ok(())
}
fn legacy_launcher_at(path: &std::path::Path) -> AppResult<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn remove_legacy_launcher_at(path: &std::path::Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "qamille-settings-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn authorization_reads_current_accounts_and_access_after_sqlite_activation() {
        use crate::application::ports::StoreRepository;
        use crate::model::Account;

        let f = Fixture::new();
        let mut backup = Store::default();
        backup.settings.mcp_access = Access::Send;
        std::fs::write(f.0.join("store.json"), serde_json::to_vec(&backup).unwrap()).unwrap();
        crate::infrastructure::store_migration::activate_sqlite(&f.0).unwrap();
        let sqlite = crate::infrastructure::sqlite_store::SqliteStoreRepository::open(
            crate::infrastructure::store_migration::sqlite_path(&f.0)
        ).unwrap();
        sqlite.transaction(&mut |store| {
            store.settings.mcp_access = Access::ReadOnly;
            store.accounts.push(Account { id: 4, ..Default::default() });
            Ok(())
        }).unwrap();
        let current = active_workspace(&f.0).unwrap();
        assert!(current.settings.mcp_access == Access::ReadOnly);
        assert!(current.account(4).is_some());
    }

    #[test]
    fn a_rejected_profile_writes_nothing() {
        let f = Fixture::new();
        let mut config = Config::default();
        config
            .profiles
            .insert("Claude Code".into(), Profile::default());
        assert!(save_at(&config, &f.0.join("data")).is_err());
        assert!(!f.0.join("data").exists());
    }

    #[test]
    fn saving_settings_never_writes_a_startup_file() {
        // Start at sign-in is Qamille in the tray now (the Run entry of `background.rs`): the old
        // flag is still read, so a settings file of an earlier version loads, but it starts nothing.
        let f = Fixture::new();
        let data = f.0.join("data");
        let config = Config {
            worker_autostart: true,
            ..Config::default()
        };
        save_at(&config, &data).unwrap();
        let mut written: Vec<String> = std::fs::read_dir(&f.0)
            .unwrap()
            .chain(std::fs::read_dir(&data).unwrap())
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        written.sort();
        assert_eq!(written, ["data", "mcp-config.json", "mcp-config.lock"]);
        assert!(load_at(&data).unwrap().worker_autostart);
    }

    #[test]
    fn legacy_launcher_reads_the_old_file_and_removes_it() {
        let f = Fixture::new();
        let launcher = f.0.join("Startup/Qamille-worker.vbs");
        assert_eq!(legacy_launcher_at(&launcher).unwrap(), None);
        // Removing a file that is not there is not an error: two copies may race for it.
        remove_legacy_launcher_at(&launcher).unwrap();
        std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
        let content = "Set shell = CreateObject(\"WScript.Shell\")\r\nshell.Run \"\"\"C:\\Apps\\Qamille.exe\"\" --worker\", 0, False\r\n";
        std::fs::write(&launcher, content).unwrap();
        assert_eq!(legacy_launcher_at(&launcher).unwrap().as_deref(), Some(content));
        remove_legacy_launcher_at(&launcher).unwrap();
        assert!(!launcher.exists());
        assert_eq!(legacy_launcher_at(&launcher).unwrap(), None);
        // A file that cannot be read as text is still reported, so it can be removed.
        std::fs::write(&launcher, [0xff, 0xfe, 0x00]).unwrap();
        assert!(legacy_launcher_at(&launcher).unwrap().is_some());
    }
}
