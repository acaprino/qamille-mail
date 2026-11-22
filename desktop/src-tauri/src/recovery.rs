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
//! - The page loads but App does not draw within `STARTUP_WAIT` (its script fails or
//!   never runs, its first render throws), or its root fails later (`frontend_failed`,
//!   from React's `onUncaughtError`): offer to reload it. The watch starts again with
//!   every page the window loads, also a reload the page starts itself, and Esc reloads
//!   too: an empty window never stays empty.
//! - "Ricarica la finestra" in the system menu: the page saves what it can,
//!   as at sign-out, then reloads. If something was not saved, or the page
//!   does not answer within `RELOAD_WAIT`, ask: never discard by itself.
//!
//! - Closing the main window: the page decides (its close guards save or ask),
//!   so a page that does not answer within `CLOSE_ANSWER`, or a close still
//!   running after `CLOSE_STUCK` when the person asks again, gets a choice here:
//!   wait, or close Qamille without the page.
//!
//! Every way out that leaves without the page (`exit_app`, `restart_app`, the forced
//! close, a failed start) first stops and saves a running recording and sends the undo
//! queue (`finish_meetings_before_exit`), as the page's close guards would: otherwise the
//! recording is found "interrupted" at the next start, and a send still undoable waits
//! for it in silence (audit Q8).
//! A failed start (`startup_failed` in `main.rs`) says why itself: from then on no dialog
//! here opens (`stop_asking`).
//!
//! A window in the tray (`background.rs`, also a start at sign-in) never opens "Qamille non
//! si è aperto" by itself: a page that did not draw, or failed, while it was hidden is
//! offered a reload at the first `background::restore` (`offer_reload_if_not_ready`). A
//! reload never shows a hidden window.
//!
//! WebView2 reports these from its own callbacks, which must not run a dialog's
//! modal loop: every dialog and reload here runs on a thread of its own, and
//! one dialog at a time covers a failure reported by several windows.

use crate::runtime::DesktopState;
use crate::shell_rules::{self, CapturePhase, Failure};
use qamille::domain::meeting::CaptureState;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::webview::{PageLoadEvent, PageLoadPayload};
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

/// The page draws within this after the window shows or reloads, or the app offers a reload.
const STARTUP_WAIT: Duration = Duration::from_secs(15);
/// The longest an exit waits for a recording to be stopped and saved (normally well under
/// a second): a way out of a broken window must stay a way out.
const EXIT_SAVE_WAIT: Duration = Duration::from_secs(20);

/// The main page now loading has drawn (`frontend_ready`); reset at every load it watches.
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
/// The question the hidden window did not ask (`watch_startup`, `frontend_failed`): `Some(opened)`
/// for `offer_reload`, asked at the first restore. Cleared by every load the watch arms.
static UNASKED: Mutex<Option<bool>> = Mutex::new(None);
/// Watches armed so far (`watch_startup`): the newest retires the older ones.
static PAGE_LOADS: AtomicU64 = AtomicU64::new(0);
static CLOSE_ASKED: AtomicU64 = AtomicU64::new(0);
static CLOSE_SEEN: AtomicU64 = AtomicU64::new(0);
static CLOSING_SINCE: Mutex<Option<Instant>> = Mutex::new(None);

static ASKING: AtomicBool = AtomicBool::new(false);
/// Qamille is leaving by itself (a failed start, `stop_asking`): no recovery dialog opens.
static STOPPED_ASKING: AtomicBool = AtomicBool::new(false);
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

