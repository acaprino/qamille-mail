//! Native look and feel for the app's WebView2 windows (docs/aspetto-nativo.md).
//!
//! A native Windows app does not reload, print, zoom or offer "Back" when the
//! user presses F5, Ctrl+P or right-clicks a label, and never shows a browser
//! error page or permission prompt. WebView2 does all of that by default; this module turns it off
//! for every window the app opens, and hands the failures WebView2 would answer
//! with its error page to `recovery.rs`. (WebView2's password autosave is off
//! by default and wry leaves it so: nothing to do here.)
//! `desktop/tests/native-feel.mjs` checks that every window calls it.

use std::collections::BTreeSet;
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, WebviewWindow, Wry, window::Color};

/// `--color-paper` of the dark theme in `desktop/src/theme/compatto.css`: the colour
/// `html`, `body` and `.app-shell` paint, so a resize never shows a strip of
/// another colour. The light value is `app.windows[0].backgroundColor` in
/// `tauri.conf.json`. The test keeps both in sync with the stylesheet.
pub const DARK_GROUND: Color = Color(0x1e, 0x1e, 0x1e, 0xff);

/// The main window is built hidden: paint it in the appearance chosen in
/// Settings (or the system theme) first, so the first frame is never the other
/// theme's colour before the page has drawn.
pub fn show_main_window(window: &WebviewWindow) {
    prepare_main_window(window);
    crate::window_shell::show_main(window);
}

/// Everything `show_main_window` does but showing: theme, zoom, position (`window_shell`) and
/// background. A start at sign-in stops here, in the tray; `background::restore` shows it later.
pub fn prepare_main_window(window: &WebviewWindow) {
    if crate::window_shell::apply_startup_preferences(window) {
        let _ = window.set_background_color(Some(DARK_GROUND));
    }
}

/// Tells WebView2 whether its window is on screen (`ICoreWebView2Controller::SetIsVisible`): hiding
/// or minimizing the host window does not, and a webview told it is hidden stops drawing, slows its
/// timers and reports `document.visibilityState` "hidden". `background.rs` calls it when the window
/// goes to the tray or is minimized, and with `true` before the window shows again.
pub fn set_webview_visible(window: &WebviewWindow, visible: bool) {
    #[cfg(windows)]
    {
        let label = window.label().to_owned();
        let result = window.with_webview(move |webview| {
            // SAFETY: the controller belongs to this live webview and is used on the thread that owns
            // it, which is where `with_webview` runs.
            if let Err(error) = unsafe { webview.controller().SetIsVisible(visible) } {
                report(&format!("could not tell the webview of {label} it is {}: {error}", if visible { "visible" } else { "hidden" }));
            }
        });
        if let Err(error) = result {
            report(&format!("webview {} unavailable to show or hide: {error}", window.label()));
        }
    }
    #[cfg(not(windows))]
    let _ = (window, visible);
}

/// The `on_navigation` hook of every window: only the app's own page stays in
/// it. A web link the page tries to open there goes to the browser and a mail link to Qamille's
/// composer (`open_outside`, the main window), or nowhere (the reminder chip).
pub fn navigation_guard(app: &AppHandle, open_outside: bool) -> impl Fn(&Url) -> bool + Send + 'static {
    let dev_origin = dev_origin(app);
    let app = app.clone();
    move |url| {
        let address = url.as_str();
        if crate::shell_rules::is_app_page(address, dev_origin.as_deref()) {
            return true;
        }
        if open_outside && crate::shell_rules::opens_outside(address) {
            if crate::launch_args::has_mail_scheme(address) {
                crate::launch_args::from_navigation(&app, address.to_owned());
            } else {
                open_in_browser(address.to_owned());
            }
        }
        false
    }
}

