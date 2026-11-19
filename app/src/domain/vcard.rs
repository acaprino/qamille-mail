//! vCard 3.0 / 4.0 reading and writing. Pure text handling.
use super::contacts::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedCard {
    pub uid: Option<String>,
    pub fields: ContactFields,
    /// Raw, unfolded lines this app does not manage (`X-*`, `URL`, `PHOTO`, ...).
    pub unmanaged: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VCardVersion {
    V3,
    V4,
}

const MANAGED: [&str; 12] = ["BEGIN", "END", "VERSION", "PRODID", "FN", "N", "ORG", "TITLE", "BDAY", "NOTE", "UID", "REV"];
const TYPE_NOISE: [&str; 5] = ["internet", "pref", "voice", "text", "x-inet"];

fn unfold(text: &str) -> Vec<String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    lines
}

/// Splits `NAME;PARAMS:VALUE` at the first colon outside quotes.
fn split_property(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

/// Components of a structured value, split on unescaped semicolons.
fn components(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            parts.last_mut().unwrap().push('\\');
            parts.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ';' {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts.into_iter().map(|p| unescape(&p)).collect()
}

fn type_label(params: &[&str]) -> String {
    let mut types = Vec::new();
    for param in params {
        let (key, value) = param.split_once('=').unwrap_or(("type", param));
        if key.eq_ignore_ascii_case("type") {
            types.extend(value.trim_matches('"').split(',').map(|t| t.trim().to_lowercase()));
        }
    }
    types.into_iter().find(|t| !t.is_empty() && !TYPE_NOISE.contains(&t.as_str())).unwrap_or_default()
}

pub fn parse_vcards(text: &str) -> Vec<ParsedCard> {
    let mut cards = Vec::new();
    let mut current: Option<ParsedCard> = None;
    let mut structured_name = String::new();
    for line in unfold(text) {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("BEGIN:VCARD") {
            current = Some(ParsedCard::default());
            structured_name.clear();
            continue;
        }
        if trimmed.eq_ignore_ascii_case("END:VCARD") {
            if let Some(mut card) = current.take() {
                if card.fields.name.is_empty() {
                    card.fields.name = std::mem::take(&mut structured_name);
                }
                cards.push(card);
            }
            continue;
        }
        let Some(card) = current.as_mut() else { continue };
        let Some((head, value)) = split_property(trimmed) else { continue };
        let mut parts = head.split(';');
        let full_name = parts.next().unwrap_or("");
        let name = full_name.rsplit('.').next().unwrap_or(full_name).to_ascii_uppercase();
        let params: Vec<&str> = parts.collect();
        match name.as_str() {
            "FN" => card.fields.name = unescape(value).trim().to_string(),
            "N" => {
                let c = components(value);
                let given = c.get(1).map(String::as_str).unwrap_or("");
                let family = c.first().map(String::as_str).unwrap_or("");
                structured_name = format!("{given} {family}").trim().to_string();
            }
            "ORG" => card.fields.org = components(value).into_iter().next().unwrap_or_default().trim().to_string(),
            "TITLE" => card.fields.title = unescape(value).trim().to_string(),
            "BDAY" => card.fields.birthday = value.trim().to_string(),
            "NOTE" => card.fields.notes = unescape(value),
            "UID" => card.uid = Some(unescape(value).trim().to_string()).filter(|u| !u.is_empty()),
            "EMAIL" => card.fields.emails.push(Labeled { label: type_label(&params), value: unescape(value).trim().to_string() }),
            "TEL" => card.fields.phones.push(Labeled { label: type_label(&params), value: unescape(value).trim().to_string() }),
            "ADR" => {
                let joined = components(value).into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(", ");
                card.fields.addresses.push(Labeled { label: type_label(&params), value: joined });
            }
            other if MANAGED.contains(&other) => {}
            _ => card.unmanaged.push(trimmed.to_string()),
        }
    }
    cards
}

/// Folds at 75 octets without splitting a UTF-8 character.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in line.chars() {
        let bytes = c.len_utf8();
        if width + bytes > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += bytes;
    }
    out
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(&fold(line));
    out.push_str("\r\n");
}

