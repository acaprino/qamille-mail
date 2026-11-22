//! What a Windows title bar does and the page cannot: the system menu
//! (Alt+Space, right-click on the bar), snap layouts on the maximize button,
//! saving at sign-out or shutdown, the system print dialog, zoom with the
//! Windows text size, and the window theme chosen in Settings. See
//! docs/aspetto-nativo.md and docs/audit-tauri-nativo.md (F2, F4, F7, F8, F9,
//! F11, F12). The pure rules live in `shell_rules.rs`, tested without Windows.
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

use crate::placement::{self, Placement, SavedPlacement, ShowPath, WorkArea};
use crate::shell_rules::{self, Appearance, ZoomStep};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Theme, WebviewWindow, WindowEvent};

/// Sent to the main window: `last: false` when Windows asks whether it may end
/// the session (save, change nothing irreversible), `last: true` when it does
/// (also stop the recording), after which the page calls `session_end_ready`.
pub const SESSION_ENDING: &str = "session-ending";
/// Hover and press of the maximize button while Windows handles its mouse (F7).
pub const MAXIMIZE_BUTTON: &str = "window-maximize-button";

#[derive(Clone, Serialize)]
pub struct SessionEnding {
    pub last: bool,
}

/// The window preferences Rust must know before the page has loaded: the
/// appearance (to paint the first frame), the app zoom and where the window was.
/// The page keeps its own copy of the appearance in localStorage for
/// `theme-boot.js`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct WindowPreferences {
    #[serde(default = "system")]
    appearance: String,
    #[serde(default = "unit")]
    zoom: f64,
    /// The interface language, for what Rust shows when the page cannot: the
    /// system menu entry and the recovery dialogs (`recovery.rs`).
    #[serde(default)]
    italian: Option<bool>,
    /// Where the window was when it last closed (`placement.rs`). A file from an
    /// older version has none, and one that cannot be read does not cost the
    /// appearance and the zoom: it reads as none.
    #[serde(default, deserialize_with = "lenient_placement", skip_serializing_if = "Option::is_none")]
    placement: Option<StoredPlacement>,
}
fn system() -> String {
    "system".into()
}
fn unit() -> f64 {
    1.0
}
impl Default for WindowPreferences {
    fn default() -> Self {
        Self { appearance: system(), zoom: unit(), italian: None, placement: None }
    }
}

/// `placement::SavedPlacement` as it is written in `window-preferences.json`
/// (`placement.rs` has no dependencies, serde included, so it is tested alone).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
struct StoredPlacement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    #[serde(default)]
    maximized: bool,
}
impl From<SavedPlacement> for StoredPlacement {
    fn from(saved: SavedPlacement) -> Self {
        Self { x: saved.x, y: saved.y, width: saved.width, height: saved.height, scale: saved.scale, maximized: saved.maximized }
    }
}
impl From<StoredPlacement> for SavedPlacement {
    fn from(stored: StoredPlacement) -> Self {
        Self { x: stored.x, y: stored.y, width: stored.width, height: stored.height, scale: stored.scale, maximized: stored.maximized }
    }
}

/// A `placement` of the wrong shape (a hand edit) reads as none instead of failing the whole file.
fn lenient_placement<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<StoredPlacement>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

static PREFERENCES: Mutex<Option<WindowPreferences>> = Mutex::new(None);

fn preferences_path() -> PathBuf {
    qamille::model::Store::data_dir().join("window-preferences.json")
}

