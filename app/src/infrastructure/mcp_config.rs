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
    match std::fs::read(Store::data_dir().join("mcp-config.json")) {
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
