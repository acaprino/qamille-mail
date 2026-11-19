//! Duplicate detection across address books. Pure and deterministic.
use super::contact_norm::*;
use super::contacts::*;
use std::collections::{HashMap, HashSet};

/// Calibrated on a synthetic corpus (`tests/contacts_corpus.rs`): 0.88-0.94 change recall (0.87 to 0.75), not precision.
pub const POSSIBLE_THRESHOLD: f64 = 0.92;
/// How many following names each name is compared with in a sorted ordering.
const NEIGHBORHOOD: usize = 8;

/// Domains shared by unrelated people; they never count as a clue.
const GENERIC_DOMAINS: [&str; 12] = [
    "gmail.com", "outlook.com", "hotmail.com", "yahoo.com", "icloud.com", "libero.it", "live.com",
    "tiscali.it", "virgilio.it", "alice.it", "me.com", "pec.it",
];

pub struct MatchContext<'a> {
    pub region: &'a str,
    pub roles: &'a [String],
    pub rejected: &'a [(String, String)],
    pub links: &'a [Link],
}

pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let window = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut matched_b = vec![false; b.len()];
    let mut matched_a = Vec::new();
    for (i, ca) in a.iter().enumerate() {
        let from = i.saturating_sub(window);
        let to = (i + window + 1).min(b.len());
        if let Some(j) = (from..to).find(|&j| !matched_b[j] && b[j] == *ca) {
            matched_b[j] = true;
            matched_a.push(*ca);
        }
    }
    if matched_a.is_empty() {
        return 0.0;
    }
    let in_order_b: Vec<char> = b.iter().zip(&matched_b).filter(|(_, m)| **m).map(|(c, _)| *c).collect();
    let transpositions = matched_a.iter().zip(&in_order_b).filter(|(x, y)| x != y).count() / 2;
    let m = matched_a.len() as f64;
    let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64) / m) / 3.0;
    let prefix = a.iter().zip(&b).take(4).take_while(|(x, y)| x == y).count() as f64;
    jaro + prefix * 0.1 * (1.0 - jaro)
}

fn eligible(source: &SourceContact) -> bool {
    !matches!(source.book.kind(), BookKind::Mail | BookKind::GoogleOther)
}

/// Keys that certify two cards as the same person. Empty values never produce a key.
fn certain_keys(source: &SourceContact, cx: &MatchContext) -> Vec<String> {
    let mut keys = Vec::new();
    for email in &source.fields.emails {
        let email = normalize_email(&email.value);
        if !email.is_empty() && !is_role_address(&email, cx.roles) {
            keys.push(format!("e:{email}"));
        }
    }
    for phone in &source.fields.phones {
        if cx.region.is_empty() {
            if let Some(tail) = phone_tail(&phone.value) {
                keys.push(format!("t:{tail}"));
            }
        } else if let Some(e164) = normalize_phone(&phone.value, cx.region) {
            keys.push(format!("p:{e164}"));
        } else if let Some(tail) = phone_tail(&phone.value) {
            keys.push(format!("t:{tail}"));
        }
    }
    if let Some(uid) = source.uid.as_deref().filter(|u| !u.trim().is_empty()) {
        keys.push(format!("u:{}", uid.trim()));
    }
    keys
}

struct Components {
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
    forbid: Vec<HashSet<usize>>,
}
impl Components {
    fn new(len: usize) -> Self {
        Self { parent: (0..len).collect(), members: (0..len).map(|i| vec![i]).collect(), forbid: vec![HashSet::new(); len] }
    }
    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
    /// Joins two cards unless a rejected pair would end up in one component.
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (small, big) = if self.members[ra].len() <= self.members[rb].len() { (ra, rb) } else { (rb, ra) };
        let blocked: Vec<usize> = self.members[small].iter().flat_map(|m| self.forbid[*m].iter().copied()).collect();
        if blocked.into_iter().any(|f| self.find(f) == big) {
            return;
        }
        let moved = std::mem::take(&mut self.members[small]);
        self.members[big].extend(moved);
        self.parent[small] = big;
    }
}

