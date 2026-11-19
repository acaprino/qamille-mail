//! A single validated grammar for desktop, native, semantic and MCP searches.
use crate::model::{Category, Email, Store};
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use unicode_normalization::UnicodeNormalization;

fn normalized(value: &str) -> String {
    if value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        value.nfc().collect::<String>().to_lowercase()
    }
}

fn normalized_contains(value: &str, needle: &str) -> bool {
    // ASCII has no composition boundaries, so a direct lowercase match is
    // already equivalent to normalized matching. Unicode still uses NFC.
    (value.is_ascii() && value.contains(needle)) || normalized(value).contains(needle)
}

#[derive(Clone, Debug)]
enum Token {
    Atom {
        value: String,
        literal: bool,
        quoted: bool,
    },
    And,
    Or,
    Not,
    Open,
    Close,
}

fn lex(text: &str) -> Result<Vec<Token>, String> {
    if text.len() > 8192 {
        return Err("Search is too long (maximum 8192 bytes)".into());
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut output = Vec::new();
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        match chars[i] {
            '(' => {
                output.push(Token::Open);
                i += 1;
                continue;
            }
            ')' => {
                output.push(Token::Close);
                i += 1;
                continue;
            }
            '-' => {
                output.push(Token::Not);
                i += 1;
                continue;
            }
            _ => {}
        }
        let literal = chars[i] == '"';
        let mut value = String::new();
        let mut in_quotes = false;
        let mut quoted = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' {
                in_quotes = !in_quotes;
                quoted = true;
                i += 1;
                continue;
            }
            if c == '\\' && in_quotes && i + 1 < chars.len() && matches!(chars[i + 1], '"' | '\\') {
                value.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if !in_quotes && (c.is_whitespace() || c == '(' || c == ')') {
                break;
            }
            value.push(c);
            i += 1;
        }
        if in_quotes {
            return Err("Close the quoted phrase with a double quote".into());
        }
        if value.is_empty() {
            return Err("Enter a word or a filter value".into());
        }
        output.push(if !quoted && value == "AND" {
            Token::And
        } else if !quoted && value == "OR" {
            Token::Or
        } else {
            Token::Atom {
                value,
                literal,
                quoted,
            }
        });
        if output.len() > 512 {
            return Err("Too many search terms".into());
        }
    }
    Ok(output)
}

#[derive(Clone, Debug)]
enum Node {
    All,
    Atom(Atom),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
}

#[derive(Clone, Debug)]
enum Atom {
    Text(String, bool),
    Person(String, String),
    Subject(String),
    Body(String),
    Date {
        start: Option<i64>,
        end: Option<i64>,
    },
    Folder(String),
    Account(Vec<u32>),
    Has(String),
    State(String),
    Category(Category),
    Label(String),
    Filename(String),
    FileType(String),
}

/// Parsed once, then evaluated against every candidate message.
#[derive(Clone, Debug)]
pub struct ParsedQuery {
    root: Node,
    now: i64,
    pub timezone: String,
    pub locale: String,
    center: Option<NaiveDate>,
    words: Vec<String>,
}

struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    store: &'a Store,
    now: i64,
    zone: String,
    locale: String,
    center: Option<NaiveDate>,
    account_id: Option<u32>,
}

