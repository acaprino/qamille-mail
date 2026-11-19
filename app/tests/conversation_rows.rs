//! Conversation rows in the mail list (ADR-0010): the direct parent of each message,
//! the other messages of a conversation in tree order, and the inbox headline counts.
use qamille::{
    application::{
        AppResult, ServiceContext,
        mail::{ConversationReply, EmailSummary, MailPage, MailQuery, MailService},
        ports::*,
    },
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::Arc;

// ---- Test doubles: only what listing needs ----

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct NoSecrets;
impl SecretStore for NoSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct NoAttachments;
impl AttachmentGateway for NoAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
struct NoMail;
impl MailGateway for NoMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}

fn service(emails: Vec<Email>) -> MailService {
    let mut store = sample_store();
    store.emails = emails;
    MailService::new(Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(NoMail),
        secrets: Arc::new(NoSecrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(NoAttachments),
    }))
}

/// A message as the provider stores it: a received reply carries its parent in
/// `thread_id` too (the In-Reply-To header), a root carries its own Message-ID.
fn mail(id: u64, folder: &str, message_id: &str, in_reply_to: &str, date: i64) -> Email {
    let thread = if in_reply_to.is_empty() { message_id } else { in_reply_to };
    Email {
        id,
        account_id: 1,
        folder: folder.into(),
        message_id: message_id.into(),
        in_reply_to: in_reply_to.into(),
        thread_id: thread.into(),
        subject: "Topic".into(),
        from_name: format!("Sender {id}"),
        from_addr: format!("sender{id}@x"),
        date,
        read: true,
        ..Default::default()
    }
}
/// A reply Qamille sent: its thread_id is the root's (the parent's own thread_id), one
/// level above its direct parent.
fn sent(id: u64, message_id: &str, in_reply_to: &str, date: i64) -> Email {
    Email { thread_id: "root@x".into(), ..mail(id, "Sent", message_id, in_reply_to, date) }
}
/// The conversation of the tree-order test: R <- A <- S(sent) <- X, newest X.
fn chain() -> Vec<Email> {
    vec![
        mail(1, "Inbox", "root@x", "", 10),
        Email { thread_id: "root@x".into(), ..mail(2, "Inbox", "a@x", "root@x", 20) },
        sent(3, "s@x", "a@x", 30),
        mail(4, "Inbox", "x@x", "s@x", 40),
    ]
}
fn inbox(service: &MailService) -> Vec<EmailSummary> {
    service
        .list(MailQuery { folder: "Inbox".into(), threads: true, limit: 100, ..Default::default() })
        .unwrap()
        .items
}
fn ids(replies: &[ConversationReply]) -> Vec<u64> {
    replies.iter().map(|r| r.id).collect()
}

#[test]
fn keep_parent_of_fills_only_an_empty_value() {
    let previous = Email { in_reply_to: "parent@x".into(), ..Default::default() };

    let mut echo = Email::default();
    echo.keep_parent_of(&previous);
    assert_eq!(echo.in_reply_to, "parent@x", "an echo without the header keeps the local copy's parent");

    let mut own = Email { in_reply_to: "other@x".into(), ..Default::default() };
    own.keep_parent_of(&previous);
    assert_eq!(own.in_reply_to, "other@x", "a value the provider supplied is never overwritten");

    let mut none = Email::default();
    none.keep_parent_of(&Email::default());
    assert_eq!(none.in_reply_to, "", "nothing to keep stays empty");
}

#[test]
fn a_thread_row_lists_every_other_message_in_tree_order() {
    let rows = inbox(&service(chain()));
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, 4, "the main row is the newest message of the view");
    assert_eq!(row.message_ids, vec![4, 2, 1], "the sent reply is shown, never acted on (INV-001)");
    assert_eq!(ids(&row.replies), vec![1, 2, 3]);
    let parents: Vec<_> = row.replies.iter().map(|r| r.parent_id).collect();
    assert_eq!(parents, vec![None, Some(1), Some(2)]);
    assert_eq!(row.replies_more, 0);
    let sent_reply = &row.replies[2];
    assert_eq!(sent_reply.folder, "Sent");
    assert_eq!(sent_reply.from_addr, "sender3@x");
}

