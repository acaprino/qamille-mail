//! Meeting reminder chips: a borderless window at the top centre of the screen,
//! above other windows and never focused when it appears (ADR-0004).
use crate::runtime::{DesktopState, blocking};
use qamille::application::{
    AppError, AppResult,
    meeting_reminders::{GOOGLE_REFRESH_SECS, MeetingReminderService, ReminderView},
};
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError},
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "meeting-reminder";
const CHANGED: &str = "reminder-changed";
const CHECK_EVERY: Duration = Duration::from_secs(15);
const GOOGLE_EVERY: Duration = Duration::from_secs(GOOGLE_REFRESH_SECS as u64);
const WIDTH: f64 = 600.0;
const CHIP_HEIGHT: f64 = 52.0;
const GAP: f64 = 8.0;
const TOP_MARGIN: f64 = 12.0;

/// One window update at a time, always from the engine's current view. It
/// holds the last window error, so a lasting failure is reported once.
/// Never taken on the main thread: window calls made under it wait for that
/// thread.
static PRESENTING: Mutex<String> = Mutex::new(String::new());
/// Set when the main window is gone: no chip may keep the app alive after it.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// The main window was destroyed: close the chip and never show it again.
/// Runs on the main thread, so it must not wait for `PRESENTING`; a chip being
/// built meanwhile sees `CLOSING` and destroys itself.
pub fn close_with_main(app: &AppHandle) {
    CLOSING.store(true, Ordering::SeqCst);
    if let Some(chip) = app.get_webview_window(LABEL) {
        let _ = chip.destroy();
    }
}

pub fn spawn(app: AppHandle, state: DesktopState) -> std::io::Result<()> {
    let (wake, wakeups) = mpsc::channel::<()>();
    // The only thread that checks the chips: every 15 seconds, or at once after
    // new Google events. Several wake-ups make one check.
    let local_app = app.clone();
    let local = state.reminders.clone();
    std::thread::Builder::new()
        .name("qamille-meeting-reminders".into())
        .spawn(move || {
            let mut notices = Notices::default();
            loop {
                check(&local_app, &local, &mut notices);
                if wakeups.recv_timeout(CHECK_EVERY) == Err(RecvTimeoutError::Disconnected) {
                    std::thread::sleep(CHECK_EVERY);
                }
                while wakeups.try_recv().is_ok() {}
            }
        })?;
    // Google: its own worker, so a slow network never delays local chips. Each
    // account is read at most once a minute.
    let google = state.reminders.clone();
    std::thread::Builder::new()
        .name("qamille-meeting-reminders-google".into())
        .spawn(move || {
            let mut notices = Notices::default();
            loop {
                match google.refresh_google() {
                    Ok(refresh) => {
                        notices.clear();
                        for notice in refresh.notices {
                            let _ = app.emit_to("main", "background-notice", notice);
                        }
                        if refresh.updated {
                            let _ = wake.send(());
                        }
                    }
                    Err(error) => notices.send(&app, error.message),
                }
                std::thread::sleep(GOOGLE_EVERY);
            }
        })?;
    Ok(())
}

#[derive(Default)]
struct Notices(String);
impl Notices {
    fn clear(&mut self) {
        self.0.clear();
    }
    fn send(&mut self, app: &AppHandle, message: String) {
        if message != self.0 {
            let _ = app.emit_to("main", "background-notice", message.clone());
            self.0 = message;
        }
    }
}

fn check(app: &AppHandle, reminders: &MeetingReminderService, notices: &mut Notices) {
    match reminders.tick() {
        Ok(tick) => {
            notices.clear();
            for notice in tick.notices {
                let _ = app.emit_to("main", "background-notice", notice);
            }
            let missing = !tick.view.chips.is_empty() && app.get_webview_window(LABEL).is_none();
            if tick.changed || missing {
                present(app, reminders);
            }
        }
        Err(error) => notices.send(app, error.message),
    }
}

/// Show, resize or close the chip window for the engine's current view.
pub fn present(app: &AppHandle, reminders: &MeetingReminderService) {
    let mut last_error = PRESENTING.lock().unwrap_or_else(PoisonError::into_inner);
    let result = reminders
        .view()
        .map_err(|e| e.message)
        .and_then(|view| show(app, &view).map_err(|e| e.to_string()));
    match result {
        Ok(()) => last_error.clear(),
        Err(message) if *last_error != message => {
            let _ = app.emit_to("main", "background-notice", message.clone());
            *last_error = message;
        }
        Err(_) => {}
    }
}

