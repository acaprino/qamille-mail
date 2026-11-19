//! Rules taken from the mechanics sheet (MECHANICS.md): schedule presets, Smart Inbox sections,
//! Gmail-style search filters and the offline classifier / auto-labeller.

use crate::model::{Category, Email};
use chrono::{Datelike, Duration, Local, NaiveDate, Offset, TimeZone, Weekday};

// ---- Schedule presets: values read from SmartMailCore.dll, see MECHANICS.md ----
pub const LATER_TODAY_SECS: i64 = 10800; // snoozeTimeModifier 0x18118a050, kind today
pub const MORNING_SECS: i64 = 32400; // same function: tomorrow, nextWeek, weekend, inAMonth = 09:00
pub const EVENING_SECS: i64 = 72000; // same function: tonight, tomorrowEve = 20:00
pub const LATER_THIS_WEEK_SECS: i64 = 86400; // same function, kind laterThisWeek
pub const ROUND_SECS: i64 = 1800; // RSMSnoozeConfigurationHelper.date 0x18118ddd0
pub const SOMEDAY_SECS: i64 = 3153600000; // somedaySnoozeDate 0x180da7f90
pub const SOMEDAY_MIN_SECS: i64 = 315360000; // minimumSomedaySnoozeDate 0x180da7fa0
pub const MAX_DAYS_REMINDER: i64 = 60; // f_defaultReminder 0x18118e8c0
pub const MAX_DAYS_SEND_LATER: i64 = 180; // f_defaultSendLater 0x18118e940
/// The original asks the system for the first day of the week (0x18118eb20); fixed to Monday here.
pub const FIRST_WEEKDAY: Weekday = Weekday::Mon; // estimated
pub const SEARCH_LIMIT: usize = 20; // SKILL.md § search
pub const PAGE_SIZE: usize = 20; // estimated
pub const ATTACHMENT_CAP_BYTES: u64 = 25 * 1024 * 1024; // SKILL.md --attach-stream

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    LaterToday,
    ThisEvening,
    Tomorrow,
    TomorrowEvening,
    Weekend,
    NextWeek,
    LaterThisWeek,
    InAMonth,
    Someday,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    Snooze,
    Reminder,
    SendLater,
}

impl Preset {
    /// What the menu shows first: the kinds the original turns on by default
    /// (f_defaultSnooze 0x18118e860, f_defaultReminder 0x18118e8c0, f_defaultSendLater 0x18118e940),
    /// in the renderer's order (chunk__63094 module 784300).
    pub fn list(kind: ScheduleKind) -> Vec<Preset> {
        use Preset::*;
        match kind {
            ScheduleKind::Reminder => vec![LaterToday, ThisEvening, Tomorrow, Weekend, NextWeek],
            _ => vec![LaterToday, ThisEvening, Tomorrow, Weekend],
        }
    }
    /// The other kinds the original can turn on in its settings.
    pub fn more(kind: ScheduleKind) -> Vec<Preset> {
        use Preset::*;
        match kind {
            ScheduleKind::Snooze => vec![TomorrowEvening, NextWeek, LaterThisWeek, InAMonth, Someday],
            ScheduleKind::Reminder => vec![TomorrowEvening, InAMonth],
            ScheduleKind::SendLater => vec![TomorrowEvening, NextWeek, InAMonth],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Preset::LaterToday => "Later Today",
            Preset::ThisEvening => "This Evening",
            Preset::Tomorrow => "Tomorrow",
            Preset::TomorrowEvening => "Tomorrow Evening",
            Preset::Weekend => "Weekend",
            Preset::NextWeek => "Next Week",
            Preset::LaterThisWeek => "Later This Week",
            Preset::InAMonth => "In a Month",
            Preset::Someday => "Someday",
        }
    }
    /// When the preset lands, or None when it no longer applies (This Evening after 20:00).
    pub fn time<Tz: TimeZone>(self, now: chrono::DateTime<Tz>) -> Option<i64> {
        let zone = now.timezone();
        let today = now.date_naive();
        let day_at = |d: NaiveDate, secs: i64| -> Option<i64> {
            let (h, m) = ((secs / 3600) as u32, ((secs % 3600) / 60) as u32);
            zone.from_local_datetime(&d.and_hms_opt(h, m, 0)?).earliest().map(|t| t.timestamp())
        };
        let next_weekday = |wd: Weekday| {
            let mut d = today.succ_opt()?;
            while d.weekday() != wd {
                d = d.succ_opt()?;
            }
            Some(d)
        };
        let ts = now.timestamp();
        let t = match self {
            Preset::LaterToday => {
                // Three hours ahead, never past midnight, on the nearest half hour, always in the future.
                let midnight = day_at(today.succ_opt()?, 0)?;
                let target = (ts + LATER_TODAY_SECS).min(midnight);
                let offset = i64::from(zone.timestamp_opt(target, 0).single()?.offset().fix().local_minus_utc());
                let local_target = target.checked_add(offset)?;
                let rounded = ((local_target + ROUND_SECS / 2).div_euclid(ROUND_SECS) * ROUND_SECS - offset).min(midnight);
                if rounded <= ts { (rounded + ROUND_SECS).min(midnight) } else { rounded }
            }
            Preset::ThisEvening => day_at(today, EVENING_SECS)?,
            Preset::Tomorrow => day_at(today.succ_opt()?, MORNING_SECS)?,
            Preset::TomorrowEvening => day_at(today.succ_opt()?, EVENING_SECS)?,
            // The original's weekend is the last day of the week.
            Preset::Weekend => day_at(next_weekday(FIRST_WEEKDAY.pred())?, MORNING_SECS)?,
            Preset::NextWeek => day_at(next_weekday(FIRST_WEEKDAY)?, MORNING_SECS)?,
            Preset::LaterThisWeek => {
                let days = LATER_THIS_WEEK_SECS / 86400;
                let left = (FIRST_WEEKDAY.pred().num_days_from_monday() as i64 - now.weekday().num_days_from_monday() as i64).rem_euclid(7);
                day_at(today.checked_add_signed(Duration::days(days.min(left)))?, LATER_THIS_WEEK_SECS % 86400)?
            }
            Preset::InAMonth => day_at(today.checked_add_months(chrono::Months::new(1))?, MORNING_SECS)?,
            Preset::Someday => ts.checked_add(SOMEDAY_SECS)?,
        };
        (t > ts).then_some(t)
    }
}

