//! Qamille as the app Windows opens for mailto: links (docs/aspetto-nativo.md, rule 32, audit Q10).
//! Windows lets no program make itself the default mail app: the person chooses, in Settings > Default
//! apps. Qamille says what it can open, for the current user only, and opens that page on its entry:
//! - a ProgID under `Software\Classes` whose command is `"<exe>" "%1"` (`launch_args.rs` takes the link);
//! - its Capabilities under Qamille's own key, with `URLAssociations\mailto` naming the ProgID;
//! - a value under `Software\RegisteredApplications` that points to them, the name
//!   `ms-settings:defaultapps?registeredAppUser=<name>` opens.
//!
//! It never writes the key of the mailto: scheme itself, nor the choice (`UserChoice`, which Windows
//! signs).
//!
//! The installed Professional gets its keys from its installer (`NSIS_HOOK_POSTINSTALL` in
//! windows/installer-hooks.nsh), with the fixed names of `handler_keys(true, …)`, and loses them when
//! it is uninstalled; native-feel check 26 keeps the hooks and `entries` in step. A portable copy
//! writes its own from Settings → General → Windows («Usa Qamille per i link mailto»), named after its
//! folder and the tag of its data folder, so two copies never share a key; «Smetti» removes them.
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

use crate::background_rules;
use qamille::application::{AppError, AppResult};
use serde::Serialize;
use std::sync::OnceLock;
use tauri::AppHandle;

/// The ProgIDs of the current user.
pub const CLASSES: &str = r"Software\Classes";
/// Where Settings > Default apps finds the apps a person can choose.
pub const REGISTERED: &str = r"Software\RegisteredApplications";
/// Qamille's own key: one key per copy under it, holding the copy's Capabilities.
pub const APPS: &str = r"Software\Qamille\MailLinks";
/// Where Windows keeps the person's choice for mailto: links. Only read.
pub const CHOICE: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\mailto";

/// The installed Professional's names, the ones its installer writes.
pub const INSTALLED_NAME: &str = "Qamille";
pub const INSTALLED_PROG_ID: &str = "Qamille.Url.mailto";
pub const INSTALLED_KEY: &str = "Installed";

/// The keys, under `HKEY_CURRENT_USER`. Parameters, so that the tests use a scratch key of their own.
pub struct Hive<'a> {
    pub classes: &'a str,
    pub registered: &'a str,
    pub apps: &'a str,
    pub choice: &'a str,
}
/// Windows' own keys.
pub const WINDOWS: Hive<'static> = Hive { classes: CLASSES, registered: REGISTERED, apps: APPS, choice: CHOICE };

/// One copy's names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerKeys {
    /// The value under `REGISTERED`, which `settings_page` names: unique, and plain in an address.
    pub name: String,
    /// The name Settings shows.
    pub display: String,
    pub prog_id: String,
    /// The copy's key under `APPS`.
    pub key: String,
}

/// The installed Professional has fixed names, which its installer writes and removes; a portable copy
/// names its folder, for the person, and the tag of its data folder (`background_rules::workspace_tag`),
/// so that two copies, even in folders of the same name, never share a key.
pub fn handler_keys(installed: bool, folder: &str, tag: &str) -> HandlerKeys {
    if installed {
        HandlerKeys {
            name: INSTALLED_NAME.into(),
            display: INSTALLED_NAME.into(),
            prog_id: INSTALLED_PROG_ID.into(),
            key: INSTALLED_KEY.into(),
        }
    } else {
        HandlerKeys {
            name: format!("{INSTALLED_NAME}-{tag}"),
            display: format!("{INSTALLED_NAME} ({folder})"),
            prog_id: format!("{INSTALLED_PROG_ID}.{tag}"),
            key: tag.to_owned(),
        }
    }
}

/// The page of Settings > Default apps where the person chooses this copy.
pub fn settings_page(keys: &HandlerKeys) -> String {
    format!("ms-settings:defaultapps?registeredAppUser={}", keys.name)
}

