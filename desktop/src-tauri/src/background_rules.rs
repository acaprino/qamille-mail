//! The pure rules of Qamille's background presence (`background.rs`): whether the close button
//! hides, whether a start stays in the tray, the name of the start-at-sign-in entry, and what to do
//! with the startup file of the old background worker. No Tauri and no Windows here, so the rules
//! are tested with `rustc --edition 2024 --test src/background_rules.rs`.

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

/// The name of the value under `HKCU\…\CurrentVersion\Run`. The installed Professional has one fixed
/// name, which its uninstaller removes; each portable copy names its folder and `tag`
/// (`workspace_tag`), so two copies, even in folders of the same name, never share an entry (Q22).
pub fn autostart_entry_name(installed: bool, folder: &str, tag: &str) -> String {
    if installed { "Qamille".to_owned() } else { format!("Qamille ({folder}, {tag})") }
}

/// The first 8 hex digits of the per-data-folder hash that `main.rs` appends to the app identifier
/// (`.workspace` followed by 16 digits), the same in every start of one copy.
pub fn workspace_tag(identifier: &str) -> Option<&str> {
    let (_, hash) = identifier.rsplit_once(".workspace")?;
    let digits = hash.get(..16)?;
    digits.bytes().all(|b| b.is_ascii_hexdigit()).then(|| &digits[..8])
}

/// «Esci da Qamille» in the system menu (Alt+Space), under «Chiudi»: it quits even while the close
/// button only hides. Below 0xF000 and a multiple of 16, like `shell_rules::SC_APP_RELOAD`.
pub const SC_APP_QUIT: u32 = 0x5C20;

/// The question when «Esci» finds actions of the undo queue that did not leave
/// (`commands::undo::undo_quit_anyway`), with what happens if the person quits anyway.
pub fn unsent_text(italian: bool, remaining: u32) -> String {
    match (italian, remaining) {
        (true, 1) => "1 messaggio non è ancora partito (nessuna connessione). Se esci, partirà al prossimo avvio di Qamille.".into(),
        (true, n) => format!("{n} messaggi non sono ancora partiti (nessuna connessione). Se esci, partiranno al prossimo avvio di Qamille."),
        (false, 1) => "1 message has not been sent yet (no connection). If you quit, it will be sent the next time Qamille starts.".into(),
        (false, n) => format!("{n} messages have not been sent yet (no connection). If you quit, they will be sent the next time Qamille starts."),
    }
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
    fn the_tag_is_the_first_eight_digits_of_the_workspace_identifier() {
        assert_eq!(workspace_tag("app.qamille.desktop.workspace0123456789abcdef"), Some("01234567"));
        assert_eq!(workspace_tag("app.qamille.desktop.workspace0123456789abcdef.preview"), Some("01234567"));
        assert_eq!(workspace_tag("app.qamille.desktop"), None);
        assert_eq!(workspace_tag("app.qamille.desktop.workspace012"), None);
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
    fn the_quit_question_says_how_many_did_not_leave() {
        assert_eq!(
            unsent_text(true, 1),
            "1 messaggio non è ancora partito (nessuna connessione). Se esci, partirà al prossimo avvio di Qamille."
        );
        assert_eq!(
            unsent_text(true, 3),
            "3 messaggi non sono ancora partiti (nessuna connessione). Se esci, partiranno al prossimo avvio di Qamille."
        );
        assert_eq!(
            unsent_text(false, 1),
            "1 message has not been sent yet (no connection). If you quit, it will be sent the next time Qamille starts."
        );
        assert_eq!(
            unsent_text(false, 2),
            "2 messages have not been sent yet (no connection). If you quit, they will be sent the next time Qamille starts."
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