/// The `on_new_window` hook of the main window, which never gets a window of its own: a link with
/// `target="_blank"`, a script's `window.open`. wry would deny it and the link would do nothing, so a
/// web or mail address (`shell_rules::opens_from_new_window`: the kinds `open_link` accepts, never the
/// app's own page) goes elsewhere instead: a web address to the system browser, a mail address to
/// Qamille's composer. The answer is always Deny.
pub fn new_window_guard(app: &AppHandle) -> impl Fn(Url, NewWindowFeatures) -> NewWindowResponse<Wry> + Send + 'static {
    let dev_origin = dev_origin(app);
    let app = app.clone();
    move |url, _features| {
        if crate::shell_rules::opens_from_new_window(url.as_str(), dev_origin.as_deref()) {
            if crate::launch_args::has_mail_scheme(url.as_str()) {
                crate::launch_args::from_navigation(&app, url.to_string());
            } else {
                open_in_browser(url.to_string());
            }
        }
        NewWindowResponse::Deny
    }
}

/// The dev server's origin in a dev build, which is the app's own page there (`shell_rules::is_app_page`).
fn dev_origin(app: &AppHandle) -> Option<String> {
    tauri::is_dev()
        .then(|| app.config().build.dev_url.as_ref().map(|url| url.origin().ascii_serialization()))
        .flatten()
}

/// The system browser, as `open_link` opens it (`open::that`); off the calling thread, since the
/// webview waits for the hook that asked. A failure has nobody to tell: the link just does not open.
fn open_in_browser(address: String) {
    std::thread::spawn(move || {
        let _ = open::that(address);
    });
}

/// The address Tauri loads for `WebviewUrl::App(page)`, as tauri 2.12's
/// `prepare_pending_webview` builds it: the dev server in a dev build, else
/// `http://tauri.localhost/` (`useHttpsScheme` is off); `index.html` is the base itself.
pub fn app_page(app: &AppHandle, page: &str) -> Option<Url> {
    let base = match (tauri::is_dev(), &app.config().build.dev_url) {
        (true, Some(dev)) => dev.clone(),
        _ => Url::parse("http://tauri.localhost/").ok()?,
    };
    if page == "index.html" { Some(base) } else { base.join(page).ok() }
}

/// A native shell failure: on stderr for development, and one line in
/// `<data>/native.log`, since a release build has no console to show it.
pub fn report(line: &str) {
    use std::io::Write;
    eprintln!("Qamille: {line}");
    let path = qamille::model::Store::data_dir().join("native.log");
    // shortcut: starts over past 256 KB instead of rotating; enough for rare native failures.
    let start_over = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 256 * 1024);
    let mut options = std::fs::OpenOptions::new();
    if start_over { options.write(true).truncate(true) } else { options.append(true) };
    if let Ok(mut file) = options.create(true).open(&path) {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = writeln!(file, "{at} {line}");
    }
}

/// Turns off WebView2's browser shortcuts, browser entries in the right-click
/// menu and its error page, and lets `recovery.rs` answer a failed page
/// instead. Refuses Back and Forward, any frame but the message document and
/// every permission the page asks for. Editing and copy shortcuts (Ctrl+C,
/// Ctrl+V, Ctrl+A, Ctrl+Z) keep working, and the page still receives every key as `keydown`.
/// `page` is the app page the window was built with: it is loaded again once
/// the settings are in, because WebView2 applies them from the next navigation.
pub fn harden_webview(window: &WebviewWindow, page: &str) {
    #[cfg(windows)]
    {
        let label = window.label().to_owned();
        let owner = window.clone();
        let page = app_page(window.app_handle(), page);
        let result = window.with_webview(move |webview| {
            // SAFETY: the controller belongs to this live webview and is used
            // on the thread that owns it, which is where `with_webview` runs.
            if let Err(error) = unsafe { webview2::harden(&webview.controller(), owner, page) } {
                report(&format!("could not set native webview behavior for {label}: {error}"));
            }
        });
        if let Err(error) = result {
            report(&format!("webview {} unavailable for native behavior: {error}", window.label()));
        }
    }
    #[cfg(not(windows))]
    let _ = (window, page);
}

/// The only right-click entries WebView2 may show (`Name` of
/// `ICoreWebView2ContextMenuItem`, unlocalized): editing, emoji, spelling
/// suggestions (`spellCheck`), and copying a link or an image of a message.
/// An allow-list: whatever a runtime adds (look up, translate, read aloud,
/// copy link to highlight with a `tauri.localhost` address…) stays out.
const MENU_ITEMS_KEPT: &[&str] = &[
    "undo",
    "redo",
    "cut",
    "copy",
    "paste",
    "pasteAndMatchStyle",
    "selectAll",
    "emoji",
    "spellCheck",
    "copyLinkLocation",
    "copyImage",
    // Debug builds only: right-click → Inspect is how developers open DevTools.
    #[cfg(debug_assertions)]
    "inspectElement",
];