impl Parser<'_> {
    fn or(&mut self, depth: usize) -> Result<Node, String> {
        let mut node = self.and(depth)?;
        while matches!(self.tokens.get(self.pos), Some(Token::Or)) {
            self.pos += 1;
            node = Node::Or(Box::new(node), Box::new(self.and(depth)?));
        }
        Ok(node)
    }
    fn and(&mut self, depth: usize) -> Result<Node, String> {
        let mut node = self.unary(depth)?;
        loop {
            if matches!(self.tokens.get(self.pos), Some(Token::And)) {
                self.pos += 1;
            } else if !matches!(
                self.tokens.get(self.pos),
                Some(Token::Atom { .. } | Token::Not | Token::Open)
            ) {
                break;
            }
            node = Node::And(Box::new(node), Box::new(self.unary(depth)?));
        }
        Ok(node)
    }
    fn unary(&mut self, depth: usize) -> Result<Node, String> {
        if depth > 32 {
            return Err("Search groups are nested too deeply".into());
        }
        let token = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or("Enter a term after the operator")?;
        self.pos += 1;
        match token {
            Token::Not => Ok(Node::Not(Box::new(self.unary(depth + 1)?))),
            Token::Open => {
                let node = self.or(depth + 1)?;
                if !matches!(self.tokens.get(self.pos), Some(Token::Close)) {
                    return Err("Close the search group with )".into());
                }
                self.pos += 1;
                Ok(node)
            }
            Token::Atom {
                value,
                literal,
                quoted,
            } => self.atom(&value, literal, quoted).map(Node::Atom),
            _ => Err("Enter a term before this operator or parenthesis".into()),
        }
    }
    fn date(&self, text: &str) -> Result<NaiveDate, String> {
        let regional = if self.locale.to_lowercase().starts_with("en-us") {
            "%m/%d/%Y"
        } else {
            "%d/%m/%Y"
        };
        for format in ["%Y-%m-%d", "%Y/%m/%d", regional] {
            let parts: Vec<_> = text
                .split(if format.contains('-') { '-' } else { '/' })
                .collect();
            let year_index = if format.starts_with("%Y") { 0 } else { 2 };
            if parts.len() != 3
                || parts[year_index].len() != 4
                || !parts.iter().all(|s| s.chars().all(|c| c.is_ascii_digit()))
            {
                continue;
            }
            if let Ok(date) = NaiveDate::parse_from_str(text, format) {
                return Ok(date);
            }
        }
        Err(format!(
            "Invalid date «{text}». Use YYYY-MM-DD with a complete year"
        ))
    }
    fn start(&self, date: NaiveDate) -> Result<i64, String> {
        crate::domain::regional::day_start(date, &self.zone)
    }
    fn atom(&mut self, raw: &str, literal: bool, quoted: bool) -> Result<Atom, String> {
        let text = normalized(raw);
        if literal || !text.contains(':') {
            return Ok(Atom::Text(text, quoted));
        }
        let (op, value) = text.split_once(':').unwrap();
        if value.is_empty() {
            return Err(format!("Enter a value after {op}:"));
        }
        match op {
            "from" | "to" | "cc" => {
                if value == "me" {
                    let names = self
                        .store
                        .accounts
                        .iter()
                        .filter(|a| self.account_id.is_none_or(|id| a.id == id))
                        .map(|a| normalized(&a.email))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let names = format!("\n{names}");
                    Ok(Atom::Person(op.into(), names))
                } else {
                    Ok(Atom::Person(op.into(), value.into()))
                }
            }
            "subject" => Ok(Atom::Subject(value.into())),
            "body" => Ok(Atom::Body(value.into())),
            "filename" => Ok(Atom::Filename(value.into())),
            "type" => {
                if !value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '+' | '.'))
                {
                    return Err("Enter a file extension or MIME type".into());
                }
                let value = value.trim_start_matches('.');
                if value.is_empty() {
                    return Err("Enter a file extension or MIME type".into());
                }
                Ok(Atom::FileType(value.into()))
            }
            "before" | "after" | "on" => {
                let day = self.date(value)?;
                let start = self.start(day)?;
                let next = self.start(day.succ_opt().ok_or("Date is out of range")?)?;
                Ok(match op {
                    "before" => Atom::Date {
                        start: None,
                        end: Some(start),
                    },
                    "after" => Atom::Date {
                        start: Some(start),
                        end: None,
                    },
                    _ => Atom::Date {
                        start: Some(start),
                        end: Some(next),
                    },
                })
            }
            "between" => {
                let (from, until) = value
                    .split_once("..")
                    .ok_or("Use between:YYYY-MM-DD..YYYY-MM-DD")?;
                let from = self.date(from)?;
                let until = self.date(until)?;
                if from > until {
                    return Err("The start date is after the end date".into());
                }
                Ok(Atom::Date {
                    start: Some(self.start(from)?),
                    end: Some(self.start(until.succ_opt().ok_or("Date is out of range")?)?),
                })
            }
            "around" => {
                // A complete date may itself use slashes. Only split a radius if
                // the entire value is not already a valid date.
                let (date, radius) = if let Ok(date) = self.date(value) {
                    (date, 6i64)
                } else {
                    let (date, radius) = value
                        .rsplit_once('/')
                        .ok_or("Use around:YYYY-MM-DD or around:YYYY-MM-DD/6d")?;
                    let digits = radius.strip_suffix('d').ok_or_else(|| {
                        let corrected = if let Ok(days) = radius.parse::<u32>() {
                            Some(days)
                        } else {
                            let unit = radius.chars().last().unwrap_or(' ');
                            let multiplier = match unit {
                                'w' => 7,
                                'm' => 30,
                                'y' => 365,
                                _ => 0,
                            };
                            radius
                                .strip_suffix(unit)
                                .and_then(|digits| digits.parse::<u32>().ok())
                                .and_then(|n| n.checked_mul(multiplier))
                                .filter(|_| multiplier != 0)
                        };
                        match corrected.filter(|days| *days <= 365) {
                            Some(days) => format!("The radius must be in days. Use /{days}d"),
                            None => "The radius must be in days, for example /6d".into(),
                        }
                    })?;
                    let radius: i64 = digits
                        .parse()
                        .map_err(|_| "Enter a whole number of days from 0 to 365")?;
                    if !(0..=365).contains(&radius) {
                        return Err("The radius must be between 0 and 365 days".into());
                    }
                    (self.date(date)?, radius)
                };
                if self.center.is_none() {
                    self.center = Some(date);
                }
                let from = date
                    .checked_sub_signed(Duration::days(radius))
                    .ok_or("Date is out of range")?;
                let end = date
                    .checked_add_signed(Duration::days(radius + 1))
                    .ok_or("Date is out of range")?;
                Ok(Atom::Date {
                    start: Some(self.start(from)?),
                    end: Some(self.start(end)?),
                })
            }
            "newer_than" | "older_than" => {
                let unit = value
                    .chars()
                    .last()
                    .ok_or("Enter a duration, for example 7d")?;
                let digits = value.strip_suffix(unit).unwrap_or("");
                let count: i64 = digits.parse().map_err(
                    |_| "Invalid duration; use a positive whole number and d, w, m or y",
                )?;
                let multiplier = match unit {
                    'd' => 1,
                    'w' => 7,
                    'm' => 30,
                    'y' => 365,
                    _ => return Err("Duration unit must be d, w, m or y".into()),
                };
                if count < 0 {
                    return Err("Duration cannot be negative".into());
                }
                let span = count
                    .checked_mul(multiplier)
                    .and_then(|n| n.checked_mul(86_400))
                    .ok_or("Duration is too large")?;
                let threshold = self.now.checked_sub(span).ok_or("Duration is too large")?;
                Ok(if op == "newer_than" {
                    Atom::Date {
                        start: Some(threshold),
                        end: None,
                    }
                } else {
                    Atom::Date {
                        start: None,
                        end: Some(threshold),
                    }
                })
            }
            "has" => {
                if !matches!(
                    value,
                    "attachment" | "document" | "spreadsheet" | "presentation" | "reminder"
                ) {
                    return Err(format!("Unknown attachment filter «{value}»"));
                }
                Ok(Atom::Has(value.into()))
            }
            "is" => {
                if !matches!(
                    value,
                    "read"
                        | "unread"
                        | "starred"
                        | "pinned"
                        | "unreplied"
                        | "done"
                        | "task"
                        | "snoozed"
                ) {
                    return Err(format!("Unknown message state «{value}»"));
                }
                Ok(Atom::State(value.into()))
            }
            "category" => Category::from_key(value)
                .map(Atom::Category)
                .ok_or_else(|| format!("Unknown category «{value}»")),
            "label" | "tag" => Ok(Atom::Label(value.into())),
            "in" | "folder" => {
                if !matches!(
                    value,
                    "inbox"
                        | "sent"
                        | "archive"
                        | "spam"
                        | "trash"
                        | "anywhere"
                        | "tasks"
                        | "snoozed"
                ) && !self
                    .store
                    .emails
                    .iter()
                    .any(|e| normalized(&e.folder) == value)
                {
                    return Err(format!("Unknown folder «{value}»"));
                }
                Ok(Atom::Folder(value.into()))
            }
            "account" => {
                let ids: Vec<u32> = self
                    .store
                    .accounts
                    .iter()
                    .filter(|a| normalized(&a.email) == value || normalized(&a.name) == value)
                    .map(|a| a.id)
                    .collect();
                if ids.is_empty() {
                    return Err(format!("Unknown account «{value}»"));
                }
                Ok(Atom::Account(ids))
            }
            _ => Err(format!(
                "Unknown filter «{op}:». Put literal text in double quotes"
            )),
        }
    }
}

