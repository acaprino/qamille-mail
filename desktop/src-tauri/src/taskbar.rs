//! The taskbar button of the main window (docs/aspetto-nativo.md, rule 30): the unread mail of the
//! Inbox as an overlay on the icon (the sidebar's number) and in the tray's tooltip, and the progress
//! of a full indexing. What is shown is in `taskbar_rules.rs`.
//!
//! Rust works it out, never the page: with the window in the tray Chromium slows the page's timers
//! down to a minute. The sink (`runtime.rs`) shows every event to `observe`; a thread of its own counts
//! the Inbox again 300 ms after the last `CountsChanged` (or an undo row, which changes what the
//! sidebar counts), at most 2 s after the first, and once a minute for what another process changed
//! (the MCP's writes reach the page through the change feed, not through the sink). Windows forgets an
//! overlay when the button goes, so both are drawn again when the window gets the focus back.
#![cfg_attr(not(windows), allow(dead_code, unused_variables))]

use crate::taskbar_rules::{self, Bar, IndexBars};
use qamille::application::{events::AppEvent, workspace::WorkspaceService};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// What the thread hears (`observe`, `reset_index`, the window's focus).
enum Signal {
    Counts,
    Index { account: u32, percent: u8 },
    Waiting { account: u32, waiting: bool },
    Reset,
    Focused,
}

static SIGNALS: OnceLock<mpsc::Sender<Signal>> = OnceLock::new();

/// The count waits for the counters to settle...
const SETTLE: Duration = Duration::from_millis(300);
/// ...but not longer than this after the first change, during a long sync.
const AT_MOST: Duration = Duration::from_secs(2);
/// What another process changed shows within this.
const REFRESH: Duration = Duration::from_secs(60);
/// Before the main window exists the count is tried again after this.
const NO_WINDOW_YET: Duration = Duration::from_millis(500);
/// A small icon at 100 % (`SM_CXSMICON`): the overlay is drawn at this times the window's scale.
const SMALL_ICON: f64 = 16.0;

fn send(signal: Signal) {
    if let Some(signals) = SIGNALS.get() {
        let _ = signals.send(signal);
    }
}

/// Every event of the sink, before anything else sees it. Cheap: a message to the thread.
pub fn observe(app: &AppHandle, event: &AppEvent) {
    let _ = app;
    match event {
        AppEvent::CountsChanged { .. } | AppEvent::UndoChanged { .. } | AppEvent::UndoRemoved { .. } | AppEvent::AccountsChanged => {
            send(Signal::Counts)
        }
        AppEvent::IndexProgress { account_id, percent } => send(Signal::Index { account: *account_id, percent: *percent }),
        AppEvent::IndexWaiting { account_id, waiting } => send(Signal::Waiting { account: *account_id, waiting: *waiting }),
        _ => {}
    }
}

/// «Reindex» (`search_reindex`): what follows is a full indexing again, shown on the taskbar.
pub fn reset_index() {
    send(Signal::Reset);
}

/// What the button shows now, so nothing is drawn twice.
#[derive(Default)]
struct Shown {
    count: Option<u32>,
    bar: Option<Option<(Bar, u8)>>,
}

/// From `DesktopState::new`, before the main window exists: the thread that keeps the button up to
/// date. Its first count waits for the window.
pub fn start(app: &AppHandle, workspace: WorkspaceService) {
    let (signals, received) = mpsc::channel();
    if SIGNALS.set(signals).is_err() {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("qamille-taskbar".into()).spawn(move || run(&app, &workspace, &received));
}

fn run(app: &AppHandle, workspace: &WorkspaceService, received: &mpsc::Receiver<Signal>) {
    let mut bars = IndexBars::default();
    let mut shown = Shown::default();
    let mut listening = false;
    // The first count, a moment after the window and the tray icon exist.
    let start = Instant::now() + Duration::from_secs(1);
    let mut due: Option<(Instant, Instant)> = Some((start, start));
    let mut refresh = start + REFRESH;
    loop {
        let wake = due.map_or(refresh, |(at, _)| at.min(refresh));
        match received.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(Signal::Counts) => {
                let now = Instant::now();
                let first = due.map_or(now, |(_, first)| first);
                due = Some(((now + SETTLE).min(first + AT_MOST), first));
            }
            Ok(Signal::Index { account, percent }) => show_bar(app, &mut shown, bars.apply(account, Some(percent), None)),
            Ok(Signal::Waiting { account, waiting }) => show_bar(app, &mut shown, bars.apply(account, None, Some(waiting))),
            Ok(Signal::Reset) => bars.reset(),
            Ok(Signal::Focused) => {
                // Back from the tray, or after Explorer restarted: the button is new.
                let bar = shown.bar.take().flatten();
                shown.count = None;
                show_bar(app, &mut shown, bar);
                due = Some((Instant::now(), Instant::now()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if !due.is_some_and(|(at, _)| now >= at) && now < refresh {
                    continue;
                }
                due = None;
                refresh = now + REFRESH;
                let Some(window) = app.get_webview_window("main") else {
                    due = Some((now + NO_WINDOW_YET, now));
                    continue;
                };
                if !listening {
                    listening = true;
                    window.on_window_event(|event| {
                        if matches!(event, tauri::WindowEvent::Focused(true)) {
                            send(Signal::Focused);
                        }
                    });
                }
                match workspace.inbox_unread() {
                    Ok(count) => show_count(app, &window, &mut shown, u32::try_from(count).unwrap_or(u32::MAX)),
                    Err(error) => crate::native_feel::report(&format!("the unread count for the taskbar failed: {}", error.message)),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The overlay and the tooltip for `count`. The tooltip is set every time: the tray icon may have
/// come after the window.
fn show_count(app: &AppHandle, window: &tauri::WebviewWindow, shown: &mut Shown, count: u32) {
    crate::background::set_tray_tooltip(app, &taskbar_rules::tray_tooltip(count, crate::window_shell::italian()));
    if shown.count == Some(count) {
        return;
    }
    let size = (SMALL_ICON * window.scale_factor().unwrap_or(1.0)).round().clamp(16.0, 64.0) as u32;
    let overlay = taskbar_rules::badge_rgba(count, size).map(|rgba| tauri::image::Image::new_owned(rgba, size, size));
    match window.set_overlay_icon(overlay) {
        Ok(()) => shown.count = Some(count),
        Err(error) => crate::native_feel::report(&format!("the taskbar overlay was not set: {error}")),
    }
}

/// The progress bar of a full indexing: green, yellow while it waits, gone at the end.
fn show_bar(app: &AppHandle, shown: &mut Shown, bar: Option<(Bar, u8)>) {
    if shown.bar == Some(bar) {
        return;
    }
    let Some(window) = app.get_webview_window("main") else { return };
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    let state = match bar {
        None => ProgressBarState { status: Some(ProgressBarStatus::None), progress: None },
        Some((Bar::Normal, percent)) => ProgressBarState { status: Some(ProgressBarStatus::Normal), progress: Some(u64::from(percent)) },
        Some((Bar::Paused, percent)) => ProgressBarState { status: Some(ProgressBarStatus::Paused), progress: Some(u64::from(percent)) },
    };
    match window.set_progress_bar(state) {
        Ok(()) => shown.bar = Some(bar),
        Err(error) => crate::native_feel::report(&format!("the taskbar progress was not set: {error}")),
    }
}