/// One entry of a context menu as WebView2 lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub name: String,
    pub separator: bool,
}

/// Indices to remove, highest first, so removing them in order keeps the rest
/// valid: entries not in `MENU_ITEMS_KEPT`, then separators left leading,
/// trailing or doubled.
pub fn menu_removals(entries: &[MenuEntry]) -> Vec<usize> {
    let kept: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].separator || MENU_ITEMS_KEPT.contains(&entries[i].name.as_str()))
        .collect();
    let mut keep = vec![false; entries.len()];
    let mut previous_separator = true;
    for &i in &kept {
        if entries[i].separator && previous_separator {
            continue;
        }
        keep[i] = true;
        previous_separator = entries[i].separator;
    }
    if let Some(&last) = kept.iter().rev().find(|&&i| keep[i])
        && entries[last].separator
    {
        keep[last] = false;
    }
    (0..entries.len()).rev().filter(|&i| !keep[i]).collect()
}

/// The names of the entries `removals` takes out that `logged` does not hold yet, in menu order,
/// added to it. Microsoft does not document the names, so the ones the filter removes are written to
/// `native.log` once each, to check `MENU_ITEMS_KEPT` against a real runtime. Separators are not named.
pub fn names_to_log(entries: &[MenuEntry], removals: &[usize], logged: &mut BTreeSet<String>) -> Vec<String> {
    removals
        .iter()
        .rev()
        .map(|&i| &entries[i])
        .filter(|entry| !entry.separator && logged.insert(entry.name.clone()))
        .map(|entry| entry.name.clone())
        .collect()
}