fn typed(name: &str, label: &str) -> String {
    let clean: String = label.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    if clean.is_empty() { name.to_string() } else { format!("{name};TYPE={clean}") }
}

pub fn write_vcard(uid: &str, fields: &ContactFields, unmanaged: &[String], version: VCardVersion) -> String {
    let mut out = String::new();
    push_line(&mut out, "BEGIN:VCARD");
    push_line(&mut out, if version == VCardVersion::V3 { "VERSION:3.0" } else { "VERSION:4.0" });
    push_line(&mut out, "PRODID:-//Qamille//Contacts//IT");
    push_line(&mut out, &format!("UID:{}", escape(uid)));
    let display = [fields.name.as_str()]
        .into_iter()
        .chain(fields.emails.first().map(|e| e.value.as_str()))
        .chain(fields.phones.first().map(|p| p.value.as_str()))
        .find(|v| !v.trim().is_empty())
        .unwrap_or("");
    push_line(&mut out, &format!("FN:{}", escape(display)));
    let mut words: Vec<&str> = fields.name.split_whitespace().collect();
    let family = words.pop().unwrap_or("");
    push_line(&mut out, &format!("N:{};{};;;", escape(family), escape(&words.join(" "))));
    if !fields.org.is_empty() {
        push_line(&mut out, &format!("ORG:{}", escape(&fields.org)));
    }
    if !fields.title.is_empty() {
        push_line(&mut out, &format!("TITLE:{}", escape(&fields.title)));
    }
    if !fields.birthday.is_empty() {
        push_line(&mut out, &format!("BDAY:{}", fields.birthday));
    }
    for email in &fields.emails {
        push_line(&mut out, &format!("{}:{}", typed("EMAIL", &email.label), escape(&email.value)));
    }
    for phone in &fields.phones {
        push_line(&mut out, &format!("{}:{}", typed("TEL", &phone.label), escape(&phone.value)));
    }
    for address in &fields.addresses {
        push_line(&mut out, &format!("{}:;;{};;;;", typed("ADR", &address.label), escape(&address.value)));
    }
    if !fields.notes.is_empty() {
        push_line(&mut out, &format!("NOTE:{}", escape(&fields.notes)));
    }
    for line in unmanaged {
        push_line(&mut out, line);
    }
    push_line(&mut out, "END:VCARD");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labeled(label: &str, value: &str) -> Labeled {
        Labeled { label: label.into(), value: value.into() }
    }
    fn full() -> ContactFields {
        ContactFields {
            name: "Ada Lovelace".into(),
            org: "Acme, Inc.".into(),
            title: "Analista; capo".into(),
            birthday: "1815-12-10".into(),
            emails: vec![labeled("work", "ada@x.it"), labeled("home", "ada@y.it")],
            phones: vec![labeled("cell", "+39 333 1234567")],
            addresses: vec![labeled("home", "Via Roma 1, Milano")],
            notes: "riga uno\nriga due, con virgola".into(),
        }
    }

    #[test]
    fn parses_folded_lines_and_escaped_commas() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada\r\n  Lovelace\r\nNOTE:uno\\, due\\nsecondo\r\nUID:abc-1\r\nEND:VCARD\r\n";
        let cards = parse_vcards(text);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].fields.name, "Ada Lovelace");
        assert_eq!(cards[0].fields.notes, "uno, due\nsecondo");
        assert_eq!(cards[0].uid.as_deref(), Some("abc-1"));
    }

    #[test]
    fn parses_bom_and_lf_only_files() {
        let cards = parse_vcards("\u{feff}BEGIN:VCARD\nVERSION:4.0\nFN:Bea\nEND:VCARD\n");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].fields.name, "Bea");
    }

    #[test]
    fn keeps_multiple_emails_with_labels() {
        let text = "BEGIN:VCARD\nVERSION:3.0\nFN:C\nEMAIL;TYPE=work:a@x.it\nEMAIL;TYPE=home,pref:b@x.it\nitem1.EMAIL;type=INTERNET:c@x.it\nEMAIL;WORK:d@x.it\nEND:VCARD\n";
        let emails = &parse_vcards(text)[0].fields.emails;
        let seen: Vec<(&str, &str)> = emails.iter().map(|e| (e.label.as_str(), e.value.as_str())).collect();
        assert_eq!(seen, vec![("work", "a@x.it"), ("home", "b@x.it"), ("", "c@x.it"), ("work", "d@x.it")]);
    }

    #[test]
    fn name_falls_back_to_the_structured_name() {
        let cards = parse_vcards("BEGIN:VCARD\nVERSION:3.0\nN:Rossi;Mario;;Dott.;\nEND:VCARD\n");
        assert_eq!(cards[0].fields.name, "Mario Rossi");
    }

    #[test]
    fn unknown_properties_survive_a_round_trip() {
        let text = "BEGIN:VCARD\nVERSION:3.0\nFN:Ada\nX-CUSTOM:1\nURL:https://example.test/ada\nEND:VCARD\n";
        let card = &parse_vcards(text)[0];
        assert_eq!(card.unmanaged, vec!["X-CUSTOM:1".to_string(), "URL:https://example.test/ada".to_string()]);
        let written = write_vcard("u1", &card.fields, &card.unmanaged, VCardVersion::V3);
        assert!(written.contains("X-CUSTOM:1\r\n") && written.contains("URL:https://example.test/ada\r\n"), "{written}");
    }

    #[test]
    fn round_trip_v3_and_v4_preserve_fields() {
        for version in [VCardVersion::V3, VCardVersion::V4] {
            let written = write_vcard("uid-9", &full(), &[], version);
            let marker = if version == VCardVersion::V3 { "VERSION:3.0" } else { "VERSION:4.0" };
            assert!(written.contains(marker), "{written}");
            let cards = parse_vcards(&written);
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].fields, full(), "{written}");
            assert_eq!(cards[0].uid.as_deref(), Some("uid-9"));
        }
    }

    #[test]
    fn two_cards_in_one_file_are_two_results() {
        let text = "BEGIN:VCARD\nVERSION:3.0\nFN:Uno\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nFN:Due\nEND:VCARD\n";
        let names: Vec<String> = parse_vcards(text).into_iter().map(|c| c.fields.name).collect();
        assert_eq!(names, vec!["Uno".to_string(), "Due".to_string()]);
    }

    #[test]
    fn card_without_name_or_address_still_parses_with_empty_fields() {
        let cards = parse_vcards("BEGIN:VCARD\nVERSION:3.0\nEND:VCARD\n");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].fields, ContactFields::default());
        assert!(parse_vcards("not a vcard").is_empty());
    }

    #[test]
    fn output_lines_never_exceed_75_octets_and_use_crlf() {
        let mut fields = full();
        fields.notes = "è".repeat(300);
        let written = write_vcard("u", &fields, &[], VCardVersion::V3);
        assert!(written.ends_with("END:VCARD\r\n"));
        assert!(!written.replace("\r\n", "").contains('\n'), "bare LF in output");
        for line in written.split("\r\n") {
            assert!(line.len() <= 75, "{} octets: {line}", line.len());
        }
        assert_eq!(parse_vcards(&written)[0].fields.notes, "è".repeat(300));
    }

    #[test]
    fn the_product_id_names_the_app() {
        let written = write_vcard("u", &full(), &[], VCardVersion::V3);
        assert!(written.contains("PRODID:-//Qamille//Contacts//IT\r\n"));
    }
}
