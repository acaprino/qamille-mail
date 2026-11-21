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
    save_at(config, &Store::data_dir(), None)
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

/// Settings and the OS launcher are one operation: rejected settings have no OS effects.
pub fn save_with_autostart(config: &Config) -> AppResult<()> {
    validate(config)?;
    #[cfg(windows)]
    {
        let launcher = startup_launcher()?;
        let content = if config.worker_autostart {
            Some(launcher_content(&std::env::current_exe()?))
        } else {
            None
        };
        save_at(
            config,
            &Store::data_dir(),
            Some((&launcher, content.as_deref())),
        )
    }
    #[cfg(not(windows))]
    {
        if config.worker_autostart {
            return Err(AppError::new(
                "unsupported_capability",
                "Automatic startup is configured by the operating system on this platform",
            ));
        }
        save(config)
    }
}

fn save_at(
    config: &Config,
    root: &std::path::Path,
    startup: Option<(&std::path::Path, Option<&[u8]>)>,
) -> AppResult<()> {
    validate(config)?;
    std::fs::create_dir_all(&root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("mcp-config.lock"))?;
    lock.lock()?;
    let temp = root.join(format!("mcp-config.{}.tmp", std::process::id()));
    let result = (|| -> AppResult<()> {
        let mut file = File::create(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(config)?)?;
        file.sync_all()?;
        drop(file);
        let previous = startup.map(|(path, _)| read_optional(path)).transpose()?;
        let publish = (|| -> AppResult<()> {
            if let Some((path, bytes)) = startup {
                write_optional(path, bytes)?;
            }
            std::fs::rename(&temp, root.join("mcp-config.json"))?;
            Ok(())
        })();
        if let Err(error) = publish {
            if let (Some((path, _)), Some(previous)) = (startup, previous) {
                if let Err(restore) = write_optional(path, previous.as_deref()) {
                    return Err(AppError::storage(format!(
                        "Settings were not saved. Restoring automatic startup failed: {}. Original error: {}",
                        restore.message, error.message
                    )));
                }
            }
            return Err(error);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn read_optional(path: &std::path::Path) -> AppResult<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn write_optional(path: &std::path::Path, bytes: Option<&[u8]>) -> AppResult<()> {
    if let Some(bytes) = bytes {
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| AppError::storage("Startup file has no parent"))?,
        )?;
        std::fs::write(path, bytes)?;
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn startup_launcher() -> AppResult<std::path::PathBuf> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| AppError::storage("Windows startup folder is unavailable"))?;
    Ok(std::path::PathBuf::from(appdata)
        .join("Microsoft/Windows/Start Menu/Programs/Startup/Qamille-worker.vbs"))
}
#[cfg(windows)]
fn launcher_content(executable: &std::path::Path) -> Vec<u8> {
    let command = format!("\"{}\" --worker", executable.display()).replace('"', "\"\"");
    format!("Set shell = CreateObject(\"WScript.Shell\")\r\nshell.Run \"{command}\", 0, False\r\n")
        .into_bytes()
}
/// What the startup folder should get when the app starts, or `None` to leave it alone. A launcher that
/// is missing or names another executable is written again, but only while automatic startup is on:
/// when it is off nothing is written, and nothing is removed (saving the settings does that).
#[cfg(windows)]
fn launcher_refresh(
    autostart: bool,
    existing: Option<&[u8]>,
    executable: &std::path::Path,
) -> Option<Vec<u8>> {
    if !autostart {
        return None;
    }
    let wanted = launcher_content(executable);
    (existing != Some(wanted.as_slice())).then_some(wanted)
}
/// `refresh_autostart` on a given data folder, startup file and executable. Returns whether the
/// launcher was written. With automatic startup off it touches nothing, not even the lock file.
#[cfg(windows)]
fn refresh_autostart_at(
    root: &std::path::Path,
    launcher: &std::path::Path,
    executable: &std::path::Path,
) -> AppResult<bool> {
    if !load_at(root)?.worker_autostart {
        return Ok(false);
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("mcp-config.lock"))?;
    lock.lock()?;
    // Settings may have been saved, and the launcher removed, while this waited for the lock.
    let autostart = load_at(root)?.worker_autostart;
    let existing = read_optional(launcher)?;
    match launcher_refresh(autostart, existing.as_deref(), executable) {
        Some(bytes) => {
            write_optional(launcher, Some(&bytes))?;
            Ok(true)
        }
        None => Ok(false),
    }
}
/// Called once when the app starts. The uninstaller that an upgrade runs first removes the startup
/// launcher, and moving the app's folder leaves it naming the old one: with automatic startup on, put
/// it back so the worker still starts at the next sign-in. It is never written when startup is off.
pub fn refresh_autostart() -> AppResult<()> {
    #[cfg(windows)]
    refresh_autostart_at(
        &Store::data_dir(),
        &startup_launcher()?,
        &std::env::current_exe()?,
    )?;
    Ok(())
}
pub fn set_autostart(enabled: bool) -> AppResult<()> {
    #[cfg(windows)]
    {
        let launcher = startup_launcher()?;
        if enabled {
            write_optional(
                &launcher,
                Some(&launcher_content(&std::env::current_exe()?)),
            )?;
        } else {
            write_optional(&launcher, None)?;
        }
    }
    #[cfg(not(windows))]
    if enabled {
        return Err(AppError::new(
            "unsupported_capability",
            "Automatic startup is configured by the operating system on this platform",
        ));
    }
    Ok(())
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
    fn rejected_profile_never_creates_or_changes_startup() {
        let f = Fixture::new();
        let launcher = f.0.join("startup/worker.vbs");
        let mut config = Config {
            worker_autostart: true,
            ..Config::default()
        };
        config
            .profiles
            .insert("Claude Code".into(), Profile::default());
        assert!(save_at(&config, &f.0.join("data"), Some((&launcher, Some(b"new")))).is_err());
        assert!(!launcher.exists());
        assert!(!f.0.join("data").exists());
    }

    #[test]
    fn failed_config_publication_restores_exact_startup_state() {
        for previous in [None, Some(b"previous launcher".as_slice())] {
            for next in [None, Some(b"new launcher".as_slice())] {
                let f = Fixture::new();
                let launcher = f.0.join("startup/worker.vbs");
                write_optional(&launcher, previous).unwrap();
                // A directory cannot be replaced by the staged settings file.
                std::fs::create_dir_all(f.0.join("data/mcp-config.json")).unwrap();
                assert!(
                    save_at(
                        &Config::default(),
                        &f.0.join("data"),
                        Some((&launcher, next))
                    )
                    .is_err()
                );
                assert_eq!(read_optional(&launcher).unwrap().as_deref(), previous);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn launcher_refresh_decides_from_the_setting_the_file_and_the_executable() {
        let here = std::path::Path::new(r"C:\Apps\Qamille\qamille-desktop.exe");
        let there = std::path::Path::new(r"D:\Portable\Qamille.exe");
        let mine = launcher_content(here);
        let theirs = launcher_content(there);
        let garbage = b"not a launcher".as_slice();
        let cases: [(bool, Option<&[u8]>, Option<Vec<u8>>); 9] = [
            // Off: never written, whatever is there.
            (false, None, None),
            (false, Some(&mine), None),
            (false, Some(&theirs), None),
            (false, Some(garbage), None),
            // On: written when missing, empty or naming another executable; kept when it is this one's.
            (true, None, Some(mine.clone())),
            (true, Some(&mine), None),
            (true, Some(&theirs), Some(mine.clone())),
            (true, Some(garbage), Some(mine.clone())),
            (true, Some(b""), Some(mine.clone())),
        ];
        for (autostart, existing, expected) in cases {
            assert_eq!(
                launcher_refresh(autostart, existing, here),
                expected,
                "autostart={autostart} existing={:?}",
                existing.map(String::from_utf8_lossy)
            );
        }
        assert!(String::from_utf8(mine).unwrap().contains("--worker"));
    }

    #[cfg(windows)]
    #[test]
    fn refresh_autostart_puts_the_launcher_back_only_while_startup_is_on() {
        let f = Fixture::new();
        let data = f.0.join("data");
        let launcher = f.0.join("startup/worker.vbs");
        let here = std::path::Path::new(r"C:\Apps\Qamille\qamille-desktop.exe");
        let moved = std::path::Path::new(r"D:\Moved\qamille-desktop.exe");
        let save = |autostart| {
            let config = Config {
                worker_autostart: autostart,
                ..Config::default()
            };
            save_at(&config, &data, None).unwrap();
        };

        // No settings file: nothing to restore, and not even the lock file is created.
        assert!(!refresh_autostart_at(&data, &launcher, here).unwrap());
        assert!(!launcher.exists());
        assert!(!data.join("mcp-config.lock").exists());

        // Off: a missing launcher stays missing and a present one is not touched.
        save(false);
        assert!(!refresh_autostart_at(&data, &launcher, here).unwrap());
        assert!(!launcher.exists());
        write_optional(&launcher, Some(b"left alone")).unwrap();
        assert!(!refresh_autostart_at(&data, &launcher, here).unwrap());
        assert_eq!(std::fs::read(&launcher).unwrap(), b"left alone");

        // On: an upgrade removed it, so it is written; the next start finds it right and keeps it;
        // after the app's folder is moved it is written again for the new executable.
        save(true);
        write_optional(&launcher, None).unwrap();
        assert!(refresh_autostart_at(&data, &launcher, here).unwrap());
        assert_eq!(std::fs::read(&launcher).unwrap(), launcher_content(here));
        assert!(!refresh_autostart_at(&data, &launcher, here).unwrap());
        assert!(refresh_autostart_at(&data, &launcher, moved).unwrap());
        assert_eq!(std::fs::read(&launcher).unwrap(), launcher_content(moved));
    }

    #[cfg(windows)]
    #[test]
    fn refresh_autostart_leaves_the_launcher_alone_when_the_settings_are_unreadable() {
        let f = Fixture::new();
        let data = f.0.join("data");
        let launcher = f.0.join("startup/worker.vbs");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("mcp-config.json"), b"{ not json").unwrap();
        write_optional(&launcher, Some(b"previous")).unwrap();
        let here = std::path::Path::new(r"C:\Apps\Qamille\qamille-desktop.exe");
        assert!(refresh_autostart_at(&data, &launcher, here).is_err());
        assert_eq!(std::fs::read(&launcher).unwrap(), b"previous");
    }

    #[test]
    fn successful_settings_save_applies_startup() {
        let f = Fixture::new();
        let launcher = f.0.join("startup/worker.vbs");
        let config = Config {
            worker_autostart: true,
            ..Config::default()
        };
        save_at(
            &config,
            &f.0.join("data"),
            Some((&launcher, Some(b"launcher"))),
        )
        .unwrap();
        assert_eq!(std::fs::read(launcher).unwrap(), b"launcher");
        let saved: Config =
            serde_json::from_slice(&std::fs::read(f.0.join("data/mcp-config.json")).unwrap())
                .unwrap();
        assert!(saved.worker_autostart);
    }
}
