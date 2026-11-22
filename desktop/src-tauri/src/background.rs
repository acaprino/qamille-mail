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
//! - Start at sign-in is a value Qamille writes under `HKCU\…\CurrentVersion\Run` (`startup`):
//!   `"<exe>" --autostart`, the path in quotes, never machine-wide. Such a start stays in the tray
//!   (`main.rs`). Its state is always read from Windows, Task Manager's switch included. It replaces
//!   the old worker's startup file (`migrate_legacy_launcher`).
//! - A restart keeps the arguments of the first start: `before_restart` says whether the window was
//!   hidden, so a window on screen comes back on screen.
//!
//! Preferences live in `data/background.json`, a file of their own.
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

#[allow(unused_imports, reason = "phase 3, Task 4: a start with a mailto: link always shows the window")]
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

const TRAY_ID: &str = "main";
/// Set for the process a restart starts (`before_restart`): "1" when the window was hidden.
const RESTART_HIDDEN: &str = "QAMILLE_RESTART_HIDDEN";

/// «Esci» is closing the window: the close goes through the page's guards instead of hiding.
static QUITTING: AtomicBool = AtomicBool::new(false);
/// The main window was minimized at its last resize (`window_resized`).
static MINIMIZED: AtomicBool = AtomicBool::new(false);
/// The name of this copy's start-at-sign-in entry (`autostart_entry`).
static ENTRY: OnceLock<String> = OnceLock::new();
/// This start is a restart: whether the window was hidden then (`take_restart_state`).
static RESTARTED_HIDDEN: OnceLock<Option<bool>> = OnceLock::new();

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
            "Qamille resta aperto nell'area di notifica: invia i messaggi programmati, mostra i promemoria e riceve la posta. Per riaprire la finestra fai clic sull'icona di Qamille (in Windows 11 può essere fra le icone nascoste, sotto la freccia ^) oppure avvia di nuovo Qamille. Per uscire usa Esci dal menu dell'icona.",
            "Qamille keeps running in the notification area: it sends scheduled messages, shows reminders and receives mail. To bring the window back, click Qamille's icon (in Windows 11 it may be among the hidden icons, under the ^ arrow) or start Qamille again. To quit, use Quit in the icon's menu.",
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
/// that kept the window during «Esci». The work runs on the main thread (`restore_on_main`): called
/// from another thread (a toast's click, an async command, the close watch), the Win32 show of a
/// maximized first show (`window_shell::show_main`) would run at once while `SetIsVisible` and tao's
/// `show()` wait in the event loop, and the window would show one frame of its ground. The caller
/// waits for it, so its next step (a question, the page's open event) comes with the window in view;
/// on the main thread Tauri runs it at once.
pub fn restore(app: &AppHandle) {
    let (done, finished) = std::sync::mpsc::channel();
    let handle = app.clone();
    let sent = app.run_on_main_thread(move || {
        restore_on_main(&handle);
        let _ = done.send(());
    });
    if sent.is_ok() {
        // Bounded: an event loop held by a modal loop of its own never keeps the caller for long.
        let _ = finished.recv_timeout(RESTORE_WAIT);
    }
}

/// How long `restore` waits for the main thread.
const RESTORE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// `restore`, on the main thread. The webview is told it is visible before the window shows, so the
/// first frame is the page, then the window leaves the taskbar and comes to the front.
fn restore_on_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    crate::native_feel::set_webview_visible(&window, true);
    // `show_main` opens a maximized saved placement maximized, the first time (rule 29).
    crate::window_shell::show_main(&window);
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

/// Qamille started again while it runs: the window comes back, then a mailto: link it was started
/// with goes to the composer (`launch_args`). `args` and `cwd` are the second start's.
pub fn second_instance(app: &AppHandle, args: Vec<String>, cwd: String) {
    let _ = &cwd;
    restore(app);
    crate::launch_args::route(app, &args);
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

/// The name of this copy's start-at-sign-in entry: «Qamille» when installed, else the folder of the
/// executable and the stable tag of the data folder (`workspace_tag`), so every copy has its own (Q22).
fn autostart_entry() -> &'static str {
    ENTRY.get_or_init(|| {
        let folder = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().and_then(|folder| folder.file_name()).map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let data = qamille::model::Store::data_dir();
        let data = data.canonicalize().unwrap_or(data);
        background_rules::autostart_entry_name(installed(), &folder, &background_rules::workspace_tag(&data.to_string_lossy()))
    })
}