/// Connected components of cards sharing a certain key. Mail and "other contacts" cards join too.
pub fn certain_groups(sources: &[SourceContact], cx: &MatchContext) -> Vec<Vec<String>> {
    let index: HashMap<&str, usize> = sources.iter().enumerate().map(|(i, s)| (s.key.as_str(), i)).collect();
    let mut components = Components::new(sources.len());
    for (a, b) in cx.rejected {
        if let (Some(&i), Some(&j)) = (index.get(a.as_str()), index.get(b.as_str())) {
            components.forbid[i].insert(j);
            components.forbid[j].insert(i);
        }
    }
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, source) in sources.iter().enumerate() {
        for key in certain_keys(source, cx) {
            buckets.entry(key).or_default().push(i);
        }
    }
    let mut ordered: Vec<(&String, &Vec<usize>)> = buckets.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (_, members) in ordered {
        for pair in members.windows(2) {
            components.union(pair[0], pair[1]);
        }
    }
    let roots: Vec<usize> = (0..sources.len()).filter(|&i| components.find(i) == i && components.members[i].len() > 1).collect();
    let mut groups: Vec<Vec<String>> = roots
        .into_iter()
        .map(|i| {
            let mut keys: Vec<String> = components.members[i].iter().map(|&m| sources[m].key.clone()).collect();
            keys.sort();
            keys
        })
        .collect();
    groups.sort();
    groups
}

fn org_key(source: &SourceContact) -> String {
    normalize_name(&source.fields.org)
}
fn domains(source: &SourceContact) -> HashSet<String> {
    source
        .fields
        .emails
        .iter()
        .filter_map(|e| normalize_email(&e.value).split_once('@').map(|(_, d)| d.to_string()))
        .filter(|d| !d.is_empty() && !GENERIC_DOMAINS.contains(&d.as_str()))
        .collect()
}
/// Two different, non-empty companies contradict a name match.
fn conflicting_company(a: &SourceContact, b: &SourceContact) -> bool {
    let (oa, ob) = (org_key(a), org_key(b));
    !oa.is_empty() && !ob.is_empty() && oa != ob
}
fn has_clue(a: &SourceContact, b: &SourceContact) -> bool {
    let (oa, ob) = (org_key(a), org_key(b));
    (!oa.is_empty() && oa == ob)
        || !domains(a).is_disjoint(&domains(b))
        || (!a.fields.birthday.trim().is_empty() && a.fields.birthday.trim() == b.fields.birthday.trim())
}

fn same_link(links: &[Link], a: &str, b: &str) -> bool {
    links.iter().any(|l| l.keys.iter().any(|k| k == a) && l.keys.iter().any(|k| k == b))
}

fn pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b { (a.into(), b.into()) } else { (b.into(), a.into()) }
}

pub fn find_duplicates(sources: &[SourceContact], cx: &MatchContext) -> Vec<DuplicateGroup> {
    find_duplicates_with(sources, cx, POSSIBLE_THRESHOLD)
}