fn capabilities_key(hive: &Hive, keys: &HandlerKeys) -> String {
    format!(r"{}\{}\Capabilities", hive.apps, keys.key)
}

/// The path Windows runs: a verbatim path (`\\?\`) written plainly.
fn plain_path(exe: &str) -> String {
    match exe.strip_prefix(r"\\?\") {
        Some(rest) => rest.strip_prefix(r"UNC\").map_or_else(|| rest.to_owned(), |share| format!(r"\\{share}")),
        None => exe.to_owned(),
    }
}

/// Every value a copy writes, as (key, value name, text), in the order they are written; an empty name
/// is the key's default value. The command has the executable and the link in quotes: unquoted, a path
/// with spaces would be tried prefix by prefix, and a link with spaces would arrive in pieces.
pub fn entries(hive: &Hive, keys: &HandlerKeys, exe: &str) -> Vec<(String, String, String)> {
    let exe = plain_path(exe);
    let command = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    let prog = format!(r"{}\{}", hive.classes, keys.prog_id);
    let capabilities = capabilities_key(hive, keys);
    vec![
        (prog.clone(), String::new(), keys.display.clone()),
        (format!(r"{prog}\DefaultIcon"), String::new(), icon.clone()),
        (format!(r"{prog}\shell\open\command"), String::new(), command),
        (capabilities.clone(), "ApplicationName".into(), keys.display.clone()),
        (capabilities.clone(), "ApplicationDescription".into(), keys.display.clone()),
        (capabilities.clone(), "ApplicationIcon".into(), icon),
        (format!(r"{capabilities}\URLAssociations"), "mailto".into(), keys.prog_id.clone()),
        // Last: Settings lists the copy only once everything it points to is there.
        (hive.registered.to_owned(), keys.name.clone(), capabilities),
    ]
}

/// Writes the copy's keys (`entries`), creating what is missing. Twice is the same.
pub fn register(hive: &Hive, keys: &HandlerKeys, exe: &str) -> Result<(), String> {
    for (key, name, value) in entries(hive, keys, exe) {
        registry::write(&key, &name, &value).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Removes the copy's keys, as the uninstaller does for the installed Professional; twice is fine.
/// Another copy's keys stay, and so does Qamille's key while one is there.
pub fn unregister(hive: &Hive, keys: &HandlerKeys) -> Result<(), String> {
    // First the value Settings lists, so the copy is never offered with half its keys.
    registry::delete_value(hive.registered, &keys.name).map_err(|error| error.to_string())?;
    registry::delete_tree(&format!(r"{}\{}", hive.classes, keys.prog_id)).map_err(|error| error.to_string())?;
    registry::delete_tree(&format!(r"{}\{}", hive.apps, keys.key)).map_err(|error| error.to_string())?;
    registry::delete_if_empty(hive.apps);
    if let Some((parent, _)) = hive.apps.rsplit_once('\\') {
        registry::delete_if_empty(parent);
    }
    Ok(())
}

/// What Windows has for the copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    /// Settings > Default apps offers the copy for mailto: links.
    pub registered: bool,
    /// The person chose it there: Windows opens mailto: links with it.
    pub chosen: bool,
}

pub fn status(hive: &Hive, keys: &HandlerKeys) -> Presence {
    let registered = registry::read(hive.registered, &keys.name).as_deref() == Some(capabilities_key(hive, keys).as_str())
        && registry::read(&format!(r"{}\{}\shell\open\command", hive.classes, keys.prog_id), "").is_some();
    // Newer Windows 11 keeps the choice under UserChoiceLatest\ProgId, older under UserChoice.
    let choice = registry::read(&format!(r"{}\UserChoiceLatest\ProgId", hive.choice), "ProgId")
        .or_else(|| registry::read(&format!(r"{}\UserChoice", hive.choice), "ProgId"));
    let chosen = registered && choice.is_some_and(|prog_id| prog_id.eq_ignore_ascii_case(&keys.prog_id));
    Presence { registered, chosen }
}

/// The current user's registry: strings only, every key under `HKEY_CURRENT_USER`.
#[cfg(windows)]
mod registry {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW,
        RegGetValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegSetKeyValueW,
    };
    use windows::core::HSTRING;

    /// A missing key or value is already removed.
    fn removed(status: WIN32_ERROR) -> windows::core::Result<()> {
        if status == ERROR_FILE_NOT_FOUND { Ok(()) } else { status.ok() }
    }

    /// Creates the key if it is missing (`RegSetKeyValueW`).
    pub fn write(key: &str, name: &str, value: &str) -> windows::core::Result<()> {
        let data: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let (key, name) = (HSTRING::from(key), HSTRING::from(name));
        // SAFETY: the strings and `data` live across the call, and the length is `data`'s.
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, &name, REG_SZ.0, Some(data.as_ptr().cast()), data.len() as u32).ok() }
    }

    pub fn read(key: &str, name: &str) -> Option<String> {
        let (key, name) = (HSTRING::from(key), HSTRING::from(name));
        let mut size = 0u32;
        // SAFETY: the strings live across both calls, and the buffer has at least the size the first gave.
        unsafe {
            RegGetValueW(HKEY_CURRENT_USER, &key, &name, RRF_RT_REG_SZ, None, None, Some(&mut size)).ok().ok()?;
            let mut data = vec![0u16; (size as usize).div_ceil(2)];
            RegGetValueW(HKEY_CURRENT_USER, &key, &name, RRF_RT_REG_SZ, None, Some(data.as_mut_ptr().cast()), Some(&mut size))
                .ok()
                .ok()?;
            data.truncate(size as usize / 2);
            Some(String::from_utf16_lossy(&data).trim_end_matches('\0').to_owned())
        }
    }

    #[cfg(test)]
    pub fn exists(key: &str) -> bool {
        let mut opened = HKEY::default();
        // SAFETY: the string lives across the call; a key that opened is closed.
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut opened).is_err() {
                return false;
            }
            let _ = RegCloseKey(opened);
        }
        true
    }

    pub fn delete_value(key: &str, name: &str) -> windows::core::Result<()> {
        // SAFETY: the strings live across the call.
        removed(unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) })
    }

    /// The key, with its subkeys and values.
    pub fn delete_tree(key: &str) -> windows::core::Result<()> {
        // SAFETY: the string lives across the call.
        removed(unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) })?;
        // SAFETY: as above; RegDeleteTreeW leaves the key itself on some versions of Windows.
        removed(unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(key)) })
    }

    /// Removes the key only when it holds nothing at all, no subkey and no value: what is not Qamille's stays.
    pub fn delete_if_empty(key: &str) {
        let wide = HSTRING::from(key);
        let mut opened = HKEY::default();
        let (mut subkeys, mut values) = (0u32, 0u32);
        // SAFETY: the string lives across the calls, the counters are written by the call, and a key
        // that opened is closed before it is deleted.
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, &wide, None, KEY_READ, &mut opened).is_err() {
                return;
            }
            let counted = RegQueryInfoKeyW(opened, None, None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None);
            let _ = RegCloseKey(opened);
            if counted.is_ok() && subkeys == 0 && values == 0 {
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &wide);
            }
        }
    }
}

