//! Windows notifications of Qamille (docs/aspetto-nativo.md, rule 28): new mail, a scheduled send
//! that failed, a sign-in that no longer works. What they say and when is in `notify_rules.rs`.
//!
//! - The facts are typed events of the crate `app` (`AppEvent::MailArrived`, `ScheduledSendFailed`,
//!   `SignInExpired`), emitted by the writer that knows them. The desktop sink (`runtime.rs`) hands
//!   them to `observe`, which takes them away from the page, and a thread of its own sends them: the
//!   writer that emitted one never waits for Windows, nor for the store read that names the senders.
//! - They go out as Windows toasts from Rust only, with `tauri-winrt-notification`, the toast backend
//!   of the official notification plugin, used directly: the plugin waits for each toast's answer on a
//!   thread of its own, which under Do not disturb would stay until Qamille exits. Here a toast holds
//!   its click handler and nothing waits (`toast`). The page has no notification permission at all.
//! - A toast goes out under `app.config().identifier`, the per-data-folder identifier of `main.rs`.
//!   Windows shows a notification of an unpackaged app reliably only for an identity it knows, so
//!   Qamille registers it for the current user under `HKCU\Software\Classes\AppUserModelId\<identifier>`
//!   («Qamille», the app's icon as `data\notification-icon.png`) while notifications are on, and takes
//!   it away when they go off. Never the process's own AUMID: the window would leave the icons pinned
//!   on the taskbar.
//! - On by default in the Professional (registered at its first start after the update), off in the
//!   open source edition. The identifier goes to `data\notification-aumid.txt` too, where the
//!   Professional's uninstaller reads it (`windows/installer-hooks.nsh`).
//! - A click brings the window back (`background::restore`) and tells the page what to open
//!   (`notification-open`, `src/lib/notification-open.ts`), while Qamille runs: a click in the
//!   notification center once Qamille is gone does nothing (no activation is registered for it).
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

use crate::notify_rules::{self, Kind, Prefs, WindowState};
use qamille::application::{AppError, AppResult, ServiceContext, events::AppEvent};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use tauri::{AppHandle, Emitter, Manager};

/// The switches, in the data folder.
const PREFERENCES_FILE: &str = "notifications.json";
/// The icon Windows shows beside the notifications (`IconUri` of the registered identity).
const ICON_FILE: &str = "notification-icon.png";
/// The registered identifier, for the uninstaller.
const AUMID_FILE: &str = "notification-aumid.txt";
const ICON: &[u8] = include_bytes!("../icons/icon.png");
/// What the page hears on a click.
const OPEN_EVENT: &str = "notification-open";

/// The facts `observe` took, for the thread that sends them (`start`).
static FACTS: OnceLock<mpsc::Sender<AppEvent>> = OnceLock::new();

/// The switches as `data/notifications.json` keeps them. `enabled` is absent until the person chose:
/// then the edition decides (`resolve`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(default = "on")]
    new_mail: bool,
    #[serde(default = "on")]
    scheduled_send: bool,
    #[serde(default = "on")]
    sign_in: bool,
}
fn on() -> bool {
    true
}
impl Default for Saved {
    fn default() -> Self {
        Self { enabled: None, new_mail: true, scheduled_send: true, sign_in: true }
    }
}

/// The switches in force: what the person chose, or on in the Professional and off otherwise.
fn resolve(saved: Saved, professional: bool) -> Prefs {
    Prefs {
        enabled: saved.enabled.unwrap_or(professional),
        new_mail: saved.new_mail,
        scheduled_send: saved.scheduled_send,
        sign_in: saved.sign_in,
    }
}

static SAVED: Mutex<Option<Saved>> = Mutex::new(None);

fn preferences_path() -> PathBuf {
    qamille::model::Store::data_dir().join(PREFERENCES_FILE)
}

fn saved() -> Saved {
    let mut cached = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    *cached.get_or_insert_with(|| {
        std::fs::read_to_string(preferences_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    })
}

fn save(next: Saved) {
    let mut cached = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    // Tolerant, like the other window preferences: a file that cannot be written means the defaults
    // next time.
    if let Ok(text) = serde_json::to_string(&next) {
        let _ = std::fs::write(preferences_path(), text);
    }
    *cached = Some(next);
}

fn current() -> Prefs {
    resolve(saved(), qamille::edition::edition().preset().is_some())
}

fn words(italian: &'static str, english: &'static str) -> &'static str {
    if crate::window_shell::italian() { italian } else { english }
}

