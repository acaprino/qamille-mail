//! Email actions shared by the window and the MCP server (names follow SKILL.md § action).

use crate::model::{Category, Email, Store};
use crate::rules::{self, Preset};
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};

pub const ACTIONS: [&str; 27] = [
    "pin", "unpin", "mute", "unmute", "snooze", "unsnooze", "changeReminder", "clearReminder", "setDue", "clearDue",
    "setAside", "archive", "moveToInbox", "moveToTrash", "moveToFolder", "attachLabel", "detachLabel", "markAsDone",
    "markAsUndone", "markAsSeen", "markAsUnseen", "markAsSpam", "markThreadAsPriority", "unmarkThreadAsPriority",
    "changeCategoryPersonal", "changeCategoryNotification", "changeCategoryNewsletters",
];

/// Accepts yyyy-MM-dd, dd/MM/yyyy, yyyy-MM-ddTHH:mm or a preset name such as "tomorrow".
pub fn parse_when(s: &str) -> Option<i64> {
    let t = s.trim();
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(time.timestamp());
    }
    let now = Local::now();
    let kind = rules::ScheduleKind::Snooze;
    for p in Preset::list(kind).into_iter().chain(Preset::more(kind)) {
        if p.label().eq_ignore_ascii_case(t) || p.label().replace(' ', "").eq_ignore_ascii_case(&t.replace([' ', '_', '-'], "")) {
            return p.time(now);
        }
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M") {
        return Local.from_local_datetime(&dt).single().map(|x| x.timestamp());
    }
    for f in ["%Y-%m-%d", "%d/%m/%Y", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(t, f) {
            return Local.from_local_datetime(&d.and_hms_opt((rules::MORNING_SECS / 3600) as u32, 0, 0)?).single().map(|x| x.timestamp());
        }
    }
    None
}

/// Service boundaries supply their clock and resolved regional preferences.
pub fn apply_at(
    store: &mut Store, id: u64, action: &str, arg: Option<&str>,
    now: i64, zone: &str, locale: &str,
) -> Result<String, String> {
    if matches!(action, "snooze" | "changeReminder" | "setDue") {
        let value = arg.filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{action} needs a value"))?;
        let when = super::regional::parse_when(value, now, zone, locale)?;
        return super::regional::set_email_date(store, id, action, when, now);
    }
    apply(store, id, action, arg)
}

pub fn apply(store: &mut Store, id: u64, action: &str, arg: Option<&str>) -> Result<String, String> {
    let e = store.email_mut(id).ok_or(format!("No email with id {id}"))?;
    fn require<'a>(action: &str, a: Option<&'a str>) -> Result<&'a str, String> {
        a.filter(|x| !x.trim().is_empty()).ok_or(format!("{action} needs a value"))
    }
    let need = |a| require(action, a);
    match action {
        "pin" => e.pinned = true,
        "unpin" => e.pinned = false,
        "mute" => e.muted = true,
        "unmute" => e.muted = false,
        "snooze" => e.snoozed_until = Some(parse_when(need(arg)?).ok_or("Unrecognised date")?),
        "unsnooze" => e.snoozed_until = None,
        "changeReminder" => {
            let t = parse_when(need(arg)?).ok_or("Unrecognised date")?;
            if t > crate::model::now_ts() + rules::MAX_DAYS_REMINDER * 86_400 {
                return Err(format!("A reminder can be at most {} days ahead.", rules::MAX_DAYS_REMINDER));
            }
            e.reminder_at = Some(t);
        }
        "clearReminder" => e.reminder_at = None,
        "setDue" => e.due = Some(parse_when(need(arg)?).ok_or("Unrecognised date")?),
        "clearDue" => e.due = None,
        "setAside" => e.set_aside = !e.set_aside,
        "archive" => e.folder = "Archive".into(),
        "moveToInbox" => {
            e.folder = "Inbox".into();
            e.done = false;
        }
        "moveToTrash" => e.folder = "Trash".into(),
        "moveToFolder" => e.folder = need(arg)?.to_string(),
        "attachLabel" => {
            let l = need(arg)?.to_string();
            if !e.labels.contains(&l) {
                e.labels.push(l);
            }
        }
        "detachLabel" => {
            let l = need(arg)?;
            e.labels.retain(|x| !x.eq_ignore_ascii_case(l));
        }
        "markAsDone" => {
            e.done = true;
            e.folder = "Archive".into();
            e.pinned = false;
            e.set_aside = false;
            e.read = true;
        }
        "markAsUndone" => e.done = false,
        "markAsSeen" => e.read = true,
        "markAsUnseen" => e.read = false,
        "markAsSpam" => e.folder = "Spam".into(),
        "markThreadAsPriority" => {
            e.priority_thread = true;
            e.category = Category::Priority;
        }
        "unmarkThreadAsPriority" => {
            e.priority_thread = false;
            e.category = Category::Personal;
        }
        "changeCategoryPersonal" => e.category = Category::Personal,
        "changeCategoryNotification" => e.category = Category::Notification,
        "changeCategoryNewsletters" => e.category = Category::Newsletter,
        other => return Err(format!("Unknown action {other}. Known: {}", ACTIONS.join(", "))),
    }
    Ok(format!("{action} done on {id}"))
}