#[cfg(not(windows))]
mod registry {
    pub fn write(_: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn read(_: &str, _: &str) -> Option<String> {
        None
    }
    pub fn delete_value(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn delete_tree(_: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn delete_if_empty(_: &str) {}
}

/// Tells Explorer and Settings that what opens a link type has changed.
#[cfg(windows)]
fn announce() {
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    // SAFETY: SHCNE_ASSOCCHANGED takes no items.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) }
}
#[cfg(not(windows))]
fn announce() {}

/// This copy's names: fixed for the installed Professional, else its folder and the tag of its data
/// folder, the same tag its start-at-sign-in entry has.
fn this_copy() -> &'static HandlerKeys {
    static KEYS: OnceLock<HandlerKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(|folder| folder.file_name()).map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let data = qamille::model::Store::data_dir();
        let data = data.canonicalize().unwrap_or(data);
        handler_keys(crate::background::installed(), &folder, &background_rules::workspace_tag(&data.to_string_lossy()))
    })
}

fn words(italian: &'static str, english: &'static str) -> &'static str {
    if crate::window_shell::italian() { italian } else { english }
}

/// Whether the setting is offered: not in a development build, whose keys would start the debug
/// executable, and not in the sample workspace.
fn supported(app: &AppHandle) -> bool {
    !tauri::is_dev() && !app.config().identifier.ends_with(".preview")
}

