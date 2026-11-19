#[path = "support/contacts.rs"]
mod support;

use qamille::{application::contact_ports::ContactStore, domain::contacts::*};
use support::*;

const TWO_CARDS: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada Lovelace\r\nEMAIL;TYPE=work:ada@x.it\r\nTEL;TYPE=cell:+39 333 1234567\r\nX-CUSTOM:1\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Bea Verdi\r\nEMAIL:bea@y.it\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nEND:VCARD\r\n";

#[test]
fn import_adds_to_local_and_reports_counts() {
    let fx = fixture(base_store());
    let report = fx.service.import_vcf(TWO_CARDS).unwrap();
    assert_eq!((report.added, report.skipped), (2, 1), "the empty card is skipped");
    let st = fx.contacts.read().unwrap();
    assert!(st.sources.iter().all(|s| s.book == BookId::local()));
    assert_eq!(st.sources.len(), 2);
    assert!(st.queue.is_empty(), "the local book has no remote writes");
    assert_eq!(all(&fx).len(), 2);
}

#[test]
fn importing_the_same_file_twice_adds_nothing() {
    let fx = fixture(base_store());
    fx.service.import_vcf(TWO_CARDS).unwrap();
    let again = fx.service.import_vcf(TWO_CARDS).unwrap();
    assert_eq!((again.added, again.skipped), (0, 3));
    assert_eq!(fx.contacts.read().unwrap().sources.len(), 2);
}

#[test]
fn export_then_import_round_trips_fields_and_unknown_properties() {
    let fx = fixture(base_store());
    fx.service.import_vcf(TWO_CARDS).unwrap();
    let exported = fx.service.export_vcf("local").unwrap();
    assert_eq!(exported.matches("BEGIN:VCARD").count(), 2);
    assert!(exported.contains("X-CUSTOM:1"), "{exported}");
    let other = fixture(base_store());
    other.service.import_vcf(&exported).unwrap();
    let mut a: Vec<ContactFields> = all(&fx).into_iter().map(|c| c.fields).collect();
    let mut b: Vec<ContactFields> = all(&other).into_iter().map(|c| c.fields).collect();
    a.sort_by(|x, y| x.name.cmp(&y.name));
    b.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(a, b);
}

#[test]
fn import_makes_duplicates_visible_without_prompting() {
    let fx = fixture(base_store());
    seed(&fx, vec![source("g1", BookId::google(1), Some("people/1"), fields("Ada L.", &["ada@x.it"]), 1)]);
    let report = fx.service.import_vcf(TWO_CARDS).unwrap();
    assert_eq!(report.added, 2);
    let groups = fx.service.duplicates(false).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].level, Level::Certain);
}

#[test]
fn export_rejects_read_only_and_unknown_books() {
    let fx = fixture(base_store());
    assert!(fx.service.export_vcf("mail:1").is_err());
    assert!(fx.service.export_vcf("nope").is_err());
    assert!(fx.service.export_vcf("local").unwrap().is_empty(), "an empty book exports nothing");
}