fn load_preferences() -> WindowPreferences {
    std::fs::read_to_string(preferences_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn preferences() -> WindowPreferences {
    let mut cached = PREFERENCES.lock().unwrap_or_else(|e| e.into_inner());
    cached.get_or_insert_with(load_preferences).clone()
}

/// Changes the preferences while holding the lock and writes them if they changed. Every change goes
/// through here, so that the commands (appearance, zoom) and the placement saver, which runs on its
/// own thread, never write back an older copy of what the other one has just saved.
fn update_preferences<R>(change: impl FnOnce(&mut WindowPreferences) -> R) -> R {
    let mut cached = PREFERENCES.lock().unwrap_or_else(|e| e.into_inner());
    let mut next = cached.clone().unwrap_or_else(load_preferences);
    let result = change(&mut next);
    if cached.as_ref() != Some(&next) {
        write_preferences(&next);
        *cached = Some(next);
    }
    result
}

/// Cosmetic: if the file cannot be written the next start uses the system theme
/// and the default window. It is written beside the file and renamed over it, so
/// an exit in the middle of a write (the placement is saved as the window closes)
/// never leaves half a file, which would read as no preferences at all.
fn write_preferences(next: &WindowPreferences) {
    let Ok(text) = serde_json::to_string(next) else { return };
    let path = preferences_path();
    let partial = path.with_extension("json.tmp");
    if std::fs::write(&partial, text).and_then(|()| std::fs::rename(&partial, &path)).is_err() {
        let _ = std::fs::remove_file(&partial);
    }
}

fn theme_of(appearance: Appearance) -> Option<Theme> {
    match appearance {
        Appearance::Dark => Some(Theme::Dark),
        Appearance::Light => Some(Theme::Light),
        Appearance::System => None,
    }
}

/// Before the hidden main window is shown: the appearance chosen in Settings
/// (title bar, WebView2 colour scheme, first frame), the zoom and where the
/// window was. The caller shows it afterwards (or restores it from the tray).
pub fn apply_startup_preferences(window: &WebviewWindow) -> bool {
    let saved = preferences();
    let appearance = shell_rules::parse_appearance(&saved.appearance);
    if let Some(theme) = theme_of(appearance) {
        let _ = window.set_theme(Some(theme));
    }
    let zoom = shell_rules::effective_zoom(saved.zoom, text_scale_factor());
    if (zoom - 1.0).abs() > f64::EPSILON {
        let _ = window.set_zoom(zoom);
    }
    // Not maximized here: tao sets the maximized style of a hidden window without moving it to the
    // monitor's bounds, and the window then shows at the wrong origin. The first `show_main` opens a
    // maximized placement.
    *pending_show() = saved.placement.and_then(|stored| apply_placement(window, &stored.into()));
    remember_start_rectangle(window);
    let system_dark = matches!(window.theme(), Ok(Theme::Dark));
    shell_rules::starts_dark(appearance, system_dark)
}

/// The least the main window may measure, in logical pixels: `minWidth` and
/// `minHeight` of its window config (1000 x 640 in `tauri.conf.json`).
fn min_logical_size(window: &WebviewWindow) -> (u32, u32) {
    let config = window.app_handle().config();
    let main = config.app.windows.iter().find(|candidate| candidate.label == window.label());
    let least = |value: Option<f64>, fallback: u32| value.filter(|v| v.is_finite() && *v > 0.0).map_or(fallback, |v| v.ceil() as u32);
    (least(main.and_then(|m| m.min_width), 1000), least(main.and_then(|m| m.min_height), 640))
}

/// Puts the hidden window where it was, if that is still on a connected monitor
/// (`placement::restore`; otherwise the window stays where the window config
/// centred it). Returns what it applied, `None` when it applied nothing.
fn apply_placement(window: &WebviewWindow, saved: &SavedPlacement) -> Option<Placement> {
    let Ok(monitors) = window.available_monitors() else { return None };
    let areas: Vec<WorkArea> = monitors
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            WorkArea {
                x: area.position.x,
                y: area.position.y,
                width: area.size.width,
                height: area.size.height,
                scale: monitor.scale_factor(),
            }
        })
        .collect();
    let target = placement::restore(saved, &areas, min_logical_size(window))?;
    // The position first, and again after the size: a move to a monitor with another scale is
    // settled by Windows (WM_DPICHANGED resizes the window) before the size is set, which would
    // otherwise be scaled once more.
    let position = PhysicalPosition::new(target.x, target.y);
    let _ = window.set_position(position);
    let _ = window.set_size(PhysicalSize::new(target.width, target.height));
    let _ = window.set_position(position);
    Some(target)
}

/// What `apply_placement` applied, until the window is first shown (`show_main`).
static PENDING_SHOW: Mutex<Option<Placement>> = Mutex::new(None);

fn pending_show() -> std::sync::MutexGuard<'static, Option<Placement>> {
    PENDING_SHOW.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shows the main window, the one place that does (a normal start and `background::restore`, which
/// the tray, a second start and a notification come through). The first show of a run is the saved
/// placement's: a maximized one opens maximized on the monitor it was on, in one step, and comes back
/// to the rectangle applied while hidden (`apply_placement`) when restored. Any later show is a plain
/// `show()`: the window is as the user left it.
///
/// Win32 shows the window (`SetWindowPlacement` with `SW_SHOWMAXIMIZED`: Windows maximizes it on its
/// monitor, with no frame at the normal size and no jump) and tao is told with `show()` right after,
/// so its visible flag matches the window. With the flag left behind, tao's next change of a window
/// flag (the maximize button) would hide the window. tao's `show()` itself, still thinking the window
/// hidden, rewrites its style without `WS_VISIBLE` and shows it again: the window would vanish for a
/// frame or two (seen on the exe: 16-44 ms). So DWM keeps it out of sight (cloaked: composed, never
/// on screen) from before the Win32 show until tao is told, and it appears once, maximized.
pub fn show_main(window: &WebviewWindow) {
    let path = placement::first_show(&mut pending_show());
    #[cfg(windows)]
    if path == ShowPath::Maximized
        && let Ok(hwnd) = window.hwnd()
    {
        let cloaked = win::cloak(hwnd, true);
        let shown = win::show_maximized(hwnd);
        if shown {
            let _ = window.show();
            // Safety net: if tao does not see the window maximized after the Win32 call, maximize it
            // through tao, so its state and the maximize button stay right.
            if !window.is_maximized().unwrap_or(true) {
                let _ = window.maximize();
            }
        }
        // Always, whatever happened above: a window left cloaked would be on screen and invisible.
        if cloaked {
            win::cloak(hwnd, false);
        }
        if shown {
            return;
        }
    }
    let _ = window.show();
    if path == ShowPath::Maximized {
        // Win32 refused (or this is not Windows): maximize now, shown. A frame at the normal size
        // first, but a window that is maximized.
        let _ = window.maximize();
    }
}

/// Settings → Appearance or the language changed: the title bar, WebView2's
/// colour scheme (context menu, scrollbars, form controls), the system menu
/// entry and the next start follow it.
#[tauri::command]
pub fn window_appearance(window: WebviewWindow, appearance: String, italian: Option<bool>) {
    let parsed = shell_rules::parse_appearance(&appearance);
    let _ = window.set_theme(theme_of(parsed));
    update_preferences(|next| {
        next.appearance = shell_rules::appearance_name(parsed).into();
        if italian.is_some() {
            next.italian = italian;
        }
    });
    crate::background::relabel(tauri::Manager::app_handle(&window), italian.unwrap_or_else(self::italian));
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        win::label_reload_entry(hwnd, self::italian());
    }
}