/// Search sees only already-extracted text supplied by its calling boundary.
pub fn attachment_text(e: &Email) -> String {
    e.attachments.iter().filter_map(|a| a.inline_text.clone()).collect::<Vec<_>>().join("\n")
}

/// Compatibility entry point; callers that display errors use search::execute.
/// Pagination belongs to the caller, after all account and folder filtering.
pub fn search(store: &Store, query: &str, folder: Option<&str>) -> Vec<u64> {
    let text = if let Some(folder) = folder {
        let filter = format!("in:\"{}\"", folder.replace('"', "\\\""));
        if query.trim().is_empty() { filter } else { format!("({query}) {filter}") }
    } else { query.to_owned() };
    crate::domain::search::execute(store, &text, None, crate::model::now_ts(), "").unwrap_or_default()
}

pub fn email_text(store: &Store, e: &Email, full: bool) -> String {
    let acc = store.account(e.account_id).map(|a| a.email.clone()).unwrap_or_default();
    let mut s = format!(
        "ID: {}\nAccount: {}\nFolder: {}\nFrom: {} <{}>\nTo: {}\nDate: {}\nSubject: {}\nCategory: {}\nLabels: {}\nFlags: {}{}{}{}{}\n",
        e.id, acc, e.folder, e.from_name, e.from_addr, e.to.join(", "),
        if e.date_missing { "Date unavailable".into() } else { crate::domain::regional::format_timestamp(&store.settings, e.date) }, e.subject,
        e.category.key(), e.labels.join(", "),
        if e.read { "read" } else { "unread" },
        if e.pinned { ", pinned" } else { "" },
        if e.done { ", done" } else { "" },
        e.due.map(|d| format!(", due {}", crate::domain::regional::format_timestamp(&store.settings, d))).unwrap_or_default(),
        e.reminder_at.map(|d| format!(", reminder {}", crate::domain::regional::format_timestamp(&store.settings, d))).unwrap_or_default(),
    );
    if !e.attachments.is_empty() {
        s.push_str("Attachments:\n");
        for a in &e.attachments {
            s.push_str(&format!("  [{}] {} ({}, {})\n", a.id, a.name, a.mime, format!("{} bytes", a.size)));
        }
    }
    if full {
        if e.remote_only { s.push_str("\nBody not downloaded. Open this message in Qamille to retrieve it.\n"); }
        else { s.push_str(&format!("\n{}\n", e.body)); }
    } else {
        s.push_str(&format!("Snippet: {}\n", e.snippet(160)));
    }
    s
}