#[cfg(windows)]
mod webview2 {
    use super::{MenuEntry, menu_removals, names_to_log, report};
    use std::collections::BTreeSet;
    use std::sync::{Mutex, Once, PoisonError};
    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
        COREWEBVIEW2_NAVIGATION_KIND, COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD, COREWEBVIEW2_PERMISSION_STATE,
        COREWEBVIEW2_PERMISSION_STATE_DEFAULT, COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_PROCESS_FAILED_KIND,
        COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, ICoreWebView2_11,
        ICoreWebView2ContextMenuRequestedEventArgs, ICoreWebView2Controller, ICoreWebView2NavigationStartingEventArgs,
        ICoreWebView2NavigationStartingEventArgs3, ICoreWebView2PermissionRequestedEventArgs3, ICoreWebView2Settings3,
    };
    use webview2_com::{
        ContextMenuRequestedEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler,
        PermissionRequestedEventHandler, ProcessFailedEventHandler, take_pwstr,
    };
    use windows_core::{BOOL, Interface, PWSTR};

    /// The menu entry names already in `native.log` from this run (`names_to_log`).
    static LOGGED_NAMES: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

    pub unsafe fn harden(controller: &ICoreWebView2Controller, window: WebviewWindow, page: Option<tauri::Url>) -> windows_core::Result<()> {
        unsafe {
            let core = controller.CoreWebView2()?;
            // Handlers first, switches last: if one step fails the later ones are not done, so
            // the engine error page is never off without `recovery.rs` there to answer instead.
            // shortcut: no rollback or tokens kept; the handlers that did register work on their
            // own, and a webview is never rebuilt in place (a reload keeps it, the chip is destroyed).
            let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                // Closed on failure: a menu the filter could not read or finish trimming would
                // show the browser's entries.
                if let Err(error) = trim_menu(&args) {
                    report(&format!("context menu not shown, its filter failed: {error}"));
                    args.SetHandled(true)?;
                }
                Ok(())
            }));
            let mut token = 0i64;
            core.cast::<ICoreWebView2_11>()?.add_ContextMenuRequested(&handler, &mut token)?;

            let failed = window.clone();
            let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                crate::recovery::process_failed(&failed, kind.0);
                Ok(())
            }));
            core.add_ProcessFailed(&handler, &mut token)?;

            // Top-level navigations only (frames have their own event).
            let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut success = BOOL(1);
                args.IsSuccess(&mut success)?;
                if success.as_bool() {
                    return Ok(());
                }
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.WebErrorStatus(&mut status)?;
                // Cancelled: refused by `navigation_guard` or replaced by a newer
                // navigation (a reload). Not a page that failed.
                if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                    crate::recovery::page_not_loaded(&window, status.0);
                }
                Ok(())
            }));
            core.add_NavigationCompleted(&handler, &mut token)?;

            // Back and Forward load nothing: the page stops the mouse's buttons (`browser-guards.ts`),
            // but a blank or crashed page runs no script, and Back would load the app again. wry's
            // handler (`navigation_guard`) sets Cancel for every navigation; this one, added later,
            // has the last word, if WebView2 calls the handlers in the order they were added. That is
            // an assumption, not something Microsoft documents: item 19 of "Verifica nell'app vera"
            // (docs/aspetto-nativo.md) checks it on the exe (Back on a reader must not step back).
            let handler = NavigationStartingEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                if steps_through_history(&args) {
                    args.SetCancel(true)?;
                }
                Ok(())
            }));
            core.add_NavigationStarting(&handler, &mut token)?;

            // A frame shows only `shell_rules::frame_may_navigate`'s documents, and never steps back
            // to an earlier message (each one the reader shows is a history entry of its frame).
            let handler = NavigationStartingEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut uri = PWSTR::null();
                // An address that cannot be read is not the message document.
                let refused = args.Uri(&mut uri).is_err() || !crate::shell_rules::frame_may_navigate(&take_pwstr(uri));
                if refused || steps_through_history(&args) {
                    args.SetCancel(true)?;
                }
                Ok(())
            }));
            core.add_FrameNavigationStarting(&handler, &mut token)?;

            // Microphone, camera, location, notifications, reading the clipboard: the app asks for none,
            // and a browser's "tauri.localhost wants to…" prompt does not belong in a Windows app. Denied
            // without asking, unless a handler added earlier (wry's, for a permission Tauri was told to
            // allow) has answered.
            let handler = PermissionRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut state = COREWEBVIEW2_PERMISSION_STATE::default();
                if args.State(&mut state).is_ok() && state != COREWEBVIEW2_PERMISSION_STATE_DEFAULT {
                    return Ok(());
                }
                // Not kept in the profile, where it would outlive a later version that allows it. Best
                // effort: a denial kept there is still a denial.
                if let Ok(args) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                    let _ = args.SetSavesInProfile(false);
                }
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)
            }));
            core.add_PermissionRequested(&handler, &mut token)?;

            let settings = core.Settings()?;
            // F5/Ctrl+R reload, Ctrl+P print, Ctrl+F find, Ctrl+±/wheel zoom, F12.
            settings.cast::<ICoreWebView2Settings3>()?.SetAreBrowserAcceleratorKeysEnabled(false)?;
            // A browser's "Can't reach this page": a failed page is the app's to
            // explain (`recovery.rs`). Off, WebView2 leaves the page blank.
            settings.SetIsBuiltInErrorPageEnabled(false)?;
            // The first load started in `build()`, before these settings: Microsoft applies
            // them from the next navigation, so F5, Ctrl+P and the error page would still work
            // on the first document. Load the page again now; the first load, still in flight,
            // ends as cancelled, which the handler above ignores.
            if let Some(page) = page {
                core.Navigate(&windows_core::HSTRING::from(page.as_str()))?;
            }
            Ok(())
        }
    }

    unsafe fn trim_menu(args: &ICoreWebView2ContextMenuRequestedEventArgs) -> windows_core::Result<()> {
        unsafe {
            let items = args.MenuItems()?;
            let mut count = 0u32;
            items.Count(&mut count)?;
            let mut entries = Vec::with_capacity(count as usize);
            for index in 0..count {
                let item = items.GetValueAtIndex(index)?;
                let mut name = PWSTR::null();
                item.Name(&mut name)?;
                let mut kind = COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND::default();
                item.Kind(&mut kind)?;
                entries.push(MenuEntry {
                    name: take_pwstr(name),
                    separator: kind == COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
                });
            }
            let removals = menu_removals(&entries);
            let names = names_to_log(&entries, &removals, &mut LOGGED_NAMES.lock().unwrap_or_else(PoisonError::into_inner));
            if !names.is_empty() {
                report(&format!("context menu entries removed: {}", names.join(", ")));
            }
            for &index in &removals {
                items.RemoveValueAtIndex(index as u32)?;
            }
            // Nothing useful left (a label, an icon): show no menu at all.
            if removals.len() == entries.len() {
                args.SetHandled(true)?;
            }
            Ok(())
        }
    }

    /// Whether a navigation steps through the history: Back or Forward. A runtime that cannot tell
    /// lets it through, said once in `native.log`: refusing every navigation would refuse the app's page.
    unsafe fn steps_through_history(args: &ICoreWebView2NavigationStartingEventArgs) -> bool {
        unsafe {
            let mut kind = COREWEBVIEW2_NAVIGATION_KIND::default();
            match args.cast::<ICoreWebView2NavigationStartingEventArgs3>().and_then(|args| args.NavigationKind(&mut kind)) {
                Ok(()) => kind == COREWEBVIEW2_NAVIGATION_KIND_BACK_OR_FORWARD,
                Err(error) => {
                    static SAID: Once = Once::new();
                    SAID.call_once(|| report(&format!("Back and Forward refused only by the page: no navigation kind ({error})")));
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(items: &[&str]) -> Vec<MenuEntry> {
        items
            .iter()
            .map(|&name| MenuEntry { name: name.into(), separator: name == "-" })
            .collect()
    }

    fn after(items: &[&str]) -> Vec<String> {
        let entries = menu(items);
        let removals = menu_removals(&entries);
        assert!(removals.windows(2).all(|pair| pair[0] > pair[1]), "removals must run highest first");
        (0..entries.len()).filter(|i| !removals.contains(i)).map(|i| entries[i].name.clone()).collect()
    }

    #[test]
    fn page_menu_on_a_label_disappears() {
        assert!(after(&["back", "forward", "reload", "-", "saveAs", "print", "-", "share"]).is_empty());
    }

    #[test]
    fn editing_entries_stay_without_stray_separators() {
        assert_eq!(
            after(&["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "-", "selectAll", "-", "print"]),
            ["emoji", "-", "undo", "redo", "-", "cut", "copy", "paste", "-", "selectAll"],
        );
    }

    #[test]
    fn spelling_suggestions_stay_and_unknown_entries_go() {
        assert_eq!(
            after(&["spellCheck", "spellCheck", "-", "lookUp", "translate", "readAloud", "-", "copy", "copyLinkToHighlight"]),
            ["spellCheck", "spellCheck", "-", "copy"],
        );
    }

    #[test]
    fn a_link_or_an_image_can_be_copied_not_opened_or_saved() {
        assert_eq!(after(&["openLinkInNewWindow", "copyLinkLocation", "saveLinkAs"]), ["copyLinkLocation"]);
        assert_eq!(after(&["saveImageAs", "copyImage", "copyImageLocation", "-", "webCapture"]), ["copyImage"]);
    }

    #[test]
    fn inspect_only_in_debug_builds() {
        assert_eq!(after(&["copy", "-", "inspectElement"]).len() == 3, cfg!(debug_assertions));
    }

    #[test]
    fn each_name_the_filter_takes_out_is_logged_once() {
        let mut logged = BTreeSet::new();
        let entries = menu(&["back", "copy", "-", "print", "-", "back"]);
        assert_eq!(names_to_log(&entries, &menu_removals(&entries), &mut logged), ["back", "print"]);
        // A later menu adds only the names not written yet; kept entries and separators never appear.
        let entries = menu(&["back", "saveAs", "-", "copy"]);
        assert_eq!(names_to_log(&entries, &menu_removals(&entries), &mut logged), ["saveAs"]);
        assert!(names_to_log(&entries, &menu_removals(&entries), &mut logged).is_empty());
        assert!(!logged.contains("copy") && !logged.contains("-"));
    }
}
