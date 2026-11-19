//! When a window cannot show the app, the app answers, never WebView2's error
//! page (docs/aspetto-nativo.md, rule 21). With F5 and Ctrl+R switched off
//! (rule 8) a dead page would otherwise leave no way out but closing.
//!
//! - The page's process exits (crash, low memory): reload, at most
//!   `RELOAD_LIMIT` times in two minutes, then ask.
//! - The page stops answering: ask whether to reload; it may still recover.
//! - WebView2's browser process exits (a runtime update, a crash): every
//!   webview is gone, ask to restart Qamille.
//! - The app's page does not load (a dev build without its dev server): ask to
//!   retry, with the reason.
//! - "Ricarica la finestra" in the system menu: the page saves what it can,
//!   as at sign-out, then reloads. If something was not saved, or the page
//!   does not answer within `RELOAD_WAIT`, ask: never discard by itself.
//!
//! - Closing the main window: the page decides (its close guards save or ask),
//!   so a page that does not answer within `CLOSE_ANSWER`, or a close still
//!   running after `CLOSE_STUCK` when the person asks again, gets a choice here:
//!   wait, or close Qamille without the page.
//!
//! WebView2 reports these from its own callbacks, which must not run a dialog's
//! modal loop: every dialog and reload here runs on a thread of its own, and
//! one dialog at a time covers a failure reported by several windows.

use crate::shell_rules::{self, Failure};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// To the main window: save what can be saved without asking, then call
/// `reload_ready` with the same number.
pub const RELOAD_REQUESTED: &str = "reload-requested";
/// Set for the process started by a restart: the old one's id. Tauri starts
/// the new process before the old one exits, and the old one still holds the
/// workspace's `ui.lock` (taken with `try_lock`, which does not wait).
const RESTART_AFTER: &str = "QAMILLE_RESTART_AFTER";
/// A live page right after startup can take some seconds to save (observed ~4 s): the question
/// comes only after this, and a dead page waits this long for it.
const RELOAD_WAIT: Duration = Duration::from_secs(10);
/// After "Attendi" to "Qamille non risponde", no new question for this long.
const HUNG_QUIET: Duration = Duration::from_secs(30);
/// A live page acknowledges a close request at once (`close_seen`).
const CLOSE_ANSWER: Duration = Duration::from_secs(3);
/// A close asked again after this long, while the first is still running,
/// may be stuck in a guard (a save that never ends): offer the way out.
const CLOSE_STUCK: Duration = Duration::from_secs(10);

/// The page draws within this after the window shows, or the app offers a reload.
const STARTUP_WAIT: Duration = Duration::from_secs(15);

static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static CLOSE_ASKED: AtomicU64 = AtomicU64::new(0);
static CLOSE_SEEN: AtomicU64 = AtomicU64::new(0);
static CLOSING_SINCE: Mutex<Option<Instant>> = Mutex::new(None);

static ASKING: AtomicBool = AtomicBool::new(false);
static ENGINE_GONE: AtomicBool = AtomicBool::new(false);
static PENDING_RELOAD: AtomicU64 = AtomicU64::new(0);
static NEXT_RELOAD: AtomicU64 = AtomicU64::new(1);
static RELOADS: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
static HUNG_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn words(italian: &'static str, english: &'static str) -> &'static str {
    if crate::window_shell::italian() { italian } else { english }
}

/// A Windows task dialog over `window` (its owner) with two buttons named by
/// what they do; `first` is the default for Enter. `Some(true)`: `first`,
/// `Some(false)`: `second`, `None`: closed with Esc or the X, which each caller
/// reads as its harmless choice.
fn ask(window: &WebviewWindow, title: &str, text: &str, first: &str, second: &str) -> Option<bool> {
    let answer = rfd::MessageDialog::new()
        .set_parent(window)
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(first.to_owned(), second.to_owned()))
        .show();
    match answer {
        rfd::MessageDialogResult::Custom(label) if label == first => Some(true),
        rfd::MessageDialogResult::Custom(label) if label == second => Some(false),
        _ => None,
    }
}

/// Runs `decide` on its own thread unless a recovery dialog is already open.
fn ask_once(window: &WebviewWindow, decide: impl FnOnce(&WebviewWindow) + Send + 'static) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let window = window.clone();
    std::thread::spawn(move || {
        decide(&window);
        ASKING.store(false, Ordering::SeqCst);
    });
}

