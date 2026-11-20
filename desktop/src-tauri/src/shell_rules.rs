//! Pure rules behind the Windows shell integration of `window_shell.rs`
//! (docs/aspetto-nativo.md). Only `std`: the tests run on any machine with
//! `rustc --edition 2024 --test src/shell_rules.rs`, without Tauri or Windows.

/// System menu commands (`WM_SYSCOMMAND`), as in WinUser.h.
pub const SC_SIZE: u32 = 0xF000;
pub const SC_MOVE: u32 = 0xF010;
pub const SC_MINIMIZE: u32 = 0xF020;
pub const SC_MAXIMIZE: u32 = 0xF030;
pub const SC_CLOSE: u32 = 0xF060;
pub const SC_RESTORE: u32 = 0xF120;

/// Which system menu entries are enabled, as Windows shows them for a normal
/// window: a maximized one can be restored but not moved, sized or maximized.
pub fn system_menu_states(maximized: bool) -> [(u32, bool); 6] {
    [
        (SC_RESTORE, maximized),
        (SC_MOVE, !maximized),
        (SC_SIZE, !maximized),
        (SC_MINIMIZE, true),
        (SC_MAXIMIZE, !maximized),
        (SC_CLOSE, true),
    ]
}

/// The command the maximize button sends: it restores a maximized window.
pub fn maximize_button_command(maximized: bool) -> u32 {
    if maximized { SC_RESTORE } else { SC_MAXIMIZE }
}

/// The app's own zoom steps (Ctrl+= / Ctrl+- / Ctrl+0), on top of the Windows
/// text size. The low steps let a large Windows text size be undone in the app.
pub const ZOOM_STEPS: &[f64] = &[0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
/// The page never gets smaller or larger than this, whatever the two factors.
pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

/// The next user zoom: the nearest step above or below, or 100 %.
pub fn next_zoom(current: f64, step: ZoomStep) -> f64 {
    let current = clamp_user_zoom(current);
    match step {
        ZoomStep::Reset => 1.0,
        ZoomStep::In => ZOOM_STEPS.iter().copied().find(|&z| z > current + 0.001).unwrap_or(current),
        ZoomStep::Out => ZOOM_STEPS.iter().rev().copied().find(|&z| z < current - 0.001).unwrap_or(current),
    }
}

/// A saved user zoom read back from disk: anything odd is 100 %.
pub fn clamp_user_zoom(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1])
    } else {
        1.0
    }
}

/// `HKCU\Software\Microsoft\Accessibility\TextScaleFactor` (Settings →
/// Accessibility → Text size) is a percentage from 100 to 225; absent is 100.
pub fn text_scale(raw: Option<u32>) -> f64 {
    match raw {
        Some(percent) if (100..=225).contains(&percent) => f64::from(percent) / 100.0,
        _ => 1.0,
    }
}

/// The zoom the webview gets: the Windows text size times the app's own zoom.
pub fn effective_zoom(user: f64, text_scale: f64) -> f64 {
    let value = clamp_user_zoom(user) * if text_scale.is_finite() { text_scale } else { 1.0 };
    // Two decimals: 1.1 × 1.25 must not drift into 1.3750000000000002 on screen.
    ((value.clamp(ZOOM_MIN, ZOOM_MAX)) * 100.0).round() / 100.0
}

/// The appearance chosen in Settings, as the page saves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    System,
    Light,
    Dark,
}

pub fn parse_appearance(value: &str) -> Appearance {
    match value {
        "dark" => Appearance::Dark,
        "light" => Appearance::Light,
        _ => Appearance::System,
    }
}

pub fn appearance_name(value: Appearance) -> &'static str {
    match value {
        Appearance::Dark => "dark",
        Appearance::Light => "light",
        Appearance::System => "system",
    }
}

/// Whether the window starts dark: an explicit choice wins over the system.
pub fn starts_dark(appearance: Appearance, system_dark: bool) -> bool {
    match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::System => system_dark,
    }
}

