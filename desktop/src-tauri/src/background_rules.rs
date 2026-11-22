//! The pure rules of Qamille's background presence (`background.rs`): whether the close button
//! hides, whether a start stays in the tray, the name and the command of the start-at-sign-in entry,
//! Task Manager's switch, and what to do with the startup file of the old background worker. No Tauri
//! and no Windows here, so the rules are tested with `rustc --edition 2024 --test src/background_rules.rs`.

/// The argument the start-at-sign-in entry passes: such a start stays in the tray.
pub const AUTOSTART_ARG: &str = "--autostart";

/// What the close button (X, Alt+F4, «Chiudi» in the system menu or on the taskbar) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// The window goes to the tray; Qamille keeps sending, reminding and receiving.
    Hide,
    /// A real close: the page's close guards decide, then Qamille exits.
    Quit,
}

/// `close_to_tray`: the setting in Settings → General. `quitting`: «Esci» was chosen (tray or system
/// menu), which closes the window through the same guards whatever the setting.
pub fn close_action(close_to_tray: bool, quitting: bool) -> CloseAction {
    if close_to_tray && !quitting { CloseAction::Hide } else { CloseAction::Quit }
}

/// Whether a start stays in the tray: it carries `AUTOSTART_ARG` (the entry Windows runs at sign-in)
/// and no `mailto:` link, which the person is waiting to see in the composer. `args` are the process
/// arguments, the program first.
pub fn starts_hidden(args: &[String]) -> bool {
    let given = args.get(1..).unwrap_or_default();
    given.iter().any(|arg| arg == AUTOSTART_ARG)
        && !given.iter().any(|arg| arg.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")))
}

/// A start that is a restart (`background::before_restart`) comes back as the window was, since a
/// restart keeps the arguments of the first start, `AUTOSTART_ARG` included. Otherwise `starts_hidden`.
pub fn stays_hidden(args: &[String], restarted_hidden: Option<bool>) -> bool {
    restarted_hidden.unwrap_or_else(|| starts_hidden(args))
}

/// The name of the value under `HKCU\…\CurrentVersion\Run`. The installed Professional has one fixed
/// name, which its uninstaller removes; each portable copy names its folder and `tag`
/// (`workspace_tag`), so two copies, even in folders of the same name, never share an entry (Q22).
pub fn autostart_entry_name(installed: bool, folder: &str, tag: &str) -> String {
    if installed { "Qamille".to_owned() } else { format!("Qamille ({folder}, {tag})") }
}

/// FNV-1a, 64 bits: a hash that never changes between builds (the standard library's does not promise
/// that), so a copy keeps the name of its entry across updates.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// 16 hex digits for a data folder (its canonical path; Windows paths ignore case): the same in every
/// start and every version of one copy, different for another copy. It ends the app identifier
/// (`main.rs`), which names the copy's notification identity in the registry (rule 28).
pub fn workspace_id(data_dir: &str) -> String {
    format!("{:016x}", fnv1a64(data_dir.to_lowercase().as_bytes()))
}

/// The first 8 digits of `workspace_id`, for the name of the start-at-sign-in entry.
pub fn workspace_tag(data_dir: &str) -> String {
    workspace_id(data_dir)[..8].to_owned()
}

/// The command of the Run value: the executable in quotes, then `AUTOSTART_ARG`. Unquoted, Windows
/// would try every prefix of a path with spaces as a program (`D:\Apps\Qamille.exe` for
/// `D:\Apps\Qamille Portable\Qamille.exe`), and anyone may create files at the root of a data drive.
/// A verbatim path (`\\?\`) is written plainly.
pub fn run_command(exe: &str) -> String {
    let exe = match exe.strip_prefix(r"\\?\") {
        Some(rest) => rest.strip_prefix(r"UNC\").map_or_else(|| rest.to_owned(), |share| format!(r"\\{share}")),
        None => exe.to_owned(),
    };
    format!("\"{exe}\" {AUTOSTART_ARG}")
}

/// The twin of the Run value under `StartupApproved\Run` that Qamille writes when it turns the entry
/// on: what Task Manager writes for an entry turned on there.
pub const APPROVED_ON: [u8; 12] = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Whether Windows runs the entry, from its twin: none means on; Task Manager and Settings > Apps >
/// Startup turn it off with an odd first byte (0x03, then when), on again with an even one (0x02, 0x06).
pub fn startup_approved(twin: Option<&[u8]>) -> bool {
    twin.and_then(|bytes| bytes.first()).is_none_or(|first| first & 1 == 0)
}

/// «Esci da Qamille» in the system menu (Alt+Space), under «Chiudi»: it quits even while the close
/// button only hides. Below 0xF000 and a multiple of 16, like `shell_rules::SC_APP_RELOAD`.
pub const SC_APP_QUIT: u32 = 0x5C20;

/// What «Esci» found left in the undo queue (`UndoService::unsettled` in the `app` crate), by what
/// quitting does to it. Queued rows stay in the queue and the next start carries them out; a row
/// on its way dies with the process and the next start marks it failed, never retrying it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueLeft {
    /// Sends still queued (no connection, or credentials the vault cannot open).
    pub sends: u32,
    /// Sends on their way to the provider.
    pub sending: u32,
    /// Other actions still queued: archive, trash, move, a schedule, a discarded draft.
    pub actions: u32,
    /// Other actions being confirmed by the provider.
    pub confirming: u32,
    /// Sends that were queued when «Esci» asked and failed while it waited (no connection): they did
    /// not leave, and their drafts are still in Drafts (`UndoService::unsettled_since` in the `app` crate).
    pub failed: u32,
}
impl QueueLeft {
    pub fn total(&self) -> u32 {
        self.sends
            .saturating_add(self.sending)
            .saturating_add(self.actions)
            .saturating_add(self.confirming)
            .saturating_add(self.failed)
    }
}


/// The question when «Esci» finds actions of the undo queue that did not leave
/// (`commands::undo::undo_quit_anyway`): one line per kind, each saying what quitting anyway does to
/// it. A send that already failed comes first, then what quitting may lose, then what merely waits,
/// and the sends before the actions.
pub fn unsent_text(italian: bool, left: QueueLeft) -> String {
    let failed = match (italian, left.failed) {
        (_, 0) => None,
        (true, 1) => Some("1 messaggio non è partito: è rimasto nelle Bozze.".to_owned()),
        (true, n) => Some(format!("{n} messaggi non sono partiti: sono rimasti nelle Bozze.")),
        (false, 1) => Some("1 message was not sent: it is still in Drafts.".to_owned()),
        (false, n) => Some(format!("{n} messages were not sent: they are still in Drafts.")),
    };
    let sending = match (italian, left.sending) {
        (_, 0) => None,
        (true, 1) => Some("1 messaggio è in invio: uscendo potrebbe non partire, controlla Inviati.".to_owned()),
        (true, n) => Some(format!("{n} messaggi sono in invio: uscendo potrebbero non partire, controlla Inviati.")),
        (false, 1) => Some("1 message is being sent: if you quit it may not leave, check Sent.".to_owned()),
        (false, n) => Some(format!("{n} messages are being sent: if you quit they may not leave, check Sent.")),
    };
    let sends = match (italian, left.sends) {
        (_, 0) => None,
        (true, 1) => Some("1 messaggio non è ancora partito. Se esci, partirà al prossimo avvio di Qamille.".to_owned()),
        (true, n) => Some(format!("{n} messaggi non sono ancora partiti. Se esci, partiranno al prossimo avvio di Qamille.")),
        (false, 1) => Some("1 message has not been sent yet. If you quit, it will be sent the next time Qamille starts.".to_owned()),
        (false, n) => Some(format!("{n} messages have not been sent yet. If you quit, they will be sent the next time Qamille starts.")),
    };
    let confirming = match (italian, left.confirming) {
        (_, 0) => None,
        (true, 1) => Some("1 azione sulla posta è in corso: uscendo potrebbe non completarsi, controlla la casella.".to_owned()),
        (true, n) => Some(format!("{n} azioni sulla posta sono in corso: uscendo potrebbero non completarsi, controlla la casella.")),
        (false, 1) => Some("1 mail action is in progress: if you quit it may not complete, check the mailbox.".to_owned()),
        (false, n) => Some(format!("{n} mail actions are in progress: if you quit they may not complete, check the mailbox.")),
    };
    let actions = match (italian, left.actions) {
        (_, 0) => None,
        (true, 1) => Some("1 azione sulla posta non è ancora stata completata. Se esci, verrà completata al prossimo avvio di Qamille.".to_owned()),
        (true, n) => Some(format!("{n} azioni sulla posta non sono ancora state completate. Se esci, verranno completate al prossimo avvio di Qamille.")),
        (false, 1) => Some("1 mail action has not been completed yet. If you quit, it will be completed the next time Qamille starts.".to_owned()),
        (false, n) => Some(format!("{n} mail actions have not been completed yet. If you quit, they will be completed the next time Qamille starts.")),
    };
    [failed, sending, sends, confirming, actions].into_iter().flatten().collect::<Vec<_>>().join("\n")
}

/// The `native.log` line at session end when the undo queue did not all leave (`undo_send_now` with
/// `session_end`): nobody can answer then, so it is written down, never asked. English, like the
/// rest of the log.
pub fn session_end_line(failed: u32, sends: u32, sending: u32, actions: u32) -> String {
    format!(
        "session end: {failed} send(s) failed while waiting and are in Drafts, {sends} send(s) still queued for the next start, {sending} send(s) cut off on their way (check Sent), {actions} other action(s) left"
    )
}

/// What to do with `Qamille-worker.vbs`, the startup file of the old background worker, now that
/// start at sign-in is Qamille itself in the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLauncher {
    /// It starts this copy's worker: turn on start at sign-in for this copy, then delete it.
    Adopt,
    /// It starts nothing (its executable is gone, or it cannot be read): delete it.
    RemoveStale,
    /// It starts another copy that still exists: leave it, that copy migrates it.
    LeaveToOther,
    /// There is no such file.
    Nothing,
}