/// Just before Qamille restarts itself (`recovery::restart_app`; an updater must call it too): the
/// new process gets the arguments of this one, `AUTOSTART_ARG` included, so it is told whether the
/// window was hidden. A window on screen comes back on screen, one in the tray stays there.
pub fn before_restart(app: &AppHandle) {
    let hidden = app.get_webview_window("main").is_some_and(|window| !window.is_visible().unwrap_or(true));
    // SAFETY: as `recovery::restart_app` sets QAMILLE_RESTART_AFTER: right before the restart, for the
    // next process only, which reads it at its start (`take_restart_state`).
    unsafe { std::env::set_var(RESTART_HIDDEN, if hidden { "1" } else { "0" }) };
}

/// First thing in `main`, before any thread: whether this start is a restart, and how the window was.
pub fn take_restart_state() {
    let state = std::env::var(RESTART_HIDDEN).ok().map(|value| value == "1");
    if state.is_some() {
        // SAFETY: called at the start of `main`, before any other thread exists.
        unsafe { std::env::remove_var(RESTART_HIDDEN) };
    }
    let _ = RESTARTED_HIDDEN.set(state);
}

/// `Some(hidden)` when this start is a restart (`before_restart`), else `None`.
pub fn restarted_hidden() -> Option<bool> {
    RESTARTED_HIDDEN.get().copied().flatten()
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
    // Ids of its own: every menu event reaches every handler, the native row menu's too.
    let show = MenuItem::with_id(app, "tray:show", show_text, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "tray:quit", quit_text, true, None::<&str>)?;
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
            "tray:show" => restore(app),
            "tray:quit" => quit(app),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(TrayMenu { show, quit: quit_item });
    Ok(())
}

/// The interface language changed (`window_shell::window_appearance`): the tray menu and the tooltip
/// (the unread count, `taskbar::tooltip`) follow it at once.
pub fn relabel(app: &AppHandle, italian: bool) {
    let Some(menu) = app.try_state::<TrayMenu>() else { return };
    let (show_text, quit_text) = tray_labels(italian);
    let _ = menu.show.set_text(show_text);
    let _ = menu.quit.set_text(quit_text);
    set_tray_tooltip(app, &crate::taskbar::tooltip(italian));
}

/// The tray icon's tooltip (it says «Qamille» until the unread count says more, `taskbar.rs`).
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

/// The start-at-sign-in entry, in the registry of the current user only: the value under `Run` that
/// Windows starts at sign-in, and its twin under `StartupApproved\Run`, where Task Manager and
/// Settings > Apps > Startup keep their switch (`background_rules::startup_approved`). The keys are
/// parameters so that the tests use a scratch key of their own.
pub(crate) mod startup {
    use crate::background_rules::{APPROVED_ON, startup_approved};

    pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    pub const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    pub struct StartupKeys<'a> {
        pub run: &'a str,
        pub approved: &'a str,
    }
    /// Windows' own keys, under `HKEY_CURRENT_USER`.
    pub const WINDOWS: StartupKeys<'static> = StartupKeys { run: RUN, approved: APPROVED };

    #[cfg(windows)]
    mod win {
        use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
        use windows::Win32::System::Registry::{
            HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, REG_VALUE_TYPE, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
        };
        use windows::core::HSTRING;

        pub fn read(key: &str, name: &str, kind: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
            let (key, name) = (HSTRING::from(key), HSTRING::from(name));
            let mut size = 0u32;
            // SAFETY: the strings live across both calls, and the buffer has the size the first call gave.
            unsafe {
                RegGetValueW(HKEY_CURRENT_USER, &key, &name, kind, None, None, Some(&mut size)).ok().ok()?;
                let mut data = vec![0u8; size as usize];
                RegGetValueW(HKEY_CURRENT_USER, &key, &name, kind, None, Some(data.as_mut_ptr().cast()), Some(&mut size)).ok().ok()?;
                data.truncate(size as usize);
                Some(data)
            }
        }

        /// Creates the key if it is missing (`RegSetKeyValueW`).
        pub fn write(key: &str, name: &str, kind: REG_VALUE_TYPE, data: &[u8]) -> windows::core::Result<()> {
            let (key, name) = (HSTRING::from(key), HSTRING::from(name));
            // SAFETY: `data` lives across the call and its length is the one passed.
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, &name, kind.0, Some(data.as_ptr().cast()), data.len() as u32).ok() }
        }

        /// A value, or a key, that is not there is already removed.
        pub fn delete(key: &str, name: &str) -> windows::core::Result<()> {
            let (key, name) = (HSTRING::from(key), HSTRING::from(name));
            // SAFETY: the strings live across the call.
            let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, &name) };
            if status == ERROR_FILE_NOT_FOUND { Ok(()) } else { status.ok() }
        }
    }

    /// The command of the entry, as Windows will run it.
    pub fn command(keys: &StartupKeys, name: &str) -> Option<String> {
        #[cfg(windows)]
        {
            let bytes = win::read(keys.run, name, windows::Win32::System::Registry::RRF_RT_REG_SZ)?;
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            return Some(String::from_utf16_lossy(&wide).trim_end_matches('\0').to_owned());
        }
        #[cfg(not(windows))]
        None
    }

    /// The entry's twin under `StartupApproved\Run`, if there is one.
    pub fn approval(keys: &StartupKeys, name: &str) -> Option<Vec<u8>> {
        #[cfg(windows)]
        return win::read(keys.approved, name, windows::Win32::System::Registry::RRF_RT_REG_BINARY);
        #[cfg(not(windows))]
        None
    }

    /// Task Manager's switch for the entry (Qamille writes it on, as Task Manager does).
    pub fn write_approval(keys: &StartupKeys, name: &str, twin: &[u8]) -> windows::core::Result<()> {
        #[cfg(windows)]
        return win::write(keys.approved, name, windows::Win32::System::Registry::REG_BINARY, twin);
        #[cfg(not(windows))]
        Ok(())
    }

    /// Writes the entry and turns its switch on: an entry turned off in Task Manager runs again.
    pub fn enable(keys: &StartupKeys, name: &str, command: &str) -> windows::core::Result<()> {
        #[cfg(windows)]
        {
            let mut data: Vec<u8> = command.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
            data.shrink_to_fit();
            win::write(keys.run, name, windows::Win32::System::Registry::REG_SZ, &data)?;
        }
        write_approval(keys, name, &APPROVED_ON)
    }

    /// Removes the entry and its twin, as the uninstaller does.
    pub fn disable(keys: &StartupKeys, name: &str) -> windows::core::Result<()> {
        #[cfg(windows)]
        {
            win::delete(keys.run, name)?;
            win::delete(keys.approved, name)?;
        }
        Ok(())
    }

    /// Whether the entry exists, on or off.
    pub fn present(keys: &StartupKeys, name: &str) -> bool {
        command(keys, name).is_some()
    }

    /// Whether Windows starts it at the next sign-in.
    pub fn enabled(keys: &StartupKeys, name: &str) -> bool {
        present(keys, name) && startup_approved(approval(keys, name).as_deref())
    }
}

/// Turns this copy's entry on: `"<exe>" --autostart`, under its name, for the current user.
fn turn_on() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let command = background_rules::run_command(&exe.to_string_lossy());
    startup::enable(&startup::WINDOWS, autostart_entry(), &command).map_err(|error| error.to_string())
}

