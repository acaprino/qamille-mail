//! What a Windows notification of Qamille says and when it is shown (docs/aspetto-nativo.md,
//! rule 28). Pure: no Tauri and no I/O, so `rustc --edition 2024 --test notify_rules.rs` tries it
//! alone. `notifications.rs` reads the preferences and the window, and sends.
#![cfg_attr(not(test), allow(dead_code))]

/// The three facts Qamille announces outside its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// New unread mail in the Inbox, from the live sync.
    NewMail,
    /// A scheduled message that could not be sent.
    ScheduledSend,
    /// A mail account, or the Qamille server, no longer accepts the sign-in.
    SignIn,
}

/// The switches of Settings → General → Windows (`data/notifications.json`): the general one and one
/// per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub enabled: bool,
    pub new_mail: bool,
    pub scheduled_send: bool,
    pub sign_in: bool,
}

/// The main window as Windows has it now. A minimized window counts as not visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowState {
    pub visible: bool,
    pub focused: bool,
}

/// Whether a fact of `kind` becomes a notification. Never with its switch off; never while the window
/// is on screen with the focus, where the list and the in-app notice already say it.
pub fn should_notify(kind: Kind, enabled: &Prefs, window: WindowState) -> bool {
    let switched_on = enabled.enabled
        && match kind {
            Kind::NewMail => enabled.new_mail,
            Kind::ScheduledSend => enabled.scheduled_send,
            Kind::SignIn => enabled.sign_in,
        };
    switched_on && !(window.visible && window.focused)
}

/// A header as one line: the line breaks of a folded header, tabs and runs of spaces become one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn no_subject(italian: bool) -> &'static str {
    if italian { "(senza oggetto)" } else { "(no subject)" }
}

/// The subject to show, or the words for none.
fn subject_or_none(subject: &str, italian: bool) -> String {
    let subject = one_line(subject);
    if subject.is_empty() { no_subject(italian).to_owned() } else { subject }
}

/// From this many messages of one arrival on, one notification names them all (Review Focus 4: no
/// avalanche). Fewer get one notification each.
pub const GROUP_FROM: usize = 3;

/// Title and text of a new-mail notification for `items` (sender, subject), newest first, of the
/// account `account`. One message: its sender and its subject. More: their number and the account,
/// then the first senders. Never a word of the body: the caller does not even pass it.
pub fn new_mail_text(items: &[(String, String)], account: &str, italian: bool) -> (String, String) {
    match items {
        [] => (String::new(), String::new()),
        [(sender, subject)] => {
            let sender = one_line(sender);
            let title = if !sender.is_empty() {
                sender
            } else if italian {
                format!("Nuovo messaggio in {account}")
            } else {
                format!("New message in {account}")
            };
            (title, subject_or_none(subject, italian))
        }
        _ => {
            let count = items.len();
            let title = if italian { format!("{count} messaggi nuovi in {account}") } else { format!("{count} new messages in {account}") };
            // Each sender once, in the order of the messages; the first three, then how many more.
            let mut senders: Vec<String> = Vec::new();
            for (sender, _) in items {
                let sender = one_line(sender);
                if !sender.is_empty() && !senders.contains(&sender) {
                    senders.push(sender);
                }
            }
            if senders.is_empty() {
                return (title, String::new());
            }
            let shown = senders.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let more = senders.len().saturating_sub(3);
            let body = match (italian, more) {
                (true, 0) => format!("Da {shown}"),
                (true, more) => format!("Da {shown} e altri {more}"),
                (false, 0) => format!("From {shown}"),
                (false, more) => format!("From {shown} and {more} more"),
            };
            (title, body)
        }
    }
}

/// The notifications of one arrival: one per message, or a single one from `GROUP_FROM` messages on.
/// Each comes with the index in `items` of the message its click opens (the newest for a group).
pub fn new_mail_notices(items: &[(String, String)], account: &str, italian: bool) -> Vec<(usize, String, String)> {
    if items.len() >= GROUP_FROM {
        let (title, body) = new_mail_text(items, account, italian);
        return vec![(0, title, body)];
    }
    (0..items.len())
        .map(|index| {
            let (title, body) = new_mail_text(&items[index..=index], account, italian);
            (index, title, body)
        })
        .collect()
}