#[test]
fn trash_and_spam_members_are_not_listed() {
    let mut emails = chain();
    emails.push(Email { thread_id: "root@x".into(), ..mail(5, "Trash", "t@x", "root@x", 25) });
    emails.push(Email { thread_id: "root@x".into(), ..mail(6, "Spam", "p@x", "root@x", 26) });
    let rows = inbox(&service(emails));
    assert_eq!(rows.len(), 1);
    assert_eq!(ids(&rows[0].replies), vec![1, 2, 3], "INV-004");
}

#[test]
fn a_sent_message_never_changes_the_row_read_flag() {
    let mut emails = chain();
    emails[2].read = false; // the sent reply is unread: it is not part of the view
    let rows = inbox(&service(emails));
    assert!(rows[0].read, "INV-007: only the view's messages decide `read`");
    assert!(!rows[0].replies[2].read, "the reply row still says what it is");

    let mut emails = chain();
    emails[1].read = false; // an unread message of the view
    assert!(!inbox(&service(emails))[0].read);
}

#[test]
fn an_orphan_is_placed_under_the_oldest_message() {
    let orphan = Email { thread_id: "root@x".into(), ..mail(7, "Sent", "q@x", "", 15) };
    let emails = vec![
        mail(1, "Inbox", "root@x", "", 10),
        orphan.clone(),
        mail(4, "Inbox", "x@x", "root@x", 40),
    ];
    let rows = inbox(&service(emails));
    let parents: Vec<_> = rows[0].replies.iter().map(|r| (r.id, r.parent_id)).collect();
    assert_eq!(parents, vec![(1, None), (7, Some(1))]);

    // The oldest message was trashed: it is not listed, so the orphan has nobody to hang under.
    let emails = vec![
        mail(1, "Trash", "root@x", "", 10),
        orphan,
        mail(4, "Inbox", "x@x", "root@x", 40),
    ];
    let rows = inbox(&service(emails));
    let parents: Vec<_> = rows[0].replies.iter().map(|r| (r.id, r.parent_id)).collect();
    assert_eq!(parents, vec![(7, None)]);
}

#[test]
fn a_reply_cycle_terminates() {
    let emails = vec![
        mail(1, "Inbox", "a@x", "b@x", 10),
        mail(2, "Inbox", "b@x", "a@x", 20),
        mail(3, "Inbox", "x@x", "a@x", 30),
    ];
    let rows = inbox(&service(emails));
    let parents: Vec<_> = rows[0].replies.iter().map(|r| (r.id, r.parent_id)).collect();
    assert_eq!(parents, vec![(1, None), (2, Some(1))], "the cycle opens at its oldest message");
}

#[test]
fn a_duplicate_of_the_main_message_is_never_a_reply() {
    let mut emails = chain();
    emails.push(Email { thread_id: "s@x".into(), ..mail(8, "All", "x@x", "s@x", 40) });
    emails.push(Email { thread_id: "root@x".into(), ..mail(9, "All", "a@x", "root@x", 20) });
    let rows = inbox(&service(emails));
    assert_eq!(rows.len(), 1);
    assert_eq!(ids(&rows[0].replies), vec![1, 2, 3], "no copy of the main message, no second copy of a reply");
}

#[test]
fn two_accounts_with_the_same_message_ids_do_not_join() {
    let mut other = chain();
    for e in &mut other {
        e.id += 100;
        e.account_id = 2;
    }
    let mut emails = chain();
    emails.extend(other);
    let rows = inbox(&service(emails));
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert!(row.replies.iter().all(|r| r.account_id == row.account_id), "{:?}", ids(&row.replies));
        assert_eq!(row.replies.len(), 3);
    }
}

#[test]
fn replies_are_capped_and_the_rest_is_counted() {
    let mut emails = vec![mail(1, "Inbox", "root@x", "", 10)];
    for i in 0..34u64 {
        emails.push(mail(10 + i, "Inbox", &format!("r{i}@x"), "root@x", 20 + i as i64));
    }
    emails.push(mail(100, "Inbox", "x@x", "root@x", 1_000));
    let rows = inbox(&service(emails));
    assert_eq!(rows[0].replies.len(), 30);
    assert_eq!(rows[0].replies_more, 5, "35 other messages, 30 listed");
}