fn later(window: &WebviewWindow, act: impl FnOnce(&WebviewWindow) + Send + 'static) {
    let window = window.clone();
    std::thread::spawn(move || act(&window));
}

fn reload(window: &WebviewWindow) {
    if let Err(error) = window.reload() {
        crate::native_feel::report(&format!("could not reload {}: {error}", window.label()));
    }
}

/// One of the window's WebView2 processes failed (`COREWEBVIEW2_PROCESS_FAILED_KIND`).
pub fn process_failed(window: &WebviewWindow, kind: i32) {
    let main = window.label() == "main";
    match shell_rules::failure_of(kind) {
        Failure::PageExited => {
            if reload_by_itself(window.label()) {
                later(window, reload);
            } else if main {
                ask_once(window, |window| {
                    let close = ask(
                        window,
                        words("La finestra di Qamille si è chiusa", "Qamille's window closed"),
                        words(
                            "La pagina di Qamille si è chiusa più volte in pochi minuti. La posta e le bozze salvate sono al sicuro.",
                            "Qamille's page closed several times in a few minutes. Your mail and saved drafts are safe.",
                        ),
                        words("Ricarica la finestra", "Reload the window"),
                        words("Chiudi Qamille", "Close Qamille"),
                    ) == Some(false);
                    if close { window.app_handle().exit(0) } else { reload(window) }
                });
            } else {
                // The reminder chip, out of automatic reloads: closed until `chip_may_open`.
                later(window, |window| {
                    let _ = window.destroy();
                });
            }
        }
        Failure::PageHung if main => {
            if HUNG_UNTIL.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|until| Instant::now() < until) {
                return;
            }
            ask_once(window, |window| {
                let reload_now = ask(
                    window,
                    words("Qamille non risponde", "Qamille is not responding"),
                    words(
                        "La finestra non risponde. Può riprendersi da sola.\n\nRicaricandola adesso, le modifiche non ancora salvate potrebbero andare perse.",
                        "The window is not responding. It may recover by itself.\n\nReloading it now may lose changes not yet saved.",
                    ),
                    words("Attendi", "Wait"),
                    words("Ricarica adesso", "Reload now"),
                ) == Some(false);
                if reload_now {
                    reload(window);
                } else {
                    *HUNG_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + HUNG_QUIET);
                }
            });
        }
        Failure::PageHung => chip_failed(window),
        Failure::EngineExited if main => {
            if ENGINE_GONE.swap(true, Ordering::SeqCst) {
                return;
            }
            // Not `ask_once`: this one must not be skipped behind another dialog.
            later(window, |window| {
                let close = ask(
                    window,
                    words("Qamille deve ripartire", "Qamille needs to restart"),
                    words(
                        "Il componente di Windows che disegna le finestre (WebView2) si è chiuso. La posta e le bozze salvate sono al sicuro.",
                        "The Windows component that draws the windows (WebView2) closed. Your mail and saved drafts are safe.",
                    ),
                    words("Riavvia Qamille", "Restart Qamille"),
                    words("Chiudi Qamille", "Close Qamille"),
                ) == Some(false);
                if close { window.app_handle().exit(0) } else { restart_app(window.app_handle()) }
            });
        }
        failure => crate::native_feel::report(&format!("a WebView2 process of {} failed ({failure:?}, kind {kind})", window.label())),
    }
}

/// The reminder chip failed (its page did not load, or it hung): closed. The
/// engine opens it again at its next check, within the reload limit (`chip_may_open`).
fn chip_failed(window: &WebviewWindow) {
    reload_by_itself(window.label());
    later(window, |window| {
        let _ = window.destroy();
    });
}

/// Whether the reminders engine may build the chip again: not after
/// `RELOAD_LIMIT` failures in the last two minutes, so a page that cannot
/// load is not rebuilt every check forever.
pub fn chip_may_open(label: &str) -> bool {
    let now = now_ms();
    let reloads = RELOADS.lock().unwrap_or_else(|e| e.into_inner());
    let earlier: Vec<u64> = reloads
        .iter()
        .filter(|(l, at)| l == label && now.saturating_sub(*at) < shell_rules::RELOAD_WINDOW_MS)
        .map(|(_, at)| *at)
        .collect();
    shell_rules::may_reload_by_itself(&earlier, now)
}

