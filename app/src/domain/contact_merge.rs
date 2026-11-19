//! Field-level merge of contact cards. Pure and deterministic.
use super::contact_norm::{normalize_email, phone_tail};
use super::contacts::*;
use std::collections::{BTreeMap, HashSet};

pub const FIELD_NAMES: [&str; 8] = ["name", "org", "title", "birthday", "emails", "phones", "addresses", "notes"];

/// Lower is better. Unlisted books follow the listed ones; mail cards come last.
pub fn rank(book: &BookId, priority: &[BookId]) -> usize {
    match book.kind() {
        BookKind::Mail => usize::MAX,
        BookKind::GoogleOther => usize::MAX - 1,
        _ => priority.iter().position(|b| b == book).unwrap_or(priority.len()),
    }
}

fn scalar<'a>(fields: &'a ContactFields, name: &str) -> &'a str {
    match name {
        "name" => &fields.name,
        "org" => &fields.org,
        "title" => &fields.title,
        _ => &fields.birthday,
    }
}

fn collapse(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Cards ordered best first, then merged field by field.
pub fn merge_fields(sources: &[&SourceContact], priority: &[BookId], pins: &BTreeMap<String, String>) -> ContactFields {
    let mut ordered: Vec<&SourceContact> = sources.to_vec();
    ordered.sort_by(|a, b| {
        rank(&a.book, priority)
            .cmp(&rank(&b.book, priority))
            .then(b.modified.cmp(&a.modified))
            .then(a.key.cmp(&b.key))
    });
    let pick = |name: &str| -> String {
        if let Some(pinned) = pins.get(name) {
            return pinned.clone();
        }
        ordered
            .iter()
            .map(|s| scalar(&s.fields, name))
            .find(|v| !v.trim().is_empty())
            .unwrap_or("")
            .to_string()
    };
    let unite = |select: fn(&ContactFields) -> &Vec<Labeled>, key: fn(&str) -> String| -> Vec<Labeled> {
        let mut seen = HashSet::new();
        let mut united = Vec::new();
        for source in &ordered {
            for item in select(&source.fields) {
                let normalized = key(&item.value);
                if !normalized.is_empty() && seen.insert(normalized) {
                    united.push(item.clone());
                }
            }
        }
        united
    };
    let mut notes: Vec<(String, String)> = Vec::new();
    for source in &ordered {
        let note = source.fields.notes.trim();
        if !note.is_empty() && !notes.iter().any(|(n, _)| n == note) {
            notes.push((note.to_string(), source.book.0.clone()));
        }
    }
    let notes = match notes.as_slice() {
        [] => String::new(),
        [(only, _)] => only.clone(),
        many => many.iter().map(|(n, book)| format!("{n}\n— {book}")).collect::<Vec<_>>().join("\n\n"),
    };
    ContactFields {
        name: pick("name"),
        org: pick("org"),
        title: pick("title"),
        birthday: pick("birthday"),
        emails: unite(|f| &f.emails, normalize_email),
        phones: unite(|f| &f.phones, |v| {
            phone_tail(v).unwrap_or_else(|| v.chars().filter(|c| c.is_ascii_digit()).collect())
        }),
        addresses: unite(|f| &f.addresses, collapse),
        notes,
    }
}

#[derive(Debug, PartialEq)]
pub enum Merge3 {
    Clean(ContactFields),
    Conflict(Vec<String>),
}

/// Applies one-sided changes; two different edits of one field are a conflict.
pub fn three_way(base: &ContactFields, local: &ContactFields, theirs: &ContactFields) -> Merge3 {
    let mut conflicts = Vec::new();
    let mut result = base.clone();
    macro_rules! decide {
        ($field:ident, $label:literal) => {
            if local.$field == theirs.$field || theirs.$field == base.$field {
                result.$field = local.$field.clone();
            } else if local.$field == base.$field {
                result.$field = theirs.$field.clone();
            } else {
                conflicts.push($label.to_string());
            }
        };
    }
    decide!(name, "name");
    decide!(org, "org");
    decide!(title, "title");
    decide!(birthday, "birthday");
    decide!(emails, "emails");
    decide!(phones, "phones");
    decide!(addresses, "addresses");
    decide!(notes, "notes");
    if conflicts.is_empty() { Merge3::Clean(result) } else { Merge3::Conflict(conflicts) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(key: &str, book: BookId, modified: i64, edit: impl FnOnce(&mut ContactFields)) -> SourceContact {
        let mut fields = ContactFields::default();
        edit(&mut fields);
        SourceContact { key: key.into(), book, modified, fields, ..Default::default() }
    }
    fn item(value: &str) -> Labeled {
        Labeled { label: "home".into(), value: value.into() }
    }
    fn merged(cards: &[SourceContact], priority: &[BookId], pins: &BTreeMap<String, String>) -> ContactFields {
        let refs: Vec<&SourceContact> = cards.iter().collect();
        merge_fields(&refs, priority, pins)
    }

    #[test]
    fn higher_priority_book_wins_single_value() {
        let cards = [
            card("a", BookId::local(), 100, |f| f.org = "Locale Srl".into()),
            card("b", BookId::google(1), 50, |f| f.org = "Google Srl".into()),
        ];
        let priority = [BookId::google(1), BookId::local()];
        assert_eq!(merged(&cards, &priority, &Default::default()).org, "Google Srl");
        let priority = [BookId::local(), BookId::google(1)];
        assert_eq!(merged(&cards, &priority, &Default::default()).org, "Locale Srl");
    }

    #[test]
    fn ties_go_to_the_most_recent_edit() {
        let cards = [
            card("a", BookId::google(1), 100, |f| f.title = "Old".into()),
            card("b", BookId::google(2), 300, |f| f.title = "New".into()),
        ];
        let priority = [BookId::google(1), BookId::google(2)];
        // Same rank is only a tie when neither book is listed.
        assert_eq!(merged(&cards, &[], &Default::default()).title, "New");
        assert_eq!(merged(&cards, &priority, &Default::default()).title, "Old");
    }

    #[test]
    fn empty_values_never_win() {
        let cards = [
            card("a", BookId::google(1), 100, |f| f.name = String::new()),
            card("b", BookId::local(), 1, |f| f.name = "Ada".into()),
        ];
        assert_eq!(merged(&cards, &[BookId::google(1), BookId::local()], &Default::default()).name, "Ada");
    }

    #[test]
    fn mail_name_never_beats_an_address_book_name() {
        let cards = [
            card("m", BookId::mail(1), 999, |f| f.name = "ada lovelace via mail".into()),
            card("l", BookId::local(), 1, |f| f.name = "Ada Lovelace".into()),
        ];
        assert_eq!(merged(&cards, &[], &Default::default()).name, "Ada Lovelace");
        let only_mail = [card("m", BookId::mail(1), 1, |f| f.name = "ada".into())];
        assert_eq!(merged(&only_mail, &[], &Default::default()).name, "ada");
    }

    #[test]
    fn a_pinned_value_beats_everything() {
        let cards = [card("a", BookId::local(), 100, |f| f.org = "Locale".into())];
        let pins = BTreeMap::from([("org".to_string(), "Scelta a mano".to_string())]);
        assert_eq!(merged(&cards, &[], &pins).org, "Scelta a mano");
    }

    #[test]
    fn phones_and_emails_are_united_without_duplicates_by_normalized_value() {
        let cards = [
            card("a", BookId::local(), 1, |f| {
                f.phones = vec![item("333 1234567")];
                f.emails = vec![item("Ada@X.it")];
            }),
            card("b", BookId::google(1), 2, |f| {
                f.phones = vec![item("+39 333 1234567"), item("06 1111111")];
                f.emails = vec![item("ada@x.it"), item("ada@y.it")];
            }),
        ];
        let result = merged(&cards, &[BookId::local(), BookId::google(1)], &Default::default());
        assert_eq!(result.phones.len(), 2, "{:?}", result.phones);
        assert_eq!(result.phones[0].value, "333 1234567");
        assert_eq!(result.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(), vec!["Ada@X.it", "ada@y.it"]);
    }

    #[test]
    fn notes_keep_provenance() {
        let cards = [
            card("a", BookId::local(), 1, |f| f.notes = "Conosciuta a Roma".into()),
            card("b", BookId::google(1), 2, |f| f.notes = "Cliente VIP".into()),
        ];
        let result = merged(&cards, &[BookId::local(), BookId::google(1)], &Default::default());
        assert!(result.notes.contains("Conosciuta a Roma") && result.notes.contains("— local"), "{}", result.notes);
        assert!(result.notes.contains("Cliente VIP") && result.notes.contains("— google:1"), "{}", result.notes);
        let same = [
            card("a", BookId::local(), 1, |f| f.notes = "Uguale".into()),
            card("b", BookId::google(1), 2, |f| f.notes = "Uguale".into()),
        ];
        assert_eq!(merged(&same, &[], &Default::default()).notes, "Uguale");
    }

    fn fields(edit: impl FnOnce(&mut ContactFields)) -> ContactFields {
        let mut f = ContactFields { name: "Ada".into(), org: "Acme".into(), ..Default::default() };
        edit(&mut f);
        f
    }

    #[test]
    fn three_way_applies_a_one_sided_change() {
        let base = fields(|_| {});
        let local = fields(|f| f.org = "Beta".into());
        assert_eq!(three_way(&base, &local, &base), Merge3::Clean(local.clone()));
        let theirs = fields(|f| f.title = "CTO".into());
        match three_way(&base, &local, &theirs) {
            Merge3::Clean(result) => assert_eq!((result.org.as_str(), result.title.as_str()), ("Beta", "CTO")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn three_way_same_change_on_both_sides_is_clean() {
        let base = fields(|_| {});
        let both = fields(|f| f.name = "Ada L.".into());
        assert_eq!(three_way(&base, &both, &both), Merge3::Clean(both.clone()));
    }

    #[test]
    fn three_way_conflicts_name_the_fields() {
        let base = fields(|_| {});
        let local = fields(|f| {
            f.name = "Ada L.".into();
            f.phones = vec![item("1")];
        });
        let theirs = fields(|f| {
            f.name = "Ada Lovelace".into();
            f.phones = vec![item("1")];
        });
        assert_eq!(three_way(&base, &local, &theirs), Merge3::Conflict(vec!["name".to_string()]));
    }
}
