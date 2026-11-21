//! Qamille in the background (docs/aspetto-nativo.md, rules 26 and 27): the tray icon, the close
//! button that puts the window there, the one way back (`restore`), «Esci», and start at sign-in
//! straight into the tray. The pure rules are in `background_rules.rs`.
//!
//! - Closing the window (X, Alt+F4, «Chiudi» in the system menu or on the taskbar) hides it while
//!   «Chiudendo la finestra, Qamille resta nell'area di notifica» is on (the default): the window
//!   and its page stay alive, so the scheduler, the reminders and the mail keep running, and its
//!   webview is told it is hidden (`set_webview_visible`), so the page stops drawing. The first time,
//!   a Windows message says where Qamille went and offers to go back to closing.
//! - «Esci» (tray menu, system menu) closes the window through the page's close guards, as the X did
//!   before: `quit` raises `QUITTING`, which makes `hides_on_close` false, and calls `close()`. A guard
//!   that keeps the window brings it back (`restore_main_window`, from `window-close.ts`), and
//!   `close_settled` lowers `QUITTING` again.
//! - Everything that brings the window back comes through `restore`: the tray, a second start,
//!   notifications, mail links.
//! - Start at sign-in is the OS entry the autostart plugin writes, with `AUTOSTART_ARG`: such a start
//!   stays in the tray (`main.rs`). Its state is always read from Windows, which also sees the switch
//!   of Task Manager. It replaces the old worker's startup file (`migrate_legacy_launcher`).
//!
//! Preferences live in `data/background.json`, a file of their own.
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

pub use crate::background_rules::AUTOSTART_ARG;
use crate::background_rules::{self, CloseAction, LegacyLauncher};
use qamille::application::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

const TRAY_ID: &str = "main";

/// «Esci» is closing the window: the close goes through the page's guards instead of hiding.
static QUITTING: AtomicBool = AtomicBool::new(false);
/// The main window was minimized at its last resize (`window_resized`).
static MINIMIZED: AtomicBool = AtomicBool::new(false);
/// The name of this copy's start-at-sign-in entry (`autostart_entry`).
static ENTRY: OnceLock<String> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Preferences {
    /// Settings → General: the close button hides the window in the tray.
    #[serde(default = "on")]
    close_to_tray: bool,
    /// The message of the first hide was shown.
    #[serde(default)]
    tray_notice_shown: bool,
    /// Start at sign-in was turned on in Qamille. Never shown (Windows says whether it is on): only so
    /// that the installed Professional can put its entry back after a manual upgrade, whose uninstaller
    /// removes it (`put_back_after_upgrade`).
    #[serde(default)]
    start_at_sign_in: bool,
}
fn on() -> bool {
    true
}
impl Default for Preferences {
    fn default() -> Self {
        Self { close_to_tray: true, tray_notice_shown: false, start_at_sign_in: false }
    }
}

static PREFERENCES: Mutex<Option<Preferences>> = Mutex::new(None);

fn preferences_path() -> PathBuf {
    qamille::model::Store::data_dir().join("background.json")
}

fn preferences() -> Preferences {
    let mut cached = PREFERENCES.lock().unwrap_or_else(|e| e.into_inner());
    cached
        .get_or_insert_with(|| {
            std::fs::read_to_string(preferences_path())
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        })
        .clone()
}

fn save_preferences(next: Preferences) {
    let mut cached = PREFERENCES.lock().unwrap_or_else(|e| e.into_inner());
    if cached.as_ref() == Some(&next) {
        return;
    }
    // Tolerant, like the window preferences: a file that cannot be written means the defaults next time.
    if let Ok(text) = serde_json::to_string(&next) {
        let _ = std::fs::write(preferences_path(), text);
    }
    *cached = Some(next);
}

fn words(italian: &'static str, english: &'static str) -> &'static str {
    if crate::window_shell::italian() { italian } else { english }
}

