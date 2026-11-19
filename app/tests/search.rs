use chrono::{NaiveDate, TimeZone, Utc};
use qamille::{
    domain::search::{ParsedQuery, execute},
    model::*,
};

fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp()
}
fn fixture() -> Store {
    let mut store = Store::default();
    store.settings.time_zone = "Europe/Rome".into();
    store.settings.locale = "it-IT".into();
    store.accounts = vec![
        Account {
            id: 1,
            email: "me@example.com".into(),
            name: "Personal".into(),
            ..Default::default()
        },
        Account {
            id: 2,
            email: "work@example.com".into(),
            name: "Work".into(),
            ..Default::default()
        },
    ];
    store.emails = vec![
        Email {
            id: 1,
            account_id: 1,
            folder: "Inbox".into(),
            from_name: "Maria Rossi".into(),
            from_addr: "maria@example.com".into(),
            to: vec!["me@example.com".into()],
            subject: "Preventivo cucina".into(),
            body: "contratto firmato preventivo allegato".into(),
            date: timestamp("2026-10-15T10:00:00Z"),
            ..Default::default()
        },
        Email {
            id: 2,
            account_id: 2,
            folder: "Archive".into(),
            from_name: "Luca".into(),
            from_addr: "luca@company.it".into(),
            to: vec!["Someone <anna@example.com>".into()],
            subject: "Contratto".into(),
            date: timestamp("2026-10-09T08:00:00Z"),
            ..Default::default()
        },
        Email {
            id: 3,
            account_id: 1,
            folder: "Trash".into(),
            from_addr: "maria@example.com.evil".into(),
            subject: "Preventivo newsletter".into(),
            date: timestamp("2026-10-21T21:59:59Z"),
            ..Default::default()
        },
        Email {
            id: 4,
            account_id: 1,
            folder: "Spam".into(),
            from_addr: "bot@company.it".into(),
            subject: "Preventivo".into(),
            date: timestamp("2026-10-22T00:00:00Z"),
            ..Default::default()
        },
    ];
    store
}
fn ids(store: &Store, query: &str) -> Vec<u64> {
    execute(store, query, None, timestamp("2026-11-01T12:00:00Z"), "").unwrap()
}

#[test]
fn calendar_ranges_include_boundaries_and_use_the_configured_zone() {
    let mut store = fixture();
    assert_eq!(ids(&store, "around:2026-10-15 in:anywhere"), vec![1, 3, 2]);
    assert_eq!(ids(&store, "around:15/10/2026/0d"), vec![1]);
    assert_eq!(
        ids(&store, "between:2026-10-09..2026-10-21 in:anywhere"),
        vec![3, 1, 2]
    );
    let query = ParsedQuery::parse(&store, "on:2026-10-25", 0).unwrap();
    let (start, end) = query.range().unwrap();
    assert_eq!(end - start, 25 * 3600);
    assert_eq!(start, timestamp("2026-10-24T22:00:00Z"));
    store.emails[0].date = timestamp("2026-10-15T22:30:00Z");
    assert!(ids(&store, "on:2026-10-15").is_empty());
    store.settings.time_zone = "UTC".into();
    assert_eq!(ids(&store, "on:2026-10-15"), vec![1]);
    store.emails[0].date_missing = true;
    assert!(ids(&store, "on:2026-10-15").is_empty());
}

#[test]
fn calendar_edges_and_regional_forms_resolve_consistently() {
    let mut store = fixture();
    for (query, first, last) in [
        ("around:2026-10-31/3d", "2026-10-28", "2026-11-04"),
        ("around:2026-12-30/3d", "2026-12-27", "2027-01-03"),
        ("around:2028-02-29/1d", "2028-02-28", "2028-03-02"),
    ] {
        let parsed = ParsedQuery::parse(&store, query, 0).unwrap();
        let day = |s| {
            qamille::infrastructure::timezone::day_start(
                NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap(),
                "Europe/Rome",
            )
            .unwrap()
        };
        assert_eq!(parsed.range(), Some((day(first), day(last))));
    }
    let italian = ParsedQuery::parse(&store, "on:10/03/2026", 0)
        .unwrap()
        .range();
    store.settings.locale = "en-US".into();
    assert_ne!(
        ParsedQuery::parse(&store, "on:10/03/2026", 0)
            .unwrap()
            .range(),
        italian
    );
    assert_eq!(
        ParsedQuery::parse(&store, "on:2026/03/10", 0)
            .unwrap()
            .range(),
        italian
    );
}