/// The interface language as the page last reported it; Italian before it has.
pub fn italian() -> bool {
    preferences().italian.unwrap_or(true)
}

/// Before the main window exists: whether its first frame is dark, from the
/// appearance chosen in Settings or, with "Sistema", the Windows app theme.
pub fn starts_dark() -> bool {
    let appearance = shell_rules::parse_appearance(&preferences().appearance);
    #[cfg(windows)]
    let system_dark = win::apps_use_dark_theme();
    #[cfg(not(windows))]
    let system_dark = false;
    shell_rules::starts_dark(appearance, system_dark)
}

#[derive(Serialize)]
pub struct ZoomState {
    /// What the webview shows: the app zoom times the Windows text size.
    pub zoom: f64,
    /// The app's own zoom, the one Ctrl+= / Ctrl+- / Ctrl+0 change.
    pub user: f64,
    pub text_scale: f64,
}

/// Ctrl+= / Ctrl+- / Ctrl+0 (`step` "in", "out", "reset"), or no step to pick up
/// a changed Windows text size. Applies and saves the zoom.
#[tauri::command]
pub fn window_zoom(window: WebviewWindow, step: Option<String>) -> tauri::Result<ZoomState> {
    let current = preferences().zoom;
    let user = match step.as_deref() {
        Some("in") => shell_rules::next_zoom(current, ZoomStep::In),
        Some("out") => shell_rules::next_zoom(current, ZoomStep::Out),
        Some("reset") => shell_rules::next_zoom(current, ZoomStep::Reset),
        _ => shell_rules::clamp_user_zoom(current),
    };
    let text_scale = text_scale_factor();
    let zoom = shell_rules::effective_zoom(user, text_scale);
    window.set_zoom(zoom)?;
    update_preferences(|next| next.zoom = user);
    Ok(ZoomState { zoom, user, text_scale })
}

/// Settings → Accessibility → Text size, 1.0 to 2.25 (1.0 off Windows).
pub fn text_scale_factor() -> f64 {
    #[cfg(windows)]
    return shell_rules::text_scale(win::text_scale_percent());
    #[cfg(not(windows))]
    return shell_rules::text_scale(None);
}

/// The page's answer to `session-ending { last: true }`: everything is saved.
#[tauri::command]
pub fn session_end_ready() {
    #[cfg(windows)]
    win::SESSION_SAVED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Right-click on the title bar (F4): the Windows system menu at that point.
/// `x`, `y` are client pixels (CSS pixels × devicePixelRatio).
#[tauri::command]
pub fn show_system_menu(window: WebviewWindow, x: f64, y: f64) -> tauri::Result<()> {
    #[cfg(windows)]
    win::post_system_menu(window.hwnd()?, Some((x.round() as i32, y.round() as i32)));
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ButtonRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the page draws its maximize button, in client pixels, or `None` when
/// it is not on screen. Windows then shows the snap layouts over it (F7).
#[tauri::command]
pub fn window_snap_button(window: WebviewWindow, rect: Option<ButtonRect>) -> tauri::Result<()> {
    let rect = rect.and_then(|r| shell_rules::pixel_rect(r.x, r.y, r.width, r.height));
    #[cfg(windows)]
    {
        let hwnd = window.hwnd()?.0 as isize;
        window.run_on_main_thread(move || win::place_snap_button(hwnd, rect))?;
    }
    Ok(())
}

/// Main window only, right after it is built: system menu keys and its
/// "Ricarica la finestra" entry, session end, the maximize button's hit test
/// and the saving of where the window is.
pub fn attach_main(window: &WebviewWindow) {
    #[cfg(windows)]
    win::attach_main(window);
    watch_placement(window);
}

/// How long after the last move, resize or scale change the placement is
/// written to disk (a drag makes dozens of events a second).
const PLACEMENT_DELAY: Duration = Duration::from_millis(500);

/// What the placement saver knows of the main window.
#[derive(Clone, Copy)]
struct Tracked {
    /// The last normal (not maximized) rectangle the window had: a maximized window reports the
    /// screen, not the rectangle it comes back to when restored.
    normal: Option<SavedPlacement>,
    maximized: bool,
    /// The window has been on screen in this run. A start in the tray never shows it, and then it
    /// has nothing to say about where it was.
    seen: bool,
}

static TRACKED: Mutex<Tracked> = Mutex::new(Tracked { normal: None, maximized: false, seen: false });
static PLACEMENT_EVENTS: AtomicU64 = AtomicU64::new(0);
static PLACEMENT_SAVER: AtomicBool = AtomicBool::new(false);

fn tracker() -> std::sync::MutexGuard<'static, Tracked> {
    TRACKED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Saves where the main window is: 500 ms after a move, resize or scale change,
/// and at once when it is asked to close and when it is destroyed. Its own
/// listener, so `main.rs` has nothing to add. Errors are ignored, as for the theme and the zoom:
/// the window then opens centred.
fn watch_placement(window: &WebviewWindow) {
    let watched = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            track_placement(&watched);
            save_placement_soon();
        }
        // Still on screen here, whether the close goes on, is vetoed by the page or hides the window.
        WindowEvent::CloseRequested { .. } => {
            track_placement(&watched);
            save_placement();
        }
        // The window is gone: only what was tracked before is left.
        WindowEvent::Destroyed => save_placement(),
        _ => {}
    });
}