/// Whether the close button hides the window now (`CloseRequested` in `main.rs`).
pub fn hides_on_close() -> bool {
    background_rules::close_action(preferences().close_to_tray, is_quitting()) == CloseAction::Hide
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// The page's close ended without closing (`recovery::close_settled`): a guard kept the window, so
/// «Esci» is over, and the next X hides again.
pub fn close_settled() {
    QUITTING.store(false, Ordering::SeqCst);
}

/// The close button with `hides_on_close`: the window goes to the tray and its webview stops drawing.
/// The first time, a message without owner (the window is gone) says where Qamille is.
pub fn hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.hide();
    crate::native_feel::set_webview_visible(&window, false);
    let mut next = preferences();
    if !next.tray_notice_shown {
        next.tray_notice_shown = true;
        save_preferences(next);
        let app = app.clone();
        // A dialog runs its own modal loop: never inside the window event that asked.
        std::thread::spawn(move || tray_notice(&app));
    }
}

fn tray_notice(app: &AppHandle) {
    let leave = words("Esci, e d'ora in poi chiudi Qamille", "Quit, and close Qamille from now on");
    let answer = rfd::MessageDialog::new()
        .set_title("Qamille")
        .set_level(rfd::MessageLevel::Info)
        .set_description(words(
            "Qamille resta aperto nell'area di notifica: invia i messaggi programmati, mostra i promemoria e riceve la posta. Per uscire usa Esci dal menu dell'icona.",
            "Qamille keeps running in the notification area: it sends scheduled messages, shows reminders and receives mail. To quit, use Quit in the icon's menu.",
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom("OK".into(), leave.into()))
        .show();
    // Esc and the X are OK: Qamille stays in the tray, as the message says.
    if answer == rfd::MessageDialogResult::Custom(leave.into()) {
        set_close_to_tray(false);
        quit(app);
    }
}

/// The one way back to the window: the tray, a second start, a notification, a mail link, a guard
/// that kept the window during «Esci». The webview is told it is visible before the window shows, so
/// the first frame is the page, then the window leaves the taskbar and comes to the front.
pub fn restore(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    crate::native_feel::set_webview_visible(&window, true);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    // A page that did not draw while the window was hidden asks now, with the window in view.
    crate::recovery::offer_reload_if_not_ready(&window);
}

/// «Esci»: the window closes as the X did before this module, through the page's guards (unsaved
/// drafts, a recording, the undo queue). Never `exit` here: the guards decide.
pub fn quit(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    QUITTING.store(true, Ordering::SeqCst);
    let _ = window.close();
}

/// Qamille started again while it runs: the window comes back. `args` and `cwd` are the second
/// start's.
pub fn second_instance(app: &AppHandle, args: Vec<String>, cwd: String) {
    let _ = (&args, &cwd);
    restore(app);
}

/// Whether this is the installed Professional: Tauri's NSIS installer writes `uninstall.exe` beside
/// the executable (`WriteUninstaller "$INSTDIR\uninstall.exe"` in its `installer.nsi`).
pub fn installed() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|folder| folder.join("uninstall.exe").is_file()))
            .unwrap_or(false)
    })
}

/// The name of this copy's start-at-sign-in entry, from the app identifier `main.rs` built (with
/// the hash of the data folder): «Qamille» when installed, else the folder and the hash (Q22).
pub fn autostart_entry(identifier: &str) -> String {
    let folder = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(|folder| folder.file_name()).map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let tag = background_rules::workspace_tag(identifier).unwrap_or("00000000");
    ENTRY.get_or_init(|| background_rules::autostart_entry_name(installed(), &folder, tag)).clone()
}