fn bounds(node: &Node) -> Result<(Option<i64>, Option<i64>), String> {
    match node {
        Node::Atom(Atom::Date { start, end }) => Ok((*start, *end)),
        Node::And(a, b) => {
            let (a_start, a_end) = bounds(a)?;
            let (b_start, b_end) = bounds(b)?;
            let start = match (a_start, b_start) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            let end = match (a_end, b_end) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            if start.zip(end).is_some_and(|(s, e)| s >= e) {
                return Err("These date periods do not overlap".into());
            }
            Ok((start, end))
        }
        Node::Or(a, b) => {
            bounds(a)?;
            bounds(b)?;
            Ok((None, None))
        }
        _ => Ok((None, None)),
    }
}

fn collect_words(node: &Node, semantic: bool, negative: bool, output: &mut Vec<String>) {
    match node {
        Node::Atom(Atom::Text(text, quoted)) if !negative && (!semantic || !quoted) => {
            output.push(text.clone())
        }
        Node::And(a, b) | Node::Or(a, b) => {
            collect_words(a, semantic, negative, output);
            collect_words(b, semantic, negative, output);
        }
        Node::Not(n) => collect_words(n, semantic, !negative, output),
        _ => {}
    }
}

fn person_matches(value: &str, name: &str, address: &str) -> bool {
    let address = normalized(address);
    // Draft recipients can be stored as `Name <address>`; compare the mailbox.
    let mailbox = address
        .rsplit_once('<')
        .map(|(_, a)| a.trim_end_matches('>').trim())
        .unwrap_or(address.trim());
    if value.contains('\n') {
        return value
            .lines()
            .filter(|v| !v.is_empty())
            .any(|v| mailbox == v);
    }
    if let Some(domain) = value.strip_prefix('@') {
        return mailbox.rsplit_once('@').is_some_and(|(_, d)| d == domain);
    }
    if value.contains('@') {
        return mailbox == value;
    }
    normalized(name).contains(value) || mailbox.split('@').next().unwrap_or("").contains(value)
}

fn text_metadata_matches(email: &Email, text: &str) -> bool {
    [&email.subject, &email.from_name, &email.from_addr]
        .iter()
        .any(|field| normalized_contains(field, text))
        || email
            .to
            .iter()
            .chain(&email.cc)
            .chain(&email.labels)
            .any(|field| normalized_contains(field, text))
        || email
            .attachments
            .iter()
            .any(|attachment| normalized_contains(&attachment.name, text))
}