fn unsupported() -> AppError {
    AppError::new(
        "unsupported_capability",
        words("Questa copia di Qamille non può aprire i link mailto di Windows.", "This copy of Qamille cannot open Windows' mailto links."),
    )
}

fn refused(error: String) -> AppError {
    AppError::external(format!("{} {error}", words("Windows non ha accettato la modifica:", "Windows did not accept the change:")))
}

#[derive(Debug, Clone, Serialize)]
pub struct MailHandlerStatus {
    /// Settings > Default apps offers this copy for mailto: links.
    pub registered: bool,
    /// The name Settings shows for this copy.
    pub name: String,
    /// The installed Professional, whose installer writes and removes the keys: no «Smetti».
    pub installed: bool,
    /// Whether the setting is offered at all (`supported`).
    pub supported: bool,
    /// Windows opens mailto: links with this copy.
    pub chosen: bool,
}

/// Settings → General reads it when it opens and when the window comes back from Windows' Settings.
#[tauri::command]
pub async fn mail_handler_status(app: AppHandle) -> MailHandlerStatus {
    let keys = this_copy();
    let supported = supported(&app);
    let presence = if supported { status(&WINDOWS, keys) } else { Presence { registered: false, chosen: false } };
    MailHandlerStatus {
        registered: presence.registered,
        name: keys.display.clone(),
        installed: crate::background::installed(),
        supported,
        chosen: presence.chosen,
    }
}

/// «Usa Qamille per i link mailto»: writes this copy's keys (again, for the installed Professional,
/// whose installer wrote them) and opens Settings > Default apps on its entry, where the person chooses.
#[tauri::command]
pub async fn mail_handler_register(app: AppHandle) -> AppResult<()> {
    if !supported(&app) {
        return Err(unsupported());
    }
    let exe = std::env::current_exe().map_err(|error| AppError::external(error.to_string()))?;
    register(&WINDOWS, this_copy(), &exe.to_string_lossy()).map_err(refused)?;
    announce();
    open::that_detached(settings_page(this_copy())).map_err(|error| AppError::external(error.to_string()))
}