fn reload_by_itself(label: &str) -> bool {
    let now = now_ms();
    let mut reloads = RELOADS.lock().unwrap_or_else(|e| e.into_inner());
    reloads.retain(|(_, at)| now.saturating_sub(*at) < shell_rules::RELOAD_WINDOW_MS);
    let earlier: Vec<u64> = reloads.iter().filter(|(l, _)| l == label).map(|(_, at)| *at).collect();
    let allowed = shell_rules::may_reload_by_itself(&earlier, now);
    if allowed {
        reloads.push((label.to_owned(), now));
    }
    allowed
}

/// The app's own page did not load (WebView2 would show its error page here).
pub fn page_not_loaded(window: &WebviewWindow, status: i32) {
    if window.label() != "main" {
        chip_failed(window);
        return;
    }
    ask_once(window, move |window| {
        let text = if tauri::is_dev() {
            format!(
                "{} ({status})",
                words(
                    "Questa è una build di sviluppo e l'interfaccia viene dal server di sviluppo, che non risponde: avvia `npm run dev` in desktop/, oppure compila con --features custom-protocol (AGENTS.md).",
                    "This is a development build and its interface comes from the dev server, which does not answer: run `npm run dev` in desktop/, or build with --features custom-protocol (AGENTS.md).",
                )
            )
        } else {
            format!(
                "{} ({status})",
                words(
                    "Qamille non è riuscito a caricare la finestra. La posta e le bozze salvate sono al sicuro.",
                    "Qamille could not load its window. Your mail and saved drafts are safe.",
                )
            )
        };
        let close = ask(
            window,
            words("Qamille non si è aperto", "Qamille did not open"),
            &text,
            words("Riprova", "Try again"),
            words("Chiudi Qamille", "Close Qamille"),
        ) == Some(false);
        if close { window.app_handle().exit(0) } else { reload(window) }
    });
}

/// "Ricarica la finestra" in the system menu, from the window procedure.
pub fn reload_requested(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let token = NEXT_RELOAD.fetch_add(1, Ordering::SeqCst);
    PENDING_RELOAD.store(token, Ordering::SeqCst);
    std::thread::spawn(move || {
        let _ = window.emit_to("main", RELOAD_REQUESTED, token);
        std::thread::sleep(RELOAD_WAIT);
        if PENDING_RELOAD.load(Ordering::SeqCst) != token {
            return;
        }
        // No answer: the page is blank, dead or still saving. Waiting keeps the
        // request: a late answer still reloads (and may land while this is open).
        ask_once(&window, move |window| {
            let now = ask(
                window,
                words("La finestra non ha risposto", "The window did not answer"),
                words(
                    "Prima di ricaricare, Qamille ha chiesto alla finestra di salvare bozze, impostazioni e appunti, ma non ha avuto risposta: la pagina è bloccata, vuota o sta ancora salvando.\n\nSe attendi, la finestra si ricarica da sola appena ha salvato. Ricaricandola adesso, le modifiche non ancora salvate potrebbero andare perse.",
                    "Before reloading, Qamille asked the window to save drafts, settings and notes, but got no answer: the page is stuck, blank or still saving.\n\nIf you wait, the window reloads by itself once it has saved. Reloading it now may lose changes not yet saved.",
                ),
                words("Attendi", "Wait"),
                words("Ricarica adesso", "Reload now"),
            ) == Some(false);
            if now && PENDING_RELOAD.compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                reload(window);
            }
        });
    });
}

/// The page's answer to `reload-requested`: `saved` when every editor saved.
/// Otherwise reloading would lose what was not saved: ask first.
#[tauri::command]
pub fn reload_ready(window: WebviewWindow, token: u64, saved: bool) {
    if PENDING_RELOAD.compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    if saved {
        // Not from inside the IPC callback that delivered this call.
        later(&window, reload);
        return;
    }
    ask_once(&window, |window| {
        let anyway = ask(
            window,
            words("Modifiche non salvate", "Changes not saved"),
            words(
                "Qamille non è riuscito a salvare tutte le modifiche prima di ricaricare la finestra. Ricaricandola, quelle non salvate andranno perse.",
                "Qamille could not save every change before reloading the window. Reloading it loses the ones not saved.",
            ),
            words("Non ricaricare", "Do not reload"),
            words("Ricarica comunque", "Reload anyway"),
        ) == Some(false);
        if anyway {
            reload(window);
        }
    });
}