/// Not in the sample workspace, which has no mail to announce and must not register an identity.
fn supported(app: &AppHandle) -> bool {
    !app.config().identifier.ends_with(".preview")
}

/// The identity the toasts go out under: the per-data-folder identifier of `main.rs`.
fn identifier(app: &AppHandle) -> String {
    app.config().identifier.clone()
}

/// Whether the toasts go out under that identity: not from a development build in `target\debug` or
/// `target\release`, which sends them under PowerShell's, as the official plugin does, and registers
/// nothing.
fn uses_identity() -> bool {
    let Ok(exe) = tauri::utils::platform::current_exe() else { return false };
    let folder = exe.parent().map(|folder| folder.display().to_string()).unwrap_or_default();
    let sep = std::path::MAIN_SEPARATOR;
    !(folder.ends_with(&format!("{sep}target{sep}debug")) || folder.ends_with(&format!("{sep}target{sep}release")))
}

/// The identity of the notifications in the registry of the current user. The root is a parameter so
/// that the tests use a scratch key of their own.
pub(crate) mod identity {
    pub const CLASSES: &str = r"Software\Classes\AppUserModelId";

    /// `AppUserModelId\<identifier>`. An identifier that would name another key is refused.
    pub fn key(root: &str, aumid: &str) -> Result<String, String> {
        if aumid.is_empty() || aumid.contains(['\\', '/']) {
            return Err(format!("not an app identifier: {aumid:?}"));
        }
        Ok(format!(r"{root}\{aumid}"))
    }

    #[cfg(windows)]
    mod win {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
        #[cfg(test)]
        use windows::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyW, RegDeleteTreeW, RegSetKeyValueW};
        use windows::core::HSTRING;