/// Runs `decide` on its own thread unless a recovery dialog is already open, or Qamille
/// has stopped asking.
fn ask_once(window: &WebviewWindow, decide: impl FnOnce(&WebviewWindow) + Send + 'static) {
    if STOPPED_ASKING.load(Ordering::SeqCst) || ASKING.swap(true, Ordering::SeqCst) {
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
    // The new page must draw again: watched from now, in case it never starts loading;
    // `page_load` arms the watch again when it does.
    if window.label() == "main" {
        watch_startup(window);
    }
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
                    if close { exit_app(window.app_handle()) } else { reload(window) }
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
            if STOPPED_ASKING.load(Ordering::SeqCst) || ENGINE_GONE.swap(true, Ordering::SeqCst) {
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
                if close { exit_app(window.app_handle()) } else { restart_app(window.app_handle()) }
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
    // In the tray nothing asks by itself: the failed load never draws, so its 15 s watch leaves the
    // question to the first restore (`offer_reload_if_not_ready`).
    if !window.is_visible().unwrap_or(true) {
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
        if close { exit_app(window.app_handle()) } else { reload(window) }
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

/// Arms the watch of the main page's current load: a page that loads but never draws
/// (its script fails or never runs, App's first render throws) would leave an empty
/// window with no message. Armed by `page_load` whenever a page starts loading, whoever
/// started it, and also when the window shows and by `reload`, so a load Qamille starts
/// is watched even if it never starts loading. The newest arm retires the older ones:
/// each load gets one watch, its whole `STARTUP_WAIT` and at most one dialog. A window in the
/// tray asks nothing: the question waits for the first restore (`offer_reload_if_not_ready`).
pub fn watch_startup(window: &WebviewWindow) {
    FRONTEND_READY.store(false, Ordering::SeqCst);
    *UNASKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let load = PAGE_LOADS.fetch_add(1, Ordering::SeqCst) + 1;
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(STARTUP_WAIT);
        if PAGE_LOADS.load(Ordering::SeqCst) != load
            || FRONTEND_READY.load(Ordering::SeqCst)
            || window.app_handle().get_webview_window("main").is_none()
        {
            return;
        }
        if !window.is_visible().unwrap_or(true) {
            *UNASKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(false);
            return;
        }
        ask_once(&window, |window| offer_reload(window, false));
    });
}

/// The first `background::restore` after the hidden window's page did not draw, or failed: the
/// question it did not ask then, now that the window is in view. Nothing when the page has drawn
/// since, or a later load is being watched.
pub fn offer_reload_if_not_ready(window: &WebviewWindow) {
    let Some(opened) = UNASKED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    if !opened && FRONTEND_READY.load(Ordering::SeqCst) {
        return;
    }
    ask_once(window, move |window| offer_reload(window, opened));
}

/// The main window's `on_page_load` (`main.rs`): every page that starts loading in it
/// (the first one, a reload by Qamille or one the page starts itself, a `location.reload()`)
/// is watched from its own start. Runs inside a WebView2 callback: arming asks nothing here.
pub fn page_load(window: WebviewWindow, payload: PageLoadPayload<'_>) {
    if payload.event() == PageLoadEvent::Started {
        watch_startup(&window);
        // The new page has no compose-mailto listener yet: a mail link waits for its take (rule 32).
        crate::launch_args::page_started();
    }
}

/// The main window is empty: its page never drew (`watch_startup`), or its root failed
/// (`frontend_failed`), after it had drawn when `opened`. Esc and the X reload too: here
/// that is the harmless choice, since any other would leave the window empty.
fn offer_reload(window: &WebviewWindow, opened: bool) {
    let (title, text) = if opened {
        (
            words("La finestra di Qamille è vuota", "Qamille's window is empty"),
            words(
                "Un errore ha lasciato vuota la finestra di Qamille. La posta e le bozze salvate sono al sicuro.",
                "An error left Qamille's window empty. Your mail and saved drafts are safe.",
            ),
        )
    } else {
        (
            words("Qamille non si è aperto", "Qamille did not open"),
            words(
                "La finestra di Qamille non ha finito di aprirsi. La posta e le bozze salvate sono al sicuro.",
                "Qamille's window did not finish opening. Your mail and saved drafts are safe.",
            ),
        )
    };
    let close = ask(
        window,
        title,
        text,
        words("Ricarica la finestra", "Reload the window"),
        words("Chiudi Qamille", "Close Qamille"),
    ) == Some(false);
    if close { exit_app(window.app_handle()) } else { reload(window) }
}

/// App's first effect ran (`src/App.tsx`): the page rendered and drew its first frame.
#[tauri::command]
pub fn frontend_ready() {
    FRONTEND_READY.store(true, Ordering::SeqCst);
}

/// The main page's root failed (React's `onUncaughtError`, `src/main.tsx`): with no error
/// boundary React removes the whole tree, so the window is empty. Logged, then the same
/// choice as a page that never drew.
#[tauri::command]
pub fn frontend_failed(window: WebviewWindow, message: String) {
    // One bounded line: the page's text goes into `native.log`.
    let message: String = message.chars().map(|c| if c.is_control() { ' ' } else { c }).take(500).collect();
    crate::native_feel::report(&format!("the page of {} failed: {message}", window.label()));
    if window.label() != "main" {
        return;
    }
    let opened = FRONTEND_READY.load(Ordering::SeqCst);
    if !window.is_visible().unwrap_or(true) {
        *UNASKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(opened);
        return;
    }
    ask_once(&window, move |window| offer_reload(window, opened));
}

/// Main's `CloseRequested` (X, Alt+F4, the taskbar, the system menu) when it closes for real
/// (`background::hides_on_close` false: «Esci», or the close button set to quit). The page's
/// close listener has the decision: Tauri cancels the close for it.
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
        // «Esci» from the tray: the window comes back with the question.
        if !window.is_visible().unwrap_or(true) {
            crate::background::restore(window.app_handle());
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
                finish_meetings_before_exit(window.app_handle());
                let _ = window.destroy();
            } else {
                // «Attendi», or Esc: «Esci» is over until the person asks again, so the next X
                // hides as usual instead of quitting.
                crate::background::close_settled();
            }
        });
    });
}