/// A client rectangle in physical pixels, as the page reports a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A rectangle from the page (CSS pixels × devicePixelRatio, maybe fractional
/// or negative while laid out): rounded outward, `None` when it covers nothing.
pub fn pixel_rect(x: f64, y: f64, width: f64, height: f64) -> Option<PixelRect> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width < 1.0 || height < 1.0 {
        return None;
    }
    let (left, top) = (x.floor(), y.floor());
    let (right, bottom) = ((x + width).ceil(), (y + height).ceil());
    if right <= 0.0 || bottom <= 0.0 || left > 32_000.0 || top > 32_000.0 {
        return None;
    }
    Some(PixelRect { x: left as i32, y: top as i32, width: (right - left) as i32, height: (bottom - top) as i32 })
}

/// Packs a screen point into an `LPARAM`-sized value (both halves signed: a
/// monitor left of the primary one has negative coordinates) and back.
pub fn pack_point(x: i32, y: i32) -> isize {
    ((x as u32 as u64) | ((y as u32 as u64) << 32)) as isize
}

pub fn unpack_point(value: isize) -> (i32, i32) {
    let value = value as u64;
    (value as u32 as i32, (value >> 32) as u32 as i32)
}

/// What the maximize button looks like while the system handles its mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Rest,
    Hover,
    Pressed,
}

pub fn button_state_name(state: ButtonState) -> &'static str {
    match state {
        ButtonState::Rest => "rest",
        ButtonState::Hover => "hover",
        ButtonState::Pressed => "pressed",
    }
}

/// The app's own system menu entry, "Ricarica la finestra": it reloads the page
/// from the native side, so it works while the page is blank or dead. Below
/// 0xF000 and a multiple of 16: Windows uses the low four bits of `wParam`.
pub const SC_APP_RELOAD: u32 = 0x5C10;

/// Automatic reloads after the page's process exits: at most this many within
/// `RELOAD_WINDOW_MS`, then the app asks instead of looping.
pub const RELOAD_LIMIT: usize = 3;
pub const RELOAD_WINDOW_MS: u64 = 120_000;

/// `earlier`: when the previous automatic reloads happened (ms on any clock
/// that only grows). True: reload once more without asking.
pub fn may_reload_by_itself(earlier: &[u64], now: u64) -> bool {
    earlier.iter().filter(|&&at| now.saturating_sub(at) < RELOAD_WINDOW_MS).count() < RELOAD_LIMIT
}

/// What the app does when one of WebView2's processes fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The page's renderer exited: the page is gone, reload it.
    PageExited,
    /// The page's renderer stopped answering: ask before reloading, it may recover.
    PageHung,
    /// The browser process exited: every webview of the app is closed for good.
    EngineExited,
    /// A helper (GPU, utility, a frame's renderer): WebView2 recovers by itself.
    Other,
}

/// `COREWEBVIEW2_PROCESS_FAILED_KIND` values, as in WebView2.h.
pub fn failure_of(kind: i32) -> Failure {
    match kind {
        0 => Failure::EngineExited,
        1 => Failure::PageExited,
        2 => Failure::PageHung,
        _ => Failure::Other,
    }
}

/// The meeting recorder's state, mirrored here (as `domain::meeting::CaptureState`)
/// so this file stays free of dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePhase {
    Idle,
    Preparing,
    Recording,
    Paused,
    Saving,
    Saved,
    Interrupted,
}

/// Whether leaving the app must stop the recorder first: the same states the page's
/// close guard stops (`captureActive` in `lib/meetings.ts`). Exiting under any of them
/// would cut the audio, and the meeting would be found "interrupted" at the next start.
pub fn must_stop_capture(phase: CapturePhase) -> bool {
    matches!(phase, CapturePhase::Preparing | CapturePhase::Recording | CapturePhase::Paused | CapturePhase::Saving)
}

/// Whether `url` is the app's own page, the only one its windows may show:
/// Tauri's local origin (`http://tauri.localhost` on Windows, or `https` with
/// `useHttpsScheme`) or, in a dev build, the dev server's origin.
pub fn is_app_page(url: &str, dev_origin: Option<&str>) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") && authority.eq_ignore_ascii_case("tauri.localhost") {
        return true;
    }
    dev_origin.is_some_and(|dev| {
        let dev = dev.trim_end_matches('/');
        dev.split_once("://").is_some_and(|(dev_scheme, dev_authority)| {
            dev_scheme.eq_ignore_ascii_case(scheme) && dev_authority.eq_ignore_ascii_case(authority)
        })
    })
}

/// A link the page tried to open in its own window goes to the browser instead,
/// but only the kinds `open_link` accepts.
pub fn opens_outside(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && !url.contains(['\r', '\n', '\0'])
}