/// Same as `find_duplicates` with an explicit similarity threshold (used to calibrate it).
pub fn find_duplicates_with(sources: &[SourceContact], cx: &MatchContext, threshold: f64) -> Vec<DuplicateGroup> {
    let by_key: HashMap<&str, &SourceContact> = sources.iter().map(|s| (s.key.as_str(), s)).collect();
    let mut found = Vec::new();
    let mut in_certain: HashSet<(String, String)> = HashSet::new();
    for group in certain_groups(sources, cx) {
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                in_certain.insert(pair_key(a, b));
            }
        }
        let keys: Vec<String> = group.into_iter().filter(|k| by_key.get(k.as_str()).is_some_and(|s| eligible(s))).collect();
        let covered = cx.links.iter().any(|l| keys.iter().all(|k| l.keys.contains(k)));
        if keys.len() >= 2 && !covered {
            found.push(DuplicateGroup { keys, level: Level::Certain });
        }
    }
    let rejected: HashSet<(String, String)> = cx.rejected.iter().map(|(a, b)| pair_key(a, b)).collect();
    let candidates: Vec<&SourceContact> = sources.iter().filter(|s| eligible(s)).collect();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut consider = |a: &SourceContact, b: &SourceContact, level: Level, found: &mut Vec<DuplicateGroup>| {
        let pair = pair_key(&a.key, &b.key);
        if pair.0 == pair.1 || in_certain.contains(&pair) || rejected.contains(&pair) || same_link(cx.links, &a.key, &b.key) || !seen.insert(pair.clone()) {
            return;
        }
        found.push(DuplicateGroup { keys: vec![pair.0, pair.1], level });
    };
    let names: Vec<String> = candidates.iter().map(|s| normalize_name(&s.fields.name)).collect();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, name) in names.iter().enumerate() {
        if !name.is_empty() {
            by_name.entry(name.as_str()).or_default().push(i);
        }
    }
    for members in by_name.values() {
        for (n, &i) in members.iter().enumerate() {
            for &j in &members[n + 1..] {
                if has_clue(candidates[i], candidates[j]) {
                    consider(candidates[i], candidates[j], Level::Probable, &mut found);
                }
            }
        }
    }
    for members in by_name.values() {
        for (n, &i) in members.iter().enumerate() {
            for &j in &members[n + 1..] {
                if !conflicting_company(candidates[i], candidates[j]) {
                    consider(candidates[i], candidates[j], Level::Possible, &mut found);
                }
            }
        }
    }
    // Sorted-neighborhood comparison: a fixed window over two orderings (forward and
    // reversed) keeps the work linear and still pairs names that differ at one end.
    let multi: Vec<usize> = (0..names.len()).filter(|&i| names[i].split_whitespace().count() >= 2).collect();
    let reversed: Vec<String> = names.iter().map(|n| n.chars().rev().collect()).collect();
    for ordering in [&names, &reversed] {
        let mut sorted = multi.clone();
        sorted.sort_by(|&a, &b| ordering[a].cmp(&ordering[b]));
        for (p, &i) in sorted.iter().enumerate() {
            for &j in sorted.iter().skip(p + 1).take(NEIGHBORHOOD) {
                if names[i] != names[j] && !conflicting_company(candidates[i], candidates[j]) && jaro_winkler(&names[i], &names[j]) >= threshold {
                    consider(candidates[i], candidates[j], Level::Possible, &mut found);
                }
            }
        }
    }
    found.sort_by(|a, b| (a.level, &a.keys).cmp(&(b.level, &b.keys)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn src(key: &str, book: BookId, name: &str, emails: &[&str], phones: &[&str], org: &str) -> SourceContact {
        let labeled = |values: &[&str]| values.iter().map(|v| Labeled { label: String::new(), value: v.to_string() }).collect();
        SourceContact {
            key: key.into(),
            book,
            fields: ContactFields {
                name: name.into(),
                org: org.into(),
                emails: labeled(emails),
                phones: labeled(phones),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn cx<'a>(rejected: &'a [(String, String)], links: &'a [Link], roles: &'a [String]) -> MatchContext<'a> {
        MatchContext { region: "IT", roles, rejected, links }
    }
    fn group(keys: &[&str], level: Level) -> DuplicateGroup {
        DuplicateGroup { keys: keys.iter().map(|k| k.to_string()).collect(), level }
    }

    #[test]
    fn same_email_in_two_books_is_one_certain_group() {
        let sources = [
            src("g1", BookId::google(1), "Ada Lovelace", &["ada@x.it"], &[], ""),
            src("l1", BookId::local(), "A. Lovelace", &["ADA@X.IT"], &[], ""),
        ];
        assert_eq!(find_duplicates(&sources, &cx(&[], &[], &[])), vec![group(&["g1", "l1"], Level::Certain)]);
    }

    #[test]
    fn same_phone_with_and_without_prefix_is_certain_in_italy() {
        let sources = [
            src("a", BookId::local(), "Mario", &[], &["333 1234567"], ""),
            src("b", BookId::google(1), "Mario R.", &[], &["+39 333 1234567"], ""),
        ];
        assert_eq!(certain_groups(&sources, &cx(&[], &[], &[])), vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn without_a_region_phones_match_on_their_last_digits() {
        let sources = [
            src("a", BookId::local(), "Mario", &[], &["333 1234567"], ""),
            src("b", BookId::google(1), "Mario R.", &[], &["+39 333 1234567"], ""),
        ];
        let context = MatchContext { region: "", roles: &[], rejected: &[], links: &[] };
        assert_eq!(certain_groups(&sources, &context).len(), 1);
    }

    #[test]
    fn three_books_with_chained_email_and_phone_form_one_certain_group() {
        let sources = [
            src("a", BookId::local(), "A", &["x@x.it"], &[], ""),
            src("b", BookId::google(1), "B", &["x@x.it"], &["+39 333 1111111"], ""),
            src("c", BookId::google(2), "C", &[], &["333 1111111"], ""),
        ];
        let groups = certain_groups(&sources, &cx(&[], &[], &[]));
        assert_eq!(groups, vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]]);
        assert_eq!(find_duplicates(&sources, &cx(&[], &[], &[])), vec![group(&["a", "b", "c"], Level::Certain)]);
    }

    #[test]
    fn role_address_never_groups() {
        let sources = [
            src("a", BookId::local(), "Ufficio A", &["info@azienda.it"], &[], ""),
            src("b", BookId::google(1), "Ufficio B", &["info@azienda.it"], &[], ""),
        ];
        assert!(certain_groups(&sources, &cx(&[], &[], &[])).is_empty());
        let extra = ["ciao".to_string()];
        let sources = [
            src("a", BookId::local(), "X", &["ciao@azienda.it"], &[], ""),
            src("b", BookId::google(1), "Y", &["ciao@azienda.it"], &[], ""),
        ];
        assert!(certain_groups(&sources, &cx(&[], &[], &extra)).is_empty());
    }

    #[test]
    fn rejected_pair_is_never_grouped_or_signaled() {
        let sources = [
            src("a", BookId::local(), "Ada", &["x@x.it"], &[], ""),
            src("b", BookId::google(1), "Bea", &["x@x.it"], &[], ""),
        ];
        let rejected = [("a".to_string(), "b".to_string())];
        assert!(certain_groups(&sources, &cx(&rejected, &[], &[])).is_empty());
        assert!(find_duplicates(&sources, &cx(&rejected, &[], &[])).is_empty());
    }

    #[test]
    fn a_rejection_blocks_the_chain_that_would_join_the_pair() {
        let sources = [
            src("a", BookId::local(), "A", &["x@x.it"], &[], ""),
            src("b", BookId::google(1), "B", &["x@x.it", "y@y.it"], &[], ""),
            src("c", BookId::google(2), "C", &["y@y.it"], &[], ""),
        ];
        let rejected = [("a".to_string(), "c".to_string())];
        let groups = certain_groups(&sources, &cx(&rejected, &[], &[]));
        assert!(groups.iter().all(|g| !(g.contains(&"a".to_string()) && g.contains(&"c".to_string()))), "{groups:?}");
    }

    #[test]
    fn sources_without_email_phone_or_name_never_group() {
        let mut one = src("a", BookId::local(), "", &[], &[], "");
        let mut two = src("b", BookId::google(1), "", &[], &[], "");
        one.fields.notes = "stessa nota".into();
        two.fields.notes = "stessa nota".into();
        let sources = [one, two, src("c", BookId::local(), "", &[], &["12"], ""), src("d", BookId::local(), "", &[], &["12"], "")];
        assert!(certain_groups(&sources, &cx(&[], &[], &[])).is_empty());
        assert!(find_duplicates(&sources, &cx(&[], &[], &[])).is_empty());
    }

    #[test]
    fn mail_only_overlap_is_absorbed_but_not_signaled() {
        let sources = [
            src("g", BookId::google(1), "Ada", &["ada@x.it"], &[], ""),
            src("m", BookId::mail(1), "ada", &["ada@x.it"], &[], ""),
        ];
        assert_eq!(certain_groups(&sources, &cx(&[], &[], &[])).len(), 1);
        assert!(find_duplicates(&sources, &cx(&[], &[], &[])).is_empty());
        let other = [
            src("g", BookId::google(1), "Ada", &["ada@x.it"], &[], ""),
            src("o", BookId::google_other(1), "Ada", &["ada@x.it"], &[], ""),
        ];
        assert!(find_duplicates(&other, &cx(&[], &[], &[])).is_empty());
    }

    #[test]
    fn confirmed_link_silences_the_signal() {
        let sources = [
            src("a", BookId::local(), "Ada", &["x@x.it"], &[], ""),
            src("b", BookId::google(1), "Ada", &["x@x.it"], &[], ""),
        ];
        let links = [Link { keys: vec!["a".into(), "b".into()], pins: Default::default() }];
        assert!(find_duplicates(&sources, &cx(&[], &links, &[])).is_empty());
    }

    #[test]
    fn same_name_is_probable_only_with_a_second_clue() {
        let plain = [
            src("a", BookId::local(), "Mario Rossi", &["m1@gmail.com"], &[], ""),
            src("b", BookId::google(1), "Rossi Mario", &["m2@gmail.com"], &[], ""),
        ];
        assert_eq!(find_duplicates(&plain, &cx(&[], &[], &[])), vec![group(&["a", "b"], Level::Possible)]);
        let same_org = [
            src("a", BookId::local(), "Mario Rossi", &[], &[], "Acme"),
            src("b", BookId::google(1), "Mario Rossi", &[], &[], "acme"),
        ];
        assert_eq!(find_duplicates(&same_org, &cx(&[], &[], &[])), vec![group(&["a", "b"], Level::Probable)]);
        let same_domain = [
            src("a", BookId::local(), "Mario Rossi", &["m@acme.it"], &[], ""),
            src("b", BookId::google(1), "Mario Rossi", &["mr@acme.it"], &[], ""),
        ];
        assert_eq!(find_duplicates(&same_domain, &cx(&[], &[], &[])), vec![group(&["a", "b"], Level::Probable)]);
    }

    #[test]
    fn same_name_in_two_different_companies_is_conflicting_evidence_and_not_proposed() {
        let sources = [
            src("a", BookId::local(), "Mario Rossi", &["m@acme.test"], &[], "Acme"),
            src("b", BookId::google(1), "Mario Rossi", &["m@beta.test"], &[], "Beta Srl"),
        ];
        assert!(find_duplicates(&sources, &cx(&[], &[], &[])).is_empty());
        let one_side_blank = [
            src("a", BookId::local(), "Mario Rossi", &["m@gmail.com"], &[], "Acme"),
            src("b", BookId::google(1), "Mario Rossi", &["mr@libero.it"], &[], ""),
        ];
        assert_eq!(find_duplicates(&one_side_blank, &cx(&[], &[], &[])), vec![group(&["a", "b"], Level::Possible)]);
        let similar = [
            src("a", BookId::local(), "Mario Rossi", &[], &[], "Acme"),
            src("b", BookId::google(1), "Mario Rosi", &[], &[], "Beta Srl"),
        ];
        assert!(find_duplicates(&similar, &cx(&[], &[], &[])).is_empty());
    }

    #[test]
    fn similar_names_are_possible_and_different_names_are_not() {
        let sources = [
            src("a", BookId::local(), "Mario Rossi", &[], &[], ""),
            src("b", BookId::google(1), "Mario Rosi", &[], &[], ""),
            src("c", BookId::google(1), "Luca Bianchi", &[], &[], ""),
        ];
        assert_eq!(find_duplicates(&sources, &cx(&[], &[], &[])), vec![group(&["a", "b"], Level::Possible)]);
    }

    #[test]
    fn jaro_winkler_is_one_for_equal_and_zero_for_disjoint_strings() {
        assert!((jaro_winkler("martha", "martha") - 1.0).abs() < 1e-9);
        assert_eq!(jaro_winkler("abc", "xyz"), 0.0);
        assert!((jaro_winkler("martha", "marhta") - 0.9611).abs() < 1e-3);
        assert_eq!(jaro_winkler("", "abc"), 0.0);
    }

    #[test]
    fn groups_come_sorted_by_level_then_first_key() {
        let sources = [
            src("p1", BookId::local(), "Anna Neri", &[], &[], "Acme"),
            src("p2", BookId::google(1), "Anna Neri", &[], &[], "Acme"),
            src("c1", BookId::local(), "Zed", &["z@z.it"], &[], ""),
            src("c2", BookId::google(1), "Zed", &["z@z.it"], &[], ""),
        ];
        let levels: Vec<Level> = find_duplicates(&sources, &cx(&[], &[], &[])).into_iter().map(|g| g.level).collect();
        assert_eq!(levels, vec![Level::Certain, Level::Probable]);
    }

    #[test]
    fn ten_thousand_sources_are_grouped_in_linear_time() {
        let first = ["Mario", "Luca", "Anna", "Paolo", "Giulia", "Marco", "Sara", "Elena", "Pietro", "Chiara"];
        let last: Vec<String> = (0..1000).map(|i| format!("Cognome{i:04}")).collect();
        let mut sources = Vec::new();
        for (i, surname) in last.iter().enumerate() {
            for (j, name) in first.iter().enumerate() {
                let n = i * 10 + j;
                sources.push(src(&format!("k{n:05}"), BookId::local(), &format!("{name} {surname}"), &[&format!("u{n}@example.test")], &[], ""));
            }
        }
        for n in 0..50 {
            let mut copy = sources[n * 7].clone();
            copy.key = format!("dup{n:02}");
            copy.book = BookId::google(1);
            sources.push(copy);
        }
        let started = Instant::now();
        let found = find_duplicates(&sources, &cx(&[], &[], &[]));
        let certain = found.iter().filter(|g| g.level == Level::Certain).count();
        assert_eq!(certain, 50);
        // Debug build: ~2.6 s measured. The quadratic version took 90 s.
        assert!(started.elapsed().as_secs_f64() < 10.0, "took {:?}", started.elapsed());
    }
}