/// The page got the close request: it is alive. `true`: its guards run and it closes the window;
/// `false`: Rust only put the window in the tray (`background::hides_on_close`), nothing closes.
#[tauri::command]
pub fn close_seen() -> bool {
    CLOSE_SEEN.store(CLOSE_ASKED.load(Ordering::SeqCst), Ordering::SeqCst);
    !crate::background::hides_on_close()
}

/// The page's close ended without closing (a guard kept the window, or failed): also the end of
/// an «Esci» (`background::close_settled`).
#[tauri::command]
pub fn close_settled() {
    *CLOSING_SINCE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    crate::background::close_settled();
}

/// A failed start says why and exits (`startup_failed`, `main.rs`): from now on no recovery
/// dialog opens over that message for the page loading behind it, whose answers could only
/// reload or quit an app that is already leaving.
pub fn stop_asking() {
    STOPPED_ASKING.store(true, Ordering::SeqCst);
}

/// Quits Qamille without the page: its recording is stopped and saved first.
fn exit_app(app: &AppHandle) {
    finish_meetings_before_exit(app);
    app.exit(0);
}

fn restart_app(app: &AppHandle) {
    finish_meetings_before_exit(app);
    // The new process gets this one's arguments, `--autostart` too: it comes back as the window was.
    crate::background::before_restart(app);
    // SAFETY: the variable is only read by the next process, at its start.
    unsafe { std::env::set_var(RESTART_AFTER, std::process::id().to_string()) };
    app.request_restart();
}