fn show(app: &AppHandle, view: &ReminderView) -> tauri::Result<()> {
    let existing = app.get_webview_window(LABEL);
    // Without the main window the app must be able to exit, so no chip either.
    if view.chips.is_empty() || CLOSING.load(Ordering::SeqCst) || app.get_webview_window("main").is_none() {
        if let Some(window) = existing {
            window.destroy()?;
        }
        return Ok(());
    }
    let count = view.chips.len() as f64;
    let height = count * CHIP_HEIGHT + (count - 1.0) * GAP;
    let (x, y) = top_centre(app)?;
    match existing {
        Some(window) => {
            window.set_size(LogicalSize::new(WIDTH, height))?;
            window.set_position(LogicalPosition::new(x, y))?;
        }
        // After repeated failures of its page: no chip for a while (recovery.rs).
        None if !crate::recovery::chip_may_open(LABEL) => return Ok(()),
        None => {
            let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("reminder.html".into()))
                .title("Qamille")
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .focused(false)
                .inner_size(WIDTH, height)
                .position(x, y)
                // Same profile as the main window, so device preferences are shared.
                .data_directory(qamille::model::Store::data_dir().join("webview"))
                .general_autofill_enabled(false)
                // wry's own drop handler would take a file dragged onto the chip, with the copy
                // cursor, for a drop nothing reads; without it the page refuses the drop.
                .disable_drag_drop_handler()
                .on_navigation(crate::native_feel::navigation_guard(app, false))
                .build()?;
            crate::native_feel::harden_webview(&window, "reminder.html");
            // A tool window: out of Alt+Tab and Win+Tab, where it would be a second
            // "Qamille" that takes the focus (F11). No owner: an owned window is
            // hidden with a minimized main window, and the chip must show (ADR-0004).
            crate::window_shell::make_tool_window(&window);
            // The main window may have closed while this one was being built.
            if CLOSING.load(Ordering::SeqCst) {
                window.destroy()?;
                return Ok(());
            }
        }
    }
    app.emit_to(LABEL, CHANGED, view)?;
    Ok(())
}

fn top_centre(app: &AppHandle) -> tauri::Result<(f64, f64)> {
    let monitor = match app.primary_monitor()? {
        Some(monitor) => Some(monitor),
        None => match app.get_webview_window("main") {
            Some(main) => main.current_monitor()?,
            None => None,
        },
    };
    Ok(match monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let left = f64::from(area.position.x) / scale;
            let width = f64::from(area.size.width) / scale;
            (left + (width - WIDTH).max(0.0) / 2.0, f64::from(area.position.y) / scale + TOP_MARGIN)
        }
        None => (0.0, TOP_MARGIN),
    })
}

#[tauri::command]
pub async fn reminder_view(state: State<'_, DesktopState>) -> AppResult<ReminderView> {
    let s = state.reminders.clone();
    blocking(move || s.view()).await
}

/// Opens the link the engine chose for the chip; the page only names the chip.
/// The chip is taken once, before the browser starts, so closing it meanwhile
/// cannot cancel the recording.
#[tauri::command]
pub async fn reminder_join(
    key: String,
    record: bool,
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> AppResult<ReminderView> {
    let s = state.reminders.clone();
    let result = blocking(move || {
        let chip = s.chip(&key)?;
        open::that(&chip.link).map_err(|e| AppError::external(e.to_string()))?;
        if record {
            s.record_chip(&chip)?;
        }
        s.dismiss(&key)
    })
    .await;
    // A started recording only adds a Meeting, which no typed event names and
    // INV-004 forbids announcing without content: the main window needs no
    // reload — its recording indicator polls the capture state every second
    // and the Meetings view refreshes when it opens.
    present(&app, &state.reminders);
    result
}

/// "Join and record" from the event editor: the link comes from the event the
/// user is looking at and must be a plain https link. A local event passes its
/// own `event_id`; a Google event passes its account and Google id instead, and
/// the recording links to the local copy Qamille keeps of it (none if it has
/// never been copied, the recording then starts unlinked).
#[tauri::command]
pub async fn event_join_record(
    url: String,
    key: String,
    title: String,
    event_id: Option<u64>,
    account_id: Option<u32>,
    google_event_id: Option<String>,
    window: tauri::Window,
    state: State<'_, DesktopState>,
) -> AppResult<()> {
    if window.label() != "main" {
        return Err(AppError::new("permission_denied", "This window cannot open links"));
    }
    if !qamille::domain::reminders::valid_link(&url) {
        return Err(AppError::validation("The meeting link is not valid"));
    }
    let s = state.reminders.clone();
    blocking(move || {
        let event_id = event_id.or_else(|| match (account_id, google_event_id) {
            (Some(account), Some(google_event)) => qamille::mcp::calendar::local_event_id_for(account, &google_event),
            _ => None,
        });
        // Recording first: with no audio device or a recording already running,
        // the meeting must not open as if it were being recorded.
        s.record_meeting(&key, title, event_id)?;
        open::that(&url).map_err(|e| AppError::external(e.to_string()))
    })
    .await
}

#[tauri::command]
pub async fn reminder_dismiss(key: String, app: AppHandle, state: State<'_, DesktopState>) -> AppResult<ReminderView> {
    let s = state.reminders.clone();
    let view = blocking(move || s.dismiss(&key)).await?;
    present(&app, &state.reminders);
    Ok(view)
}