        /// A string value; creates the key if it is missing.
        pub fn write(key: &str, name: &str, value: &str) -> windows::core::Result<()> {
            let data: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            let (key, name) = (HSTRING::from(key), HSTRING::from(name));
            // SAFETY: `data` lives across the call and its length is the one passed.
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, &name, REG_SZ.0, Some(data.as_ptr().cast()), data.len() as u32).ok() }
        }

        #[cfg(test)]
        pub fn read(key: &str, name: &str) -> Option<String> {
            let (key, name) = (HSTRING::from(key), HSTRING::from(name));
            let mut size = 0u32;
            // SAFETY: the strings live across both calls, and the buffer has the size the first call gave.
            unsafe {
                RegGetValueW(HKEY_CURRENT_USER, &key, &name, RRF_RT_REG_SZ, None, None, Some(&mut size)).ok().ok()?;
                let mut data = vec![0u8; size as usize];
                RegGetValueW(HKEY_CURRENT_USER, &key, &name, RRF_RT_REG_SZ, None, Some(data.as_mut_ptr().cast()), Some(&mut size)).ok().ok()?;
                let wide: Vec<u16> = data[..size as usize].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
                Some(String::from_utf16_lossy(&wide).trim_end_matches('\0').to_owned())
            }
        }

        /// The key with its values; a key that is not there is already removed.
        pub fn delete(key: &str) -> windows::core::Result<()> {
            let key = HSTRING::from(key);
            let gone = |status: windows::Win32::Foundation::WIN32_ERROR| {
                if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND { Ok(()) } else { status.ok() }
            };
            // SAFETY: the string lives across both calls.
            unsafe {
                gone(RegDeleteTreeW(HKEY_CURRENT_USER, &key))?;
                gone(RegDeleteKeyW(HKEY_CURRENT_USER, &key))
            }
        }
    }

    /// Writes the identity: the name and the icon Windows shows with the notifications.
    pub fn register(root: &str, aumid: &str, display_name: &str, icon: &str) -> Result<(), String> {
        let key = key(root, aumid)?;
        #[cfg(windows)]
        {
            win::write(&key, "DisplayName", display_name).map_err(|error| error.to_string())?;
            win::write(&key, "IconUri", icon).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Takes the identity away; twice is fine.
    pub fn unregister(root: &str, aumid: &str) -> Result<(), String> {
        let key = key(root, aumid)?;
        #[cfg(windows)]
        win::delete(&key).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// A value of the identity, if it is registered.
    #[cfg(test)]
    pub fn value(root: &str, aumid: &str, name: &str) -> Option<String> {
        let key = key(root, aumid).ok()?;
        #[cfg(windows)]
        return win::read(&key, name);
        #[cfg(not(windows))]
        None
    }
}

/// Registers the identity with its icon, and writes the identifier for the uninstaller.
fn register(app: &AppHandle) -> Result<(), String> {
    let aumid = identifier(app);
    let data = qamille::model::Store::data_dir();
    let icon = data.join(ICON_FILE);
    if std::fs::read(&icon).ok().as_deref() != Some(ICON) {
        std::fs::write(&icon, ICON).map_err(|error| format!("{}: {error}", icon.display()))?;
    }
    identity::register(identity::CLASSES, &aumid, "Qamille", &icon.to_string_lossy())?;
    std::fs::write(data.join(AUMID_FILE), &aumid).map_err(|error| error.to_string())
}

/// Takes the identity away, with the files that went with it.
fn unregister(app: &AppHandle) -> Result<(), String> {
    identity::unregister(identity::CLASSES, &identifier(app))?;
    let data = qamille::model::Store::data_dir();
    for file in [AUMID_FILE, ICON_FILE] {
        match std::fs::remove_file(data.join(file)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(())
}

/// The registry follows the general switch. Nothing for a development build (`uses_identity`).
fn apply_identity(app: &AppHandle, on: bool) -> Result<(), String> {
    if !uses_identity() {
        return Ok(());
    }
    if on { register(app) } else { unregister(app) }
}

/// From `DesktopState::new`: the thread that sends what `observe` takes. First it puts the identity in
/// step with the switch: on writes it again, also after a manual upgrade whose uninstaller removed it.
pub fn start(app: &AppHandle, context: Arc<ServiceContext>) {
    let (facts, received) = mpsc::channel();
    if FACTS.set(facts).is_err() {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("qamille-notifications".into()).spawn(move || {
        if !supported(&app) {
            // Taken and dropped: they never reach the page either way.
            for _ in received {}
            return;
        }
        if let Err(error) = apply_identity(&app, current().enabled) {
            crate::native_feel::report(&format!("the notification identity was not updated: {error}"));
        }
        for fact in received {
            announce(&app, &context, fact);
        }
    });
}

/// The sink's question for every event (`runtime.rs`): `true` for the three facts, which are taken
/// here and never forwarded to the page; `false` for the rest.
pub fn observe(app: &AppHandle, event: &AppEvent) -> bool {
    let _ = app;
    match event {
        AppEvent::MailArrived { .. } | AppEvent::ScheduledSendFailed { .. } | AppEvent::SignInExpired { .. } => {
            if let Some(facts) = FACTS.get() {
                let _ = facts.send(event.clone());
            }
            true
        }
        _ => false,
    }
}

/// The main window as Windows has it now; minimized is not visible.
fn window_state(app: &AppHandle) -> WindowState {
    let Some(window) = app.get_webview_window("main") else {
        return WindowState { visible: false, focused: false };
    };
    let minimized = window.is_minimized().unwrap_or(false);
    WindowState {
        visible: window.is_visible().unwrap_or(false) && !minimized,
        focused: window.is_focused().unwrap_or(false),
    }
}

fn announce(app: &AppHandle, context: &ServiceContext, fact: AppEvent) {
    let prefs = current();
    let window = window_state(app);
    let italian = crate::window_shell::italian();
    match fact {
        AppEvent::MailArrived { account_id, ids } => {
            if !notify_rules::should_notify(Kind::NewMail, &prefs, window) {
                return;
            }
            let Ok(store) = context.read_shared() else { return };
            let Some(account) = store.account(account_id) else { return };
            // Still unread in the Inbox: a message read or moved meanwhile is not announced.
            let mut arrived: Vec<_> = ids
                .iter()
                .filter_map(|id| store.email(*id))
                .filter(|email| email.folder == "Inbox" && !email.read && !email.done)
                .collect();
            arrived.sort_by_key(|email| std::cmp::Reverse(email.date));
            // Sender and subject; never the body.
            let items: Vec<(String, String)> = arrived
                .iter()
                .map(|email| {
                    let sender = if email.from_name.trim().is_empty() { &email.from_addr } else { &email.from_name };
                    (sender.clone(), email.subject.clone())
                })
                .collect();
            for (index, title, body) in notify_rules::new_mail_notices(&items, &account.email, italian) {
                show(app, &title, &body, Tone::Mail, json!({ "kind": "mail", "email_id": arrived[index].id, "account_id": account_id }));
            }
        }
        AppEvent::ScheduledSendFailed { draft_id, subject } => {
            if notify_rules::should_notify(Kind::ScheduledSend, &prefs, window) {
                let (title, body) = notify_rules::scheduled_send_text(&subject, italian);
                show(app, &title, &body, Tone::Default, json!({ "kind": "scheduled", "draft_id": draft_id }));
            }
        }
        AppEvent::SignInExpired { account_id } => {
            if !notify_rules::should_notify(Kind::SignIn, &prefs, window) {
                return;
            }
            let account = match account_id {
                Some(id) => match context.read_shared().ok().and_then(|store| store.account(id).map(|a| a.email.clone())) {
                    Some(email) => Some(email),
                    None => return,
                },
                None => None,
            };
            let (title, body) = notify_rules::sign_in_text(account.as_deref(), italian);
            let open = match account_id {
                Some(id) => json!({ "kind": "sign-in", "account_id": id }),
                None => json!({ "kind": "sign-in" }),
            };
            show(app, &title, &body, Tone::Default, open);
        }
        _ => {}
    }
}

/// The sound of a toast.
#[derive(Clone, Copy, Debug)]
enum Tone {
    Mail,
    Default,
}

/// One toast under `aumid`, ready to show. Windows calls `on_click` when the toast is clicked, on a
/// thread of its own, as long as it is on screen or in the notification center; the toast holds it and
/// nothing of Qamille waits for an answer, so a toast that is never clicked leaves no thread or handle
/// behind.
#[cfg(windows)]
fn toast(aumid: &str, title: &str, body: &str, tone: Tone, on_click: impl Fn() + Send + 'static) -> tauri_winrt_notification::Toast {
    use tauri_winrt_notification::{Sound, Toast};
    Toast::new(aumid)
        .title(title)
        .text1(body)
        .sound(Some(match tone {
            Tone::Mail => Sound::Mail,
            Tone::Default => Sound::Default,
        }))
        .on_activated(move |_action| {
            on_click();
            Ok(())
        })
}

/// One notification; `open` is what its click opens: the window comes back through the one way back,
/// then the page opens what the notification was about. A toast Windows refused is a line in
/// `native.log`. Windows may still keep it out of sight (Do not disturb, the app's switch in Windows
/// Settings): the in-app notice stays, so nothing is lost.
fn show(app: &AppHandle, title: &str, body: &str, tone: Tone, open: serde_json::Value) {
    #[cfg(windows)]
    {
        let aumid = if uses_identity() { identifier(app) } else { tauri_winrt_notification::Toast::POWERSHELL_APP_ID.to_owned() };
        let clicked = app.clone();
        let shown = toast(&aumid, title, body, tone, move || {
            crate::background::restore(&clicked);
            let _ = clicked.emit_to("main", OPEN_EVENT, open.clone());
        })
        .show();
        if let Err(error) = shown {
            crate::native_feel::report(&format!("a notification could not be shown: {error}"));
        }
    }
}

/// What Settings → General → Windows shows and sends back.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotificationPrefs {
    pub enabled: bool,
    pub new_mail: bool,
    pub scheduled_send: bool,
    pub sign_in: bool,
    /// Read only: a copy without an uninstaller, whose switch says that it writes in the registry.
    #[serde(default)]
    pub portable: bool,
    /// Read only: false in the sample workspace, which shows no switch.
    #[serde(default)]
    pub supported: bool,
}

#[tauri::command]
pub fn notifications_get(app: AppHandle) -> NotificationPrefs {
    let prefs = current();
    NotificationPrefs {
        enabled: prefs.enabled,
        new_mail: prefs.new_mail,
        scheduled_send: prefs.scheduled_send,
        sign_in: prefs.sign_in,
        portable: !crate::background::installed(),
        supported: supported(&app),
    }
}

/// Saves the switches; the general one also registers the identity, or takes it away, first: a
/// change Windows refused is not saved.
#[tauri::command]
pub async fn notifications_set(app: AppHandle, prefs: NotificationPrefs) -> AppResult<()> {
    if !supported(&app) {
        return Err(AppError::new(
            "unsupported_capability",
            words("Il workspace di esempio non mostra notifiche.", "The sample workspace shows no notifications."),
        ));
    }
    if prefs.enabled != current().enabled {
        apply_identity(&app, prefs.enabled).map_err(|error| {
            AppError::external(format!("{} {error}", words("Windows non ha accettato la modifica:", "Windows did not accept the change:")))
        })?;
    }
    save(Saved {
        enabled: Some(prefs.enabled),
        new_mail: prefs.new_mail,
        scheduled_send: prefs.scheduled_send,
        sign_in: prefs.sign_in,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_are_on_by_default_only_in_the_professional() {
        assert!(resolve(Saved::default(), true).enabled);
        assert!(!resolve(Saved::default(), false).enabled);
        // What the person chose wins, in either edition.
        assert!(!resolve(Saved { enabled: Some(false), ..Saved::default() }, true).enabled);
        assert!(resolve(Saved { enabled: Some(true), ..Saved::default() }, false).enabled);
        // A file of an older version, or with only some switches, keeps the others on.
        let partial: Saved = serde_json::from_str(r#"{"new_mail":false}"#).unwrap();
        assert_eq!(resolve(partial, true), Prefs { enabled: true, new_mail: false, scheduled_send: true, sign_in: true });
        let empty: Saved = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Saved::default());
        // Until the person chooses, the file says nothing about the general switch.
        assert_eq!(serde_json::to_string(&Saved::default()).unwrap(), r#"{"new_mail":true,"scheduled_send":true,"sign_in":true}"#);
    }

    /// Showing a toast leaves nothing of Qamille waiting for it: the click handler, which holds what
    /// the click opens, lives in the toast alone (Windows keeps it while the toast is shown or in the
    /// notification center, and calls it from a thread of its own). Building one is enough to see it:
    /// a real toast never appears in a test.
    #[cfg(windows)]
    #[test]
    fn a_toast_holds_its_click_and_nothing_else_does() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let clicks = Arc::new(AtomicUsize::new(0));
        let held = clicks.clone();
        let toast = toast("com.qamille.desktop.workspace-test", "Anna Rossi", "Preventivo", Tone::Mail, move || {
            held.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(Arc::strong_count(&clicks), 2, "the toast's click handler holds what it opens");
        drop(toast);
        assert_eq!(Arc::strong_count(&clicks), 1, "no thread, channel or handle of Qamille keeps it");
        assert_eq!(clicks.load(Ordering::SeqCst), 0, "building a toast clicks nothing");
    }

    #[test]
    fn the_identity_key_is_one_under_app_user_model_id() {
        assert_eq!(identity::key(identity::CLASSES, "com.qamille.desktop.workspace0123").unwrap(), r"Software\Classes\AppUserModelId\com.qamille.desktop.workspace0123");
        for wrong in ["", r"com.qamille.desktop\..\Other", "a/b"] {
            assert!(identity::key(identity::CLASSES, wrong).is_err(), "{wrong:?}");
        }
    }

    /// A key of its own under `HKCU\Software`, removed with everything in it when the test ends: the
    /// tests never touch `Software\Classes`, nor the identities of the owner's copies.
    #[cfg(windows)]
    struct Scratch(String);
    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW, RegDeleteTreeW};
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &windows::core::HSTRING::from(self.0.as_str()));
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &windows::core::HSTRING::from(self.0.as_str()));
                // The parent goes with the last test: it cannot be deleted while another test's key is in it.
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, windows::core::w!(r"Software\Qamille tests"));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_identity_is_written_with_its_name_and_icon_and_taken_away() {
        let scratch = Scratch(format!(r"Software\Qamille tests\aumid-{}", std::process::id()));
        let root = scratch.0.as_str();
        let aumid = "com.qamille.desktop.workspace00000000000000aa";
        assert_eq!(identity::value(root, aumid, "DisplayName"), None);
        // Twice is fine, before and after.
        identity::unregister(root, aumid).unwrap();
        identity::register(root, aumid, "Qamille", r"D:\Apps\Qamille\data\notification-icon.png").unwrap();
        identity::register(root, aumid, "Qamille", r"D:\Apps\Qamille\data\notification-icon.png").unwrap();
        assert_eq!(identity::value(root, aumid, "DisplayName").as_deref(), Some("Qamille"));
        assert_eq!(identity::value(root, aumid, "IconUri").as_deref(), Some(r"D:\Apps\Qamille\data\notification-icon.png"));
        assert_eq!(identity::value(root, "com.qamille.desktop.other", "DisplayName"), None, "another copy's identity is another key");
        identity::unregister(root, aumid).unwrap();
        assert_eq!(identity::value(root, aumid, "DisplayName"), None);
        identity::unregister(root, aumid).unwrap();
    }
}
