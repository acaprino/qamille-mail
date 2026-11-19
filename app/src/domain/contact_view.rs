//! The unified, derived address book. Nothing here is stored.
use super::contact_match::{MatchContext, certain_groups};
use super::contact_merge::{merge_fields, rank};
use super::contacts::*;
use super::model::Email;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq)]
pub struct ObservedPerson {
    pub account_id: u32,
    pub name: String,
    pub address: String,
}

/// `"Ada L. <ADA@x.it>"` becomes `("Ada L.", "ada@x.it")`; a bare address has no name.
pub fn split_person(value: &str) -> (String, String) {
    match value.rsplit_once('<') {
        Some((name, rest)) => (
            name.trim().trim_matches('"').trim().to_string(),
            rest.trim().trim_end_matches('>').trim().to_lowercase(),
        ),
        None => (String::new(), value.trim().to_lowercase()),
    }
}

/// People seen in mail per account: senders override earlier names, recipients never do.
pub fn observed_people(emails: &[Email]) -> Vec<ObservedPerson> {
    let mut people: BTreeMap<(u32, String), String> = BTreeMap::new();
    for email in emails {
        let sender = email.from_addr.trim().to_lowercase();
        if !sender.is_empty() {
            people.insert((email.account_id, sender), email.from_name.clone());
        }
        for value in email.to.iter().chain(&email.cc) {
            let (name, address) = split_person(value);
            if !address.is_empty() {
                people.entry((email.account_id, address)).or_insert(name);
            }
        }
    }
    people.into_iter().map(|((account_id, address), name)| ObservedPerson { account_id, name, address }).collect()
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Origin {
    pub key: String,
    pub book: BookId,
    pub modified: i64,
    pub sync: SyncState,
    pub writable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnifiedContact {
    /// The smallest member key; stable only while the membership stays the same.
    pub id: String,
    pub fields: ContactFields,
    pub origins: Vec<Origin>,
    pub hidden: bool,
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Joins automatic certain groups and confirmed links, then merges each group into one card.
pub fn unify(
    sources: &[SourceContact],
    links: &[Link],
    hidden: &[String],
    priority: &[BookId],
    cx: &MatchContext,
) -> Vec<UnifiedContact> {
    let index: HashMap<&str, usize> = sources.iter().enumerate().map(|(i, s)| (s.key.as_str(), i)).collect();
    let mut parent: Vec<usize> = (0..sources.len()).collect();
    let join = |parent: &mut Vec<usize>, members: Vec<usize>| {
        for pair in members.windows(2) {
            let (a, b) = (find(parent, pair[0]), find(parent, pair[1]));
            if a != b {
                parent[b] = a;
            }
        }
    };
    for group in certain_groups(sources, cx) {
        join(&mut parent, group.iter().filter_map(|k| index.get(k.as_str()).copied()).collect());
    }
    for link in links {
        join(&mut parent, link.keys.iter().filter_map(|k| index.get(k.as_str()).copied()).collect());
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..sources.len() {
        let root = find(&mut parent, i);
        components.entry(root).or_default().push(i);
    }
    let mut unified: Vec<UnifiedContact> = components
        .into_values()
        .map(|mut members| {
            members.sort_by(|&a, &b| {
                rank(&sources[a].book, priority).cmp(&rank(&sources[b].book, priority)).then(sources[a].key.cmp(&sources[b].key))
            });
            let keys: Vec<&str> = members.iter().map(|&m| sources[m].key.as_str()).collect();
            let mut pins = BTreeMap::new();
            for link in links.iter().filter(|l| l.keys.iter().any(|k| keys.contains(&k.as_str()))) {
                pins.extend(link.pins.clone());
            }
            let cards: Vec<&SourceContact> = members.iter().map(|&m| &sources[m]).collect();
            UnifiedContact {
                id: keys.iter().min().map(|k| k.to_string()).unwrap_or_default(),
                fields: merge_fields(&cards, priority, &pins),
                origins: cards
                    .iter()
                    .map(|s| Origin { key: s.key.clone(), book: s.book.clone(), modified: s.modified, sync: s.sync.clone(), writable: s.book.writable() })
                    .collect(),
                hidden: keys.iter().all(|k| hidden.iter().any(|h| h == k)),
            }
        })
        .collect();
    unified.sort_by(|a, b| a.fields.name.to_lowercase().cmp(&b.fields.name.to_lowercase()).then(a.id.cmp(&b.id)));
    unified
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(account: u32, from: (&str, &str), to: &[&str], cc: &[&str]) -> Email {
        Email {
            account_id: account,
            from_name: from.0.into(),
            from_addr: from.1.into(),
            to: to.iter().map(|s| s.to_string()).collect(),
            cc: cc.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }
    fn card(key: &str, book: BookId, name: &str, emails: &[&str]) -> SourceContact {
        SourceContact {
            key: key.into(),
            book,
            fields: ContactFields {
                name: name.into(),
                emails: emails.iter().map(|e| Labeled { label: String::new(), value: e.to_string() }).collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn cx<'a>(links: &'a [Link]) -> MatchContext<'a> {
        MatchContext { region: "IT", roles: &[], rejected: &[], links }
    }

    #[test]
    fn split_person_handles_names_and_bare_addresses() {
        assert_eq!(split_person("Ada L. <ADA@x.it>"), ("Ada L.".into(), "ada@x.it".into()));
        assert_eq!(split_person("\"Rossi, Mario\" <m@x.it>"), ("Rossi, Mario".into(), "m@x.it".into()));
        assert_eq!(split_person(" Bea@Y.it "), (String::new(), "bea@y.it".into()));
    }

    #[test]
    fn observed_people_are_per_account_and_senders_override_names() {
        let emails = [
            email(1, ("", "ada@x.it"), &["Bea <bea@y.it>"], &["carla@z.it"]),
            email(1, ("Ada Lovelace", "ADA@x.it"), &["bea@y.it"], &[]),
            email(2, ("Dan", "dan@w.it"), &["bea@y.it"], &[]),
        ];
        let people = observed_people(&emails);
        let find = |account: u32, address: &str| people.iter().find(|p| p.account_id == account && p.address == address);
        assert_eq!(find(1, "ada@x.it").unwrap().name, "Ada Lovelace");
        assert_eq!(find(1, "bea@y.it").unwrap().name, "Bea");
        assert_eq!(find(2, "bea@y.it").unwrap().name, "");
        assert!(find(1, "carla@z.it").is_some());
        assert_eq!(people.iter().filter(|p| p.address == "ada@x.it").count(), 1);
    }

    #[test]
    fn same_person_in_three_books_is_one_card_with_three_origins() {
        let sources = [
            card("g1", BookId::google(1), "Ada Lovelace", &["ada@x.it"]),
            card("l1", BookId::local(), "Ada L.", &["ADA@x.it"]),
            card("mail:2:ada@x.it", BookId::mail(2), "ada", &["ada@x.it"]),
            card("l2", BookId::local(), "Bea", &["bea@y.it"]),
        ];
        let unified = unify(&sources, &[], &[], &[BookId::google(1), BookId::local()], &cx(&[]));
        assert_eq!(unified.len(), 2);
        let ada = unified.iter().find(|c| c.fields.name == "Ada Lovelace").unwrap();
        assert_eq!(ada.id, "g1");
        assert_eq!(ada.origins.len(), 3);
        assert_eq!(ada.origins.iter().map(|o| o.book.0.as_str()).collect::<Vec<_>>(), vec!["google:1", "local", "mail:2"]);
        assert!(ada.origins.iter().any(|o| !o.writable));
    }

    #[test]
    fn a_confirmed_link_joins_cards_with_nothing_in_common() {
        let sources = [card("a", BookId::local(), "Ada", &["a@x.it"]), card("b", BookId::google(1), "Ada Lovelace", &["b@y.it"])];
        let links = [Link { keys: vec!["a".into(), "b".into()], pins: BTreeMap::from([("name".to_string(), "Ada Byron".to_string())]) }];
        let unified = unify(&sources, &links, &[], &[], &cx(&links));
        assert_eq!(unified.len(), 1);
        assert_eq!(unified[0].fields.name, "Ada Byron");
        assert_eq!(unified[0].fields.emails.len(), 2);
    }

    #[test]
    fn a_link_to_a_missing_key_is_ignored() {
        let sources = [card("a", BookId::local(), "Ada", &["a@x.it"])];
        let links = [Link { keys: vec!["a".into(), "gone".into()], pins: BTreeMap::new() }];
        assert_eq!(unify(&sources, &links, &[], &[], &cx(&links)).len(), 1);
    }

    #[test]
    fn a_card_is_hidden_only_when_every_member_is_hidden() {
        let sources = [card("g1", BookId::google(1), "Ada", &["ada@x.it"]), card("m", BookId::mail(1), "ada", &["ada@x.it"])];
        let one = unify(&sources, &[], &["m".to_string()], &[], &cx(&[]));
        assert!(!one[0].hidden);
        let all = unify(&sources, &[], &["m".to_string(), "g1".to_string()], &[], &cx(&[]));
        assert!(all[0].hidden);
    }

    #[test]
    fn cards_are_sorted_by_name() {
        let sources = [card("z", BookId::local(), "Zeta", &[]), card("a", BookId::local(), "alfa", &[]), card("m", BookId::local(), "", &["x@x.it"])];
        let names: Vec<String> = unify(&sources, &[], &[], &[], &cx(&[])).into_iter().map(|c| c.fields.name).collect();
        assert_eq!(names, vec!["".to_string(), "alfa".to_string(), "Zeta".to_string()]);
    }
}