#[test]
fn grammar_preserves_boolean_precedence_quotes_and_exact_constraints() {
    let store = fixture();
    assert_eq!(
        ids(
            &store,
            "(from:maria@example.com OR from:luca@company.it) -subject:newsletter"
        ),
        vec![1, 2]
    );
    assert_eq!(
        ids(
            &store,
            "from:maria@example.com OR from:luca@company.it subject:contratto"
        ),
        vec![1, 2]
    );
    assert_eq!(ids(&store, "body:\"contratto firmato\""), vec![1]);
    assert!(
        ids(&store, "\"cucina contratto\"").is_empty(),
        "phrase must not cross field boundaries"
    );
    let query = ParsedQuery::parse(
        &store,
        "from:maria@example.com \"contratto firmato\" estimate -body:newsletter",
        0,
    )
    .unwrap();
    assert_eq!(query.semantic_words(), "estimate");
    assert!(query.semantic_matches(&store.emails[0], &store));
    assert!(!query.matches(&store.emails[0], &store));
    assert!(!query.semantic_matches(&store.emails[1], &store));
    let literal = ParsedQuery::parse(&store, "\"10:30\"", 0).unwrap();
    assert_eq!(literal.words(), vec!["10:30"]);
    assert_eq!(ids(&store, "from:@company.it"), vec![2]);
    assert!(ids(&store, "to:example").is_empty());
    assert_eq!(ids(&store, "to:anna@example.com"), vec![2]);
}

#[test]
fn hidden_folders_are_enabled_only_by_a_satisfied_positive_branch() {
    let store = fixture();
    assert_eq!(ids(&store, "subject:preventivo"), vec![1]);
    assert_eq!(ids(&store, "subject:preventivo OR in:trash"), vec![3, 1]);
    assert_eq!(ids(&store, "subject:preventivo in:anywhere"), vec![4, 3, 1]);
    assert_eq!(ids(&store, "-in:inbox"), vec![2]);
    assert_eq!(ids(&store, "in:trash"), vec![3]);
    assert_eq!(ids(&store, "in:spam"), vec![4]);
    assert!(ids(&store, "in:trash from:maria@example.com").is_empty());
}

#[test]
fn account_filtering_precedes_all_limits_and_me_uses_selected_accounts() {
    let mut store = fixture();
    store.emails.clear();
    for id in 1..=65 {
        store.emails.push(Email {
            id,
            account_id: if id > 60 { 2 } else { 1 },
            folder: "Inbox".into(),
            subject: "Budget".into(),
            date: id as i64,
            ..Default::default()
        });
    }
    assert_eq!(ids(&store, "budget").len(), 65);
    assert_eq!(execute(&store, "budget", Some(2), 0, "").unwrap().len(), 5);
    store.emails[0].from_addr = "work@example.com".into();
    assert!(
        execute(&store, "from:me", Some(1), 0, "")
            .unwrap()
            .is_empty()
    );
    assert_eq!(ids(&store, "from:me"), vec![1]);
    assert!(ids(&Store::default(), "from:me").is_empty());
}

#[test]
fn invalid_filters_never_silently_expand_the_search_or_panic() {
    let store = fixture();
    assert!(
        ParsedQuery::parse(&store, "around:2026-10-15/2w", 0)
            .unwrap_err()
            .contains("/14d")
    );
    assert!(
        ParsedQuery::parse(&store, "around:2026-10-15/6", 0)
            .unwrap_err()
            .contains("/6d")
    );
    for query in [
        "around:31/02/2026",
        "from:",
        "\"open phrase",
        "around:2026-10-15/400d",
        "around:2026-10-15/2w",
        "around:2026-10-15/6",
        "around:2026-10-15/-1d",
        "between:2026-10-31..2026-10-01",
        "is:foo",
        "has:foo",
        "category:foo",
        "newer_than:7è",
        "newer_than:7x",
        "newer_than:9999999999999999999y",
        "on:15/10/26",
        "in:missing",
        "account:missing",
        "10:30",
        "from:maria AND",
        "()",
        "(budget",
        "budget)",
        "on:2026-10-15 on:2026-10-16",
    ] {
        assert!(
            ParsedQuery::parse(&store, query, 0).is_err(),
            "query should fail: {query}"
        );
    }
}