fn atom_matches(atom: &Atom, email: &Email, now: i64, semantic: bool, negative: bool) -> bool {
    match atom {
        Atom::Text(text, quoted) => {
            if semantic && !quoted && !negative {
                return true;
            }
            text_metadata_matches(email, text)
                || normalized_contains(&email.body, text)
                || email.attachments.iter().any(|a| {
                    a.inline_text.as_ref()
                        .is_some_and(|t| normalized_contains(&t, text))
                })
                // Under an exclusion, unreadable content cannot establish
                // absence. Treat it as a possible match so NOT excludes it.
                || (negative && email.attachments.iter().any(|a| {
                    a.inline_text.as_ref().is_none_or(|t| t.trim().is_empty())
                }))
        }
        Atom::Person(field, value) => match field.as_str() {
            "from" => person_matches(value, &email.from_name, &email.from_addr),
            "to" => email.to.iter().any(|a| person_matches(value, "", a)),
            _ => email.cc.iter().any(|a| person_matches(value, "", a)),
        },
        Atom::Subject(value) => normalized_contains(&email.subject, value),
        Atom::Body(value) => normalized_contains(&email.body, value),
        Atom::Date { start, end } => {
            !email.date_missing
                && start.is_none_or(|t| email.date >= t)
                && end.is_none_or(|t| email.date < t)
        }
        Atom::Folder(value) => match value.as_str() {
            "anywhere" => true,
            "tasks" => email.is_task(),
            "snoozed" => email.is_snoozed(now),
            _ => normalized(&email.folder) == *value,
        },
        Atom::Account(ids) => ids.contains(&email.account_id),
        Atom::Has(value) => match value.as_str() {
            "attachment" => !email.attachments.is_empty(),
            "reminder" => email.reminder_at.is_some(),
            "document" => email.attachments.iter().any(|a| {
                ["pdf", "docx", "doc", "txt"].contains(
                    &a.name
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str(),
                )
            }),
            "spreadsheet" => email.attachments.iter().any(|a| {
                ["xlsx", "xls", "csv"].contains(
                    &a.name
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str(),
                )
            }),
            "presentation" => email.attachments.iter().any(|a| {
                ["pptx", "ppt", "key"].contains(
                    &a.name
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str(),
                )
            }),
            _ => false,
        },
        Atom::State(value) => match value.as_str() {
            "read" => email.read,
            "unread" => !email.read,
            "starred" => email.starred,
            "pinned" => email.pinned,
            "unreplied" => !email.replied,
            "done" => email.done,
            "task" => email.is_task(),
            "snoozed" => email.is_snoozed(now),
            _ => false,
        },
        Atom::Category(value) => email.category == *value,
        Atom::Label(value) => email.labels.iter().any(|l| normalized(l) == *value),
        Atom::Filename(value) => email
            .attachments
            .iter()
            .any(|a| normalized_contains(&a.name, value)),
        Atom::FileType(value) => email.attachments.iter().any(|a| {
            normalized(&a.mime) == *value
                || a.name
                    .rsplit('.')
                    .next()
                    .is_some_and(|ext| normalized(ext) == *value)
                || match value.as_str() {
                    "pdf" => a.mime.eq_ignore_ascii_case("application/pdf"),
                    "image" => a.mime.starts_with("image/"),
                    "audio" => a.mime.starts_with("audio/"),
                    "video" => a.mime.starts_with("video/"),
                    _ => false,
                }
        }),
    }
}

/// Return both truth and a positive folder grant from the same satisfied branch.
/// This prevents `subject:x OR in:trash` from exposing unrelated spam.
fn evaluate(node: &Node, email: &Email, now: i64, semantic: bool, negative: bool) -> (bool, bool) {
    evaluate_as(node, email, now, semantic, negative, false)
}

/// `server`: text and body were already decided by the provider's search, in
/// their own polarity; every other predicate is checked on the metadata.
fn evaluate_as(node: &Node, email: &Email, now: i64, semantic: bool, negative: bool, server: bool) -> (bool, bool) {
    match node {
        Node::All => (true, false),
        Node::Atom(Atom::Text(..) | Atom::Body(_)) if server => (!negative, false),
        Node::Atom(atom) => {
            let matches = atom_matches(atom, email, now, semantic, negative);
            let scope = !negative
                && matches
                && matches!(atom, Atom::Folder(v) if matches!(v.as_str(), "anywhere" | "trash" | "spam"));
            (matches, scope)
        }
        Node::Not(n) => (!evaluate_as(n, email, now, semantic, !negative, server).0, false),
        Node::And(a, b) => {
            let a = evaluate_as(a, email, now, semantic, negative, server);
            if !a.0 {
                return (false, false);
            }
            let b = evaluate_as(b, email, now, semantic, negative, server);
            (b.0, b.0 && (a.1 || b.1))
        }
        Node::Or(a, b) => {
            let a = evaluate_as(a, email, now, semantic, negative, server);
            let hidden = matches!(email.folder.to_lowercase().as_str(), "trash" | "spam");
            if a.0 && (a.1 || !hidden) {
                return a;
            }
            let b = evaluate_as(b, email, now, semantic, negative, server);
            (a.0 || b.0, (a.0 && a.1) || (b.0 && b.1))
        }
    }
}