/// The rectangle the window has now, if it is one to come back to (`placement::capture`).
fn current_rectangle(window: &WebviewWindow) -> Option<SavedPlacement> {
    let position = window.outer_position().ok()?;
    let size = window.inner_size().ok()?;
    placement::capture(position.x, position.y, size.width, size.height, window.scale_factor().ok()?)
}

/// Whether the window is maximized. Windows is asked as well as tao, whose flag only changes with
/// WM_SIZE: a maximize moves the window (WM_MOVE) first, and then the screen-sized rectangle
/// would be taken for the normal one.
fn is_maximized(window: &WebviewWindow) -> Option<bool> {
    let maximized = window.is_maximized().ok()?;
    #[cfg(windows)]
    let maximized = maximized || window.hwnd().is_ok_and(win::is_zoomed);
    Some(maximized)
}

/// What the window was given at startup, which the saver falls back on when the
/// window is maximized and nothing else has been seen. It does not count as seen.
fn remember_start_rectangle(window: &WebviewWindow) {
    if let Some(rectangle) = current_rectangle(window) {
        tracker().normal = Some(rectangle);
    }
}

/// Takes in the window's state after a move, resize or scale change. Never from a
/// minimized window (Windows parks it off screen) or a hidden one (in the tray):
/// neither is somewhere to reopen.
fn track_placement(window: &WebviewWindow) {
    if window.is_minimized().unwrap_or(true) || !window.is_visible().unwrap_or(false) {
        return;
    }
    let Some(maximized) = is_maximized(window) else { return };
    let rectangle = if maximized { None } else { current_rectangle(window) };
    let mut tracked = tracker();
    tracked.seen = true;
    tracked.maximized = maximized;
    if rectangle.is_some() {
        tracked.normal = rectangle;
    }
}

/// Writes the tracked placement into the preferences file.
fn save_placement() {
    let current = *tracker();
    if !current.seen {
        return;
    }
    update_preferences(|saved| {
        let previous = saved.placement.map(SavedPlacement::from);
        saved.placement = placement::to_store(current.normal, current.maximized, previous).map(StoredPlacement::from);
    });
}

/// `save_placement` once the window has been still for `PLACEMENT_DELAY`.
fn save_placement_soon() {
    PLACEMENT_EVENTS.fetch_add(1, Ordering::SeqCst);
    if PLACEMENT_SAVER.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("qamille-placement".into()).spawn(|| {
        loop {
            let seen = PLACEMENT_EVENTS.load(Ordering::SeqCst);
            std::thread::sleep(PLACEMENT_DELAY);
            if PLACEMENT_EVENTS.load(Ordering::SeqCst) == seen {
                break;
            }
        }
        PLACEMENT_SAVER.store(false, Ordering::SeqCst);
        save_placement();
    });
    if spawned.is_err() {
        PLACEMENT_SAVER.store(false, Ordering::SeqCst);
    }
}

/// The reminder chip is a tool window: out of Alt+Tab and Win+Tab (F11).
pub fn make_tool_window(window: &WebviewWindow) {
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        win::make_tool_window(hwnd);
    }
}

/// "Stampa" in the app's preview opens the Windows print dialog, not Edge's
/// print preview on top of the app's own (F8). Without it (an old WebView2
/// runtime) the page's `window.print()` is the fallback.
pub fn print_with_system_dialog(window: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    {
        let fallback = window.clone();
        return window.with_webview(move |webview| {
            // SAFETY: `with_webview` runs on the thread that owns the controller.
            if unsafe { win::show_system_print(&webview.controller()) }.is_err() {
                let _ = fallback.print();
            }
        });
    }
    #[cfg(not(windows))]
    window.print()
}

fn emit_soon<S: Serialize + Clone + Send + 'static>(app: &AppHandle, event: &'static str, payload: S) {
    // Never from inside the window procedure itself: Tauri would run the
    // script there and then, possibly while it is busy with this very window.
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = app.emit_to("main", event, payload);
    });
}