/// The executable the startup file runs, as `mcp_config::launcher_content` wrote it:
/// `shell.Run """<exe>"" --worker", 0, False` (VBScript doubles the quotes inside a string).
pub fn launcher_target(content: &str) -> Option<String> {
    let (_, after) = content.split_once("shell.Run \"\"\"")?;
    let (executable, _) = after.split_once("\"\"")?;
    (!executable.is_empty()).then(|| executable.to_owned())
}

/// `content`: the startup file, if there is one. `this_exe`: the running executable.
/// `named_exe_exists`: whether the executable it names (`launcher_target`) exists. Windows paths
/// are compared without case.
pub fn legacy_launcher(content: Option<&str>, this_exe: &str, named_exe_exists: bool) -> LegacyLauncher {
    let Some(content) = content else { return LegacyLauncher::Nothing };
    let same = |a: &str, b: &str| a.replace('/', "\\").to_lowercase() == b.replace('/', "\\").to_lowercase();
    match launcher_target(content) {
        Some(target) if same(&target, this_exe) => LegacyLauncher::Adopt,
        Some(_) if named_exe_exists => LegacyLauncher::LeaveToOther,
        _ => LegacyLauncher::RemoveStale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn close_hides_unless_quitting_or_switched_off() {
        assert_eq!(close_action(true, false), CloseAction::Hide);
        // Quit (the tray, the system menu) goes through the page's guards, whatever the setting.
        assert_eq!(close_action(true, true), CloseAction::Quit);
        // Switched off in Settings: the X closes Qamille as before.
        assert_eq!(close_action(false, false), CloseAction::Quit);
        assert_eq!(close_action(false, true), CloseAction::Quit);
    }

    #[test]
    fn only_the_autostart_argument_starts_hidden() {
        let exe = r"C:\Apps\Qamille\Qamille.exe";
        assert!(starts_hidden(&args(&[exe, AUTOSTART_ARG])));
        assert!(!starts_hidden(&args(&[exe])));
        assert!(!starts_hidden(&args(&[])));
        // Only the argument itself: not a look-alike, not the program's own path.
        for other in ["--autostart=1", "--Autostart", "autostart", "--worker", "-autostart"] {
            assert!(!starts_hidden(&args(&[exe, other])), "{other}");
        }
        assert!(!starts_hidden(&args(&["--autostart"])), "the program path is not an argument");
        // A mail link to open wins: the person is waiting for the composer.
        assert!(!starts_hidden(&args(&[exe, AUTOSTART_ARG, "mailto:someone@example.com"])));
        assert!(!starts_hidden(&args(&[exe, "MAILTO:someone@example.com", AUTOSTART_ARG])));
    }

    #[test]
    fn entry_name_is_fixed_when_installed_and_per_folder_otherwise() {
        assert_eq!(autostart_entry_name(true, "Qamille", "1a2b3c4d"), "Qamille");
        assert_eq!(autostart_entry_name(true, "Elsewhere", "ffffffff"), "Qamille");
        assert_eq!(autostart_entry_name(false, "Qamille-Portable", "1a2b3c4d"), "Qamille (Qamille-Portable, 1a2b3c4d)");
        // Two portable copies in folders of the same name still get one entry each.
        assert_ne!(
            autostart_entry_name(false, "Qamille", "1a2b3c4d"),
            autostart_entry_name(false, "Qamille", "9e8d7c6b"),
        );
    }

    #[test]
    fn the_tag_is_a_stable_hash_of_the_data_folder() {
        // FNV-1a 64, the published test vectors: the same in every Rust release, unlike DefaultHasher.
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
        // One copy's tag, pinned: a future build must give the entry the same name.
        assert_eq!(workspace_tag(r"\\?\D:\Software\Qamille\data"), "6a5c8aa2");
        // Windows paths ignore case.
        assert_eq!(workspace_tag(r"\\?\d:\software\QAMILLE\DATA"), "6a5c8aa2");
        assert_ne!(workspace_tag(r"\\?\D:\Software\Qamille-Professional\data"), "6a5c8aa2");
        assert_eq!(workspace_tag(r"\\?\D:\Software\Qamille-Professional\data").len(), 8);
    }

    #[test]
    fn the_identifier_suffix_is_the_whole_stable_hash_of_the_data_folder() {
        // One copy's suffix, pinned: it names the copy's notification identity in the registry
        // (`AppUserModelId\com.qamille.desktop.workspace<id>`), which a future build must find again.
        assert_eq!(workspace_id(r"\\?\D:\Software\Qamille\data"), "6a5c8aa20e00690a");
        assert_eq!(workspace_id(r"\\?\d:\software\QAMILLE\DATA"), "6a5c8aa20e00690a");
        assert_eq!(workspace_id(r"\\?\D:\Software\Qamille-Professional\data"), "b4fdf48fe39f6e12");
        // The start-at-sign-in tag is its first 8 digits.
        assert_eq!(workspace_tag(r"\\?\D:\Software\Qamille\data"), workspace_id(r"\\?\D:\Software\Qamille\data")[..8]);
    }

    #[test]
    fn the_run_command_quotes_the_executable() {
        // Unquoted, Windows would try `D:\Apps\Qamille.exe` first, a file anyone may plant on a data drive.
        assert_eq!(run_command(r"D:\Apps\Qamille Portable\Qamille.exe"), r#""D:\Apps\Qamille Portable\Qamille.exe" --autostart"#);
        assert_eq!(run_command(r"C:\Qamille\Qamille.exe"), r#""C:\Qamille\Qamille.exe" --autostart"#);
        assert_eq!(run_command(r"\\server\share\Qamille\Qamille.exe"), r#""\\server\share\Qamille\Qamille.exe" --autostart"#);
        // A verbatim path is written the way Explorer and Task Manager show paths.
        assert_eq!(run_command(r"\\?\D:\Apps\Qamille\Qamille.exe"), r#""D:\Apps\Qamille\Qamille.exe" --autostart"#);
        assert_eq!(run_command(r"\\?\UNC\server\share\Qamille.exe"), r#""\\server\share\Qamille.exe" --autostart"#);
        // A Windows path has no quotes of its own: the two are the command's, around the whole path.
        for exe in [r"D:\Apps\Qamille Portable\Qamille.exe", r"\\server\share\Qamille.exe"] {
            let command = run_command(exe);
            assert_eq!(command.matches('"').count(), 2, "{command}");
            assert!(command.starts_with('"') && command.ends_with(&format!("\" {AUTOSTART_ARG}")), "{command}");
        }
    }

    #[test]
    fn task_manager_switch_decides_whether_the_entry_runs() {
        // No twin, or the one Qamille writes when it turns the entry on: it runs.
        assert!(startup_approved(None));
        assert!(startup_approved(Some(&APPROVED_ON)));
        assert!(startup_approved(Some(&[0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        assert!(startup_approved(Some(&[])));
        // Turned off in Task Manager or Settings > Apps > Startup: odd first byte, then when.
        assert!(!startup_approved(Some(&[0x03, 0, 0, 0, 0x10, 0x5c, 0x3f, 0x7a, 0x2e, 0x1d, 0xdb, 0x01])));
        assert!(!startup_approved(Some(&[0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        assert_eq!(APPROVED_ON, [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_restart_comes_back_as_the_window_was() {
        let exe = r"C:\Apps\Qamille\Qamille.exe";
        // Not a restart: the arguments decide.
        assert!(stays_hidden(&args(&[exe, AUTOSTART_ARG]), None));
        assert!(!stays_hidden(&args(&[exe]), None));
        // A restart keeps the arguments of the first start: a window on screen comes back on screen,
        // one in the tray stays there.
        assert!(!stays_hidden(&args(&[exe, AUTOSTART_ARG]), Some(false)));
        assert!(stays_hidden(&args(&[exe]), Some(true)));
        assert!(stays_hidden(&args(&[exe, AUTOSTART_ARG]), Some(true)));
    }

    /// The file `mcp_config::launcher_content` wrote for an executable, byte for byte.
    fn launcher(executable: &str) -> String {
        let command = format!("\"{executable}\" --worker").replace('"', "\"\"");
        format!("Set shell = CreateObject(\"WScript.Shell\")\r\nshell.Run \"{command}\", 0, False\r\n")
    }

    #[test]
    fn the_launcher_names_its_executable() {
        assert_eq!(launcher_target(&launcher(r"C:\Apps\Qamille\Qamille.exe")).as_deref(), Some(r"C:\Apps\Qamille\Qamille.exe"));
        assert_eq!(launcher_target(&launcher(r"D:\My Apps\Qamille (2)\Qamille.exe")).as_deref(), Some(r"D:\My Apps\Qamille (2)\Qamille.exe"));
        assert_eq!(launcher_target("not a launcher"), None);
        assert_eq!(launcher_target(""), None);
    }

    #[test]
    fn the_quit_question_says_what_quitting_does_to_each_kind_of_action() {
        let left = |sends, sending, actions, confirming| QueueLeft { sends, sending, actions, confirming, failed: 0 };
        // A queued send stays in the queue, and the next start sends it. Not always for want of a
        // connection: credentials the vault cannot open hold it too.
        assert_eq!(unsent_text(true, left(1, 0, 0, 0)), "1 messaggio non è ancora partito. Se esci, partirà al prossimo avvio di Qamille.");
        assert_eq!(unsent_text(true, left(3, 0, 0, 0)), "3 messaggi non sono ancora partiti. Se esci, partiranno al prossimo avvio di Qamille.");
        assert_eq!(unsent_text(false, left(1, 0, 0, 0)), "1 message has not been sent yet. If you quit, it will be sent the next time Qamille starts.");
        assert_eq!(unsent_text(false, left(2, 0, 0, 0)), "2 messages have not been sent yet. If you quit, they will be sent the next time Qamille starts.");
        // A send already on its way is cut off by the quit, and the next start does not send it again.
        assert_eq!(unsent_text(true, left(0, 1, 0, 0)), "1 messaggio è in invio: uscendo potrebbe non partire, controlla Inviati.");
        assert_eq!(unsent_text(true, left(0, 2, 0, 0)), "2 messaggi sono in invio: uscendo potrebbero non partire, controlla Inviati.");
        assert_eq!(unsent_text(false, left(0, 1, 0, 0)), "1 message is being sent: if you quit it may not leave, check Sent.");
        assert_eq!(unsent_text(false, left(0, 2, 0, 0)), "2 messages are being sent: if you quit they may not leave, check Sent.");
        // An archive or a move is an action, not a message.
        assert_eq!(unsent_text(true, left(0, 0, 1, 0)), "1 azione sulla posta non è ancora stata completata. Se esci, verrà completata al prossimo avvio di Qamille.");
        assert_eq!(unsent_text(true, left(0, 0, 4, 0)), "4 azioni sulla posta non sono ancora state completate. Se esci, verranno completate al prossimo avvio di Qamille.");
        assert_eq!(unsent_text(false, left(0, 0, 1, 0)), "1 mail action has not been completed yet. If you quit, it will be completed the next time Qamille starts.");
        assert_eq!(unsent_text(false, left(0, 0, 4, 0)), "4 mail actions have not been completed yet. If you quit, they will be completed the next time Qamille starts.");
        assert_eq!(unsent_text(true, left(0, 0, 0, 1)), "1 azione sulla posta è in corso: uscendo potrebbe non completarsi, controlla la casella.");
        assert_eq!(unsent_text(true, left(0, 0, 0, 2)), "2 azioni sulla posta sono in corso: uscendo potrebbero non completarsi, controlla la casella.");
        assert_eq!(unsent_text(false, left(0, 0, 0, 1)), "1 mail action is in progress: if you quit it may not complete, check the mailbox.");
        assert_eq!(unsent_text(false, left(0, 0, 0, 2)), "2 mail actions are in progress: if you quit they may not complete, check the mailbox.");
        // Several kinds at once: one line each, what may be lost first, the sends before the actions.
        assert_eq!(
            unsent_text(true, left(1, 1, 2, 1)),
            [
                "1 messaggio è in invio: uscendo potrebbe non partire, controlla Inviati.",
                "1 messaggio non è ancora partito. Se esci, partirà al prossimo avvio di Qamille.",
                "1 azione sulla posta è in corso: uscendo potrebbe non completarsi, controlla la casella.",
                "2 azioni sulla posta non sono ancora state completate. Se esci, verranno completate al prossimo avvio di Qamille.",
            ]
            .join("\n")
        );
        assert_eq!(left(1, 1, 2, 1).total(), 5);
        assert_eq!(QueueLeft::default().total(), 0);
    }

    #[test]
    fn session_end_writes_down_what_did_not_leave() {
        assert_eq!(
            session_end_line(1, 2, 0, 3),
            "session end: 1 send(s) failed while waiting and are in Drafts, 2 send(s) still queued for the next start, 0 send(s) cut off on their way (check Sent), 3 other action(s) left"
        );
    }

    #[test]
    fn a_send_that_failed_while_quitting_is_said_first_and_is_in_drafts() {
        // Network off: «Esci» made the send due, it failed during the wait, and the draft stayed. Never a
        // silent quit: the question says it did not leave and where it is.
        let failed = |failed| QueueLeft { failed, ..QueueLeft::default() };
        assert_eq!(unsent_text(true, failed(1)), "1 messaggio non è partito: è rimasto nelle Bozze.");
        assert_eq!(unsent_text(true, failed(2)), "2 messaggi non sono partiti: sono rimasti nelle Bozze.");
        assert_eq!(unsent_text(false, failed(1)), "1 message was not sent: it is still in Drafts.");
        assert_eq!(unsent_text(false, failed(3)), "3 messages were not sent: they are still in Drafts.");
        assert_eq!(failed(2).total(), 2);
        let both = QueueLeft { failed: 1, sending: 1, ..QueueLeft::default() };
        assert_eq!(
            unsent_text(false, both),
            "1 message was not sent: it is still in Drafts.
1 message is being sent: if you quit it may not leave, check Sent."
        );
    }


    #[test]
    fn the_quit_entry_is_an_app_command_of_the_system_menu() {
        // Below 0xF000 (the system's own commands) and a multiple of 16: Windows uses the low four
        // bits of `wParam`. Not the reload entry's 0x5C10 (`shell_rules::SC_APP_RELOAD`).
        assert!(SC_APP_QUIT < 0xF000 && SC_APP_QUIT % 16 == 0 && SC_APP_QUIT != 0x5C10);
    }

    #[test]
    fn legacy_launcher_is_adopted_only_when_it_names_this_copy() {
        let here = r"C:\Apps\Qamille\Qamille.exe";
        let there = r"D:\Portable\Qamille\Qamille.exe";
        // This copy's: start at sign-in moves to the Run entry of this copy, and the file goes.
        assert_eq!(legacy_launcher(Some(&launcher(here)), here, true), LegacyLauncher::Adopt);
        // Paths that differ only in case are the same executable on Windows.
        assert_eq!(legacy_launcher(Some(&launcher(r"c:\APPS\qamille\qamille.EXE")), here, true), LegacyLauncher::Adopt);
        // It names an executable that is gone: it would only make Windows Script Host report an error.
        assert_eq!(legacy_launcher(Some(&launcher(there)), here, false), LegacyLauncher::RemoveStale);
        // Another copy that still exists: that copy migrates it at its own next start.
        assert_eq!(legacy_launcher(Some(&launcher(there)), here, true), LegacyLauncher::LeaveToOther);
        // Unreadable: it starts nothing, and it is ours (the file name is Qamille's).
        assert_eq!(legacy_launcher(Some("garbage"), here, false), LegacyLauncher::RemoveStale);
        assert_eq!(legacy_launcher(None, here, false), LegacyLauncher::Nothing);
    }
}