impl ParsedQuery {
    pub fn parse(store: &Store, text: &str, now: i64) -> Result<Self, String> {
        Self::parse_scoped(store, text, now, None)
    }
    pub fn parse_scoped(
        store: &Store,
        text: &str,
        now: i64,
        account_id: Option<u32>,
    ) -> Result<Self, String> {
        let zone = if store.settings.time_zone.is_empty() { "UTC" } else { store.settings.time_zone.as_str() };
        let locale = if store.settings.locale.is_empty() { "en-US" } else { store.settings.locale.as_str() };
        Self::parse_regional(store, text, now, account_id, zone, locale)
    }
    /// Parses with the time zone and locale the caller resolved (the system ones when the
    /// settings leave them empty), without copying the store to put them in.
    pub fn parse_regional(
        store: &Store,
        text: &str,
        now: i64,
        account_id: Option<u32>,
        zone: &str,
        locale: &str,
    ) -> Result<Self, String> {
        let zone = if zone.is_empty() { "UTC".to_string() } else { zone.to_string() };
        let locale = if locale.is_empty() { "en-US".to_string() } else { locale.to_string() };
        let mut parser = Parser {
            tokens: lex(text)?,
            pos: 0,
            store,
            now,
            zone: zone.clone(),
            locale: locale.clone(),
            center: None,
            account_id,
        };
        let root = if parser.tokens.is_empty() {
            Node::All
        } else {
            parser.or(0)?
        };
        if parser.pos != parser.tokens.len() {
            return Err("Unexpected closing parenthesis".into());
        }
        bounds(&root)?;
        let root = prioritize_metadata(root);
        let mut words = Vec::new();
        collect_words(&root, false, false, &mut words);
        Ok(Self {
            root,
            now,
            timezone: zone,
            locale,
            center: parser.center,
            words,
        })
    }
    pub fn matches(&self, email: &Email, _store: &Store) -> bool {
        self.match_mode(email, false)
    }
    pub fn semantic_matches(&self, email: &Email, _store: &Store) -> bool {
        self.match_mode(email, true)
    }
    /// Indexed token/phrase predicates share the exact same Boolean and folder
    /// rules as source searches. The caller supplies an index, never source text.
    pub fn indexed_matches(
        &self,
        email: &Email,
        semantic: bool,
        lookup: &dyn Fn(Option<&str>, &str) -> bool,
    ) -> bool {
        self.three_valued_matches(email, semantic, lookup, email.search_index.partial)
    }
    /// With `unknown_content`, text absent from headers, cache and `lookup` is
    /// unknown rather than absent: it can neither satisfy nor exclude a match.
    fn three_valued_matches(
        &self,
        email: &Email,
        semantic: bool,
        lookup: &dyn Fn(Option<&str>, &str) -> bool,
        unknown_content: bool,
    ) -> bool {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Truth {
            False,
            Unknown,
            True,
        }
        impl Truth {
            fn not(self) -> Self {
                match self {
                    Self::False => Self::True,
                    Self::Unknown => Self::Unknown,
                    Self::True => Self::False,
                }
            }
            fn and(self, other: Self) -> Self {
                match (self, other) {
                    (Self::False, _) | (_, Self::False) => Self::False,
                    (Self::True, Self::True) => Self::True,
                    _ => Self::Unknown,
                }
            }
            fn or(self, other: Self) -> Self {
                match (self, other) {
                    (Self::True, _) | (_, Self::True) => Self::True,
                    (Self::False, Self::False) => Self::False,
                    _ => Self::Unknown,
                }
            }
            fn content(matched: bool, partial: bool) -> Self {
                if matched {
                    Self::True
                } else if partial {
                    Self::Unknown
                } else {
                    Self::False
                }
            }
        }
        fn visit(
            node: &Node,
            email: &Email,
            now: i64,
            semantic: bool,
            negative: bool,
            lookup: &dyn Fn(Option<&str>, &str) -> bool,
            unknown: bool,
        ) -> (Truth, bool) {
            match node {
                Node::All => (Truth::True, false),
                Node::Atom(atom) => {
                    let truth = match atom {
                        Atom::Text(_, quoted) if semantic && !quoted && !negative => Truth::True,
                        Atom::Text(text, _) => {
                            // Metadata remains known after source eviction and may
                            // change without changing the MIME hash (e.g. labels).
                            let metadata = text_metadata_matches(email, text);
                            let cached_source = !email.remote_only
                                && (normalized_contains(&email.body, text)
                                    || email.attachments.iter().any(|attachment| {
                                        attachment
                                            .inline_text
                                            .as_ref()
                                            .is_some_and(|body| normalized_contains(body, text))
                                    }));
                            Truth::content(
                                metadata || cached_source || lookup(None, text),
                                unknown,
                            )
                        }
                        Atom::Body(text) => Truth::content(
                            lookup(Some("body"), text)
                                || (!email.remote_only && normalized_contains(&email.body, text)),
                            unknown,
                        ),
                        _ => {
                            if atom_matches(atom, email, now, semantic, negative) {
                                Truth::True
                            } else {
                                Truth::False
                            }
                        }
                    };
                    (
                        truth,
                        truth == Truth::True
                            && !negative
                            && matches!(atom, Atom::Folder(value) if matches!(value.as_str(), "anywhere" | "trash" | "spam")),
                    )
                }
                Node::Not(node) => (
                    visit(node, email, now, semantic, !negative, lookup, unknown).0.not(),
                    false,
                ),
                Node::And(a, b) => {
                    let a = visit(a, email, now, semantic, negative, lookup, unknown);
                    if a.0 == Truth::False {
                        return (Truth::False, false);
                    }
                    let b = visit(b, email, now, semantic, negative, lookup, unknown);
                    let truth = a.0.and(b.0);
                    (truth, truth == Truth::True && (a.1 || b.1))
                }
                Node::Or(a, b) => {
                    let a = visit(a, email, now, semantic, negative, lookup, unknown);
                    if a.0 == Truth::True
                        && (a.1
                            || !matches!(email.folder.to_lowercase().as_str(), "trash" | "spam"))
                    {
                        return a;
                    }
                    let b = visit(b, email, now, semantic, negative, lookup, unknown);
                    (
                        a.0.or(b.0),
                        (a.0 == Truth::True && a.1) || (b.0 == Truth::True && b.1),
                    )
                }
            }
        }
        // An incomplete index can prove presence but cannot prove absence.
        // Preserve Unknown inside its branch so known OR/AND predicates still
        // determine the result, rather than rejecting the entire expression.
        let (matched, scope) = visit(&self.root, email, self.now, semantic, false, lookup, unknown_content);
        matched == Truth::True
            && (!matches!(email.folder.to_lowercase().as_str(), "trash" | "spam") || scope)
    }
    fn match_mode(&self, email: &Email, semantic: bool) -> bool {
        // A missing body is unknown, not an empty string: a negative content
        // predicate must not become true after cache eviction. Headers (sender,
        // recipients, subject, labels) stay known and still prove presence.
        if email.remote_only && requires_source_text(&self.root, semantic, false) {
            return self.three_valued_matches(email, semantic, &|_, _| false, true);
        }
        let (matched, scope) = evaluate(&self.root, email, self.now, semantic, false);
        matched && (!matches!(email.folder.to_lowercase().as_str(), "trash" | "spam") || scope)
    }
    /// Filters a result of the provider's search (`search_remote`): text and
    /// body were matched by the provider, the rest is checked on the metadata.
    pub fn server_result_matches(&self, email: &Email) -> bool {
        let (matched, scope) = evaluate_as(&self.root, email, self.now, false, false, true);
        matched && (!matches!(email.folder.to_lowercase().as_str(), "trash" | "spam") || scope)
    }
    pub fn words(&self) -> Vec<String> {
        self.words.clone()
    }
    pub fn semantic_words(&self) -> String {
        let mut words = Vec::new();
        collect_words(&self.root, true, false, &mut words);
        words.join(" ")
    }
    /// Whether complete body availability is needed to rule out a candidate.
    pub fn needs_content(&self) -> bool {
        needs_content(&self.root, false)
    }
    /// Free text can match attachment contents, including under an exclusion.
    pub fn needs_attachment_content(&self) -> bool {
        needs_content(&self.root, true)
    }
    pub fn range(&self) -> Option<(i64, i64)> {
        let (a, b) = bounds(&self.root).ok()?;
        a.zip(b)
    }
    pub fn around_center(&self) -> Option<i64> {
        self.center
            .and_then(|d| crate::domain::regional::day_start(d, &self.timezone).ok())
    }
    fn distance(&self, timestamp: i64) -> i64 {
        let Some(center) = self.center else {
            return 0;
        };
        let Ok(zone) = self.timezone.parse::<chrono_tz::Tz>() else {
            return 0;
        };
        Utc.timestamp_opt(timestamp, 0)
            .single()
            .map(|t| {
                (t.with_timezone(&zone).date_naive() - center)
                    .num_days()
                    .abs()
            })
            .unwrap_or(i64::MAX)
    }
    pub fn date_distance(&self, timestamp: i64) -> i64 {
        self.distance(timestamp)
    }
}