/// Right after the main window shows: a page that loads but never draws (its
/// script fails, or never runs) would leave an empty window with no message.
pub fn watch_startup(window: &WebviewWindow) {
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(STARTUP_WAIT);
        if FRONTEND_READY.load(Ordering::SeqCst) || window.app_handle().get_webview_window("main").is_none() {
            return;
        }
        ask_once(&window, |window| {
            let answer = ask(
                window,
                words("Qamille non si è aperto", "Qamille did not open"),
                words(
                    "La finestra di Qamille non ha finito di aprirsi. La posta e le bozze salvate sono al sicuro.",
                    "Qamille's window did not finish opening. Your mail and saved drafts are safe.",
                ),
                words("Ricarica la finestra", "Reload the window"),
                words("Chiudi Qamille", "Close Qamille"),
            );
            match answer {
                Some(true) => reload(window),
                Some(false) => window.app_handle().exit(0),
                None => {}
            }
        });
    });
}

/// The page's first frame is drawn (`src/main.tsx`).
#[tauri::command]
pub fn frontend_ready() {
    FRONTEND_READY.store(true, Ordering::SeqCst);
}

/// Main's `CloseRequested` (X, Alt+F4, the taskbar, the system menu). The
/// page's close listener has the decision: Tauri cancels the close for it.
pub fn close_requested(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let request = CLOSE_ASKED.fetch_add(1, Ordering::SeqCst) + 1;
    let since = *CLOSING_SINCE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Instant::now);
    let stuck = since.elapsed() >= CLOSE_STUCK;
    std::thread::spawn(move || {
        if !stuck {
            std::thread::sleep(CLOSE_ANSWER);
            // Answered, or already closed (no listener yet at startup: Tauri closed it).
            if CLOSE_SEEN.load(Ordering::SeqCst) >= request || window.app_handle().get_webview_window("main").is_none() {
                return;
            }
        }
        ask_once(&window, |window| {
            let close = ask(
                window,
                words("Qamille non si chiude", "Qamille does not close"),
                words(
                    "La finestra non ha finito di chiudersi: è bloccata, vuota o sta ancora salvando. La posta e le bozze salvate sono al sicuro.\n\nChiudendo adesso, le modifiche non ancora salvate potrebbero andare perse.",
                    "The window has not finished closing: it is stuck, blank or still saving. Your mail and saved drafts are safe.\n\nClosing now may lose changes not yet saved.",
                ),
                words("Attendi", "Wait"),
                words("Chiudi Qamille", "Close Qamille"),
            ) == Some(false);
            if close {
                let _ = window.destroy();
            }
        });
    });
}

/// The page got the close request: it is alive and its guards are running.
#[tauri::command]
pub fn close_seen() {
    CLOSE_SEEN.store(CLOSE_ASKED.load(Ordering::SeqCst), Ordering::SeqCst);
}

/// The page's close ended without closing (a guard kept the window, or failed).
#[tauri::command]
pub fn close_settled() {
    *CLOSING_SINCE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn restart_app(app: &AppHandle) {
    // SAFETY: the variable is only read by the next process, at its start.
    unsafe { std::env::set_var(RESTART_AFTER, std::process::id().to_string()) };
    app.request_restart();
}

/// First thing in `main`: after a restart, wait (at most 10 s) for the process
/// that started this one to exit, so it no longer holds the workspace lock.
/// (The single-instance plugin releases its own lock before the restart.)
pub fn wait_for_previous_instance() {
    let Some(pid) = std::env::var(RESTART_AFTER).ok().and_then(|value| value.parse::<u32>().ok()) else {
        return;
    };
    // SAFETY: called at the start of `main`, before any other thread exists.
    unsafe { std::env::remove_var(RESTART_AFTER) };
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(process, 10_000);
            let _ = CloseHandle(process);
        }
    }
    #[cfg(not(windows))]
    let _ = pid;
}
