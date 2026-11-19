//! Native look and feel for the app's WebView2 windows (docs/aspetto-nativo.md).
//!
//! A native Windows app does not reload, print, zoom or offer "Back" when the
//! user presses F5, Ctrl+P or right-clicks a label, and never shows a browser
//! error page. WebView2 does all of that by default; this module turns it off
//! for every window the app opens, and hands the failures WebView2 would answer
//! with its error page to `recovery.rs`. (WebView2's password autosave is off
//! by default and wry leaves it so: nothing to do here.)
//! `desktop/tests/native-feel.mjs` checks that every window calls it.

use tauri::{AppHandle, Manager, Url, WebviewWindow, window::Color};

/// `--ground` of the dark theme in `desktop/src/ds/tokens/theme.css`: the colour
/// `html`, `body` and `.app-shell` paint, so a resize never shows a strip of
/// another colour. The light value is `app.windows[0].backgroundColor` in
/// `tauri.conf.json`. The test keeps both in sync with the stylesheet.
pub const DARK_GROUND: Color = Color(0x12, 0x16, 0x1e, 0xff);

/// The main window is built hidden: paint it in the appearance chosen in
/// Settings (or the system theme) first, so the first frame is never the other
/// theme's colour before the page has drawn.
pub fn show_main_window(window: &WebviewWindow) {
    if crate::window_shell::apply_startup_preferences(window) {
        let _ = window.set_background_color(Some(DARK_GROUND));
    }
    let _ = window.show();
}

/// The `on_navigation` hook of every window: only the app's own page stays in
/// it. A web or mail link the page tries to open there goes to the browser
/// (`open_outside`, the main window) or nowhere (the reminder chip).
pub fn navigation_guard(app: &AppHandle, open_outside: bool) -> impl Fn(&Url) -> bool + Send + 'static {
    let dev_origin = tauri::is_dev()
        .then(|| app.config().build.dev_url.as_ref().map(|url| url.origin().ascii_serialization()))
        .flatten();
    move |url| {
        let address = url.as_str();
        if crate::shell_rules::is_app_page(address, dev_origin.as_deref()) {
            return true;
        }
        if open_outside && crate::shell_rules::opens_outside(address) {
            let address = address.to_owned();
            std::thread::spawn(move || {
                let _ = open::that(address);
            });
        }
        false
    }
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
/// instead. Editing and copy shortcuts (Ctrl+C, Ctrl+V, Ctrl+A,
/// Ctrl+Z) keep working, and the page still receives every key as `keydown`.
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

#[cfg(windows)]
mod webview2 {
    use super::{MenuEntry, menu_removals};
    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND, COREWEBVIEW2_CONTEXT_MENU_ITEM_KIND_SEPARATOR,
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_WEB_ERROR_STATUS,
        COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED, ICoreWebView2_11,
        ICoreWebView2ContextMenuRequestedEventArgs, ICoreWebView2Controller, ICoreWebView2Settings3,
    };
    use webview2_com::{
        ContextMenuRequestedEventHandler, NavigationCompletedEventHandler, ProcessFailedEventHandler, take_pwstr,
    };
    use windows_core::{BOOL, Interface, PWSTR};

    pub unsafe fn harden(controller: &ICoreWebView2Controller, window: WebviewWindow, page: Option<tauri::Url>) -> windows_core::Result<()> {
        unsafe {
            let core = controller.CoreWebView2()?;
            // Handlers first, switches last: if one step fails the later ones are not done, so
            // the engine error page is never off without `recovery.rs` there to answer instead.
            // shortcut: no rollback or tokens kept; the handlers that did register work on their
            // own, and a webview is never rebuilt in place (a reload keeps it, the chip is destroyed).
            let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                match args {
                    Some(args) => trim_menu(&args),
                    None => Ok(()),
                }
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
}