/// A read-only view of the parsed tree, for translations to other search
/// languages (the Gmail search of `search_remote`). `Node` and `Atom` stay private.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryPart<'a> {
    All,
    Text { value: &'a str, exact: bool },
    /// `value` is `None` for `from:me`, `to:me`, `cc:me` (the user's own accounts).
    Person { field: &'a str, value: Option<&'a str> },
    Subject(&'a str),
    Body(&'a str),
    Date { start: Option<i64>, end: Option<i64> },
    Folder(&'a str),
    Label(&'a str),
    Has(&'a str),
    Filename(&'a str),
    FileType(&'a str),
    State(&'a str),
    /// A predicate that only exists on this computer (`category:`, `account:`).
    Local,
    Not(Box<QueryPart<'a>>),
    And(Box<QueryPart<'a>>, Box<QueryPart<'a>>),
    Or(Box<QueryPart<'a>>, Box<QueryPart<'a>>),
}

fn part(node: &Node) -> QueryPart<'_> {
    match node {
        Node::All => QueryPart::All,
        Node::Not(n) => QueryPart::Not(Box::new(part(n))),
        Node::And(a, b) => QueryPart::And(Box::new(part(a)), Box::new(part(b))),
        Node::Or(a, b) => QueryPart::Or(Box::new(part(a)), Box::new(part(b))),
        Node::Atom(atom) => match atom {
            Atom::Text(value, exact) => QueryPart::Text { value, exact: *exact },
            Atom::Person(field, value) => QueryPart::Person {
                field,
                value: (!value.starts_with('\n')).then_some(value.as_str()),
            },
            Atom::Subject(v) => QueryPart::Subject(v),
            Atom::Body(v) => QueryPart::Body(v),
            Atom::Date { start, end } => QueryPart::Date { start: *start, end: *end },
            Atom::Folder(v) => QueryPart::Folder(v),
            Atom::Label(v) => QueryPart::Label(v),
            Atom::Has(v) => QueryPart::Has(v),
            Atom::Filename(v) => QueryPart::Filename(v),
            Atom::FileType(v) => QueryPart::FileType(v),
            Atom::State(v) => QueryPart::State(v),
            Atom::Category(_) | Atom::Account(_) => QueryPart::Local,
        },
    }
}

fn top_accounts(node: &Node, output: &mut Option<Vec<u32>>) {
    match node {
        Node::And(a, b) => {
            top_accounts(a, output);
            top_accounts(b, output);
        }
        Node::Atom(Atom::Account(ids)) => {
            *output = Some(match output.take() {
                Some(previous) => previous.into_iter().filter(|id| ids.contains(id)).collect(),
                None => ids.clone(),
            });
        }
        _ => {}
    }
}

impl ParsedQuery {
    /// The tree as a read-only view.
    pub fn tree(&self) -> QueryPart<'_> {
        part(&self.root)
    }
    /// The accounts an `account:` filter at the top level allows; `None` means all.
    pub fn accounts(&self) -> Option<Vec<u32>> {
        let mut output = None;
        top_accounts(&self.root, &mut output);
        output
    }
}

/// AND is commutative: evaluate inexpensive metadata before content scans.
fn prioritize_metadata(node: Node) -> Node {
    match node {
        Node::And(a, b) => {
            let a = prioritize_metadata(*a);
            let b = prioritize_metadata(*b);
            if needs_content(&a, false) && !needs_content(&b, false) {
                Node::And(Box::new(b), Box::new(a))
            } else {
                Node::And(Box::new(a), Box::new(b))
            }
        }
        Node::Or(a, b) => Node::Or(
            Box::new(prioritize_metadata(*a)),
            Box::new(prioritize_metadata(*b)),
        ),
        Node::Not(node) => Node::Not(Box::new(prioritize_metadata(*node))),
        other => other,
    }
}

fn needs_content(node: &Node, attachments_only: bool) -> bool {
    match node {
        Node::Atom(Atom::Text(..)) => true,
        Node::Atom(Atom::Body(_)) => !attachments_only,
        Node::Not(n) => needs_content(n, attachments_only),
        Node::And(a, b) | Node::Or(a, b) => {
            needs_content(a, attachments_only) || needs_content(b, attachments_only)
        }
        _ => false,
    }
}

// Positive, unquoted concepts are satisfied by the semantic score. Exact text,
// body predicates and exclusions still require source content; a vector cannot
// establish whether a word or a quoted phrase is literally present or absent.
fn requires_source_text(node: &Node, semantic: bool, negative: bool) -> bool {
    match node {
        Node::Atom(Atom::Text(_, quoted)) => !semantic || *quoted || negative,
        Node::Atom(Atom::Body(_)) => true,
        Node::Not(node) => requires_source_text(node, semantic, !negative),
        Node::And(a, b) | Node::Or(a, b) => {
            requires_source_text(a, semantic, negative)
                || requires_source_text(b, semantic, negative)
        }
        _ => false,
    }
}

/// All filtering, including the selected account, precedes ranking and pagination.
pub fn execute(
    store: &Store,
    text: &str,
    account_id: Option<u32>,
    now: i64,
    sort: &str,
) -> Result<Vec<u64>, String> {
    let sort = match sort {
        "date" => "newest",
        "closest" => "nearest",
        other => other,
    };
    let query = ParsedQuery::parse_scoped(store, text, now, account_id)?;
    let nearby =
        sort == "nearest" || (sort.is_empty() && query.words.is_empty() && query.center.is_some());
    let relevance = sort != "newest" && sort != "oldest" && !nearby && !query.words.is_empty();
    // Ranking values are computed once per hit, rather than allocating and
    // parsing the zone repeatedly inside the O(n log n) sorting comparator.
    let mut matches: Vec<_> = store
        .emails
        .iter()
        .filter(|e| account_id.is_none_or(|id| e.account_id == id) && query.matches(e, store))
        .map(|email| {
            (
                email,
                if relevance {
                    crate::rules::score(email, &query.words)
                } else {
                    0
                },
                if nearby || relevance {
                    query.distance(email.date)
                } else {
                    0
                },
            )
        })
        .collect();
    matches.sort_by(|a, b| {
        let main = if nearby {
            a.2.cmp(&b.2)
        } else if relevance {
            b.1.cmp(&a.1).then(a.2.cmp(&b.2))
        } else {
            std::cmp::Ordering::Equal
        };
        main.then(if sort == "oldest" {
            a.0.date.cmp(&b.0.date)
        } else {
            b.0.date.cmp(&a.0.date)
        })
        .then(b.0.id.cmp(&a.0.id))
    });
    Ok(matches.into_iter().map(|(e, _, _)| e.id).collect())
}

#[cfg(test)]
mod indexed_match_tests {
    use super::*;
    use crate::model::Attachment;

    fn fixture() -> (Store, Email) {
        let mut store = Store::default();
        store.settings.time_zone = "UTC".into();
        store.settings.locale = "en-US".into();
        let email = Email {
            subject: "urgent invoice".into(),
            from_addr: "alice@example.com".into(),
            folder: "Inbox".into(),
            remote_only: true,
            ..Default::default()
        };
        (store, email)
    }

    fn indexed(store: &Store, email: &Email, query: &str, semantic: bool) -> bool {
        ParsedQuery::parse(store, query, 0)
            .unwrap()
            .indexed_matches(email, semantic, &|_, text| text == "needle")
    }

    #[test]
    fn online_mail_uses_current_labels_in_positive_and_negative_text_queries() {
        let (store, mut email) = fixture();
        email.labels = vec!["projectz".into()];
        assert!(indexed(&store, &email, "projectz", false));
        assert!(indexed(&store, &email, "\"projectz\"", false));
        assert!(!indexed(&store, &email, "-projectz", false));
        assert!(!indexed(&store, &email, "-projectz", true));
        assert!(indexed(&store, &email, "-cancelled", false));
        // A label edit changes metadata without rebuilding the MIME index.
        email.labels.clear();
        assert!(!indexed(&store, &email, "projectz", false));
        assert!(indexed(&store, &email, "-projectz", false));
        assert!(indexed(&store, &email, "invoice", false));
    }

    #[test]
    fn partial_content_does_not_cancel_a_known_true_or_branch() {
        let (store, mut email) = fixture();
        email.search_index.partial = true;
        for remote_only in [true, false] {
            email.remote_only = remote_only;
            assert!(indexed(
                &store,
                &email,
                "subject:urgent OR (needle -cancelled)",
                false
            ));
            assert!(indexed(
                &store,
                &email,
                "(needle -cancelled) OR subject:urgent",
                false
            ));
            assert!(indexed(
                &store,
                &email,
                "subject:urgent OR -body:cancelled",
                false
            ));
            assert!(!indexed(
                &store,
                &email,
                "subject:other AND -body:cancelled",
                false
            ));
        }
        // Negating a branch proved false by its metadata is also definite.
        // This grammar spells negation with '-', not the literal word NOT.
        assert!(indexed(
            &store,
            &email,
            "-(cancelled AND from:bob@example.com)",
            false
        ));
    }

    #[test]
    fn missing_partial_content_cannot_prove_absence() {
        let (store, mut email) = fixture();
        email.search_index.partial = true;
        assert!(!indexed(&store, &email, "needle -cancelled", false));
        assert!(!indexed(&store, &email, "needle -cancelled", true));
        assert!(!indexed(&store, &email, "-cancelled", false));
        assert!(!indexed(&store, &email, "-body:cancelled", false));
        assert!(!indexed(&store, &email, "-(-cancelled)", false));
        assert!(indexed(&store, &email, "needle", false));
        email.labels = vec!["cancelled".into()];
        assert!(!indexed(&store, &email, "-cancelled", false));
    }

    #[test]
    fn semantic_concepts_still_use_vectors_but_exact_phrases_require_presence() {
        let (store, mut email) = fixture();
        email.search_index.partial = true;
        assert!(indexed(&store, &email, "travel", true));
        assert!(!indexed(&store, &email, "travel", false));
        assert!(!indexed(&store, &email, "\"travel\"", true));
        assert!(!indexed(&store, &email, "\"travel\"", false));
        assert!(indexed(&store, &email, "\"needle\"", true));
    }

    #[test]
    fn unknown_branches_do_not_grant_hidden_folder_access() {
        let (store, mut email) = fixture();
        email.search_index.partial = true;
        email.folder = "Spam".into();
        assert!(!indexed(
            &store,
            &email,
            "subject:urgent OR in:trash",
            false
        ));
        assert!(!indexed(
            &store,
            &email,
            "subject:urgent OR -cancelled",
            false
        ));
        assert!(!indexed(&store, &email, "in:spam AND -cancelled", false));
        assert!(indexed(&store, &email, "in:spam OR -cancelled", false));
        assert!(indexed(&store, &email, "-cancelled OR in:spam", false));
        assert!(indexed(
            &store,
            &email,
            "subject:urgent AND in:anywhere",
            false
        ));
        email.folder = "Trash".into();
        assert!(!indexed(&store, &email, "subject:urgent OR in:spam", false));
        assert!(indexed(&store, &email, "-cancelled OR in:trash", false));
    }

    #[test]
    fn cached_source_remains_searchable_without_inventing_unknown_attachment_matches() {
        let (store, mut email) = fixture();
        email.remote_only = false;
        email.body = "cached full body".into();
        email.attachments.push(Attachment {
            name: "report.pdf".into(),
            ..Default::default()
        });
        assert!(indexed(&store, &email, "\"cached full body\"", false));
        assert!(indexed(&store, &email, "body:cached", false));
        assert!(indexed(&store, &email, "-cancelled", false));
        assert!(
            ParsedQuery::parse(&store, "\"cached full body\"", 0)
                .unwrap()
                .matches(&email, &store)
        );
    }
}
