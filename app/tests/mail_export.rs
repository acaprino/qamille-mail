use qamille::{
    domain::mail_export,
    model::{Attachment, Email, Store},
};

fn email(id: u64, message: &str, thread: &str) -> Email {
    Email {
        id,
        account_id: 1,
        message_id: message.into(),
        thread_id: thread.into(),
        from_name: "Giulia Caffè".into(),
        from_addr: "giulia@example.com".into(),
        to: vec!["alfio@example.com".into()],
        cc: vec!["team@example.com".into()],
        subject: "Caffè: revisione ✓".into(),
        body: "Ciao!\nFrom a colleague\n>From quoted\n>>From nested\nFine ✓".into(),
        date: 1_700_000_000 + id as i64,
        ..Default::default()
    }
}

#[test]
fn eml_round_trips_unicode_headers_body_and_binary_attachment() {
    use mail_parser::MimeHeaders;
    let mut email = email(1, "original@example.com", "");
    email.attachments.push(Attachment {
        name: "report.pdf".into(),
        mime: "application/pdf".into(),
        ..Default::default()
    });
    let binary = vec![0, 255, 1, 128, b'\r', b'\n'];
    let raw = mail_export::eml(&email, vec![binary.clone()]).unwrap();
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    assert_eq!(parsed.subject(), Some(email.subject.as_str()));
    assert_eq!(
        parsed.from().unwrap().first().unwrap().name(),
        Some("Giulia Caffè")
    );
    assert_eq!(
        parsed.cc().unwrap().first().unwrap().address(),
        Some("team@example.com")
    );
    assert_eq!(parsed.message_id(), Some("original@example.com"));
    assert_eq!(parsed.date().unwrap().to_timestamp(), email.date);
    assert_eq!(
        parsed.body_text(0).unwrap().replace("\r\n", "\n"),
        email.body
    );
    let file = parsed.attachments().next().unwrap();
    assert_eq!(file.attachment_name(), Some("report.pdf"));
    assert_eq!(file.contents(), binary);
}

#[test]
fn mbox_escapes_nested_from_lines_and_keeps_non_utf8_bytes() {
    let email = email(1, "", "");
    let raw = b"Subject: raw\r\nX-Test: From header\r\n\r\nFrom line\r\n>From quote\r\n>>From quote\r\n\xff\r\n";
    let mut out = Vec::new();
    mail_export::append_mbox(&mut out, &email, raw);
    mail_export::append_mbox(&mut out, &email, raw);
    assert_eq!(
        out.split(|b| *b == b'\n')
            .filter(|l| l.starts_with(b"From "))
            .count(),
        2
    );
    let escaped = b">From line\n>>From quote\n>>>From quote\n\xff\n\n";
    assert!(out.windows(escaped.len()).any(|s| s == escaped));
    assert!(out.starts_with(b"From giulia@example.com Tue Nov 14 22:13:21 2023\nSubject:"));
    assert!(!out.contains(&b'\r'));
}

#[test]
fn conversation_follows_parent_links_across_folders_without_merging_accounts_or_empty_ids() {
    let root = email(1, "root", "root");
    let mut reply = email(2, "reply", "<root>");
    reply.folder = "Sent".into();
    let mut last = email(3, "last", "reply");
    last.folder = "Archive".into();
    let mut other_account = email(4, "root", "root");
    other_account.account_id = 2;
    let mut store = Store::default();
    store.emails = vec![last.clone(), reply, email(5, "", ""), other_account, root];
    assert_eq!(
        mail_export::thread(&store, &last)
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(mail_export::thread(&store, &store.emails[2]).len(), 1);
}

#[test]
fn export_filename_cannot_escape_the_destination_or_use_windows_reserved_names() {
    let filename = mail_export::filename("../../CON:\r\nreport?*. ", 42, "eml");
    assert!(
        !filename
            .chars()
            .any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c))
    );
    assert!(filename.starts_with("email-42-"));
    assert!(filename.ends_with(".eml"));
}
