//! The Professional's updates (docs/edizioni.md, «Firma e aggiornamenti»). Only an installed
//! Professional built with the updater's public key and the address of its `latest.json`
//! (`QAMILLE_UPDATER_PUBKEY`, `QAMILLE_UPDATER_ENDPOINT`, at build time) registers the plugin; every
//! other build never does, and nothing in it checks for updates. The plugin is used from Rust only:
//! the page has no permission of its own for it, only the two commands below.
//!
//! - A minute after the start, then every 24 hours of wall-clock time (an hour after a check that
//!   failed), `latest.json` is read. A newer version is downloaded at once, in the background, and its
//!   signature checked against the key compiled in.
//! - A package ready: `update-available` to the page, which shows a notice that stays
//!   (`UpdateNotice.tsx`) with «Riavvia e aggiorna»; `update_status` answers a page that loaded after.
//! - The click (`update_install`) marks the package and quits as «Esci» does (`background::quit`): the
//!   page's close guards and the undo queue decide. A guard that keeps the window ends the mark, so a
//!   later «Esci» installs nothing; only the main window closed while quitting arms the install. At the
//!   end of the run (`run_event`, `RunEvent::Exit`) the installer starts and Qamille exits; the
//!   installer starts it again, on screen. An update never installs past the guards.
//!
//! The pure rules are at the bottom (`rules`).
use crate::background;
use qamille::application::{AppError, AppResult};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Compiled in by the Professional's release pipeline; absent from every other build.
const PUBKEY: Option<&str> = option_env!("QAMILLE_UPDATER_PUBKEY");
const ENDPOINT: Option<&str> = option_env!("QAMILLE_UPDATER_ENDPOINT");

const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
/// How often the wait looks at the wall clock: a sleep does not count the time the PC was asleep.
const LOOK_EVERY: Duration = Duration::from_secs(10 * 60);
/// How often the end of a quit started by «Riavvia e aggiorna» is looked for (`watch_quit`).
const QUIT_LOOK: Duration = Duration::from_millis(100);

/// What the page shows of a package ready to install.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

/// A package downloaded and verified, waiting for «Riavvia e aggiorna».
struct Ready {
    update: Update,
    bytes: Vec<u8>,
    info: UpdateInfo,
}

/// The plugin is registered (`plugin` gave it to the builder): only then may anything ask it.
static ON: AtomicBool = AtomicBool::new(false);
static READY: Mutex<Option<Ready>> = Mutex::new(None);
static INSTALL: Mutex<rules::Install> = Mutex::new(rules::Install::Idle);

fn words(italian: &'static str, english: &'static str) -> &'static str {
    if crate::window_shell::italian() { italian } else { english }
}

/// The updater plugin, or `None`: then it is never registered. The key goes to the plugin from here
/// (`Builder::pubkey`); since the plugin does not start without its settings either (`plugins.updater`,
/// whose `pubkey` field it requires), they are written into `config` with the same key, never in
/// `tauri.conf.json`. The address goes to each check (`UpdaterBuilder::endpoints`), so an address the
/// plugin refuses fails a check, never the start.
pub fn plugin(config: &mut tauri::Config) -> Option<TauriPlugin<Wry, tauri_plugin_updater::Config>> {
    if !rules::updater_on(PUBKEY, ENDPOINT, qamille::edition::edition().preset().is_some(), background::installed()) {
        return None;
    }
    let key = PUBKEY.unwrap_or_default().trim();
    config.plugins.0.insert(
        "updater".into(),
        // Passive: the installer shows its progress, asks nothing, and can start Qamille again (`/R`).
        serde_json::json!({ "pubkey": key, "windows": { "installMode": "passive" } }),
    );
    ON.store(true, Ordering::SeqCst);
    Some(tauri_plugin_updater::Builder::new().pubkey(key).build())
}