#[cfg(windows)]
mod win {
    use super::{MAXIMIZE_BUTTON, SESSION_ENDING, SessionEnding, emit_soon};
    use crate::shell_rules::{self, ButtonState, PixelRect};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Manager, WebviewWindow};
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
        COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM, ICoreWebView2_16, ICoreWebView2Controller,
    };
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::{BOOL, PCWSTR, w};
    use windows_core::Interface;

    /// Our message to the main window: open the system menu. `wParam` 1 = at
    /// `lParam` (a packed screen point), 0 = where Alt+Space opens it.
    const WM_SHOW_SYSTEM_MENU: u32 = WM_APP + 0x5C1;
    const SUBCLASS_ID: usize = 0x5C1_0001;
    const SNAP_CLASS: PCWSTR = w!("QAMILLE_SNAP_BUTTON");
    /// What Windows waits for us at the end of a session before tao exits the
    /// process. It shows "Qamille: saving…" meanwhile and lets the user end
    /// the session anyway.
    const SESSION_WAIT: Duration = Duration::from_secs(8);

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static MAIN: AtomicIsize = AtomicIsize::new(0);
    static SNAP: AtomicIsize = AtomicIsize::new(0);
    static SNAP_STATE: AtomicU8 = AtomicU8::new(0);
    static SESSION_QUERIED: AtomicBool = AtomicBool::new(false);
    static SESSION_ENDING_SENT: AtomicBool = AtomicBool::new(false);
    pub static SESSION_SAVED: AtomicBool = AtomicBool::new(false);
    static SESSION_DEADLINE: OnceLock<Instant> = OnceLock::new();

    fn hwnd_of(value: isize) -> Option<HWND> {
        (value != 0).then_some(HWND(value as _))
    }

    pub fn is_zoomed(hwnd: HWND) -> bool {
        unsafe { IsZoomed(hwnd) }.as_bool()
    }

    /// Shows the hidden window maximized in one step: its own placement (the rectangle `apply_placement`
    /// gave it, in the workspace coordinates Windows wants, which a round trip through
    /// `GetWindowPlacement` keeps right) with the show command `SW_SHOWMAXIMIZED`. Windows maximizes
    /// it on the monitor the rectangle is on, and un-maximizing returns to that rectangle. Whether it
    /// worked, or the window is as it was.
    /// DWM's cloak on the window (`DWMWA_CLOAK`): on, the window is composed but not shown. Whether
    /// DWM took it.
    pub fn cloak(hwnd: HWND, on: bool) -> bool {
        use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
        let value = BOOL::from(on);
        // SAFETY: `value` lives across the call, and the size passed is its own.
        unsafe { DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, (&raw const value).cast(), std::mem::size_of::<BOOL>() as u32).is_ok() }
    }

    pub fn show_maximized(hwnd: HWND) -> bool {
        let mut window_placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        unsafe {
            if GetWindowPlacement(hwnd, &mut window_placement).is_err() {
                return false;
            }
            window_placement.showCmd = SW_SHOWMAXIMIZED.0 as u32;
            SetWindowPlacement(hwnd, &window_placement).is_ok()
        }
    }

    pub fn attach_main(window: &WebviewWindow) {
        let Ok(hwnd) = window.hwnd() else { return };
        let _ = APP.set(window.app_handle().clone());
        MAIN.store(hwnd.0 as isize, Ordering::SeqCst);
        add_reload_entry(hwnd, super::italian());
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(main_proc), SUBCLASS_ID, 0);
            // tao exits the process from its hidden event window on WM_ENDSESSION,
            // which may get the message before the main window does.
            let _ = EnumThreadWindows(GetCurrentThreadId(), Some(subclass_event_window), LPARAM(0));
        }
        let raw = hwnd.0 as isize;
        let result = window.with_webview(move |webview| {
            // SAFETY: on the thread that owns the controller.
            if let Err(error) = unsafe { alt_space(&webview.controller(), raw) } {
                crate::native_feel::report(&format!("Alt+Space will not open the system menu: {error}"));
            }
        });
        if let Err(error) = result {
            crate::native_feel::report(&format!("webview unavailable for the system menu: {error}"));
        }
    }

    unsafe extern "system" fn subclass_event_window(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut name = [0u16; 64];
        let length = unsafe { GetClassNameW(hwnd, &mut name) }.max(0) as usize;
        if String::from_utf16_lossy(&name[..length]) == "Tao Thread Event Target" {
            unsafe {
                let _ = SetWindowSubclass(hwnd, Some(main_proc), SUBCLASS_ID, 0);
            }
        }
        BOOL(1)
    }

    /// Alt+Space with the focus in the page: WebView2 keeps system keys for
    /// itself, so the host never sees WM_SYSKEYDOWN (WebView2Feedback #1703).
    unsafe fn alt_space(controller: &ICoreWebView2Controller, hwnd: isize) -> windows_core::Result<()> {
        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            unsafe {
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
                args.KeyEventKind(&mut kind)?;
                let mut key = 0u32;
                args.VirtualKey(&mut key)?;
                if kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN && key == u32::from(VK_SPACE_CODE) {
                    args.SetHandled(true)?;
                    post_system_menu(HWND(hwnd as _), None);
                }
            }
            Ok(())
        }));
        let mut token = 0i64;
        unsafe { controller.add_AcceleratorKeyPressed(&handler, &mut token) }
    }
    const VK_SPACE_CODE: u16 = 0x20;

    /// Opens the menu from the window procedure, not from the WebView2 or IPC
    /// callback that asked: the menu runs a modal loop, which those callbacks
    /// must not do.
    pub fn post_system_menu(hwnd: HWND, client_point: Option<(i32, i32)>) {
        let (flag, packed) = match client_point {
            Some((x, y)) => {
                let mut point = POINT { x, y };
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut point);
                }
                (1, shell_rules::pack_point(point.x, point.y))
            }
            None => (0, 0),
        };
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_SHOW_SYSTEM_MENU, WPARAM(flag), LPARAM(packed));
        }
    }

    unsafe fn show_system_menu(hwnd: HWND, at: Option<(i32, i32)>) {
        unsafe {
            let menu = GetSystemMenu(hwnd, false);
            if menu.is_invalid() {
                return;
            }
            let maximized = IsZoomed(hwnd).as_bool();
            for (command, enabled) in shell_rules::system_menu_states(maximized) {
                let state = if enabled { MF_ENABLED } else { MF_GRAYED };
                let _ = EnableMenuItem(menu, command, MF_BYCOMMAND | state);
            }
            let _ = SetMenuDefaultItem(menu, shell_rules::SC_CLOSE, 0);
            let (x, y) = at.unwrap_or_else(|| keyboard_menu_point(hwnd));
            let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, hwnd, None);
            if command.0 != 0 {
                // "Chiudi" sends SC_CLOSE, i.e. WM_CLOSE: the close guards run.
                let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command.0 as usize), LPARAM(0));
            }
        }
    }

    fn reload_label(italian: bool) -> PCWSTR {
        if italian { w!("Ricarica la finestra") } else { w!("Reload window") }
    }

    fn quit_label(italian: bool) -> PCWSTR {
        if italian { w!("Esci da Qamille") } else { w!("Quit Qamille") }
    }

    /// The app's own entries at the end of the system menu: «Esci da Qamille» right under «Chiudi»,
    /// which only hides the window while it goes to the tray (`background::quit`), and a reload that
    /// works from Alt+Space or Maiusc+clic destro on the taskbar button even when the page is blank
    /// or dead (`recovery::reload_requested`).
    fn add_reload_entry(hwnd: HWND, italian: bool) {
        unsafe {
            let menu = GetSystemMenu(hwnd, false);
            if menu.is_invalid() || GetMenuState(menu, shell_rules::SC_APP_RELOAD, MF_BYCOMMAND) != u32::MAX {
                return;
            }
            let _ = AppendMenuW(menu, MF_STRING, crate::background_rules::SC_APP_QUIT as usize, quit_label(italian));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, shell_rules::SC_APP_RELOAD as usize, reload_label(italian));
        }
    }

    pub fn label_reload_entry(hwnd: HWND, italian: bool) {
        unsafe {
            let menu = GetSystemMenu(hwnd, false);
            if !menu.is_invalid() {
                let command = shell_rules::SC_APP_RELOAD as usize;
                let _ = ModifyMenuW(menu, command as u32, MF_BYCOMMAND | MF_STRING, command, reload_label(italian));
                let command = crate::background_rules::SC_APP_QUIT as usize;
                let _ = ModifyMenuW(menu, command as u32, MF_BYCOMMAND | MF_STRING, command, quit_label(italian));
            }
        }
    }

    /// `AppsUseLightTheme` of the current user: 0 means dark apps.
    pub fn apps_use_dark_theme() -> bool {
        let mut value = 1u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        status.is_ok() && value == 0
    }

    /// Where Windows opens the menu of a normal title bar for Alt+Space: at the
    /// left edge, just under the caption.
    unsafe fn keyboard_menu_point(hwnd: HWND) -> (i32, i32) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetWindowRect(hwnd, &mut rect);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let caption = GetSystemMetricsForDpi(SM_CYCAPTION, dpi)
                + GetSystemMetricsForDpi(SM_CYFRAME, dpi)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
            let mut origin = POINT { x: 0, y: 0 };
            let _ = ClientToScreen(hwnd, &mut origin);
            (origin.x.max(rect.left), origin.y.max(rect.top) + caption)
        }
    }

    unsafe extern "system" fn main_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        unsafe {
            match msg {
                WM_QUERYENDSESSION => {
                    session_query();
                    LRESULT(1)
                }
                WM_ENDSESSION => {
                    if wparam.0 != 0 {
                        session_end();
                    } else {
                        session_cancelled();
                    }
                    // tao's event window exits the process here (tao event_loop.rs).
                    DefSubclassProc(hwnd, msg, wparam, lparam)
                }
                WM_SHOW_SYSTEM_MENU => {
                    let at = (wparam.0 == 1).then(|| shell_rules::unpack_point(lparam.0));
                    show_system_menu(hwnd, at);
                    LRESULT(0)
                }
                WM_SYSCOMMAND if (wparam.0 as u32 & 0xFFF0) == shell_rules::SC_APP_RELOAD => {
                    if let Some(app) = APP.get() {
                        crate::recovery::reload_requested(app);
                    }
                    LRESULT(0)
                }
                // Not from inside this window procedure: the close runs Tauri's window events and
                // the page's close listener.
                WM_SYSCOMMAND if (wparam.0 as u32 & 0xFFF0) == crate::background_rules::SC_APP_QUIT => {
                    if let Some(app) = APP.get().cloned() {
                        std::thread::spawn(move || crate::background::quit(&app));
                    }
                    LRESULT(0)
                }
                _ => DefSubclassProc(hwnd, msg, wparam, lparam),
            }
        }
    }

    fn main_hwnd() -> Option<HWND> {
        hwnd_of(MAIN.load(Ordering::SeqCst))
    }

    /// WM_QUERYENDSESSION: tell Windows why to wait, and let the page save
    /// what it can without changing anything irreversible. The session may
    /// still be cancelled by another app.
    fn session_query() {
        if SESSION_QUERIED.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(hwnd) = main_hwnd() {
            unsafe {
                let _ = ShutdownBlockReasonCreate(hwnd, w!("Salvataggio di bozze, impostazioni e registrazione…"));
            }
        }
        if let Some(app) = APP.get() {
            emit_soon(app, SESSION_ENDING, SessionEnding { last: false });
        }
    }

    /// WM_ENDSESSION(TRUE): the session ends. The page stops the recording and
    /// saves; this window procedure keeps the messages flowing (IPC, scripts)
    /// until it answers or `SESSION_WAIT` runs out. Re-entered for the second
    /// window while waiting, it waits for the same answer.
    fn session_end() {
        // Windows ends the process once the session is over, and the last half second of moving may
        // still be waiting for its delayed save.
        super::save_placement();
        session_query();
        let deadline = *SESSION_DEADLINE.get_or_init(|| Instant::now() + SESSION_WAIT);
        if !SESSION_ENDING_SENT.swap(true, Ordering::SeqCst)
            && let Some(app) = APP.get()
        {
            emit_soon(app, SESSION_ENDING, SessionEnding { last: true });
        }
        unsafe { pump_until(deadline) };
        if let Some(hwnd) = main_hwnd() {
            unsafe {
                let _ = ShutdownBlockReasonDestroy(hwnd);
            }
        }
    }

    fn session_cancelled() {
        if SESSION_QUERIED.swap(false, Ordering::SeqCst)
            && let Some(hwnd) = main_hwnd()
        {
            unsafe {
                let _ = ShutdownBlockReasonDestroy(hwnd);
            }
        }
    }

    unsafe fn pump_until(deadline: Instant) {
        let mut msg = MSG::default();
        while !SESSION_SAVED.load(Ordering::SeqCst) {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        PostQuitMessage(msg.wParam.0 as i32);
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                    if SESSION_SAVED.load(Ordering::SeqCst) {
                        return;
                    }
                }
                let left = deadline.saturating_duration_since(Instant::now()).as_millis().min(50) as u32;
                MsgWaitForMultipleObjectsEx(None, left, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
    }

    pub fn make_tool_window(hwnd: HWND) {
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let next = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            if next != style {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }

    pub fn text_scale_percent() -> Option<u32> {
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Accessibility"),
                w!("TextScaleFactor"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        status.is_ok().then_some(value)
    }

    pub unsafe fn show_system_print(controller: &ICoreWebView2Controller) -> windows_core::Result<()> {
        unsafe { controller.CoreWebView2()?.cast::<ICoreWebView2_16>()?.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM) }
    }

    // ---- Snap layouts (F7) ----------------------------------------------------
    //
    // A child window over the page's maximize button answers WM_NCHITTEST with
    // HTMAXBUTTON, the way Tauri's own resize borders answer HTTOP. Windows 11
    // then shows the snap layouts on hover. Windows handles the mouse there, so
    // this window also does the click (maximize or restore) and tells the page
    // to draw hover and press; keyboard focus and Enter stay with the page.

    pub fn place_snap_button(parent: isize, rect: Option<PixelRect>) {
        let Some(parent) = hwnd_of(parent) else { return };
        unsafe {
            let existing = hwnd_of(SNAP.load(Ordering::SeqCst));
            let Some(rect) = rect else {
                if let Some(snap) = existing {
                    let _ = ShowWindow(snap, SW_HIDE);
                    set_button_state(ButtonState::Rest);
                }
                return;
            };
            let snap = match existing {
                Some(snap) => snap,
                None => match create_snap_button(parent) {
                    Some(snap) => snap,
                    None => return,
                },
            };
            // Just under Tauri's resize strip, so the top edge still resizes
            // over the button as it does on a native title bar; above the page.
            let after = FindWindowExW(Some(parent), None, w!("TAURI_DRAG_RESIZE_BORDERS"), PCWSTR::null())
                .ok()
                .filter(|h| !h.is_invalid())
                .unwrap_or(HWND_TOP);
            let _ = SetWindowPos(
                snap,
                Some(after),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    unsafe fn create_snap_button(parent: HWND) -> Option<HWND> {
        unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?;
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(snap_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: SNAP_CLASS,
                ..Default::default()
            };
            // Fails harmlessly when the class is already registered.
            RegisterClassExW(&class);
            let snap = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                SNAP_CLASS,
                PCWSTR::null(),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(instance.into()),
                None,
            )
            .ok()?;
            SNAP.store(snap.0 as isize, Ordering::SeqCst);
            Some(snap)
        }
    }

    fn set_button_state(state: ButtonState) {
        let code = state as u8;
        if SNAP_STATE.swap(code, Ordering::SeqCst) != code
            && let Some(app) = APP.get()
        {
            emit_soon(app, MAXIMIZE_BUTTON, shell_rules::button_state_name(state));
        }
    }

    fn pressed() -> bool {
        SNAP_STATE.load(Ordering::SeqCst) == ButtonState::Pressed as u8
    }

    unsafe extern "system" fn snap_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            let on_button = wparam.0 == HTMAXBUTTON as usize;
            match msg {
                WM_NCHITTEST => LRESULT(HTMAXBUTTON as isize),
                WM_NCMOUSEMOVE if on_button => {
                    if !pressed() {
                        set_button_state(ButtonState::Hover);
                    }
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                    LRESULT(0)
                }
                WM_NCMOUSELEAVE => {
                    set_button_state(ButtonState::Rest);
                    LRESULT(0)
                }
                // Not passed on: DefWindowProc would draw a classic button.
                WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_button => {
                    set_button_state(ButtonState::Pressed);
                    LRESULT(0)
                }
                WM_NCLBUTTONUP if on_button => {
                    if pressed()
                        && let Ok(top) = GetAncestor(hwnd, GA_ROOT).ok_or(())
                    {
                        let command = shell_rules::maximize_button_command(IsZoomed(top).as_bool());
                        let _ = PostMessageW(Some(top), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
                    }
                    set_button_state(ButtonState::Hover);
                    LRESULT(0)
                }
                WM_NCRBUTTONUP => {
                    if let Ok(top) = GetAncestor(hwnd, GA_ROOT).ok_or(()) {
                        let point = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
                        let _ = PostMessageW(
                            Some(top),
                            WM_SHOW_SYSTEM_MENU,
                            WPARAM(1),
                            LPARAM(shell_rules::pack_point(point.0, point.1)),
                        );
                    }
                    LRESULT(0)
                }
                WM_NCDESTROY => {
                    SNAP.store(0, Ordering::SeqCst);
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            }
        }
    }

    trait OkOr {
        fn ok_or<E>(self, error: E) -> Result<HWND, E>;
    }
    impl OkOr for HWND {
        fn ok_or<E>(self, error: E) -> Result<HWND, E> {
            if self.is_invalid() { Err(error) } else { Ok(self) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preferences_file_from_before_the_placement_still_reads() {
        let old: WindowPreferences = serde_json::from_str(r#"{"appearance":"dark","zoom":1.25,"italian":false}"#).unwrap();
        assert_eq!((old.appearance.as_str(), old.zoom, old.italian), ("dark", 1.25, Some(false)));
        assert_eq!(old.placement, None);
        // And a file without a placement is written without one.
        assert!(!serde_json::to_string(&old).unwrap().contains("placement"));
    }

    #[test]
    fn the_placement_is_written_and_read_back() {
        let stored = StoredPlacement { x: -1800, y: 60, width: 1440, height: 900, scale: 1.25, maximized: true };
        let preferences = WindowPreferences { placement: Some(stored), ..WindowPreferences::default() };
        let text = serde_json::to_string(&preferences).unwrap();
        assert_eq!(serde_json::from_str::<WindowPreferences>(&text).unwrap(), preferences);
        // `maximized` is optional in the file.
        let text = r#"{"placement":{"x":10,"y":20,"width":1440,"height":900,"scale":1.0}}"#;
        let read = serde_json::from_str::<WindowPreferences>(text).unwrap().placement.unwrap();
        assert_eq!((read.x, read.y, read.maximized), (10, 20, false));
    }

    #[test]
    fn a_placement_that_cannot_be_read_costs_neither_the_appearance_nor_the_zoom() {
        for broken in [
            r#""left""#,
            "null",
            "42",
            r#"{"x":"a","y":0,"width":1,"height":1,"scale":1.0}"#,
            r#"{"x":0,"y":0,"width":-5,"height":1,"scale":1.0}"#,
            r#"{"x":0,"y":0,"width":1}"#,
        ] {
            let text = format!(r#"{{"appearance":"light","zoom":1.5,"placement":{broken}}}"#);
            let read: WindowPreferences = serde_json::from_str(&text).unwrap();
            assert_eq!((read.appearance.as_str(), read.zoom), ("light", 1.5), "{broken}");
            assert_eq!(read.placement, None, "{broken}");
        }
    }
}