struct TrayMenu {
    show: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

fn tray_labels(italian: bool) -> (&'static str, &'static str) {
    if italian { ("Mostra Qamille", "Esci") } else { ("Show Qamille", "Quit") }
}

/// The tray icon, from setup: the app's icon, «Qamille» as its tooltip, and a menu with «Mostra
/// Qamille» and «Esci» on the right button. A left click brings the window back.
pub fn install(app: &tauri::App) -> tauri::Result<()> {
    let (show_text, quit_text) = tray_labels(crate::window_shell::italian());
    let show = MenuItem::with_id(app, "show", show_text, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", quit_text, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit_item])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Qamille")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                restore(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => restore(app),
            "quit" => quit(app),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(TrayMenu { show, quit: quit_item });
    Ok(())
}

/// The interface language changed (`window_shell::window_appearance`): the tray menu follows it.
pub fn relabel(app: &AppHandle, italian: bool) {
    let Some(menu) = app.try_state::<TrayMenu>() else { return };
    let (show_text, quit_text) = tray_labels(italian);
    let _ = menu.show.set_text(show_text);
    let _ = menu.quit.set_text(quit_text);
}

/// The tray icon's tooltip (it says «Qamille» until a caller says more).
#[allow(dead_code, reason = "phase 3, Task 3 shows the unread count here")]
pub fn set_tray_tooltip(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(text));
    }
}

/// Minimizing pauses the webview, as hiding does (Microsoft: set `IsVisible` false while the app
/// window is minimized); bringing it back from the taskbar resumes it. Every resize passes here, so
/// only a change of the minimized state does something.
pub fn window_resized(window: &tauri::Window) {
    let minimized = window.is_minimized().unwrap_or(false);
    if MINIMIZED.swap(minimized, Ordering::SeqCst) == minimized {
        return;
    }
    let Some(main) = window.app_handle().get_webview_window(window.label()) else { return };
    let visible = !minimized && window.is_visible().unwrap_or(true);
    crate::native_feel::set_webview_visible(&main, visible);
}

fn set_close_to_tray(on: bool) {
    let mut next = preferences();
    next.close_to_tray = on;
    save_preferences(next);
}

fn set_start_at_sign_in(on: bool) {
    let mut next = preferences();
    next.start_at_sign_in = on;
    save_preferences(next);
}

/// Whether the setting is offered: not in a development build, whose entry would start the debug
/// executable, and not in the sample workspace.
fn autostart_supported(app: &AppHandle) -> bool {
    !tauri::is_dev() && !app.config().identifier.ends_with(".preview")
}

/// An elevated Qamille (UAC off, or «Esegui come amministratore»): auto-launch writes the
/// machine-wide `HKLM\…\Run` when it can, which would start this copy for every user of the PC.
fn elevated() -> bool {
    #[cfg(windows)]
    return unsafe { windows::Win32::UI::Shell::IsUserAnAdmin() }.as_bool();
    #[cfg(not(windows))]
    false
}

/// Whether this copy's value exists under `HKCU\…\Run`, on or off: Task Manager turns an entry off
/// in its twin under `StartupApproved` and leaves the value.
fn entry_present() -> bool {
    let Some(name) = ENTRY.get() else { return false };
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
        return RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            &windows::core::HSTRING::from(name.as_str()),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok();
    }
    #[cfg(not(windows))]
    false
}

/// Once at start, off the main thread and never in the demo (`main.rs`): the worker's startup file
/// of an earlier version becomes this copy's start at sign-in, or goes if it starts nothing; the
/// worker's old switch is cleared; and the installed Professional's entry is put back after a manual
/// upgrade. Failures go to `native.log` and never stop Qamille.
pub fn migrate_legacy_launcher(app: &AppHandle) {
    use qamille::infrastructure::mcp_config;
    if !autostart_supported(app) {
        return;
    }
    match mcp_config::legacy_launcher() {
        Ok(Some((path, content))) => {
            let this = std::env::current_exe().map(|exe| exe.display().to_string()).unwrap_or_default();
            let exists = background_rules::launcher_target(&content).is_some_and(|target| std::path::Path::new(&target).is_file());
            match background_rules::legacy_launcher(Some(&content), &this, exists) {
                // Elevated, the entry would go machine-wide: the next ordinary start migrates it.
                LegacyLauncher::Adopt if elevated() => {}
                LegacyLauncher::Adopt => match app.autolaunch().enable() {
                    Ok(()) => {
                        set_start_at_sign_in(true);
                        remove_legacy(&path);
                    }
                    Err(error) => crate::native_feel::report(&format!("start at sign-in was not moved from the worker's startup file: {error}")),
                },
                LegacyLauncher::RemoveStale => remove_legacy(&path),
                LegacyLauncher::LeaveToOther | LegacyLauncher::Nothing => {}
            }
        }
        Ok(None) => {}
        Err(error) => crate::native_feel::report(&format!("the worker's startup file could not be read: {}", error.message)),
    }
    match mcp_config::load() {
        Ok(mut config) if config.worker_autostart => {
            config.worker_autostart = false;
            if let Err(error) = mcp_config::save(&config) {
                crate::native_feel::report(&format!("the worker's startup switch was not cleared: {}", error.message));
            }
        }
        _ => {}
    }
    put_back_after_upgrade(app);
}