/// Windows has the last word: when this copy's entry exists, whether it is on (Task Manager's switch
/// included) is what Qamille remembers, so an entry turned off there stays off after an upgrade.
/// `None` when there is no entry.
fn follow_windows() -> Option<bool> {
    if !startup::present(&startup::WINDOWS, autostart_entry()) {
        return None;
    }
    let enabled = startup::enabled(&startup::WINDOWS, autostart_entry());
    if enabled != preferences().start_at_sign_in {
        set_start_at_sign_in(enabled);
    }
    Some(enabled)
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
                LegacyLauncher::Adopt => match turn_on() {
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
    put_back_after_upgrade();
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
fn put_back_after_upgrade() {
    if follow_windows().is_some() {
        return;
    }
    if !installed() || !preferences().start_at_sign_in {
        if !installed() && preferences().start_at_sign_in {
            set_start_at_sign_in(false);
        }
        return;
    }
    if let Err(error) = turn_on() {
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
    // Read from Windows, and remembered as Windows has it (a switch turned off in Task Manager).
    let enabled = supported && follow_windows().unwrap_or(false);
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
    let result = if enabled {
        turn_on()
    } else {
        startup::disable(&startup::WINDOWS, autostart_entry()).map_err(|error| error.to_string())
    };
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

#[cfg(all(test, windows))]
mod tests {
    use super::startup::{self, StartupKeys};
    use crate::background_rules::{APPROVED_ON, run_command};
    use crate::test_registry::ScratchKey;

    /// The test's own key (`test_registry`), with a `Run` and a `StartupApproved\Run` under it: the
    /// tests never touch Windows' own `Run` key, nor the entries of the owner's copies.
    struct Scratch {
        run: String,
        approved: String,
        _key: ScratchKey,
    }
    impl Scratch {
        fn new(test: &str) -> Self {
            let key = ScratchKey::new("background", test);
            Self { run: key.under("Run"), approved: key.under(r"StartupApproved\Run"), _key: key }
        }
        fn keys(&self) -> StartupKeys<'_> {
            StartupKeys { run: &self.run, approved: &self.approved }
        }
    }

    const NAME: &str = "Qamille (Qamille Portable, 6a5c8aa2)";

    #[test]
    fn the_entry_is_written_quoted_and_reads_back_on() {
        let scratch = Scratch::new("write");
        let keys = scratch.keys();
        assert!(!startup::present(&keys, NAME) && !startup::enabled(&keys, NAME));
        // The keys do not exist yet: turning it on creates them.
        startup::enable(&keys, NAME, &run_command(r"D:\Apps\Qamille Portable\Qamille.exe")).unwrap();
        assert_eq!(startup::command(&keys, NAME).as_deref(), Some(r#""D:\Apps\Qamille Portable\Qamille.exe" --autostart"#));
        assert_eq!(startup::approval(&keys, NAME), Some(APPROVED_ON.to_vec()));
        assert!(startup::present(&keys, NAME) && startup::enabled(&keys, NAME));
        // Another copy's entry is another value.
        assert!(!startup::present(&keys, "Qamille"));
    }

    #[test]
    fn the_task_manager_switch_is_honoured_and_turning_it_on_again_clears_it() {
        let scratch = Scratch::new("switch");
        let keys = scratch.keys();
        startup::enable(&keys, NAME, &run_command(r"C:\Qamille\Qamille.exe")).unwrap();
        // What Task Manager writes when the entry is turned off there.
        startup::write_approval(&keys, NAME, &[0x03, 0, 0, 0, 0x10, 0x5c, 0x3f, 0x7a, 0x2e, 0x1d, 0xdb, 0x01]).unwrap();
        assert!(startup::present(&keys, NAME), "the value stays: Task Manager only flips its switch");
        assert!(!startup::enabled(&keys, NAME));
        startup::enable(&keys, NAME, &run_command(r"C:\Qamille\Qamille.exe")).unwrap();
        assert!(startup::enabled(&keys, NAME));
    }

    #[test]
    fn turning_it_off_removes_both_values_and_twice_is_fine() {
        let scratch = Scratch::new("off");
        let keys = scratch.keys();
        startup::disable(&keys, NAME).unwrap();
        startup::enable(&keys, NAME, &run_command(r"C:\Qamille\Qamille.exe")).unwrap();
        startup::disable(&keys, NAME).unwrap();
        assert_eq!(startup::command(&keys, NAME), None);
        assert_eq!(startup::approval(&keys, NAME), None);
        assert!(!startup::present(&keys, NAME) && !startup::enabled(&keys, NAME));
        startup::disable(&keys, NAME).unwrap();
    }
}