#[test]
fn unicode_normalization_and_real_attachment_content_work_without_ai() {
    let mut store = fixture();
    store.emails[0].subject = "caffé".into();
    assert_eq!(ids(&store, "caffe\u{301}"), vec![1]);
    let path = std::env::temp_dir().join(format!("qamille-search-{}.txt", std::process::id()));
    std::fs::write(&path, "numero ordine ORD-4821").unwrap();
    store.emails[0].attachments.push(Attachment {
        id: 1,
        name: "contratto.txt".into(),
        path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    });
    assert_eq!(
        qamille::infrastructure::search_support::execute(&store, "\"ORD-4821\"", None, 0, "")
            .unwrap(),
        vec![1]
    );
    assert_eq!(ids(&store, "filename:contratto type:txt"), vec![1]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sorting_is_stable_and_accepts_ui_names() {
    let store = fixture();
    assert_eq!(
        execute(&store, "around:2026-10-15 in:anywhere", None, 0, "closest").unwrap()[0],
        1
    );
    assert_eq!(
        execute(&store, "around:2026-10-15 in:anywhere", None, 0, "date").unwrap()[0],
        3
    );
    assert_eq!(Utc.timestamp_opt(0, 0).single().unwrap().timestamp(), 0);
}

#[test]
fn missing_content_cannot_satisfy_negative_text_queries() {
    let mut store = fixture();
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    assert!(ids(&store, "from:maria@example.com -body:missing").is_empty());
    assert!(ids(&store, "from:maria@example.com -missing").is_empty());
    assert_eq!(
        ids(&store, "from:maria@example.com subject:preventivo"),
        vec![1]
    );
    let query = ParsedQuery::parse(&store, "-body:missing", 0).unwrap();
    assert!(query.needs_content());
    assert!(!query.needs_attachment_content());
    assert!(
        ParsedQuery::parse(&store, "-missing", 0)
            .unwrap()
            .needs_attachment_content()
    );
}

#[test]
fn headers_of_mail_without_a_downloaded_body_still_match_plain_words() {
    let mut store = fixture();
    store.emails[0].remote_only = true;
    store.emails[0].body.clear();
    // Sender name, address and subject are known without the body.
    assert_eq!(ids(&store, "rossi"), vec![1]);
    assert_eq!(ids(&store, "Maria"), vec![1]);
    assert_eq!(ids(&store, "cucina"), vec![1]);
    // The missing body can neither prove presence nor absence.
    assert!(ids(&store, "firmato").is_empty());
    assert!(ids(&store, "rossi -firmato").is_empty());
    assert!(!ids(&store, "-rossi").contains(&1));
    assert!(!ids(&store, "-firmato").contains(&1));
    assert!(ids(&store, "body:cucina").is_empty());
}

#[test]
fn mcp_surfaces_validation_errors_and_exposes_more_than_twenty_matches() {
    let mut store = fixture();
    assert!(
        qamille::mcp::call(
            &mut store,
            "search_emails",
            &serde_json::json!({"query":"newer_than:7è"})
        )
        .is_err()
    );
    assert!(
        qamille::mcp::call(
            &mut store,
            "semantic_search",
            &serde_json::json!({"query":"around:2026-10-15/400d"})
        )
        .is_err()
    );
    store.emails.clear();
    for id in 1..=30 {
        store.emails.push(Email {
            id,
            account_id: 1,
            folder: "Inbox".into(),
            subject: "Budget".into(),
            ..Default::default()
        });
    }
    let (page, changed) = qamille::mcp::call(
        &mut store,
        "search_emails",
        &serde_json::json!({"query":"budget","page":2}),
    )
    .unwrap();
    assert!(!changed);
    assert!(page.contains("30 local results"));
    assert_eq!(page.matches("\nAccount:").count(), 10);
    assert!(page.contains("coverage is not verified"));
}

#[test]
fn unreadable_attachment_cannot_establish_absence_of_a_free_word() {
    let mut store = fixture();
    store.emails[0].attachments.push(Attachment {
        id: 99,
        name: "scan.png".into(),
        mime: "image/png".into(),
        ..Default::default()
    });
    assert!(ids(&store, "from:maria@example.com -secret").is_empty());
    assert_eq!(ids(&store, "from:maria@example.com -body:secret"), vec![1]);
    assert_eq!(ids(&store, "from:maria@example.com preventivo"), vec![1]);
    assert_eq!(
        ids(&store, "from:maria@example.com has:attachment"),
        vec![1]
    );
}

#[test]
#[ignore = "manual timing on a synthetic 50k-message dataset, no external services"]
fn local_fifty_thousand_messages_timing() {
    let mut store = fixture();
    let mut base = store.emails[0].clone();
    base.subject = "Preventivo cucina ORD-4821".into();
    base.body =
        "Il preventivo aggiornato per la cucina contiene dettagli, condizioni e conferma ordine."
            .repeat(12);
    store.emails = (1..=50_000)
        .map(|id| {
            let mut email = base.clone();
            email.id = id;
            email.date += id as i64 * 60;
            email
        })
        .collect();
    for query in [
        "from:maria@example.com",
        "preventivo",
        "conferma",
        "around:2026-10-15/30d preventivo",
    ] {
        let mut samples = Vec::new();
        for _ in 0..10 {
            let started = std::time::Instant::now();
            let hits = execute(&store, query, None, 0, "").unwrap();
            assert!(!hits.is_empty());
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "engine query={query:?}; messages=50000; debug build; n=10; p95={:.1}ms; median={:.1}ms",
            samples[9], samples[4]
        );
    }
}