/// The run's events (`main.rs`): the checks start when the app is ready; the main window closed while
/// quitting arms a requested install, which runs at the very end.
pub fn run_event(app: &AppHandle, event: &RunEvent) {
    match event {
        RunEvent::Ready => spawn_checks(app),
        RunEvent::WindowEvent { label, event: WindowEvent::Destroyed, .. } if label == "main" => {
            let mut install = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
            *install = install.window_destroyed(background::is_quitting());
        }
        RunEvent::Exit => install_if_armed(app),
        _ => {}
    }
}

/// The checks, on a thread of their own: a minute after the start, then a day apart. Without the
/// plugin, nothing.
pub fn spawn_checks(app: &AppHandle) {
    if !ON.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("qamille-updates".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        // Each failure once in native.log, not once an hour while the PC stays offline.
        let mut last_error = String::new();
        loop {
            let wait = match check(&app) {
                Ok(()) => {
                    last_error.clear();
                    CHECK_EVERY
                }
                Err(error) => {
                    if error != last_error {
                        crate::native_feel::report(&format!("the update check failed: {error}"));
                        last_error = error;
                    }
                    RETRY_AFTER
                }
            };
            let since = SystemTime::now();
            while !rules::due(since, SystemTime::now(), wait) {
                std::thread::sleep(LOOK_EVERY);
            }
        }
    });
    if let Err(error) = spawned {
        crate::native_feel::report(&format!("the update checks could not start: {error}"));
    }
}

/// One check: a newer version is downloaded and verified, kept for «Riavvia e aggiorna», and the page
/// is told. A version already downloaded is not downloaded again.
fn check(app: &AppHandle) -> Result<(), String> {
    let endpoint: tauri::Url = ENDPOINT.unwrap_or_default().trim().parse().map_err(|e| format!("the address of latest.json: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        // The installer starts Qamille again without the arguments of its first start: neither the
        // `--autostart` of a start at sign-in (the person asked from the window, it comes back on
        // screen) nor a mail link already opened.
        .restart_after_install(false)
        .installer_args(rules::RESTART_ARGS)
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let ready = READY.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|ready| ready.info.version.clone());
    if !rules::needs_download(ready.as_deref(), &update.version) {
        return Ok(());
    }
    // `download` checks the signature against the key: a package that does not match never gets here.
    let bytes = tauri::async_runtime::block_on(update.download(|_, _| {}, || {})).map_err(|e| e.to_string())?;
    let info = UpdateInfo { version: update.version.clone(), notes: update.body.clone().unwrap_or_default() };
    *READY.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ready { update, bytes, info: info.clone() });
    let _ = app.emit_to("main", "update-available", info);
    Ok(())
}

/// The package ready to install, for a page that loaded after `update-available`.
#[tauri::command]
pub fn update_status() -> Option<UpdateInfo> {
    READY.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|ready| ready.info.clone())
}

/// «Riavvia e aggiorna»: the package waits for the quit, which goes through the page's close guards as
/// «Esci» does. A guard that keeps the window ends it (`watch_quit`); otherwise `run_event` installs.
#[tauri::command]
pub fn update_install(app: AppHandle) -> AppResult<()> {
    if READY.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        return Err(AppError::new("not_found", words("Nessun aggiornamento è pronto da installare.", "No update is ready to install.")));
    }
    let first = {
        let mut install = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
        let first = *install == rules::Install::Idle;
        *install = install.request();
        first
    };
    background::quit(&app);
    if first {
        watch_quit(&app);
    }
    Ok(())
}

/// While the quit runs: a guard that kept the window ends «Esci» (`background::close_settled`) with the
/// window still there, and the request with it.
fn watch_quit(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("qamille-update-quit".into()).spawn(move || loop {
        std::thread::sleep(QUIT_LOOK);
        let open = app.get_webview_window("main").is_some();
        let mut install = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
        *install = install.quit_settled(background::is_quitting(), open);
        if !install.waiting() {
            break;
        }
    });
}