fn remove_legacy(path: &std::path::Path) {
    if let Err(error) = qamille::infrastructure::mcp_config::remove_legacy_launcher() {
        crate::native_feel::report(&format!("the worker's startup file {} was not removed: {}", path.display(), error.message));
    }
}

/// Windows has the last word on start at sign-in: an entry turned off or on in Task Manager is
/// remembered as such. A missing entry is put back only for the installed Professional, whose
/// uninstaller removes it also on a manual upgrade («uninstall before installing»), and only if it
/// was turned on in Qamille; a portable copy has no uninstaller, so a missing entry was removed on
/// purpose.
fn put_back_after_upgrade(app: &AppHandle) {
    if entry_present() {
        if let Ok(enabled) = app.autolaunch().is_enabled()
            && enabled != preferences().start_at_sign_in
        {
            set_start_at_sign_in(enabled);
        }
        return;
    }
    if !installed() || !preferences().start_at_sign_in || elevated() {
        if !installed() && preferences().start_at_sign_in {
            set_start_at_sign_in(false);
        }
        return;
    }
    if let Err(error) = app.autolaunch().enable() {
        crate::native_feel::report(&format!("start at sign-in was not put back after the upgrade: {error}"));
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    /// Whether the setting is offered at all (`autostart_supported`).
    pub supported: bool,
}

fn status(app: &AppHandle) -> AutostartStatus {
    let supported = autostart_supported(app);
    let enabled = supported
        && app.autolaunch().is_enabled().unwrap_or_else(|error| {
            crate::native_feel::report(&format!("could not read start at sign-in: {error}"));
            false
        });
    AutostartStatus { enabled, supported }
}

/// Settings → General reads it each time it opens: from Windows, never a copy.
#[tauri::command]
pub async fn autostart_status(app: AppHandle) -> AutostartStatus {
    status(&app)
}

/// Turns start at sign-in on or off for this copy, under its own name (`autostart_entry`).
#[tauri::command]
pub async fn autostart_set(app: AppHandle, enabled: bool) -> AppResult<AutostartStatus> {
    if !autostart_supported(&app) {
        return Err(AppError::new(
            "unsupported_capability",
            words("Questa copia di Qamille non può avviarsi all'accesso.", "This copy of Qamille cannot start at sign-in."),
        ));
    }
    if enabled && elevated() {
        return Err(AppError::validation(words(
            "Qamille è aperto come amministratore: l'avvio all'accesso varrebbe per tutti gli utenti del computer. Riapri Qamille normalmente per attivarlo.",
            "Qamille is running as administrator: start at sign-in would apply to every user of this computer. Open Qamille normally to turn it on.",
        )));
    }
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|error| AppError::external(format!("{} {error}", words("Windows non ha accettato la modifica:", "Windows did not accept the change:"))))?;
    set_start_at_sign_in(enabled);
    Ok(status(&app))
}

#[tauri::command]
pub fn close_to_tray_get() -> bool {
    preferences().close_to_tray
}

#[tauri::command]
pub fn close_to_tray_set(on: bool) {
    set_close_to_tray(on);
}

/// A guard kept the window during «Esci», or asks something: the window comes back first.
#[tauri::command]
pub fn restore_main_window(app: AppHandle) {
    restore(&app);
}