/// Snoozed so far ahead that it counts as Someday (isSomedaySnooze 0x180da7fb0).
pub fn is_someday(when: i64, now: i64) -> bool {
    when >= now + SOMEDAY_MIN_SECS
}

/// Text for a date, with Someday spelled out.
pub fn fmt_when(when: i64, now: i64) -> String {
    if is_someday(when, now) { "Someday".to_string() } else { fmt_full(when) }
}

// ---- Smart Inbox: order and limits from the default inbox, sub_180b2b4c0 and sub_180b2b3c0 ----
pub const CARD_LIMIT_NEW: usize = 5;
pub const CARD_LIMIT: usize = 3;
pub const MAX_VISIBLE_ITEMS: usize = 10; // RSMSmartInboxCardConfiguration.maxVisibleItems 0x180afb650

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Priority,
    New,
    Notifications,
    Newsletters,
    Pins,
    Seen,
}

impl Section {
    pub const ORDER: [Section; 6] =
        [Section::Priority, Section::New, Section::Notifications, Section::Newsletters, Section::Pins, Section::Seen];
    pub fn label(self) -> &'static str {
        match self {
            Section::Priority => "Priority",
            Section::New => "New",
            Section::Notifications => "Notifications",
            Section::Newsletters => "Newsletters",
            Section::Pins => "Pins",
            Section::Seen => "Seen",
        }
    }
    /// How many a card shows before "show more". Seen has no limit.
    pub fn limit(self) -> Option<usize> {
        match self {
            Section::New => Some(CARD_LIMIT_NEW),
            Section::Seen => None,
            _ => Some(CARD_LIMIT),
        }
    }
}

/// Unread mail goes to its category's card, read mail to Seen (index
/// conversation_index_smart_inbox_new on inInbox, unseen, category; 0x181cdf630).
pub fn section_of(e: &Email) -> Section {
    if e.pinned {
        return Section::Pins;
    }
    if e.read {
        return Section::Seen;
    }
    match e.category {
        Category::Priority => Section::Priority,
        Category::Notification => Section::Notifications,
        Category::Newsletter => Section::Newsletters,
        _ => Section::New,
    }
}

/// A mail you just opened keeps its card until you move on, so the list doesn't jump while you read
/// (lockedMessageQueryFlagsToIgnore 0x180687f10; locked flags sub_180532090).
pub fn section_with_lock(e: &Email, locked: Option<(u64, Section)>) -> Section {
    match locked {
        Some((id, section)) if id == e.id && !e.pinned => section,
        _ => section_of(e),
    }
}