/// A window the page asks for (`target="_blank"`, `window.open`) opens in the browser when it is a web or
/// mail address, and never when it is the app's own page: only a relative link, which resolves against
/// the app's address, gets there, and it is no link of a message.
pub fn opens_from_new_window(url: &str, dev_origin: Option<&str>) -> bool {
    opens_outside(url) && !is_app_page(url, dev_origin)
}

/// Whether a frame of the app's pages may load `uri`: the message document, which `MessageHtml.tsx`
/// gives its frame as `srcdoc`, or an empty frame. A link in a message opens in the browser, never in
/// its frame, and no frame loads the app page (a second app inside the first) or the web. A link to a
/// part of the message (`#…`) is no exception: a `srcdoc` frame has no address of its own, so the
/// browser resolves it against the app's (`http://tauri.localhost/#…`) and would start a navigation to
/// it, which this refuses. `MessageHtml.tsx` cancels the click and scrolls to the part itself, so the
/// frame never asks.
pub fn frame_may_navigate(uri: &str) -> bool {
    matches!(uri, "about:srcdoc" | "about:blank")
}

/// Microsoft's download of the WebView2 Runtime (Evergreen bootstrapper).
pub const WEBVIEW2_DOWNLOAD: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// The message of a start that failed (`startup_failed` in `main.rs`): the reason, or, when the
/// WebView2 Runtime is missing, what that is and where to get it (its lookup error names nothing
/// a person could act on; `native.log` keeps it).
pub fn startup_failure_text(error: &str, italian: bool, webview_missing: bool) -> String {
    let headline = if italian { "Qamille non è riuscito ad avviarsi." } else { "Qamille could not start." };
    if !webview_missing {
        return format!("{headline}\n\n{error}");
    }
    let advice = if italian {
        "Manca il componente di Windows che disegna le finestre (Microsoft Edge WebView2 Runtime). Installalo da"
    } else {
        "The Windows component that draws the windows (Microsoft Edge WebView2 Runtime) is missing. Install it from"
    };
    let again = if italian { "e riapri Qamille." } else { "and open Qamille again." };
    format!("{headline}\n\n{advice} {WEBVIEW2_DOWNLOAD} {again}")
}

