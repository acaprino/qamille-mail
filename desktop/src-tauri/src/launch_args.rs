//! The mailto: links that reach Qamille from outside its page (docs/aspetto-nativo.md, rule 32, audit
//! Q10): Windows starts Qamille with one (`"<exe>" "%1"`, the command `mail_handler.rs` registers), or
//! hands it to the running Qamille as a second start, and a link of the app's own page that tries to
//! navigate comes back from the navigation hooks (`native_feel.rs`). Each goes to the page, which reads
//! it (`src/lib/mailto.ts`, the one parser) and opens the composer; nothing here sends or reads a
//! header.
//!
//! A link waits until the page has asked for it (`take_launch_mailto`, which the page calls once its
//! `compose-mailto` listener is in); from then on each link goes to the page at once, as
//! `compose-mailto`. A second start brings the window back first (`background::second_instance` calls
//! `background::restore`, then `route`). The pure rules are at the bottom, with their tests.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

static INBOX: Mutex<Inbox> = Mutex::new(Inbox { asked: false, waiting: None });

/// A start's mailto: argument (a cold start from `setup_app`, a second start from
/// `background::second_instance`) to the page, now or when it asks.
pub fn route(app: &AppHandle, args: &[String]) {
    if let Some(link) = mailto_arg(args) {
        deliver(app, link);
    }
}

/// A mail link the app's own page tried to navigate to, from a navigation hook of the main window
/// (`native_feel::navigation_guard`, `new_window_guard`): off the hook's thread, since the webview
/// waits for the hook that asked.
pub fn from_navigation(app: &AppHandle, link: String) {
    let app = app.clone();
    std::thread::spawn(move || deliver(&app, link));
}

fn deliver(app: &AppHandle, link: String) {
    let now = INBOX.lock().unwrap_or_else(|e| e.into_inner()).arrive(link);
    if let Some(link) = now {
        let _ = app.emit_to("main", "compose-mailto", link);
    }
}

/// The page asks once, when its `compose-mailto` listener is in (`installMailtoRouting` in
/// `src/lib/mailto.ts`): the link Qamille was started with, if one waits. Only the main window's page.
#[tauri::command]
pub fn take_launch_mailto(window: tauri::Window) -> Option<String> {
    if window.label() != "main" {
        return None;
    }
    INBOX.lock().unwrap_or_else(|e| e.into_inner()).take()
}

// The pure rules.

/// The longest link that is passed on, in bytes (`MAILTO_LIMIT` in `src/lib/mailto.ts`).
pub const MAIL_LINK_LIMIT: usize = 8 * 1024;

/// Whether `text` is a whole mail link the page may read: `mailto:` (in any case) at its start, no NUL
/// or line break anywhere, at most `MAIL_LINK_LIMIT` bytes.
pub fn is_mail_link(text: &str) -> bool {
    text.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
        && text.len() <= MAIL_LINK_LIMIT
        && !text.contains(['\0', '\r', '\n'])
}

/// The mailto: link among a start's arguments (the program first, never counted): the first argument
/// that is a whole mail link (`is_mail_link`). A path, an option or an argument that only contains
/// `mailto:` is not one.
pub fn mailto_arg(args: &[String]) -> Option<String> {
    args.get(1..)?.iter().find(|arg| is_mail_link(arg)).cloned()
}

/// What waits for the page: one link, the latest (one composer for the link clicked last), until the
/// page has asked; after that nothing waits.
#[derive(Debug, Default)]
struct Inbox {
    asked: bool,
    waiting: Option<String>,
}

impl Inbox {
    /// A link arrives: the one to send to the page now, or `None` when it waits.
    fn arrive(&mut self, link: String) -> Option<String> {
        if self.asked {
            Some(link)
        } else {
            self.waiting = Some(link);
            None
        }
    }

    /// The page asks: what waits, and from now on every link goes straight to it.
    fn take(&mut self) -> Option<String> {
        self.asked = true;
        self.waiting.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::AUTOSTART_ARG;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }
    const EXE: &str = r"C:\Program Files\Qamille\Qamille.exe";

    #[test]
    fn mailto_arg_takes_only_a_whole_mailto_argument() {
        // What Windows passes for a mailto: link ("<exe>" "%1"), also next to the start-at-sign-in argument.
        assert_eq!(mailto_arg(&args(&[EXE, "mailto:a@b.it?subject=x"])).as_deref(), Some("mailto:a@b.it?subject=x"));
        assert_eq!(mailto_arg(&args(&[EXE, AUTOSTART_ARG, "MAILTO:a@b.it"])).as_deref(), Some("MAILTO:a@b.it"));
        assert_eq!(mailto_arg(&args(&[EXE, "mailto:"])).as_deref(), Some("mailto:"));
        // The first one counts.
        assert_eq!(mailto_arg(&args(&[EXE, "mailto:a@b.it", "mailto:c@d.it"])).as_deref(), Some("mailto:a@b.it"));
        for list in [
            &[EXE][..],
            &[EXE, AUTOSTART_ARG],
            &[EXE, r"C:\Users\me\mailto:a@b.it"],
            &[EXE, r"C:\Users\me\Downloads\message.eml"],
            &[EXE, "--mailto:a@b.it"],
            &[EXE, "--compose=mailto:a@b.it"],
            &[EXE, " mailto:a@b.it"],
            &[EXE, "mailto"],
            &[EXE, "mail:a@b.it"],
            // A NUL or a line break is never part of an address: the argument is not one.
            &[EXE, "mailto:a@b.it\0--worker"],
            &[EXE, "mailto:a@b.it\r\nBcc: c@d.it"],
            &[EXE, "mailto:a@b.it\n"],
        ] {
            assert_eq!(mailto_arg(&args(list)), None, "{list:?}");
        }
        // The program's own path never counts, whatever its name.
        assert_eq!(mailto_arg(&args(&["mailto:a@b.it"])), None);
        assert_eq!(mailto_arg(&[]), None);
        // At most 8 KB, as the page reads it (src/lib/mailto.ts, MAILTO_LIMIT).
        let head = "mailto:a@b.it?body=";
        let exactly = format!("{head}{}", "x".repeat(MAIL_LINK_LIMIT - head.len()));
        assert_eq!(mailto_arg(&args(&[EXE, &exactly])), Some(exactly.clone()));
        assert_eq!(mailto_arg(&args(&[EXE, &format!("{exactly}x")])), None);
    }

    #[test]
    fn a_mail_link_of_the_page_is_a_whole_mailto_address() {
        assert!(is_mail_link("mailto:a@b.it"));
        assert!(is_mail_link("MailTo:a@b.it?subject=x"));
        for other in ["https://example.com/", "https://example.com/mailto:a@b.it", "mailto", "", "mailto:a@b.it\r\nx", "mailto:\0"] {
            assert!(!is_mail_link(other), "{other:?}");
        }
    }

    #[test]
    fn a_link_waits_for_the_page_and_then_goes_straight_to_it() {
        let mut inbox = Inbox::default();
        // Before the page has asked (a cold start, a second start while the page loads): it waits.
        assert_eq!(inbox.arrive("mailto:a@b.it".into()), None);
        // A later one replaces it: one composer, the link the person clicked last.
        assert_eq!(inbox.arrive("mailto:c@d.it".into()), None);
        assert_eq!(inbox.take().as_deref(), Some("mailto:c@d.it"));
        // Asked once: nothing more waits, and every later link goes to the page at once.
        assert_eq!(inbox.take(), None);
        assert_eq!(inbox.arrive("mailto:e@f.it".into()).as_deref(), Some("mailto:e@f.it"));
        assert_eq!(inbox.take(), None);
    }
}