#[test]
fn search_and_email_mode_have_no_replies() {
    let svc = service(chain());
    let flat = svc
        .list(MailQuery { folder: "Inbox".into(), threads: false, limit: 100, ..Default::default() })
        .unwrap();
    assert!(flat.items.iter().all(|r| r.replies.is_empty() && r.conversation_id == 0));

    let found = svc
        .list(MailQuery { search: "Topic".into(), threads: true, limit: 100, ..Default::default() })
        .unwrap();
    assert!(!found.items.is_empty());
    assert!(found.items.iter().all(|r| r.replies.is_empty() && r.replies_more == 0));
}

#[test]
fn conversation_id_is_stable_when_newer_mail_arrives() {
    let before = inbox(&service(chain()));
    let mut more = chain();
    more.push(mail(200, "Inbox", "newer@x", "x@x", 2_000));
    let after = inbox(&service(more));
    assert_eq!(before[0].id, 4);
    assert_eq!(after[0].id, 200, "the newest message is the main row");
    assert_ne!(before[0].conversation_id, 0);
    assert_eq!(before[0].conversation_id, after[0].conversation_id);
}

/// `read_rows` flattens the same summary: a row patched by id must not carry empty
/// conversation fields that would wipe what `list_mail` gave it.
#[test]
fn empty_conversation_fields_stay_out_of_the_json() {
    let svc = service(chain());
    let flat = svc
        .list(MailQuery { folder: "Inbox".into(), threads: false, limit: 100, ..Default::default() })
        .unwrap();
    let flat = serde_json::to_value(&flat.items[0]).unwrap();
    for key in ["conversation_id", "replies", "replies_more"] {
        assert!(flat.get(key).is_none(), "{key} is left out when empty");
    }

    let row = serde_json::to_value(&inbox(&svc)[0]).unwrap();
    assert_eq!(row["replies"].as_array().unwrap().len(), 3);
    assert!(row["conversation_id"].as_u64().unwrap() > 0);
    assert!(row.get("replies_more").is_none(), "nothing more to load: the key is left out");
}

// ---- Inbox headline: "N priority · N to read" ----

fn unread(id: u64, account: u32, message_id: &str) -> Email {
    Email { account_id: account, read: false, ..mail(id, "Inbox", message_id, "", 1_000 + id as i64) }
}
fn priority(id: u64, message_id: &str, in_reply_to: &str) -> Email {
    Email {
        read: false,
        category: Category::Priority,
        ..mail(id, "Inbox", message_id, in_reply_to, 1_000 + id as i64)
    }
}
fn page(svc: &MailService, folder: &str, threads: bool, account: Option<u32>, limit: usize) -> MailPage {
    svc.list(MailQuery { folder: folder.into(), threads, account_id: account, limit, ..Default::default() })
        .unwrap()
}

#[test]
fn inbox_counts_cover_the_whole_view_not_one_page() {
    let mut emails: Vec<Email> = (1..=75).map(|i| unread(i, 1, &format!("u{i}@x"))).collect();
    emails.push(Email { read: true, ..mail(200, "Inbox", "read@x", "", 50) });
    emails.push(Email { done: true, ..unread(201, 1, "done@x") });
    emails.push(Email { snoozed_until: Some(3_000_000_000), ..unread(202, 1, "later@x") });
    let svc = service(emails);
    let first = page(&svc, "Inbox", true, None, 20);
    assert_eq!(first.items.len(), 20);
    assert_eq!(first.unread_total, Some(75), "read, done and snoozed mail are not counted");
    assert_eq!(page(&svc, "", false, None, 20).unread_total, Some(75), "an empty folder is the Inbox");
}

#[test]
fn inbox_counts_follow_the_account_filter() {
    let mut emails: Vec<Email> = (1..=30).map(|i| unread(i, 1, &format!("a{i}@x"))).collect();
    emails.extend((31..=50).map(|i| unread(i, 2, &format!("b{i}@x"))));
    let svc = service(emails);
    assert_eq!(page(&svc, "Inbox", true, Some(2), 60).unread_total, Some(20));
    assert_eq!(page(&svc, "Inbox", true, None, 60).unread_total, Some(50));
}