// ---- Filters ----
/// Splits a query into tokens, keeping "quoted phrases" together.
pub fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// True if the email matches every operator in the query. Free words are returned for ranking.
pub fn matches(e: &Email, query: &str, now: i64, _attachment_text: &dyn Fn(&Email) -> String) -> (bool, Vec<String>) {
    let store = crate::model::Store { emails: vec![e.clone()], ..Default::default() };
    match super::search::ParsedQuery::parse(&store, query, now) {
        Ok(parsed) => (parsed.matches(e, &store), parsed.words()),
        Err(_) => (false, Vec::new()),
    }
}

/// Relevance: subject hits weigh more than body hits; newer mail wins ties.
pub fn score(e: &Email, words: &[String]) -> i64 {
    let lower = |value: &str| if value.is_ascii() { value.to_ascii_lowercase() } else { value.to_lowercase() };
    let s = lower(&e.subject);
    let b = lower(&e.body);
    let from = lower(&e.from_name);
    let mut sc = 0i64;
    for w in words {
        sc += 5 * s.matches(w.as_str()).count() as i64 + b.matches(w.as_str()).count() as i64;
        if from.contains(w.as_str()) {
            sc += 3;
        }
    }
    sc
}

// ---- Offline classifier and auto labels ----
pub fn classify_offline(e: &Email) -> Category {
    let from = e.from_addr.to_lowercase();
    let subj = e.subject.to_lowercase();
    let body = e.body.to_lowercase();
    // Machine-mail checks read off the headers and the address, like the original's isCalendarMessage,
    // isBulkMail, isAutoSubmitted and isNoReplySender (rule text 0x181f3fa30).
    if e.calendar || subj.starts_with("invitation:") || subj.starts_with("invito:") || e.attachments.iter().any(|a| a.name.ends_with(".ics")) {
        return Category::Invitation;
    }
    if subj.starts_with("accepted:") || subj.starts_with("declined:") || subj.starts_with("tentative:") {
        return Category::InvitationResponse;
    }
    if e.bulk || body.contains("unsubscribe") || body.contains("disiscriviti") || from.contains("newsletter") || from.contains("digest") || from.starts_with("news@") {
        return Category::Newsletter;
    }
    let robot = ["no-reply@", "noreply@", "donotreply@", "notifications", "notification", "alerts", "mailer-daemon", "automated", "booking@", "receipts"];
    if e.auto_submitted || robot.iter().any(|r| from.contains(r)) || body.contains("this is an automated message") {
        return Category::Notification;
    }
    let urgent = ["urgent", "asap", "urgente", "by thursday", "by friday", "deadline", "scadenza", "review by"];
    if urgent.iter().any(|w| subj.contains(w) || body.contains(w)) {
        return Category::Priority;
    }
    Category::Personal
}

pub const LABEL_RULES: [(&str, &[&str]); 8] = [
    ("Finance", &["invoice", "fattura", "payment", "pagamento", "receipt", "statement", "iban", "bank"]),
    ("Travel", &["flight", "volo", "booking", "hotel", "check-in", "boarding", "train", "treno"]),
    ("Projects", &["proposal", "project", "progetto", "milestone", "deadline", "budget", "launch"]),
    ("Team", &["review", "standup", "sync", "meeting", "riunione", "figma"]),
    ("Dev", &["issue", "pull request", "commit", "build", "deploy", "github", "bug"]),
    ("Shopping", &["order", "ordine", "shipped", "spedito", "delivery", "consegna", "cart"]),
    ("Reading", &["newsletter", "issue:", "digest", "read the full", "weekly"]),
    ("Social", &["dinner", "cena", "party", "photos", "foto", "weekend", "birthday"]),
];

pub fn labels_offline(e: &Email) -> Vec<String> {
    let hay = format!("{} {} {}", e.subject, e.body, e.from_addr).to_lowercase();
    let mut out: Vec<String> = LABEL_RULES
        .iter()
        .filter(|(_, kws)| kws.iter().any(|k| hay.contains(k)))
        .map(|(l, _)| l.to_string())
        .take(2)
        .collect();
    out.dedup();
    out
}

pub fn fmt_time(ts: i64) -> String {
    let t = Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now);
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if (now - t).num_days() < 6 && t < now {
        t.format("%a").to_string()
    } else {
        t.format("%d %b").to_string()
    }
}