/// The text a panic carries (`panic!` gives a `&str` or a `String`), so a start that panicked
/// can say why like one that failed.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "no message".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maximized_window_can_only_be_restored_minimized_or_closed() {
        let enabled = |max| system_menu_states(max).iter().filter(|(_, on)| *on).map(|(c, _)| *c).collect::<Vec<_>>();
        assert_eq!(enabled(true), [SC_RESTORE, SC_MINIMIZE, SC_CLOSE]);
        assert_eq!(enabled(false), [SC_MOVE, SC_SIZE, SC_MINIMIZE, SC_MAXIMIZE, SC_CLOSE]);
        assert_eq!(maximize_button_command(true), SC_RESTORE);
        assert_eq!(maximize_button_command(false), SC_MAXIMIZE);
    }

    #[test]
    fn zoom_moves_one_step_and_stops_at_the_ends() {
        assert_eq!(next_zoom(1.0, ZoomStep::In), 1.1);
        assert_eq!(next_zoom(1.0, ZoomStep::Out), 0.9);
        assert_eq!(next_zoom(2.0, ZoomStep::In), 2.0);
        assert_eq!(next_zoom(0.5, ZoomStep::Out), 0.5);
        assert_eq!(next_zoom(1.75, ZoomStep::Reset), 1.0);
        // A value between steps goes to the next one, not two steps.
        assert_eq!(next_zoom(1.2, ZoomStep::In), 1.25);
        assert_eq!(next_zoom(1.2, ZoomStep::Out), 1.1);
        assert_eq!(next_zoom(f64::NAN, ZoomStep::In), 1.1);
    }

    #[test]
    fn windows_text_size_multiplies_the_app_zoom_within_bounds() {
        assert_eq!(text_scale(None), 1.0);
        assert_eq!(text_scale(Some(150)), 1.5);
        assert_eq!(text_scale(Some(99)), 1.0);
        assert_eq!(text_scale(Some(400)), 1.0);
        assert_eq!(effective_zoom(1.0, 1.0), 1.0);
        assert_eq!(effective_zoom(1.1, 1.25), 1.38);
        assert_eq!(effective_zoom(2.0, 2.25), ZOOM_MAX);
        assert_eq!(effective_zoom(0.5, 1.0), ZOOM_MIN);
        assert_eq!(effective_zoom(0.67, 1.5), 1.01);
    }

    #[test]
    fn an_explicit_appearance_wins_over_the_system_theme() {
        assert!(starts_dark(parse_appearance("dark"), false));
        assert!(!starts_dark(parse_appearance("light"), true));
        assert!(starts_dark(parse_appearance("system"), true));
        assert!(!starts_dark(parse_appearance("nonsense"), false));
        for a in [Appearance::System, Appearance::Light, Appearance::Dark] {
            assert_eq!(parse_appearance(appearance_name(a)), a);
        }
    }

    #[test]
    fn page_rectangles_round_outward_and_empty_ones_are_none() {
        assert_eq!(pixel_rect(10.4, 0.0, 45.2, 32.0), Some(PixelRect { x: 10, y: 0, width: 46, height: 32 }));
        assert_eq!(pixel_rect(0.0, 0.0, 0.0, 32.0), None);
        assert_eq!(pixel_rect(f64::NAN, 0.0, 46.0, 32.0), None);
        assert_eq!(pixel_rect(-100.0, 0.0, 46.0, 32.0), None);
    }

    #[test]
    fn screen_points_survive_packing_also_left_of_the_primary_monitor() {
        for (x, y) in [(0, 0), (1919, 1079), (-1280, 12), (-5, -7), (i32::MAX, i32::MIN)] {
            assert_eq!(unpack_point(pack_point(x, y)), (x, y));
        }
    }

    #[test]
    fn the_page_reloads_by_itself_three_times_in_two_minutes_then_asks() {
        assert!(may_reload_by_itself(&[], 0));
        assert!(may_reload_by_itself(&[1_000, 2_000], 3_000));
        assert!(!may_reload_by_itself(&[1_000, 2_000, 3_000], 4_000));
        // Old reloads stop counting once the window has passed.
        assert!(may_reload_by_itself(&[1_000, 2_000, 3_000], 1_000 + RELOAD_WINDOW_MS));
        assert!(SC_APP_RELOAD < 0xF000 && SC_APP_RELOAD % 16 == 0);
    }

    #[test]
    fn process_failures_map_to_what_the_app_does() {
        assert_eq!(failure_of(0), Failure::EngineExited);
        assert_eq!(failure_of(1), Failure::PageExited);
        assert_eq!(failure_of(2), Failure::PageHung);
        for helper in [3, 4, 5, 6, 7, 8, 9, -1] {
            assert_eq!(failure_of(helper), Failure::Other);
        }
    }

    #[test]
    fn only_the_app_origin_stays_in_the_window() {
        assert!(is_app_page("http://tauri.localhost/", None));
        assert!(is_app_page("https://tauri.localhost/index.html?x#y", None));
        assert!(is_app_page("http://TAURI.localhost", None));
        assert!(!is_app_page("http://tauri.localhost.example.com/", None));
        assert!(!is_app_page("http://tauri.localhost@example.com/", None));
        assert!(!is_app_page("https://example.com/", None));
        assert!(!is_app_page("tauri.localhost", None));
        assert!(!is_app_page("http://127.0.0.1:1420/", None));
        assert!(is_app_page("http://127.0.0.1:1420/reminder.html", Some("http://127.0.0.1:1420")));
        assert!(is_app_page("http://127.0.0.1:1420/", Some("http://127.0.0.1:1420/")));
        assert!(!is_app_page("https://127.0.0.1:1420/", Some("http://127.0.0.1:1420")));
        assert!(!is_app_page("http://127.0.0.1:1421/", Some("http://127.0.0.1:1420")));
    }

    #[test]
    fn only_web_and_mail_links_open_in_the_browser() {
        assert!(opens_outside("https://example.com/a"));
        assert!(opens_outside("HTTP://example.com"));
        assert!(opens_outside("mailto:someone@example.com"));
        assert!(!opens_outside("file:///C:/Windows/System32/calc.exe"));
        assert!(!opens_outside("javascript:alert(1)"));
        assert!(!opens_outside("https://example.com/\r\nx"));
    }

    #[test]
    fn a_new_window_never_sends_the_apps_own_page_to_the_browser() {
        assert!(opens_from_new_window("https://example.com/a", None));
        assert!(opens_from_new_window("mailto:someone@example.com", Some("http://127.0.0.1:1420")));
        // A relative link of a message resolves against the app's own address.
        assert!(!opens_from_new_window("http://tauri.localhost/foo", None));
        assert!(!opens_from_new_window("HTTP://TAURI.LOCALHOST/%C2%A0https://example.com/x", None));
        assert!(!opens_from_new_window("http://127.0.0.1:1420/foo", Some("http://127.0.0.1:1420")));
        // Without a dev server that address is just a web address.
        assert!(opens_from_new_window("http://127.0.0.1:1420/foo", None));
        assert!(!opens_from_new_window("file:///C:/x", None));
        assert!(!opens_from_new_window("https://example.com/\r\nx", None));
    }

    #[test]
    fn only_the_message_document_may_load_in_a_frame() {
        assert!(frame_may_navigate("about:srcdoc"));
        assert!(frame_may_navigate("about:blank"));
        assert!(!frame_may_navigate("https://x.it/"));
        assert!(!frame_may_navigate("http://tauri.localhost/index.html"));
        // Chromium's mark for a navigation it blocked: refused, the frame keeps its message.
        assert!(!frame_may_navigate("about:blank#blocked"));
        for refused in [
            "",
            "about:",
            "about:srcdocs",
            "about://blank",
            "data:text/html,<p>x</p>",
            "javascript:alert(1)",
            "blob:http://tauri.localhost/4f1c",
            "file:///C:/x.html",
            "mailto:a@x.it",
            "http://127.0.0.1:1420/",
        ] {
            assert!(!frame_may_navigate(refused), "{refused}");
        }
    }

    #[test]
    fn only_an_active_or_paused_capture_is_stopped_before_exit() {
        assert!(!must_stop_capture(CapturePhase::Idle));
        assert!(must_stop_capture(CapturePhase::Recording));
        assert!(must_stop_capture(CapturePhase::Paused));
        // Starting and saving are live too: the page's close guard stops them (`captureActive`).
        assert!(must_stop_capture(CapturePhase::Preparing));
        assert!(must_stop_capture(CapturePhase::Saving));
        // Already ended, saved or interrupted: nothing left to stop.
        assert!(!must_stop_capture(CapturePhase::Saved));
        assert!(!must_stop_capture(CapturePhase::Interrupted));
    }

    #[test]
    fn startup_failure_names_the_reason_and_the_runtime() {
        let busy = "Questa cartella è già aperta in un'altra interfaccia Qamille.";
        let italian = startup_failure_text(busy, true, false);
        assert!(italian.starts_with("Qamille non è riuscito ad avviarsi."), "{italian}");
        assert!(italian.contains(busy), "the reason must be said: {italian}");
        let english = startup_failure_text("file is not a database", false, false);
        assert!(english.starts_with("Qamille could not start."), "{english}");
        assert!(english.contains("file is not a database"), "the reason must be said: {english}");
        // The runtime's own error names no component: the text must say what is missing and where to get it.
        let lookup = "The system cannot find the file specified. (0x80070002)";
        for italian in [true, false] {
            let missing = startup_failure_text(lookup, italian, true);
            assert!(missing.contains("WebView2"), "{missing}");
            assert!(missing.contains("https://go.microsoft.com/fwlink/p/?LinkId=2124703"), "{missing}");
        }
        assert!(startup_failure_text(lookup, true, true).starts_with("Qamille non è riuscito ad avviarsi."));
        assert!(startup_failure_text(lookup, false, true).starts_with("Qamille could not start."));
    }

    #[test]
    fn a_panic_at_startup_keeps_its_message() {
        fn payload(start: fn()) -> Box<dyn std::any::Any + Send> {
            std::panic::catch_unwind(start).expect_err("the start must panic")
        }
        // `panic!` with a literal carries a `&str`, with arguments a `String`.
        assert_eq!(panic_message(&*payload(|| panic!("store missing"))), "store missing");
        assert_eq!(panic_message(&*payload(|| panic!("{} rows unreadable", 3))), "3 rows unreadable");
        assert_eq!(panic_message(&*payload(|| std::panic::panic_any(7_u8))), "no message");
    }
}