/// «Smetti»: removes this copy's keys. Windows asks which app to use at the next mailto: link if this
/// copy was the one chosen. The installed Professional's keys go with its uninstaller.
#[tauri::command]
pub async fn mail_handler_unregister(app: AppHandle) -> AppResult<()> {
    if !supported(&app) || crate::background::installed() {
        return Err(unsupported());
    }
    unregister(&WINDOWS, this_copy()).map_err(refused)?;
    announce();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"D:\Apps\Qamille Portable\Qamille.exe";

    #[test]
    fn installed_keys_are_fixed_and_a_portable_copy_carries_its_tag() {
        let installed = handler_keys(true, "Qamille", "6a5c8aa2");
        assert_eq!(
            (installed.name.as_str(), installed.display.as_str(), installed.prog_id.as_str(), installed.key.as_str()),
            ("Qamille", "Qamille", "Qamille.Url.mailto", "Installed")
        );
        let portable = handler_keys(false, "Qamille Portable", "6a5c8aa2");
        // The registered name goes into an ms-settings: address: letters, digits and a hyphen only.
        assert_eq!(portable.name, "Qamille-6a5c8aa2");
        assert_eq!(portable.display, "Qamille (Qamille Portable)");
        assert_eq!(portable.prog_id, "Qamille.Url.mailto.6a5c8aa2");
        assert_eq!(portable.key, "6a5c8aa2");
        // Two copies in folders of the same name never share a key.
        let other = handler_keys(false, "Qamille Portable", "0badf00d");
        assert_ne!(other.name, portable.name);
        assert_ne!(other.prog_id, portable.prog_id);
        assert_ne!(other.key, portable.key);
        assert_eq!(settings_page(&portable), "ms-settings:defaultapps?registeredAppUser=Qamille-6a5c8aa2");
        assert_eq!(settings_page(&installed), "ms-settings:defaultapps?registeredAppUser=Qamille");
    }

    #[test]
    fn the_entries_open_this_executable_with_the_link_in_quotes() {
        let keys = handler_keys(false, "Qamille Portable", "6a5c8aa2");
        let written = entries(&WINDOWS, &keys, EXE);
        let entry = |key: &str, name: &str, value: &str| (key.to_owned(), name.to_owned(), value.to_owned());
        assert_eq!(
            written,
            vec![
                entry(r"Software\Classes\Qamille.Url.mailto.6a5c8aa2", "", "Qamille (Qamille Portable)"),
                entry(r"Software\Classes\Qamille.Url.mailto.6a5c8aa2\DefaultIcon", "", r#""D:\Apps\Qamille Portable\Qamille.exe",0"#),
                entry(
                    r"Software\Classes\Qamille.Url.mailto.6a5c8aa2\shell\open\command",
                    "",
                    r#""D:\Apps\Qamille Portable\Qamille.exe" "%1""#
                ),
                entry(r"Software\Qamille\MailLinks\6a5c8aa2\Capabilities", "ApplicationName", "Qamille (Qamille Portable)"),
                entry(r"Software\Qamille\MailLinks\6a5c8aa2\Capabilities", "ApplicationDescription", "Qamille (Qamille Portable)"),
                entry(r"Software\Qamille\MailLinks\6a5c8aa2\Capabilities", "ApplicationIcon", r#""D:\Apps\Qamille Portable\Qamille.exe",0"#),
                entry(r"Software\Qamille\MailLinks\6a5c8aa2\Capabilities\URLAssociations", "mailto", "Qamille.Url.mailto.6a5c8aa2"),
                // Last: Windows lists the copy only once what it points to is there.
                entry(r"Software\RegisteredApplications", "Qamille-6a5c8aa2", r"Software\Qamille\MailLinks\6a5c8aa2\Capabilities"),
            ]
        );
        // A verbatim path is written plainly, as Windows runs it.
        let verbatim = entries(&WINDOWS, &keys, r"\\?\D:\Apps\Qamille\Qamille.exe");
        assert_eq!(verbatim[2].2, r#""D:\Apps\Qamille\Qamille.exe" "%1""#);
        let share = entries(&WINDOWS, &keys, r"\\?\UNC\server\apps\Qamille.exe");
        assert_eq!(share[2].2, r#""\\server\apps\Qamille.exe" "%1""#);
        // The installed Professional: the keys its installer writes (native-feel check 26 reads them there).
        let installed = entries(&WINDOWS, &handler_keys(true, "Qamille", "6a5c8aa2"), r"C:\Users\me\AppData\Local\Qamille\Qamille.exe");
        assert_eq!(installed[0].0, r"Software\Classes\Qamille.Url.mailto");
        assert_eq!(installed[7], entry(r"Software\RegisteredApplications", "Qamille", r"Software\Qamille\MailLinks\Installed\Capabilities"));
    }

    /// A key of its own under `HKCU\Software`, removed with everything in it when the test ends: the
    /// tests never write Windows' own keys, nor those of the owner's copies.
    #[cfg(windows)]
    struct Scratch {
        root: String,
        classes: String,
        registered: String,
        apps: String,
        choice: String,
    }
    #[cfg(windows)]
    impl Scratch {
        fn new(test: &str) -> Self {
            let root = format!(r"Software\Qamille tests\mail-{test}-{}", std::process::id());
            Self {
                classes: format!(r"{root}\Classes"),
                registered: format!(r"{root}\RegisteredApplications"),
                apps: format!(r"{root}\Qamille\MailLinks"),
                choice: format!(r"{root}\UrlAssociations\mailto"),
                root,
            }
        }
        fn hive(&self) -> Hive<'_> {
            Hive { classes: &self.classes, registered: &self.registered, apps: &self.apps, choice: &self.choice }
        }
    }
    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW, RegDeleteTreeW};
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &windows::core::HSTRING::from(self.root.as_str()));
                // The parent goes with the last test: it cannot be deleted while another test's key is in it.
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, windows::core::w!(r"Software\Qamille tests"));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn registering_writes_every_entry_and_unregistering_removes_them() {
        let scratch = Scratch::new("register");
        let hive = scratch.hive();
        let keys = handler_keys(false, "Qamille Portable", "6a5c8aa2");
        assert!(!status(&hive, &keys).registered);
        // The keys do not exist yet: registering creates them.
        register(&hive, &keys, EXE).unwrap();
        for (key, name, value) in entries(&hive, &keys, EXE) {
            assert_eq!(registry::read(&key, &name).as_deref(), Some(value.as_str()), r"{key}\{name}");
        }
        let now = status(&hive, &keys);
        assert!(now.registered && !now.chosen);
        // Twice is the same.
        register(&hive, &keys, EXE).unwrap();
        assert!(status(&hive, &keys).registered);
        unregister(&hive, &keys).unwrap();
        for (key, name, _) in entries(&hive, &keys, EXE) {
            assert_eq!(registry::read(&key, &name), None, r"{key}\{name}");
        }
        assert!(!registry::exists(&format!(r"{}\{}", hive.classes, keys.prog_id)), "the ProgID goes, subkeys and all");
        assert!(!registry::exists(&format!(r"{}\{}", hive.apps, keys.key)), "the copy's key goes, subkeys and all");
        assert!(!registry::exists(hive.apps), "an empty parent goes too");
        assert!(!status(&hive, &keys).registered);
        unregister(&hive, &keys).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn one_copy_never_touches_another_copys_keys() {
        let scratch = Scratch::new("copies");
        let hive = scratch.hive();
        let mine = handler_keys(false, "Qamille", "6a5c8aa2");
        let other = handler_keys(false, "Qamille", "0badf00d");
        register(&hive, &other, r"E:\Qamille\Qamille.exe").unwrap();
        assert!(!status(&hive, &mine).registered, "another copy's keys are not this copy's");
        register(&hive, &mine, EXE).unwrap();
        unregister(&hive, &mine).unwrap();
        assert!(status(&hive, &other).registered, "removing this copy leaves the other registered");
        for (key, name, value) in entries(&hive, &other, r"E:\Qamille\Qamille.exe") {
            assert_eq!(registry::read(&key, &name).as_deref(), Some(value.as_str()), r"{key}\{name}");
        }
        assert!(registry::exists(hive.apps), "a parent that is not empty stays");
    }

    #[cfg(windows)]
    #[test]
    fn the_choice_is_read_from_windows_the_newer_key_first() {
        let scratch = Scratch::new("choice");
        let hive = scratch.hive();
        let keys = handler_keys(false, "Qamille", "6a5c8aa2");
        register(&hive, &keys, EXE).unwrap();
        // What Settings writes when the person picks an app (here in the scratch key, never Windows' own).
        registry::write(&format!(r"{}\UserChoice", hive.choice), "ProgId", &keys.prog_id).unwrap();
        assert!(status(&hive, &keys).chosen);
        // Newer Windows 11 keeps the choice under UserChoiceLatest\ProgId: that one has the last word.
        registry::write(&format!(r"{}\UserChoiceLatest\ProgId", hive.choice), "ProgId", "Outlook.URL.mailto.15").unwrap();
        assert!(!status(&hive, &keys).chosen);
        registry::write(&format!(r"{}\UserChoiceLatest\ProgId", hive.choice), "ProgId", &keys.prog_id.to_uppercase()).unwrap();
        assert!(status(&hive, &keys).chosen, "a ProgID is matched without regard to case");
    }
}