pub fn fmt_full(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%a %d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, d, h, m, 0).single().unwrap()
    }
    fn shown(p: Preset, now: chrono::DateTime<Local>) -> String {
        Local.timestamp_opt(p.time(now).unwrap(), 0).single().unwrap().format("%d %H:%M").to_string()
    }

    #[test]
    fn preset_times_match_the_original() {
        let wed = at(4, 10, 10);
        assert_eq!(shown(Preset::LaterToday, wed), "04 13:00");
        assert_eq!(shown(Preset::ThisEvening, wed), "04 20:00");
        assert_eq!(shown(Preset::Tomorrow, wed), "05 09:00");
        assert_eq!(shown(Preset::TomorrowEvening, wed), "05 20:00");
        assert_eq!(shown(Preset::Weekend, wed), "08 09:00");
        assert_eq!(shown(Preset::NextWeek, wed), "09 09:00");
        assert_eq!(shown(Preset::LaterThisWeek, wed), "05 00:00");
    }

    #[test]
    fn later_today_rounds_and_stops_at_midnight() {
        assert_eq!(shown(Preset::LaterToday, at(4, 10, 20)), "04 13:30");
        assert_eq!(shown(Preset::LaterToday, at(4, 22, 40)), "05 00:00");
    }

    #[test]
    fn this_evening_disappears_after_twenty() {
        assert!(Preset::ThisEvening.time(at(4, 21, 0)).is_none());
    }

    #[test]
    fn weekend_on_the_last_day_moves_a_week() {
        assert_eq!(shown(Preset::Weekend, at(8, 10, 0)), "15 09:00");
    }

    #[test]
    fn someday_is_far_ahead() {
        let now = at(4, 10, 0);
        let t = Preset::Someday.time(now).unwrap();
        assert!(is_someday(t, now.timestamp()));
        assert!(!is_someday(now.timestamp() + 86400, now.timestamp()));
    }

    #[test]
    fn default_menus_follow_the_original() {
        assert_eq!(Preset::list(ScheduleKind::Snooze).len(), 4);
        assert!(Preset::list(ScheduleKind::Reminder).contains(&Preset::NextWeek));
        assert!(!Preset::list(ScheduleKind::SendLater).contains(&Preset::NextWeek));
    }

    #[test]
    fn cards_sort_read_mail_into_seen() {
        let mut e = Email { category: Category::Newsletter, ..Default::default() };
        assert_eq!(section_of(&e), Section::Newsletters);
        e.read = true;
        assert_eq!(section_of(&e), Section::Seen);
        e.pinned = true;
        assert_eq!(section_of(&e), Section::Pins);
        assert_eq!(Section::ORDER[0], Section::Priority);
        assert_eq!(Section::ORDER[5], Section::Seen);
    }

    #[test]
    fn opened_mail_stays_in_its_card_until_you_move_on() {
        let mut e = Email { id: 7, category: Category::Personal, ..Default::default() };
        let locked = Some((7, section_of(&e)));
        e.read = true;
        assert_eq!(section_with_lock(&e, locked), Section::New);
        assert_eq!(section_with_lock(&e, Some((8, Section::New))), Section::Seen);
        assert_eq!(section_with_lock(&e, None), Section::Seen);
        e.pinned = true;
        assert_eq!(section_with_lock(&e, locked), Section::Pins, "pinning always moves it");
    }

    #[test]
    fn headers_decide_machine_mail() {
        let e = Email { bulk: true, from_addr: "a@b.c".into(), ..Default::default() };
        assert_eq!(classify_offline(&e), Category::Newsletter);
        let e = Email { auto_submitted: true, from_addr: "a@b.c".into(), ..Default::default() };
        assert_eq!(classify_offline(&e), Category::Notification);
        let e = Email { calendar: true, from_addr: "a@b.c".into(), ..Default::default() };
        assert_eq!(classify_offline(&e), Category::Invitation);
        let e = Email { from_addr: "noreply@b.c".into(), ..Default::default() };
        assert_eq!(classify_offline(&e), Category::Notification);
    }

    #[test]
    fn filters_work() {
        let e = Email { from_addr: "alice@co.com".into(), subject: "Budget".into(), read: false, ..Default::default() };
        let f = |_: &Email| String::new();
        assert!(matches(&e, "from:alice is:unread budget", 0, &f).0);
        assert!(!matches(&e, "is:read", 0, &f).0);
    }
}