#[test]
fn priority_total_counts_rows_in_thread_mode() {
    let svc = service(vec![
        priority(1, "p@x", ""),
        priority(2, "p2@x", "p@x"), // the same conversation: one row
        priority(3, "q@x", ""),
        unread(4, 1, "plain@x"),
    ]);
    assert_eq!(page(&svc, "Inbox", true, None, 60).priority_total, Some(2), "rows, not messages");
    assert_eq!(page(&svc, "Inbox", false, None, 60).priority_total, Some(3), "Email mode counts messages");
}

#[test]
fn other_folders_and_search_have_no_counts() {
    let mut emails = chain();
    emails.push(unread(9, 1, "u@x"));
    let svc = service(emails);
    let sent = page(&svc, "Sent", true, None, 60);
    assert_eq!((sent.unread_total, sent.priority_total), (None, None));
    let found = svc
        .list(MailQuery { search: "Topic".into(), threads: true, limit: 60, ..Default::default() })
        .unwrap();
    assert_eq!((found.unread_total, found.priority_total), (None, None));
}

// ---- read_replies: what lies beyond the cap ----

/// Root, 34 replies and the newest main message: 35 other messages, 30 of them in the row.
fn long_conversation() -> Vec<Email> {
    let mut emails = vec![mail(1, "Inbox", "root@x", "", 10)];
    for i in 0..34u64 {
        emails.push(mail(10 + i, "Inbox", &format!("r{i}@x"), "root@x", 20 + i as i64));
    }
    emails.push(mail(100, "Inbox", "x@x", "root@x", 1_000));
    emails
}

#[test]
fn read_replies_returns_the_rest_after_the_cap() {
    let svc = service(long_conversation());
    let row = &inbox(&svc)[0];
    assert_eq!((row.replies.len(), row.replies_more), (30, 5));

    let rest = svc.replies(row.id, "Inbox", 30, 30).unwrap();
    assert_eq!(rest.len(), 5);
    let everything = svc.replies(row.id, "Inbox", 0, 100).unwrap();
    assert_eq!(everything.len(), 35);
    assert_eq!(ids(&everything[..30]), ids(&row.replies), "the same order as the row");
    assert_eq!(rest, everything[30..], "the rest continues exactly where the row stopped");

    assert_eq!(svc.replies(row.id, "Inbox", 0, 0).unwrap().len(), 30, "no limit means the row's cap");
    assert!(svc.replies(row.id, "Inbox", 40, 30).unwrap().is_empty(), "past the end is empty");
}

#[test]
fn read_replies_for_an_unknown_id_is_an_error() {
    let svc = service(chain());
    assert!(svc.replies(9_999, "Inbox", 0, 30).is_err());
}

// ---- Final review, Important 1: an unread reply beyond the cap is not invisible ----

#[test]
fn an_unread_reply_beyond_the_cap_is_flagged_on_the_row() {
    // 35 other messages, 30 listed. The row itself and the listed replies are all read.
    let mut emails = long_conversation();
    let beyond = emails.iter_mut().find(|e| e.id == 10 + 33).unwrap();
    beyond.read = false;
    let svc = service(emails);
    let row = &inbox(&svc)[0];
    assert!(!ids(&row.replies).contains(&(10 + 33)), "the unread reply is one of the five not listed");
    assert!(row.replies.iter().all(|r| r.read));
    assert!(row.replies_more_unread, "the row says an unread reply is waiting beyond the cap");
    assert_eq!(serde_json::to_value(row).unwrap()["replies_more_unread"], true);

    // Nothing unread beyond the cap, or nothing beyond it at all: the key is left out.
    let svc = service(long_conversation());
    let row = &inbox(&svc)[0];
    assert!(!row.replies_more_unread);
    assert!(serde_json::to_value(row).unwrap().get("replies_more_unread").is_none());
    let short = inbox(&service(chain()));
    assert!(!short[0].replies_more_unread);
}

// ---- Final review, Important 4: the Trash and Spam views hold their own members ----

