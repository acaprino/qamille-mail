//! Translates a Qamille search into the Gmail search language (`X-GM-RAW`).
//!
//! Pure: no I/O. What Gmail cannot express (tasks, snooze, categories,
//! accounts, reminders) is left out of the string and checked again on this
//! computer with `ParsedQuery::server_result_matches`.
use super::search::{ParsedQuery, QueryPart};
use serde::{Deserialize, Serialize};

/// A folder searched besides All Mail, which holds everything but these two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteScope {
    Spam,
    Trash,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GmailQuery {
    /// The Gmail search, or `None` when nothing of the query exists on Gmail.
    pub raw: Option<String>,
    /// The query is only about data that exists on this computer: no request.
    pub local_only: bool,
    /// Some part was left out and must be checked again locally.
    pub needs_local_filter: bool,
    pub scopes: Vec<RemoteScope>,
}

/// A value Gmail reads as one term, quoted when it is not a plain word.
fn term(value: &str) -> String {
    let plain = !value.is_empty()
        && !value.starts_with('-')
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "\"(){}:\\".contains(c))
        && !matches!(value, "OR" | "AND" | "or" | "and");
    if plain { value.to_string() } else { quote(value) }
}

fn quote(value: &str) -> String {
    let clean: String = value.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A translated part that Gmail must read as one operand.
fn grouped(text: String) -> String {
    let compound = text.contains(' ') && !(text.starts_with('"') && text.ends_with('"') && text.matches('"').count() == 2);
    if compound { format!("({text})") } else { text }
}

struct Translation {
    lossy: bool,
    scopes: Vec<RemoteScope>,
}

impl Translation {
    fn scope(&mut self, scope: RemoteScope) {
        if !self.scopes.contains(&scope) {
            self.scopes.push(scope);
        }
    }
    /// `Some("")` matches everything; `None` cannot be said to Gmail.
    fn part(&mut self, node: &QueryPart<'_>) -> Option<String> {
        let local = |this: &mut Self| {
            this.lossy = true;
            None
        };
        match node {
            QueryPart::All => Some(String::new()),
            QueryPart::Text { value, exact } => Some(if *exact { quote(value) } else { term(value) }),
            QueryPart::Person { field, value } => {
                Some(format!("{field}:{}", value.map_or_else(|| "me".to_string(), term)))
            }
            QueryPart::Subject(value) => Some(if value.starts_with('-') || value.chars().any(|c| "(){}\"\\".contains(c)) {
                format!("subject:{}", quote(value))
            } else {
                format!("subject:({value})")
            }),
            QueryPart::Body(value) => Some(term(value)),
            QueryPart::Date { start, end } => {
                let mut parts = Vec::new();
                if let Some(start) = start {
                    parts.push(format!("after:{start}"));
                }
                if let Some(end) = end {
                    parts.push(format!("before:{end}"));
                }
                Some(parts.join(" "))
            }
            QueryPart::Folder(folder) => match *folder {
                "inbox" | "sent" => Some(format!("in:{folder}")),
                "archive" => Some("-in:inbox -in:sent".into()),
                "spam" => {
                    self.scope(RemoteScope::Spam);
                    Some("in:spam".into())
                }
                "trash" => {
                    self.scope(RemoteScope::Trash);
                    Some("in:trash".into())
                }
                "anywhere" => {
                    self.scope(RemoteScope::Spam);
                    self.scope(RemoteScope::Trash);
                    Some("in:anywhere".into())
                }
                _ => local(self),
            },
            QueryPart::Label(label) => Some(format!("label:{}", quote(label))),
            QueryPart::Has(kind) => match *kind {
                "attachment" | "document" | "spreadsheet" | "presentation" => {
                    Some(format!("has:{kind}"))
                }
                _ => local(self),
            },
            QueryPart::Filename(name) | QueryPart::FileType(name) => {
                Some(format!("filename:{}", term(name)))
            }
            QueryPart::State(state) => match *state {
                "read" | "unread" | "starred" => Some(format!("is:{state}")),
                _ => local(self),
            },
            QueryPart::Local => local(self),
            QueryPart::Not(inner) => match self.part(inner) {
                Some(text) if !text.is_empty() => Some(if text.contains(' ') {
                    format!("-({text})")
                } else {
                    format!("-{text}")
                }),
                _ => local(self),
            },
            QueryPart::And(a, b) => {
                let a = self.part(a);
                let b = self.part(b);
                match (a, b) {
                    (Some(a), Some(b)) => Some(
                        [a, b].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
                    ),
                    (Some(one), None) | (None, Some(one)) => Some(one),
                    (None, None) => None,
                }
            }
            QueryPart::Or(a, b) => {
                // A half that Gmail cannot say makes the whole alternative unknown.
                let before = self.scopes.clone();
                let a = self.part(a);
                let b = self.part(b);
                match (a, b) {
                    (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => {
                        Some(format!("{{{} {}}}", grouped(a), grouped(b)))
                    }
                    (Some(_), Some(_)) => Some(String::new()),
                    _ => {
                        self.scopes = before;
                        local(self)
                    }
                }
            }
        }
    }
}

pub fn to_gmail_query(query: &ParsedQuery) -> GmailQuery {
    let mut translation = Translation { lossy: false, scopes: Vec::new() };
    let raw = translation.part(&query.tree()).filter(|text| !text.trim().is_empty());
    GmailQuery {
        local_only: raw.is_none() && translation.lossy,
        raw,
        needs_local_filter: translation.lossy,
        scopes: translation.scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Account, Store};

    fn store() -> Store {
        let mut store = Store::default();
        store.settings.time_zone = "UTC".into();
        store.settings.locale = "it-IT".into();
        store.accounts.push(Account { id: 1, email: "io@example.com".into(), name: "Io".into(), ..Default::default() });
        store
    }
    fn translate(text: &str) -> GmailQuery {
        to_gmail_query(&ParsedQuery::parse(&store(), text, 1_700_000_000).unwrap())
    }
    fn raw(text: &str) -> String {
        translate(text).raw.unwrap_or_default()
    }

    #[test]
    fn every_row_of_the_table_translates() {
        let rows = [
            ("fattura", "fattura"),
            ("\"fattura enel\"", "\"fattura enel\""),
            ("from:anna", "from:anna"),
            ("to:anna@example.com", "to:anna@example.com"),
            ("cc:@example.com", "cc:@example.com"),
            ("from:me", "from:me"),
            ("subject:offerta", "subject:(offerta)"),
            ("body:contratto", "contratto"),
            ("after:2024-01-01", "after:1704067200"),
            ("before:2024-01-01", "before:1704067200"),
            ("in:inbox", "in:inbox"),
            ("in:sent", "in:sent"),
            ("in:archive", "-in:inbox -in:sent"),
            ("in:spam", "in:spam"),
            ("in:trash", "in:trash"),
            ("in:anywhere", "in:anywhere"),
            ("label:clienti", "label:\"clienti\""),
            ("has:attachment", "has:attachment"),
            ("has:spreadsheet", "has:spreadsheet"),
            ("filename:preventivo", "filename:preventivo"),
            ("type:pdf", "filename:pdf"),
            ("is:unread", "is:unread"),
            ("is:starred", "is:starred"),
            ("-fattura", "-fattura"),
            ("fattura enel", "fattura enel"),
            ("fattura OR bolletta", "{fattura bolletta}"),
        ];
        for (input, expected) in rows {
            assert_eq!(raw(input), expected, "{input}");
        }
    }

    #[test]
    fn dates_are_unix_seconds_and_on_is_a_closed_day() {
        assert_eq!(raw("on:2024-01-01"), "after:1704067200 before:1704153600");
    }

    #[test]
    fn a_partly_local_query_sends_the_rest_and_asks_for_a_local_filter() {
        // EX-3
        let query = translate("is:task from:anna");
        assert_eq!(query.raw.as_deref(), Some("from:anna"));
        assert!(query.needs_local_filter && !query.local_only);
    }

    #[test]
    fn a_purely_local_query_never_reaches_the_server() {
        // EX-4
        let query = translate("is:snoozed");
        assert_eq!(query.raw, None);
        assert!(query.local_only);
        assert!(translate("in:tasks").local_only);
        assert!(translate("has:reminder").local_only);
    }

    #[test]
    fn an_alternative_with_a_local_half_is_left_out_whole() {
        let query = translate("from:anna (is:task OR subject:offerta)");
        assert_eq!(query.raw.as_deref(), Some("from:anna"));
        assert!(query.needs_local_filter);
        let mixed = translate("(from:anna subject:offerta) OR fattura");
        assert_eq!(mixed.raw.as_deref(), Some("{(from:anna subject:(offerta)) fattura}"));
        assert!(!mixed.needs_local_filter);
    }

    #[test]
    fn a_negated_local_part_is_left_out() {
        let query = translate("fattura -is:task");
        assert_eq!(query.raw.as_deref(), Some("fattura"));
        assert!(query.needs_local_filter);
        assert_eq!(raw("-(from:anna subject:x)"), "-(from:anna subject:(x))");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(raw(r#""say \"hi\"""#), r#""say \"hi\"""#);
        assert_eq!(raw(r#""a\\b""#), r#""a\\b""#);
        assert_eq!(raw("subject:\"(urgente)\""), "subject:\"(urgente)\"");
        assert_eq!(raw("subject:\"-urgente\""), "subject:\"-urgente\"");
        assert_eq!(raw("subject:\"{bozza}\""), "subject:\"{bozza}\"");
    }

    #[test]
    fn non_ascii_words_travel_as_they_are() {
        assert_eq!(raw("perché"), "perché");
        assert_eq!(raw("\"perché no\""), "\"perché no\"");
    }

    #[test]
    fn spam_and_trash_add_their_folders_to_the_search() {
        assert_eq!(translate("in:spam fattura").scopes, [RemoteScope::Spam]);
        assert_eq!(translate("in:anywhere").scopes, [RemoteScope::Spam, RemoteScope::Trash]);
        assert!(translate("fattura").scopes.is_empty());
        assert!(translate("in:trash OR is:task").scopes.is_empty(), "a dropped alternative adds no folder");
    }

    #[test]
    fn categories_and_accounts_are_local() {
        let query = translate("category:newsletter fattura");
        assert_eq!(query.raw.as_deref(), Some("fattura"));
        assert!(query.needs_local_filter);
        let query = translate("account:io@example.com fattura");
        assert_eq!(query.raw.as_deref(), Some("fattura"));
    }

    #[test]
    fn the_local_filter_trusts_the_server_for_text_and_checks_metadata() {
        let store = store();
        let parsed = ParsedQuery::parse(&store, "fattura -bolletta is:task", 1_700_000_000).unwrap();
        let mut email = crate::model::Email { remote_only: true, folder: "Archive".into(), ..Default::default() };
        assert!(!parsed.server_result_matches(&email), "not a task on this computer");
        email.pinned = true;
        assert!(parsed.server_result_matches(&email), "text was decided by the server");
        email.folder = "Trash".into();
        assert!(!parsed.server_result_matches(&email), "trash only when asked");
    }
}