/// Before the app leaves without its page: stops and saves a running recording, then
/// stops the meeting jobs for the app closing, as the page's meetings close guard does
/// (`App.tsx`), and sends the undo queue, as its undo-queue guard does (`lib/undo-close.ts`,
/// audit Q8). Blocks until that is done, at most `EXIT_SAVE_WAIT` for both: call it from a
/// thread of its own, or from `startup_failed` (`main.rs`) just before its exit, never from a
/// WebView2 callback. Without the app's state it does nothing. Failures go to `native.log`
/// and never stop the exit; what did not leave in time goes at the next start.
pub fn finish_meetings_before_exit(app: &AppHandle) {
    let (done, finished) = std::sync::mpsc::channel();
    let handle = app.clone();
    // A little before the wait below gives up, so a queue still sending is not reported as a
    // recording not saved.
    let deadline = Instant::now() + EXIT_SAVE_WAIT.saturating_sub(Duration::from_millis(500));
    let spawned = std::thread::Builder::new().name("qamille-exit-meetings".into()).spawn(move || {
        stop_recording_and_jobs(&handle);
        send_undo_queue(&handle, deadline);
        let _ = done.send(());
    });
    match spawned {
        Ok(_) => {
            if let Err(error) = finished.recv_timeout(EXIT_SAVE_WAIT) {
                crate::native_feel::report(&format!("the recording was not confirmed saved before exit: {error}"));
            }
        }
        Err(error) => crate::native_feel::report(&format!("could not stop the recording before exit: {error}")),
    }
}

fn stop_recording_and_jobs(app: &AppHandle) {
    let Some(state) = app.try_state::<DesktopState>() else { return };
    // `capture_state` reads the recorder alone, so unlike `capture_status` it cannot fail on
    // the repository while the recorder is live. If even that fails, stop anyway: stopping
    // a recorder that is not recording changes nothing.
    let live = match state.meetings.capture_state() {
        Ok(capture) => shell_rules::must_stop_capture(capture_phase(&capture)),
        Err(error) => {
            crate::native_feel::report(&format!("could not read the recorder before exit: {}", error.message));
            true
        }
    };
    if live {
        if let Err(error) = state.meetings.stop() {
            crate::native_feel::report(&format!("could not stop the recording before exit: {}", error.message));
        }
    }
    state.meetings.cancel_all_jobs();
}

/// Every action still undoable leaves now (`UndoService::send_now`); then waits, until
/// `deadline`, for the queue to settle, or less when it stops moving for `UNDO_STALL` (no
/// connection, or a failed start whose queue thread never ran). Queued ones stay for the next
/// start; one still on its way is cut off, and the next start marks it failed without retrying it.
fn send_undo_queue(app: &AppHandle, deadline: Instant) {
    const UNDO_STALL: Duration = Duration::from_secs(5);
    let Some(state) = app.try_state::<DesktopState>() else { return };
    if let Err(error) = state.undo.send_now() {
        crate::native_feel::report(&format!("the undo queue was not sent before exit: {}", error.message));
        return;
    }
    let mut last = (usize::MAX, Instant::now());
    loop {
        match state.undo.unsettled() {
            Ok(left) if left.total() == 0 => return,
            Ok(left) if Instant::now() >= deadline || (left.total() == last.0 && last.1.elapsed() >= UNDO_STALL) => {
                crate::native_feel::report(&format!(
                    "undo queue at exit: {} send(s) and {} action(s) queued for the next start, {} send(s) and {} action(s) cut off on their way (the next start marks them failed)",
                    left.sends, left.actions, left.sending, left.confirming
                ));
                return;
            }
            Ok(left) => {
                let remaining = left.total();
                if remaining != last.0 {
                    last = (remaining, Instant::now());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => {
                crate::native_feel::report(&format!("could not read the undo queue before exit: {}", error.message));
                return;
            }
        }
    }
}

/// No wildcard: a new recorder state must be sorted here.
fn capture_phase(state: &CaptureState) -> CapturePhase {
    match state {
        CaptureState::Idle => CapturePhase::Idle,
        CaptureState::Preparing => CapturePhase::Preparing,
        CaptureState::Recording => CapturePhase::Recording,
        CaptureState::Paused => CapturePhase::Paused,
        CaptureState::Saving => CapturePhase::Saving,
        CaptureState::Saved => CapturePhase::Saved,
        CaptureState::Interrupted => CapturePhase::Interrupted,
    }
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