/// The end of the run: with the install armed, the installer starts (Tauri's own cleanup first) and the
/// process exits (`Update::install` does not return then). A failure is said: the person expected
/// Qamille to come back.
fn install_if_armed(app: &AppHandle) {
    if !INSTALL.lock().unwrap_or_else(|e| e.into_inner()).installs_on_exit() {
        return;
    }
    let Some(ready) = READY.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    // The window was on screen when the person asked: the next Qamille comes back on screen.
    background::before_restart(app);
    if let Err(error) = ready.update.install(&ready.bytes) {
        crate::native_feel::report(&format!("the update to {} could not start: {error}", ready.info.version));
        let text = if crate::window_shell::italian() {
            format!("Qamille non ha potuto avviare l'aggiornamento alla versione {}. Al prossimo avvio lo scarica di nuovo.\n\n{error}", ready.info.version)
        } else {
            format!("Qamille could not start the update to version {}. It downloads it again at its next start.\n\n{error}", ready.info.version)
        };
        rfd::MessageDialog::new().set_title("Qamille").set_description(text).set_level(rfd::MessageLevel::Error).show();
    }
}

/// The pure rules of the updates, without Tauri.
pub mod rules {
    use std::time::{Duration, SystemTime};

    /// Whether the updater runs: the build has the updater's public key and an `https://` address of
    /// `latest.json` (the plugin refuses any other), the edition is the Professional, and it was
    /// installed by its installer (the update is that installer; a portable copy has none). Anything
    /// missing: no plugin, no check.
    pub fn updater_on(pubkey: Option<&str>, endpoint: Option<&str>, professional: bool, installed: bool) -> bool {
        let key = pubkey.is_some_and(|key| !key.trim().is_empty());
        let address = endpoint.is_some_and(|address| address.trim().to_ascii_lowercase().starts_with("https://"));
        key && address && professional && installed
    }

    /// An install asked with «Riavvia e aggiorna», through the quit that follows.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Install {
        /// Nothing to install at the end.
        Idle,
        /// Asked: the close guards are deciding.
        Requested,
        /// The guards let the window go: the installer starts at the end of the run.
        Armed,
    }

    impl Install {
        pub fn request(self) -> Self {
            match self {
                Install::Armed => Install::Armed,
                _ => Install::Requested,
            }
        }

        /// Looked at while the quit runs: «Esci» over (`quitting` false) with the window still there
        /// means a guard kept it, and the request ends with it.
        pub fn quit_settled(self, quitting: bool, window_open: bool) -> Self {
            match self {
                Install::Requested if window_open && !quitting => Install::Idle,
                other => other,
            }
        }

        /// The main window was destroyed: through the guards of this quit only while quitting.
        pub fn window_destroyed(self, quitting: bool) -> Self {
            match self {
                Install::Requested if quitting => Install::Armed,
                Install::Requested => Install::Idle,
                other => other,
            }
        }

        /// The quit has not ended yet.
        pub fn waiting(self) -> bool {
            self == Install::Requested
        }

        pub fn installs_on_exit(self) -> bool {
            self == Install::Armed
        }
    }

    /// Whether `wait` has passed since `since` on the wall clock. A clock set back is a check now,
    /// rather than a wait of unknown length.
    pub fn due(since: SystemTime, now: SystemTime, wait: Duration) -> bool {
        now.duration_since(since).map_or(true, |passed| passed >= wait)
    }

    /// Whether `found` must be downloaded: not when it is the version already ready.
    pub fn needs_download(ready: Option<&str>, found: &str) -> bool {
        ready != Some(found)
    }

    /// What the installer gets after its own options: start Qamille again (`/R`) with what follows
    /// `/ARGS`, that is nothing.
    pub const RESTART_ARGS: [&str; 2] = ["/R", "/ARGS"];
}

#[cfg(test)]
mod tests {
    use super::rules::{self, Install};
    use std::time::{Duration, SystemTime};

    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk=";
    const ADDRESS: &str = "https://updates.qamille.example/latest.json";