/// A scheduled message that did not leave: what it was, and where to find it.
pub fn scheduled_send_text(subject: &str, italian: bool) -> (String, String) {
    let subject = subject_or_none(subject, italian);
    if italian {
        ("Invio programmato non riuscito".into(), format!("«{subject}» non è partito. È nelle Bozze: aprilo per riprovare."))
    } else {
        ("Scheduled message not sent".into(), format!("“{subject}” was not sent. It is in Drafts: open it to try again."))
    }
}

/// A sign-in that no longer works: of the mail account `account`, or of the Qamille server (`None`).
pub fn sign_in_text(account: Option<&str>, italian: bool) -> (String, String) {
    match (account, italian) {
        (Some(account), true) => (
            format!("Accedi di nuovo a {account}"),
            "Qamille non riceve più la posta di questo account. Apri Impostazioni → Account.".into(),
        ),
        (Some(account), false) => (
            format!("Sign in to {account} again"),
            "Qamille no longer receives this account's mail. Open Settings → Accounts.".into(),
        ),
        (None, true) => (
            "Accedi di nuovo al server Qamille".into(),
            "Il server non riconosce più questo computer: le caselle non si sincronizzano. Apri Impostazioni → Calcolo.".into(),
        ),
        (None, false) => (
            "Sign in to the Qamille server again".into(),
            "The server no longer recognizes this computer: the mailboxes are not synchronized. Open Settings → Compute.".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ON: Prefs = Prefs { enabled: true, new_mail: true, scheduled_send: true, sign_in: true };
    const IN_FRONT: WindowState = WindowState { visible: true, focused: true };
    const BEHIND: WindowState = WindowState { visible: true, focused: false };
    const IN_TRAY: WindowState = WindowState { visible: false, focused: false };

    fn items(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(sender, subject)| (sender.to_string(), subject.to_string())).collect()
    }

    #[test]
    fn notify_rules_new_mail_is_quiet_while_the_window_has_focus() {
        assert!(!should_notify(Kind::NewMail, &ALL_ON, IN_FRONT), "the list on screen already shows it");
        assert!(should_notify(Kind::NewMail, &ALL_ON, BEHIND), "on screen behind another window");
        assert!(should_notify(Kind::NewMail, &ALL_ON, IN_TRAY), "in the tray or minimized");
        // A window reported focused while hidden is not in front of anyone.
        assert!(should_notify(Kind::NewMail, &ALL_ON, WindowState { visible: false, focused: true }));
        // The other two facts follow the same rule: a window in front already shows its notice.
        for kind in [Kind::ScheduledSend, Kind::SignIn] {
            assert!(!should_notify(kind, &ALL_ON, IN_FRONT), "{kind:?}");
            assert!(should_notify(kind, &ALL_ON, BEHIND), "{kind:?}: in front of the person but without the focus");
            assert!(should_notify(kind, &ALL_ON, IN_TRAY), "{kind:?}");
        }
    }

    #[test]
    fn notify_rules_three_messages_become_one_notice() {
        let two = items(&[("Anna Rossi", "Preventivo"), ("Marco Bianchi", "Riunione di lunedì")]);
        let notices = new_mail_notices(&two, "giulia@example.com", true);
        assert_eq!(notices.len(), 2, "two messages are two notifications: {notices:?}");
        assert_eq!(notices[0], (0, "Anna Rossi".into(), "Preventivo".into()));
        assert_eq!(notices[1], (1, "Marco Bianchi".into(), "Riunione di lunedì".into()));

        let three = items(&[("Anna Rossi", "Preventivo"), ("Marco Bianchi", "Riunione"), ("Anna Rossi", "Re: Preventivo")]);
        let notices = new_mail_notices(&three, "giulia@example.com", true);
        assert_eq!(notices.len(), 1, "three messages are one notification: {notices:?}");
        let (open, title, body) = &notices[0];
        assert_eq!(*open, 0, "its click opens the newest message");
        assert_eq!(title, "3 messaggi nuovi in giulia@example.com");
        assert_eq!(body, "Da Anna Rossi, Marco Bianchi", "each sender once, in order");
        let (title, body) = new_mail_text(&three, "giulia@example.com", false);
        assert_eq!(title, "3 new messages in giulia@example.com");
        assert_eq!(body, "From Anna Rossi, Marco Bianchi");

        // Many senders: the first three, then how many more.
        let many: Vec<(String, String)> = (1..=7).map(|n| (format!("Sender {n}"), format!("Subject {n}"))).collect();
        let notices = new_mail_notices(&many, "giulia@example.com", false);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].1, "7 new messages in giulia@example.com");
        assert_eq!(notices[0].2, "From Sender 1, Sender 2, Sender 3 and 4 more");
        let (_, body) = new_mail_text(&many, "giulia@example.com", true);
        assert_eq!(body, "Da Sender 1, Sender 2, Sender 3 e altri 4");

        assert!(new_mail_notices(&[], "giulia@example.com", true).is_empty(), "nothing arrived, nothing said");
    }

    #[test]
    fn notify_rules_a_switched_off_kind_never_notifies() {
        let off = Prefs { enabled: false, ..ALL_ON };
        for kind in [Kind::NewMail, Kind::ScheduledSend, Kind::SignIn] {
            assert!(!should_notify(kind, &off, IN_TRAY), "{kind:?}: the general switch is off");
        }
        let no_mail = Prefs { new_mail: false, ..ALL_ON };
        assert!(!should_notify(Kind::NewMail, &no_mail, IN_TRAY));
        assert!(should_notify(Kind::ScheduledSend, &no_mail, IN_TRAY) && should_notify(Kind::SignIn, &no_mail, IN_TRAY), "only its own kind goes quiet");
        assert!(!should_notify(Kind::ScheduledSend, &Prefs { scheduled_send: false, ..ALL_ON }, IN_TRAY));
        assert!(!should_notify(Kind::SignIn, &Prefs { sign_in: false, ..ALL_ON }, IN_TRAY));
    }

    #[test]
    fn notify_rules_texts_name_sender_and_subject_never_the_body() {
        // The caller passes sender and subject only; the texts say them and nothing else.
        let one = items(&[("Anna Rossi", "Preventivo per il sito")]);
        assert_eq!(new_mail_text(&one, "giulia@example.com", true), ("Anna Rossi".into(), "Preventivo per il sito".into()));
        assert_eq!(new_mail_text(&one, "giulia@example.com", false), ("Anna Rossi".into(), "Preventivo per il sito".into()));
        // A message without a subject, or without a sender name, still says something.
        let bare = items(&[("", "")]);
        assert_eq!(new_mail_text(&bare, "giulia@example.com", true), ("Nuovo messaggio in giulia@example.com".into(), "(senza oggetto)".into()));
        assert_eq!(new_mail_text(&bare, "giulia@example.com", false), ("New message in giulia@example.com".into(), "(no subject)".into()));
        // Line breaks in a header never become extra lines of the notification.
        let folded = items(&[("Anna\r\nRossi", "Preventivo\n per il sito")]);
        assert_eq!(new_mail_text(&folded, "g@example.com", true), ("Anna Rossi".into(), "Preventivo per il sito".into()));

        let (title, body) = scheduled_send_text("Offerta Q4", true);
        assert_eq!(title, "Invio programmato non riuscito");
        assert_eq!(body, "«Offerta Q4» non è partito. È nelle Bozze: aprilo per riprovare.");
        let (title, body) = scheduled_send_text("Offerta Q4", false);
        assert_eq!(title, "Scheduled message not sent");
        assert_eq!(body, "“Offerta Q4” was not sent. It is in Drafts: open it to try again.");
        assert_eq!(scheduled_send_text("", false).1, "“(no subject)” was not sent. It is in Drafts: open it to try again.");

        assert_eq!(
            sign_in_text(Some("giulia@example.com"), true),
            ("Accedi di nuovo a giulia@example.com".into(), "Qamille non riceve più la posta di questo account. Apri Impostazioni → Account.".into())
        );
        assert_eq!(
            sign_in_text(Some("giulia@example.com"), false),
            ("Sign in to giulia@example.com again".into(), "Qamille no longer receives this account's mail. Open Settings → Accounts.".into())
        );
        assert_eq!(
            sign_in_text(None, true),
            ("Accedi di nuovo al server Qamille".into(), "Il server non riconosce più questo computer: le caselle non si sincronizzano. Apri Impostazioni → Calcolo.".into())
        );
        assert_eq!(
            sign_in_text(None, false),
            ("Sign in to the Qamille server again".into(), "The server no longer recognizes this computer: the mailboxes are not synchronized. Open Settings → Compute.".into())
        );
    }
}