#[test]
fn the_trash_view_lists_the_trashed_members_of_a_conversation() {
    // R and A are in Trash, X (the newest) too; I is a live Inbox member and P is in Spam.
    let emails = vec![
        mail(1, "Trash", "root@x", "", 10),
        mail(2, "Trash", "a@x", "root@x", 20),
        mail(3, "Inbox", "i@x", "a@x", 30),
        mail(4, "Spam", "p@x", "a@x", 35),
        mail(5, "Trash", "x@x", "a@x", 40),
    ];
    let svc = service(emails);
    let rows = svc
        .list(MailQuery { folder: "Trash".into(), threads: true, limit: 100, ..Default::default() })
        .unwrap()
        .items;
    assert_eq!(rows.len(), 1, "one conversation in Trash");
    let row = &rows[0];
    assert_eq!(row.id, 5, "the main row is the newest message of the Trash view");
    assert_eq!(row.message_ids, vec![5, 2, 1], "only trashed messages are the view's");
    assert_eq!(ids(&row.replies), vec![1, 2, 3], "its own members and the live one shown; Spam is not the view");
    assert_eq!(
        svc.replies(row.id, "Trash", 0, 100).unwrap().iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "read_replies answers the same for the same view"
    );

    // The Inbox view still hides both Trash and Spam (INV-004).
    let inbox_rows = inbox(&svc);
    assert_eq!(inbox_rows.len(), 1);
    assert_eq!(ids(&inbox_rows[0].replies), Vec::<u64>::new(), "nothing else of that conversation is live");
    let spam = svc
        .list(MailQuery { folder: "Spam".into(), threads: true, limit: 100, ..Default::default() })
        .unwrap()
        .items;
    assert_eq!(ids(&spam[0].replies), vec![3], "the Spam view lists the live member, not Trash");
}

/// A store written before `in_reply_to` existed (the JSON engine keeps it as it was): received mail
/// has no parent stored, but its `thread_id` is the header it answered.
fn legacy(id: u64, folder: &str, message_id: &str, thread_id: &str, date: i64) -> Email {
    Email { in_reply_to: String::new(), thread_id: thread_id.into(), ..mail(id, folder, message_id, "", date) }
}

#[test]
fn received_mail_without_a_stored_parent_uses_its_thread_id() {
    let emails = vec![
        legacy(1, "Inbox", "root@x", "root@x", 10),
        legacy(2, "Inbox", "a@x", "root@x", 20),
        legacy(3, "Inbox", "b@x", "a@x", 30),
        legacy(4, "Inbox", "x@x", "b@x", 40),
    ];
    let rows = inbox(&service(emails));
    assert_eq!(rows[0].id, 4);
    assert_eq!(ids(&rows[0].replies), vec![1, 2, 3]);
    let parents: Vec<_> = rows[0].replies.iter().map(|r| r.parent_id).collect();
    assert_eq!(parents, vec![None, Some(1), Some(2)], "each reply hangs under the message it answered");
}

#[test]
fn a_sent_message_without_a_stored_parent_does_not_trust_its_thread_id() {
    // A sent reply copied its parent's thread, one level too high: it must not link to `b@x`.
    let emails = vec![
        legacy(1, "Inbox", "root@x", "root@x", 10),
        legacy(2, "Inbox", "b@x", "root@x", 20),
        legacy(3, "Sent", "s@x", "b@x", 30),
        legacy(4, "Inbox", "x@x", "root@x", 40),
    ];
    let rows = inbox(&service(emails));
    let sent_reply = rows[0].replies.iter().find(|r| r.id == 3).expect("the sent reply is listed");
    assert_eq!(sent_reply.parent_id, Some(1), "falls back to the oldest message, like any orphan");
}

#[test]
fn a_stored_parent_wins_over_the_thread_id() {
    let emails = vec![
        mail(1, "Inbox", "root@x", "", 10),
        mail(2, "Inbox", "a@x", "root@x", 20),
        Email { thread_id: "root@x".into(), ..mail(3, "Inbox", "b@x", "a@x", 30) },
        mail(4, "Inbox", "x@x", "root@x", 40),
    ];
    let rows = inbox(&service(emails));
    let b = rows[0].replies.iter().find(|r| r.id == 3).unwrap();
    assert_eq!(b.parent_id, Some(2), "In-Reply-To is the direct parent, thread_id only a fallback");
}