    #[test]
    fn updater_is_off_without_key_endpoint_professional_and_install() {
        // Every combination of the four: only all of them turn the updater on.
        for mask in 0..16u8 {
            let key = (mask & 1 != 0).then_some(KEY);
            let address = (mask & 2 != 0).then_some(ADDRESS);
            let professional = mask & 4 != 0;
            let installed = mask & 8 != 0;
            assert_eq!(
                rules::updater_on(key, address, professional, installed),
                mask == 15,
                "key {key:?}, address {address:?}, professional {professional}, installed {installed}"
            );
        }
        // A value set to nothing is no value, and an address that is not https:// is no address: the
        // plugin would refuse it.
        for (key, address) in [
            ("", ADDRESS),
            ("  ", ADDRESS),
            (KEY, ""),
            (KEY, " "),
            (KEY, "http://updates.qamille.example/latest.json"),
            (KEY, "updates.qamille.example/latest.json"),
        ] {
            assert!(!rules::updater_on(Some(key), Some(address), true, true), "{key:?} {address:?}");
        }
        assert!(rules::updater_on(Some(KEY), Some(" HTTPS://updates.qamille.example/latest.json "), true, true));
    }

    #[test]
    fn a_vetoed_quit_installs_nothing() {
        // «Riavvia e aggiorna», then a guard keeps the window (an unsaved draft, «Resta aperto»): «Esci»
        // ends with the window still there, and nothing is left to install.
        let asked = Install::Idle.request();
        assert!(asked.waiting());
        let running = asked.quit_settled(true, true);
        assert_eq!(running, Install::Requested, "the guards are still deciding");
        let vetoed = running.quit_settled(false, true);
        assert!(!vetoed.waiting());
        assert!(!vetoed.installs_on_exit());
        // A later «Esci» from the tray closes the window for good: still nothing to install.
        assert!(!vetoed.window_destroyed(true).installs_on_exit());
        // An exit that never closed the window through the page (WebView2 failed, recovery.rs) installs
        // nothing either.
        assert!(!Install::Idle.request().quit_settled(true, true).installs_on_exit());
        // A window closed while not quitting is not this quit.
        assert!(!Install::Idle.request().window_destroyed(false).installs_on_exit());
        // Only the guards letting the window go arm the install.
        let closed = Install::Idle.request().quit_settled(true, true).window_destroyed(true);
        assert!(closed.installs_on_exit());
        assert!(!closed.waiting());
        // The window gone before its Destroyed event is seen: still waiting for that event.
        assert_eq!(Install::Idle.request().quit_settled(true, false), Install::Requested);
        assert_eq!(Install::Idle.request().quit_settled(false, false), Install::Requested);
        // Once armed, nothing undoes it, and a second click changes nothing.
        assert_eq!(closed.quit_settled(false, true), Install::Armed);
        assert_eq!(closed.request(), Install::Armed);
        assert!(!Install::Idle.installs_on_exit());
    }

    #[test]
    fn updates_are_checked_a_day_apart_on_the_wall_clock() {
        let day = Duration::from_secs(24 * 60 * 60);
        let since = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(!rules::due(since, since, day));
        assert!(!rules::due(since, since + day - Duration::from_secs(1), day));
        assert!(rules::due(since, since + day, day));
        // The clock set back: a check now rather than a wait of unknown length.
        assert!(rules::due(since, since - Duration::from_secs(60), day));
    }

    #[test]
    fn updates_download_only_a_version_not_already_ready() {
        assert!(rules::needs_download(None, "1.2.0"));
        assert!(!rules::needs_download(Some("1.2.0"), "1.2.0"));
        assert!(rules::needs_download(Some("1.2.0"), "1.3.0"));
    }

    #[test]
    fn updates_restart_qamille_on_screen_without_the_first_starts_arguments() {
        // The installer starts Qamille again (`/R`) with what follows `/ARGS`: nothing, so neither the
        // `--autostart` of a start at sign-in nor a mail link already opened comes back.
        assert_eq!(rules::RESTART_ARGS, ["/R", "/ARGS"]);
    }
}
