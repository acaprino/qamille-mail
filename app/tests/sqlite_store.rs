//! SQLite (WAL) implementation of `StoreRepository`, ADR-0007 decision 1.
//! Not wired into the app yet: these tests are its whole contract (EX-005, INV-002).
use rusqlite::Connection;
use serde_json::Value;
use qamille::{
    application::ports::StoreRepository,
    domain::{
        indexing::{FolderProgress, IndexProgress, MessageIndexState},
        model::{
            Access, Account, AccountKind, AiProvider, Attachment, BodyFormat, Category, Draft,
            Email, Event, Meeting, RemoteLocation, Rsvp, Settings, Signature, SignatureBinding,
            Store, sample_store,
        },
        outbox::{Delivery, DeliveryStatus},
        storage::{AccountStoragePolicy, StorageLimit},
        undo::{UndoAction, UndoOperation, UndoStatus},
    },
    infrastructure::{
        portable_files::resolve_path, repository::JsonStoreRepository,
        sqlite_store::SqliteStoreRepository,
    },
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qamille-sqlite-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        Self(root)
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
    fn db(&self) -> PathBuf {
        self.data().join("store.sqlite")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn json(store: &Store) -> Value {
    serde_json::to_value(store).unwrap()
}

/// What a store looks like after one persistence round trip: received HTML is
/// reader-only memory and is never written (same rule as the JSON adapter).
fn persisted(store: &Store) -> Store {
    let mut store = store.clone();
    for email in &mut store.emails {
        email.body_html.clear();
    }
    store
}

fn save(repo: &SqliteStoreRepository, store: &Store) {
    let store = store.clone();
    repo.transaction(&mut |current| {
        *current = store.clone();
        Ok(())
    })
    .unwrap();
}

// ---------------------------------------------------------------------------
// Fixture in which every persisted field of every record has a distinct,
// non-default value, plus one sparse record of each kind (nulls, empty lists).
// ---------------------------------------------------------------------------

fn full_store(data: &Path) -> Store {
    let managed = |relative: &str| resolve_path(&format!("workspace:/{relative}"), data).unwrap();
    let draft = |id: u64, tag: &str| Draft {
        id,
        account_id: 1,
        to: format!("{tag}-to@example.com"),
        cc: format!("{tag}-cc@example.com"),
        bcc: format!("{tag}-bcc@example.com"),
        request_id: format!("{tag}-request"),
        remote_id: Some(format!("{tag}-remote-id")),
        remote_revision: Some(format!("{tag}-revision")),
        cloud_content_hash: format!("{tag}-hash"),
        cloud_sync_key: format!("{tag}-sync-key"),
        cloud_sync_pending: true,
        subject: format!("{tag} subject"),
        body: format!("{tag} body"),
        reply_to: Some(101),
        send_at: Some(1_800_000_001),
        attachments: vec![managed(&format!("draft-attachments/{tag}/one.txt")), "/outside/two.txt".into()],
        updated: 1_800_000_002,
        format: BodyFormat::Html,
        signature_id: Some(7_001),
        quote: format!("{tag} quote"),
    };
    let sparse_draft = Draft { id: 202, account_id: 2, ..Default::default() };

    let mut store = Store {
        outbox: vec![
            Delivery {
                id: 301,
                key: "delivery-key".into(),
                request_hash: "request-hash".into(),
                draft: draft(5_001, "outbox"),
                plain: "plain text".into(),
                html: Some("<p>html</p>".into()),
                sender_name: "Sender Name".into(),
                attachment_bytes: 4_096,
                status: DeliveryStatus::NeedsReview,
                created_at: 1_800_000_010,
                updated_at: 1_800_000_011,
                send_at: 1_800_000_012,
                requested_send_at: Some(1_800_000_013),
                error: Some("delivery error".into()),
                owner_pid: Some(4_321),
                requires_mcp_send: true,
                source_profile: Some("profile-a".into()),
            },
            Delivery {
                id: 302,
                key: "second-key".into(),
                request_hash: "second-hash".into(),
                draft: sparse_draft.clone(),
                plain: String::new(),
                html: None,
                sender_name: String::new(),
                attachment_bytes: 0,
                status: DeliveryStatus::Queued,
                created_at: 1,
                updated_at: 2,
                send_at: 3,
                requested_send_at: None,
                error: None,
                owner_pid: None,
                requires_mcp_send: false,
                source_profile: None,
            },
        ],
        cache_removals: vec![managed("attachments/gmail-1.bin"), "/outside/gmail-2.bin".into()],
        storage_maintenance_pending: true,
        storage_index_rebuild_required: true,
        undo_actions: vec![
            UndoAction {
                id: 401,
                operation: UndoOperation::Mail { email_id: 101, action: "star".into(), value: Some("on".into()) },
                subject: "Starred".into(),
                created_at_ms: 1_800_000_000_000,
                expires_at_ms: 1_800_000_005_000,
                status: UndoStatus::Committing,
                error: Some("undo error".into()),
                retry_at_ms: Some(1_800_000_060_000),
                attempts: 2,
            },
            UndoAction {
                id: 402,
                operation: UndoOperation::SendDraft { draft_id: 5_001 },
                subject: "Send".into(),
                created_at_ms: 1,
                expires_at_ms: 2,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            },
            UndoAction {
                id: 403,
                operation: UndoOperation::DiscardDraft { draft_id: 5_002 },
                subject: "Discard".into(),
                created_at_ms: 3,
                expires_at_ms: 4,
                status: UndoStatus::Failed,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            },
            UndoAction {
                id: 404,
                operation: UndoOperation::ScheduleDraft { draft_id: 5_003, send_at: 1_800_000_099 },
                subject: "Schedule".into(),
                created_at_ms: 5,
                expires_at_ms: 6,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            },
        ],
        accounts: vec![
            Account {
                provider_id: "gmail".into(),
                search_index: IndexProgress {
                    enabled: false,
                    folders: vec![FolderProgress {
                        name: "INBOX".into(),
                        role: "inbox".into(),
                        uid_validity: Some(11),
                        after_uid: 12,
                        before_uid: 3,
                        floor_uid: 2,
                        total: 13,
                        complete: true,
                        arrivals_only: true,
                        view: true,
                    }],
                    state: "running".into(),
                    scanned: 14,
                    indexed: 15,
                    partial: 16,
                    last_check: 17,
                    last_discovery: 18,
                    prefer_history: true,
                    storage_limited: true,
                    notice: Some("index notice".into()),
                    inventory_total: Some(19),
                    inventory_attempted: true,
                },
                id: 1,
                name: "Work".into(),
                email: "work@example.com".into(),
                color: [10, 20, 30],
                kind: AccountKind::Gmail,
                imap_host: "imap.example.com".into(),
                imap_port: 993,
                smtp_host: "smtp.example.com".into(),
                smtp_port: 465,
                username: "work-user".into(),
                password: String::new(), // credential vault, never in the data file
                oauth_client_id: "oauth-client".into(),
                signature: SignatureBinding::Id(7_001),
                last_signature: Some(7_002),
                archive_folder: "Archive".into(),
                trash_folder: "Bin".into(),
                last_sync: Some(1_800_000_020),
                fetch_count: 250,
                storage_policy: AccountStoragePolicy {
                    limit: StorageLimit::Limited { max_bytes: 123_456_789 },
                    keep_all_offline: true,
                    include_spam_trash: true,
                },
                offline_sync_complete: true,
                offline_sync_error: Some("sync error".into()),
                history: qamille::domain::history::HistoryProgress {
                    folder: "[Gmail]/All Mail".into(),
                    uid_validity: Some(4),
                    before_uid: 30,
                    highest_seen: 900,
                    total: 870,
                    fetched: 860,
                    complete: true,
                    last_error: Some("history error".into()),
                    stopped_for_space: Some(1_700_000_000),
                    unread_notice_shown: true,
                    unread_inbox: 140,
                    requested: vec!["Spam".into()],
                    mark_read_before: Some(1_690_000_000),
                },
            },
            Account {
                provider_id: String::new(),
                search_index: IndexProgress::default(),
                id: 2,
                name: "Sample".into(),
                email: String::new(),
                color: [0, 0, 0],
                kind: AccountKind::Sample,
                imap_host: String::new(),
                imap_port: 0,
                smtp_host: String::new(),
                smtp_port: 0,
                username: String::new(),
                password: String::new(),
                oauth_client_id: String::new(),
                signature: SignatureBinding::None,
                last_signature: None,
                archive_folder: String::new(),
                trash_folder: String::new(),
                last_sync: None,
                fetch_count: 0,
                storage_policy: AccountStoragePolicy { limit: StorageLimit::Unlimited, keep_all_offline: false, include_spam_trash: false },
                offline_sync_complete: false,
                offline_sync_error: None,
                history: Default::default(),
            },
        ],
        emails: vec![
            Email {
                search_index: MessageIndexState {
                    version: 3,
                    content_hash: "content-hash".into(),
                    partial: true,
                    notice: Some("message notice".into()),
                    retry_at: 77,
                    attempts: 2,
                    semantic_pending: true,
                },
                id: 101,
                account_id: 1,
                folder: "Inbox".into(),
                thread_id: "thread-1".into(),
                message_id: "<m1@example.com>".into(),
                in_reply_to: "parent-message@example.com".into(),
                local_sent: true,
                remote_deleted: true,
                outgoing_fingerprint: "fingerprint".into(),
                uid: Some(4_242),
                remote_folder: Some("[Gmail]/All Mail".into()),
                remote_uid_validity: Some(99),
                remote_content_hash: "remote-content-hash".into(),
                remote_global_id: Some(u64::MAX - 3),
                remote_locations: vec![
                    RemoteLocation { folder: "Label A".into(), uid: 5, uid_validity: Some(6) },
                    RemoteLocation { folder: "Label B".into(), uid: 7, uid_validity: None },
                ],
                from_name: "From Name".into(),
                from_addr: "from@example.com".into(),
                to: vec!["to-1@example.com".into(), "to-2@example.com".into()],
                cc: vec!["cc-1@example.com".into()],
                subject: "Subject line".into(),
                body: "Body text\nwith lines".into(),
                body_html: "<p>reader-only html</p>".into(),
                remote_only: true,
                storage_evicted: true,
                date: 1_700_000_000,
                date_missing: true,
                read: true,
                starred: true,
                pinned: true,
                done: true,
                muted: true,
                set_aside: true,
                replied: true,
                priority_thread: true,
                category: Category::Newsletter,
                labels: vec!["Gamma".into(), "Alpha".into(), "Beta".into()],
                snoozed_until: Some(1_800_000_030),
                reminder_at: Some(1_800_000_031),
                due: Some(1_800_000_032),
                assigned_to: Some("assignee@example.com".into()),
                attachments: vec![
                    Attachment {
                        id: u64::MAX - 11, // beyond i64: must survive the integer column
                        name: "report.pdf".into(),
                        size: 2_400,
                        mime: "application/pdf".into(),
                        path: Some(managed("attachments/report.pdf")),
                        inline_text: Some("inline text".into()),
                    },
                    Attachment {
                        id: 9_002,
                        name: "outside.txt".into(),
                        size: 17,
                        mime: "text/plain".into(),
                        path: Some("/outside/outside.txt".into()),
                        inline_text: None,
                    },
                ],
                classified: true,
                returned_at: Some(1_800_000_033),
                bulk: true,
                auto_submitted: true,
                calendar: true,
            },
            Email { id: 102, account_id: 2, ..Default::default() },
            Email { id: 103, account_id: 1, folder: "Sent".into(), labels: vec!["Alpha".into()], ..Default::default() },
        ],
        drafts: vec![draft(201, "first"), sparse_draft],
        events: vec![
            Event {
                id: 501,
                title: "Planning".into(),
                start: 1_800_000_100,
                end: 1_800_003_700,
                location: "Room 1".into(),
                attendees: vec!["a@example.com".into(), "b@example.com".into()],
                meeting_link: Some("https://meet.example.com/x".into()),
                conference: "Meet".into(),
                rsvp: Rsvp::Maybe,
                notes: "Bring notes".into(),
                color: [1, 2, 3],
            },
            Event { id: 502, ..Default::default() },
        ],
        meetings: vec![
            Meeting {
                id: 601,
                title: "Standup".into(),
                date: 1_800_000_200,
                duration_secs: 900,
                audio_path: Some(managed("meetings/601.wav")),
                transcript: "transcript".into(),
                notes: "notes".into(),
                summary: "summary".into(),
                key_points: vec!["point 1".into(), "point 2".into()],
                action_items: vec!["item 1".into()],
                event_id: Some(501),
            },
            Meeting { id: 602, ..Default::default() },
        ],
        settings: Settings {
            time_zone: "Europe/Rome".into(),
            locale: "it-IT".into(),
            undo_seconds: 9,
            user_name: "Alex".into(),
            signature: "legacy signature".into(), // kept as is: the signature manager is not empty
            ai_provider: AiProvider::Anthropic,
            ai_base_url: "https://ai.example.com".into(),
            ai_key: String::new(), // credential vault
            ai_model: "model-x".into(),
            auto_classify: true,
            auto_tag: true,
            mcp_access: Access::Send,
            dark: true,
            push_labels: true,
            google_client_id: "google-client".into(),
            google_client_secret: String::new(), // credential vault
            meeting_reminders: true,
            reminder_lead_minutes: 15,
            meeting_language: "en".into(),
            meeting_engine: "external".into(),
            meeting_model: "small".into(),
            meeting_remote_model: "whisper-large".into(),
            meeting_microphone: "mic-device-id".into(),
            meeting_system_audio: "speakers-device-id".into(),
            compute_mode: qamille::domain::model::ComputeMode::Server,
            compute_ca_pem: Some("-----BEGIN CERTIFICATE-----".into()),
            compute_ca_url: Some("https://calcolo.example".into()),
            compute_consented_urls: vec!["https://calcolo.example".into()],
            mailbox_sync: true,
            mailbox_sync_consent: Some("https://calcolo.example".into()),
            llm_consent: Some(qamille::domain::model::LlmConsent { url: "https://calcolo.example".into(), label: "Server".into(), external: true }),
            onboarding_done: true,
            legacy_local_notice_dismissed: true,
        },
        next_id: 9_999,
        signatures: vec![
            Signature { id: 7_001, name: "Work".into(), body: "<b>Work</b>".into(), modified: 1_800_000_300, html: true },
            Signature { id: 7_002, name: "Home".into(), body: "Home".into(), modified: 0, html: false },
        ],
    };
    // A second page of mail so the order test has something to reorder.
    for id in 104..110 {
        store.emails.push(Email { id, account_id: 1, subject: format!("filler {id}"), ..Default::default() });
    }
    store
}

/// Every JSON path that exists in the fixture must carry a non-default value in
/// at least one record, otherwise a column that silently drops it would go unnoticed.
fn paths(value: &Value, at: String, all: &mut BTreeSet<String>, filled: &mut BTreeSet<String>) {
    all.insert(at.clone());
    match value {
        Value::Object(map) => {
            if !map.is_empty() {
                filled.insert(at.clone());
            }
            for (key, child) in map {
                paths(child, format!("{at}.{key}"), all, filled);
            }
        }
        Value::Array(items) => {
            if !items.is_empty() {
                filled.insert(at.clone());
            }
            for child in items {
                paths(child, format!("{at}[]"), all, filled);
            }
        }
        Value::Null => {}
        Value::Bool(flag) => {
            if *flag {
                filled.insert(at);
            }
        }
        Value::Number(number) => {
            if number.as_f64() != Some(0.0) {
                filled.insert(at);
            }
        }
        Value::String(text) => {
            if !text.is_empty() {
                filled.insert(at);
            }
        }
    }
}

#[test]
fn fixture_exercises_every_persisted_field() {
    let scratch = Scratch::new();
    let expected = json(&persisted(&full_store(&scratch.data())));
    let (mut all, mut filled) = (BTreeSet::new(), BTreeSet::new());
    paths(&expected, "$".into(), &mut all, &mut filled);
    let untouched: Vec<_> = all.difference(&filled).collect();
    // Settings are one JSON document, and the account/signature/event colours
    // of the sparse records are legitimately zero: only these may stay default.
    assert!(
        untouched.is_empty(),
        "fixture leaves persisted fields at their default value: {untouched:?}"
    );
}

#[test]
fn roundtrip_store() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let store = full_store(&scratch.data());
    save(&repo, &store);

    let read = repo.read().unwrap();
    assert_eq!(json(&read), json(&persisted(&store)));
    // Fields that serde skips on purpose stay out of the database as well.
    assert!(read.accounts.iter().all(|a| a.password.is_empty()));
    assert!(read.emails.iter().all(|e| e.body_html.is_empty()));

    // A second handle (another process) sees the identical aggregate.
    let other = SqliteStoreRepository::open(scratch.db()).unwrap();
    assert_eq!(json(&other.read().unwrap()), json(&read));
}

#[test]
fn roundtrip_sample_store_and_empty_database() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    assert_eq!(json(&repo.read().unwrap()), json(&Store::default()), "empty database reads as an empty store");

    let sample = sample_store();
    save(&repo, &sample);
    assert_eq!(json(&repo.read().unwrap()), json(&persisted(&sample)));
}

#[test]
fn managed_file_references_are_portable_on_disk() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));

    let raw = Connection::open(scratch.db()).unwrap();
    let stored: String = raw
        .query_row("SELECT path FROM attachments WHERE id = -12", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, "workspace:/attachments/report.pdf");
    let outside: String = raw
        .query_row("SELECT path FROM attachments WHERE id = 9002", [], |row| row.get(0))
        .unwrap();
    assert_eq!(outside, "/outside/outside.txt", "files outside the data directory are kept verbatim");
    let removal: String = raw
        .query_row("SELECT path FROM cache_removals ORDER BY position LIMIT 1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(removal, "workspace:/attachments/gmail-1.bin");
}

#[test]
fn records_keep_their_order_across_edits() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let store = full_store(&scratch.data());
    save(&repo, &store);
    let ids = |s: &Store| s.emails.iter().map(|e| e.id).collect::<Vec<_>>();

    // Remove from the middle and append: relative order of the rest is kept.
    repo.transaction(&mut |s| {
        s.emails.retain(|e| e.id != 102 && e.id != 106);
        s.emails.push(Email { id: 500, account_id: 1, ..Default::default() });
        Ok(())
    })
    .unwrap();
    assert_eq!(ids(&repo.read().unwrap()), vec![101, 103, 104, 105, 107, 108, 109, 500]);

    // Moving a record to the end and inserting at the front are persisted too.
    repo.transaction(&mut |s| {
        let moved = s.emails.remove(1);
        s.emails.push(moved);
        s.emails.insert(0, Email { id: 600, account_id: 2, ..Default::default() });
        Ok(())
    })
    .unwrap();
    let expected = vec![600, 101, 104, 105, 107, 108, 109, 500, 103];
    let read = repo.read().unwrap();
    assert_eq!(ids(&read), expected);
    // Children travelled with the re-inserted row.
    assert_eq!(read.emails[1].labels, store.emails[0].labels);
    assert_eq!(read.emails[1].attachments.len(), 2);
    assert_eq!(read.emails.last().unwrap().labels, vec!["Alpha".to_string()]);

    // Same for drafts and undo actions, whose order is meaningful.
    repo.transaction(&mut |s| {
        s.undo_actions.reverse();
        s.drafts.swap(0, 1);
        Ok(())
    })
    .unwrap();
    let read = repo.read().unwrap();
    assert_eq!(read.undo_actions.iter().map(|a| a.id).collect::<Vec<_>>(), vec![404, 403, 402, 401]);
    assert_eq!(read.drafts.iter().map(|d| d.id).collect::<Vec<_>>(), vec![202, 201]);
}

#[test]
fn transaction_writes_only_changed_rows() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));

    // Triggers record every row the repository touches, per table.
    let raw = Connection::open(scratch.db()).unwrap();
    let mut script = String::from("CREATE TABLE audit (tbl TEXT NOT NULL, op TEXT NOT NULL, ref INTEGER);");
    for (table, key) in [
        ("accounts", "id"),
        ("emails", "id"),
        ("email_labels", "email_id"),
        ("remote_locations", "email_id"),
        ("attachments", "email_id"),
        ("drafts", "id"),
        ("outbox", "id"),
        ("undo_actions", "id"),
        ("events", "id"),
        ("meetings", "id"),
        ("signatures", "id"),
        ("settings", "id"),
        ("meta", "rowid"),
        ("cache_removals", "position"),
    ] {
        for (op, row) in [("insert", "NEW"), ("update", "NEW"), ("delete", "OLD")] {
            script += &format!(
                "CREATE TRIGGER audit_{table}_{op} AFTER {} ON {table} BEGIN INSERT INTO audit VALUES ('{table}', '{op}', {row}.{key}); END;",
                op.to_uppercase()
            );
        }
    }
    raw.execute_batch(&script).unwrap();
    // The store generation (snapshot key) moves with every write that changed
    // something: a deliberate extra meta row, left out of the comparison.
    let audit = || -> Vec<(String, String, i64)> {
        let mut statement = raw.prepare("SELECT tbl, op, ref FROM audit WHERE NOT (tbl = 'meta' AND ref IN (SELECT rowid FROM meta WHERE name = 'generation')) ORDER BY rowid").unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    repo.transaction(&mut |_| Ok(())).unwrap();
    assert_eq!(audit(), vec![], "a transaction that changes nothing writes nothing");

    repo.transaction(&mut |s| {
        s.emails.iter_mut().find(|e| e.id == 103).unwrap().starred = true;
        Ok(())
    })
    .unwrap();
    assert_eq!(audit(), vec![("emails".into(), "update".into(), 103)]);

    raw.execute("DELETE FROM audit", []).unwrap();
    repo.transaction(&mut |s| {
        s.emails.iter_mut().find(|e| e.id == 101).unwrap().labels.push("Delta".into());
        Ok(())
    })
    .unwrap();
    let writes = audit();
    assert!(
        writes.iter().all(|(table, _, id)| table == "email_labels" && *id == 101),
        "only the labels of email 101 are rewritten: {writes:?}"
    );
    assert!(!writes.is_empty());

    raw.execute("DELETE FROM audit", []).unwrap();
    repo.transaction(&mut |s| {
        s.emails.retain(|e| e.id != 109);
        s.next_id += 1;
        Ok(())
    })
    .unwrap();
    let writes = audit();
    assert!(writes.contains(&("emails".into(), "delete".into(), 109)));
    assert!(
        writes.iter().all(|(table, _, _)| table == "emails" || table == "meta"),
        "unrelated tables untouched: {writes:?}"
    );
    assert_eq!(writes.len(), 2, "one email delete and one counter update: {writes:?}");
}

#[test]
fn transaction_commits_only_on_ok() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let store = full_store(&scratch.data());
    save(&repo, &store);

    let error = repo
        .transaction(&mut |s| {
            s.emails.clear();
            s.settings.user_name = "Changed".into();
            s.next_id += 100;
            s.drafts.clear();
            Err(qamille::application::AppError::validation("refused"))
        })
        .unwrap_err();
    assert_eq!(error.message, "refused");
    assert_eq!(json(&repo.read().unwrap()), json(&persisted(&store)), "a refused transaction leaves no trace");

    // The closure runs once, and sees what the previous commit left.
    let mut calls = 0;
    repo.transaction(&mut |s| {
        calls += 1;
        assert_eq!(s.settings.user_name, "Alex");
        assert_eq!(s.emails.len(), store.emails.len());
        s.settings.user_name = "Renamed".into();
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(repo.read().unwrap().settings.user_name, "Renamed");
}

#[test]
fn duplicate_ids_are_refused_not_dropped() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));
    let before = json(&repo.read().unwrap());
    let error = repo
        .transaction(&mut |s| {
            let copy = s.emails[0].clone();
            s.emails.push(copy);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.code, "storage");
    assert_eq!(json(&repo.read().unwrap()), before);
}

/// A write transaction that stays open on its own thread until `finish`, and for
/// at least 500 ms after it opened: the overlap with the caller's reads is
/// guaranteed by construction instead of hoped for with a sleep.
struct HeldWrite {
    release: mpsc::Sender<()>,
    thread: thread::JoinHandle<()>,
}
impl HeldWrite {
    fn open(repo: Arc<SqliteStoreRepository>, change: impl Fn(&mut Store) + Send + 'static) -> Self {
        let (opened_tx, opened_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            repo.transaction(&mut |store| {
                change(store);
                opened_tx.send(()).unwrap();
                let opened = Instant::now();
                // Never forever: a reader that wrongly queues behind us must fail, not hang.
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
                if let Some(rest) = Duration::from_millis(500).checked_sub(opened.elapsed()) {
                    thread::sleep(rest);
                }
                Ok(())
            })
            .unwrap();
        });
        opened_rx.recv().unwrap(); // the writer slot is taken from here on
        Self { release, thread }
    }
    fn still_open(&self) -> bool {
        !self.thread.is_finished()
    }
    fn finish(self) {
        let _ = self.release.send(());
        self.thread.join().unwrap();
    }
}

/// A reader blocked by the writer would wait for the whole hold (and then hit the
/// busy timeout), every time. So the typical read must be fast, and no read may come
/// anywhere near the hold; a single slow read from OS scheduling noise on a loaded
/// machine says nothing about locking, and does not fail the test.
fn assert_reads_do_not_wait(mut durations: Vec<Duration>) {
    durations.sort();
    let median = durations[durations.len() / 2];
    let slowest = *durations.last().unwrap();
    assert!(median < Duration::from_millis(100), "median read during a write took {median:?} ({durations:?})");
    assert!(slowest < Duration::from_millis(450), "a read during a write took {slowest:?} ({durations:?})");
}

#[test]
fn read_during_write_does_not_block() {
    // EX-005 / INV-002: a reader never waits for a writer.
    let scratch = Scratch::new();
    let repo = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&repo, &sample_store());
    let starred = repo.read().unwrap().emails[0].starred;

    let write = HeldWrite::open(Arc::clone(&repo), |s| s.emails[0].starred = !s.emails[0].starred);
    let mut durations = Vec::new();
    for _ in 0..9 {
        let begun = Instant::now();
        let seen = repo.read().unwrap();
        durations.push(begun.elapsed());
        assert_eq!(seen.emails[0].starred, starred, "an uncommitted change must stay invisible");
    }
    assert!(write.still_open(), "the reads did not overlap the write: the test proves nothing");
    write.finish();
    assert_reads_do_not_wait(durations);
    assert_eq!(repo.read().unwrap().emails[0].starred, !starred, "the commit becomes visible");
}

#[test]
fn read_in_another_instance_does_not_block_on_a_write() {
    // Desktop and MCP are two processes, i.e. two independent repositories on one file.
    let scratch = Scratch::new();
    let desktop = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    let mcp = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&desktop, &sample_store());

    let write = HeldWrite::open(Arc::clone(&desktop), |s| s.emails[0].labels.push("from-desktop".into()));
    let mut durations = Vec::new();
    for _ in 0..9 {
        let begun = Instant::now();
        let seen = mcp.read().unwrap();
        durations.push(begun.elapsed());
        assert!(!seen.emails[0].labels.contains(&"from-desktop".to_string()));
    }
    assert!(write.still_open());
    write.finish();
    assert_reads_do_not_wait(durations);
    assert!(mcp.read().unwrap().emails[0].labels.contains(&"from-desktop".to_string()));
}

#[test]
fn writers_in_two_instances_serialize_without_losing_updates() {
    let scratch = Scratch::new();
    let a = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    let b = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&a, &sample_store());
    let start = a.read().unwrap().next_id;

    let (started, opened) = mpsc::channel();
    let first = {
        let a = Arc::clone(&a);
        thread::spawn(move || {
            a.transaction(&mut |s| {
                s.next_id += 1;
                s.emails[0].labels.push("by-a".into());
                started.send(()).unwrap();
                thread::sleep(Duration::from_millis(400));
                Ok(())
            })
            .unwrap();
        })
    };
    opened.recv().unwrap();
    let seen_by_b = {
        let mut seen = None;
        b.transaction(&mut |s| {
            // B had to wait for A, so it must see A's committed work.
            seen = Some((s.next_id, s.emails[0].labels.clone()));
            s.next_id += 1;
            s.emails[0].labels.push("by-b".into());
            Ok(())
        })
        .unwrap();
        seen.unwrap()
    };
    first.join().unwrap();
    assert_eq!(seen_by_b.0, start + 1);
    assert!(seen_by_b.1.contains(&"by-a".to_string()));

    let end = a.read().unwrap();
    assert_eq!(end.next_id, start + 2);
    assert!(end.emails[0].labels.ends_with(&["by-a".to_string(), "by-b".to_string()]));
}

#[test]
fn opening_applies_wal_pragmas_and_records_the_schema_version() {
    let scratch = Scratch::new();
    drop(SqliteStoreRepository::open(scratch.db()).unwrap());
    let reopened = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&reopened, &sample_store());

    let raw = Connection::open(scratch.db()).unwrap();
    let mode: String = raw.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    assert_eq!(mode, "wal");
    let version: String = raw
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "5");
    let tables: BTreeSet<String> = raw
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for table in [
        "accounts", "emails", "email_labels", "remote_locations", "attachments", "drafts",
        "undo_actions", "outbox", "settings", "meta", "change_feed",
    ] {
        assert!(tables.contains(table), "missing table {table}: {tables:?}");
    }

    // A database written by a newer release is not silently downgraded.
    raw.execute("UPDATE meta SET value = '999' WHERE name = 'schema_version'", []).unwrap();
    drop(raw);
    drop(reopened);
    let error = SqliteStoreRepository::open(scratch.db()).err().expect("newer schema must be refused");
    assert_eq!(error.code, "storage");
}

#[test]
fn many_readers_and_a_writer_make_progress_together() {
    let scratch = Scratch::new();
    let repo = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&repo, &sample_store());
    let writers: Vec<_> = (0..3)
        .map(|n| {
            let repo = Arc::clone(&repo);
            thread::spawn(move || {
                for round in 0..10 {
                    repo.transaction(&mut |s| {
                        s.emails[0].labels.push(format!("w{n}-{round}"));
                        Ok(())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let repo = Arc::clone(&repo);
            thread::spawn(move || {
                for _ in 0..30 {
                    let store = repo.read().unwrap();
                    // Every snapshot is internally consistent: never a torn aggregate.
                    assert!(store.emails.len() >= 1);
                }
            })
        })
        .collect();
    for handle in writers.into_iter().chain(readers) {
        handle.join().unwrap();
    }
    let labels = repo.read().unwrap().emails[0].labels.clone();
    for n in 0..3 {
        for round in 0..10 {
            assert!(labels.contains(&format!("w{n}-{round}")), "lost update w{n}-{round}");
        }
    }
}

// ---------------------------------------------------------------------------
// Parity with the JSON adapter for everything that is not plain persistence.
// ---------------------------------------------------------------------------

fn pair() -> (Scratch, JsonStoreRepository, Scratch, SqliteStoreRepository) {
    let json_dir = Scratch::new();
    let sqlite_dir = Scratch::new();
    let json_repo = JsonStoreRepository::new(json_dir.data().join("store.json"));
    let sqlite_repo = SqliteStoreRepository::open(sqlite_dir.db()).unwrap();
    (json_dir, json_repo, sqlite_dir, sqlite_repo)
}

fn quota_store() -> Store {
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 1,
        name: "Quota".into(),
        kind: AccountKind::Gmail,
        storage_policy: AccountStoragePolicy { limit: StorageLimit::Unlimited, keep_all_offline: false, include_spam_trash: false },
        ..Default::default()
    });
    store.emails.push(Email {
        id: 10,
        account_id: 1,
        uid: Some(5),
        folder: "Inbox".into(),
        message_id: "quota@example.com".into(),
        body: "Synthetic quota body. ".repeat(1_000),
        pinned: true, // its identity survives eviction of the replaceable content
        ..Default::default()
    });
    store.next_id = 10;
    store
}

#[test]
fn storage_admission_matches_the_json_adapter() {
    let (_j, json_repo, _s, sqlite_repo) = pair();
    let run = |repo: &dyn StoreRepository| {
        repo.transaction(&mut |s| {
            *s = quota_store();
            Ok(())
        })
        .unwrap();
        // Lowering the limit evicts replaceable content but keeps the pinned task.
        repo.transaction(&mut |s| {
            s.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 8_000 };
            Ok(())
        })
        .unwrap();
        // A limit that protected data cannot fit in is refused and not saved.
        let error = repo
            .transaction(&mut |s| {
                s.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 100 };
                s.emails[0].body = "z".repeat(50_000);
                s.emails[0].remote_only = false;
                s.emails[0].storage_evicted = false;
                s.drafts.push(Draft { id: 99, account_id: 1, reply_to: Some(10), body: "d".repeat(20_000), ..Default::default() });
                Ok(())
            })
            .unwrap_err();
        (repo.read().unwrap(), error.code)
    };
    let (json_state, json_code) = run(&json_repo);
    let (sqlite_state, sqlite_code) = run(&sqlite_repo);
    assert_eq!(sqlite_code, json_code);
    assert_eq!(json(&sqlite_state), json(&json_state));
    assert!(sqlite_state.emails[0].remote_only, "the quota really evicted the content");
    assert!(sqlite_state.emails[0].body.is_empty());
    assert!(sqlite_state.storage_maintenance_pending);
}

#[test]
fn reconcile_storage_matches_the_json_adapter() {
    let (json_dir, json_repo, sqlite_dir, sqlite_repo) = pair();
    let run = |scratch: &Scratch, repo: &dyn StoreRepository| {
        let cache = scratch.data().join("attachments");
        std::fs::create_dir_all(&cache).unwrap();
        let orphan = cache.join("gmail-orphan.bin");
        let kept = cache.join("gmail-kept.bin");
        std::fs::write(&orphan, b"orphan").unwrap();
        std::fs::write(&kept, b"kept").unwrap();
        let (orphan_path, kept_path) = (orphan.to_string_lossy().into_owned(), kept.to_string_lossy().into_owned());
        repo.transaction(&mut |s| {
            *s = quota_store();
            s.emails[0].attachments.push(Attachment {
                id: 1,
                name: "kept.bin".into(),
                size: 4,
                mime: "application/octet-stream".into(),
                path: Some(kept_path.clone()),
                inline_text: None,
            });
            s.cache_removals = vec![orphan_path.clone(), kept_path.clone()];
            Ok(())
        })
        .unwrap();
        assert!(repo.read().unwrap().storage_maintenance_pending, "removals are pending until reconciled");
        repo.reconcile_storage().unwrap();
        let state = repo.read().unwrap();
        (state, orphan.exists(), kept.exists())
    };
    let (json_state, json_orphan, json_kept) = run(&json_dir, &json_repo);
    let (sqlite_state, sqlite_orphan, sqlite_kept) = run(&sqlite_dir, &sqlite_repo);
    assert!(!json_orphan && json_kept, "sanity: the JSON adapter removes only the unreferenced file");
    assert_eq!((sqlite_orphan, sqlite_kept), (json_orphan, json_kept));
    assert!(sqlite_state.cache_removals.is_empty());
    assert!(!sqlite_state.storage_maintenance_pending);
    // Different scratch directories: compare with paths neutralised.
    let neutral = |state: &Store, scratch: &Scratch| {
        json(state).to_string().replace(&scratch.data().to_string_lossy().replace('\\', "\\\\"), "<data>")
    };
    assert_eq!(neutral(&sqlite_state, &sqlite_dir), neutral(&json_state, &json_dir));
}

#[test]
fn storage_measurements_match_the_json_adapter() {
    let (json_dir, json_repo, sqlite_dir, sqlite_repo) = pair();
    let measure = |scratch: &Scratch, repo: &dyn StoreRepository| {
        let folder = scratch.data().join("draft-attachments").join("abc");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("note.txt");
        std::fs::write(&file, vec![b'x'; 1_234]).unwrap();
        let mut store = quota_store();
        store.drafts.push(Draft {
            id: 7,
            account_id: 1,
            attachments: vec![file.to_string_lossy().into_owned()],
            ..Default::default()
        });
        let drafts = repo.draft_storage_bytes(&store).unwrap();
        let messages = repo.message_storage_bytes(&store).unwrap();
        (drafts.get(&1).map(|d| d.additional_bytes), messages)
    };
    let json_measured = measure(&json_dir, &json_repo);
    let sqlite_measured = measure(&sqlite_dir, &sqlite_repo);
    assert_eq!(json_measured.0, Some(1_234), "sanity: the draft file is charged");
    assert_eq!(sqlite_measured, json_measured);
}

// ---------------------------------------------------------------------------
// Crash safety: a process killed in the middle of a write leaves the last
// committed state, whole, and does not keep the write slot.
// ---------------------------------------------------------------------------

const CRASH_EMAILS: u64 = 1_200;
const CRASH_BODY: usize = 3_000;

/// A store whose every record carries the same generation tag: a torn commit
/// shows up as two generations in one aggregate.
fn generation_store(generation: u64) -> Store {
    let mut store = Store::default();
    store.accounts.push(Account {
        id: 1,
        name: "Crash".into(),
        kind: AccountKind::Gmail,
        storage_policy: AccountStoragePolicy { limit: StorageLimit::Unlimited, keep_all_offline: false, include_spam_trash: false },
        ..Default::default()
    });
    for id in 1..=CRASH_EMAILS {
        store.emails.push(Email {
            id,
            account_id: 1,
            folder: "Inbox".into(),
            body: "x".repeat(CRASH_BODY),
            ..Default::default()
        });
    }
    set_generation(&mut store, generation);
    store
}

fn set_generation(store: &mut Store, generation: u64) {
    let tag = format!("gen-{generation}");
    store.next_id = generation;
    store.settings.user_name = tag.clone();
    for email in &mut store.emails {
        email.subject = tag.clone();
        email.labels = vec![tag.clone()];
    }
}

/// The generation of a store that must be entirely one generation.
fn generation_of(store: &Store) -> u64 {
    let generation = store.next_id;
    let tag = format!("gen-{generation}");
    assert_eq!(store.emails.len() as u64, CRASH_EMAILS, "an email was lost or added by a torn commit");
    assert_eq!(store.settings.user_name, tag, "settings come from another generation");
    for email in &store.emails {
        assert_eq!(email.subject, tag, "email {} comes from another generation: torn commit", email.id);
        assert_eq!(email.labels, vec![tag.clone()], "labels of email {} are torn", email.id);
        assert_eq!(email.body.len(), CRASH_BODY);
    }
    generation
}

fn integrity_ok(path: &Path) {
    let raw = Connection::open(path).unwrap();
    let result: String = raw.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
    assert_eq!(result, "ok");
}

/// Body of the child process the crash tests kill. A normal run does nothing.
#[test]
#[ignore = "child process of the crash-safety tests; started by them, not by hand"]
fn crash_child() {
    use std::io::Write;
    let Ok(dir) = std::env::var("QAMILLE_CRASH_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let repo = SqliteStoreRepository::open(dir.join("data").join("store.sqlite")).unwrap();
    let say = |line: String| {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    };
    match std::env::var("QAMILLE_CRASH_MODE").unwrap_or_default().as_str() {
        // Rewrite every record, generation after generation, until killed.
        "generations" => loop {
            let mut generation = 0;
            repo.transaction(&mut |store| {
                generation = store.next_id + 1;
                say(format!("writing {generation}"));
                set_generation(store, generation);
                Ok(())
            })
            .unwrap();
            say(format!("committed {generation}"));
        },
        // Take the write slot with uncommitted changes and stay there.
        "hold" => {
            repo.transaction(&mut |store| {
                store.settings.user_name = "uncommitted".into();
                store.emails[0].subject = "uncommitted".into();
                std::fs::write(dir.join("holding"), b"1").unwrap();
                thread::sleep(Duration::from_secs(120));
                Ok(())
            })
            .unwrap();
        }
        other => panic!("unknown crash mode {other:?}"),
    }
}

/// What the child announced: `writing G` once its transaction for generation G
/// is open, `committed G` after the commit returned.
struct Child {
    process: std::process::Child,
    events: mpsc::Receiver<(String, u64)>,
    reader: Option<thread::JoinHandle<()>>,
    committed: u64,
    writing: u64,
}
impl Child {
    fn spawn(scratch: &Scratch, mode: &str) -> Self {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let mut process = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "crash_child", "--nocapture", "--test-threads=1"])
            .env("QAMILLE_CRASH_DIR", &scratch.0)
            .env("QAMILLE_CRASH_MODE", mode)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = process.stdout.take().unwrap();
        let (sender, events) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let mut words = line.split_whitespace();
                if let (Some(kind @ ("writing" | "committed")), Some(Ok(number))) =
                    (words.next(), words.next().map(str::parse::<u64>))
                {
                    if sender.send((kind.to_string(), number)).is_err() {
                        break;
                    }
                }
            }
        });
        Self { process, events, reader: Some(reader), committed: 0, writing: 0 }
    }
    fn note(&mut self, (kind, number): (String, u64)) {
        if kind == "writing" {
            self.writing = number;
        } else {
            self.committed = number;
        }
    }
    /// Waits for an announcement that satisfies `wanted`.
    fn wait(&mut self, wanted: impl Fn(&str, u64) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = self.events.recv_timeout(left).expect("the child process did not make progress");
            let done = wanted(&event.0, event.1);
            self.note(event);
            if done {
                return;
            }
        }
    }
    /// Hard kill (no destructors, no flushing), then everything it announced.
    fn kill(&mut self) {
        let _ = self.process.kill();
        self.process.wait().unwrap();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        let rest: Vec<_> = self.events.try_iter().collect();
        for event in rest {
            self.note(event);
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn killing_the_process_mid_write_leaves_the_last_committed_state() {
    let scratch = Scratch::new();
    save(&SqliteStoreRepository::open(scratch.db()).unwrap(), &generation_store(0));

    let mut rolled_back = 0;
    let mut floor = 0;
    for (cycle, delay_ms) in [60u64, 140, 220, 290, 340, 400].into_iter().enumerate() {
        // Odd cycles keep a reader of this process open next to the child's writes.
        let reader = (cycle % 2 == 1).then(|| SqliteStoreRepository::open(scratch.db()).unwrap());
        let mut child = Child::spawn(&scratch, "generations");
        // Past start-up and one full commit: from here the child is always writing.
        child.wait(|kind, _| kind == "committed");
        let seen = child.committed;
        child.wait(|kind, number| kind == "writing" && number > seen);
        let kill_at = Instant::now() + Duration::from_millis(delay_ms);
        loop {
            if let Some(reader) = &reader {
                generation_of(&reader.read().unwrap()); // whole generations only, while it writes
            }
            if Instant::now() >= kill_at {
                break;
            }
        }
        child.kill();
        drop(reader);

        let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
        let generation = generation_of(&repo.read().unwrap());
        assert!(
            generation == child.committed || generation == child.writing,
            "cycle {cycle}: store is at generation {generation}, child committed {} and was writing {}",
            child.committed,
            child.writing
        );
        assert!(generation >= floor, "cycle {cycle}: an acknowledged commit was lost");
        if child.committed < child.writing && generation == child.committed {
            rolled_back += 1;
        }
        integrity_ok(&scratch.db());

        // The dead process does not keep the write slot.
        let begun = Instant::now();
        repo.transaction(&mut |store| {
            set_generation(store, generation + 1);
            Ok(())
        })
        .unwrap();
        assert!(begun.elapsed() < Duration::from_secs(3), "write after a crash waited {:?}", begun.elapsed());
        floor = generation + 1;
        assert_eq!(generation_of(&repo.read().unwrap()), floor);
    }
    assert!(rolled_back >= 1, "no kill landed inside a write: the test proved nothing");
}

#[test]
fn killing_the_process_inside_an_open_transaction_changes_nothing() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));
    let before = json(&repo.read().unwrap());

    let mut child = Child::spawn(&scratch, "hold");
    let marker = scratch.0.join("holding");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "the child never opened its transaction");
        thread::sleep(Duration::from_millis(10));
    }
    // The other process holds the write slot with pending changes: reads are unaffected.
    let begun = Instant::now();
    assert_eq!(json(&repo.read().unwrap()), before, "uncommitted changes of another process are invisible");
    assert!(begun.elapsed() < Duration::from_millis(450));

    child.kill();
    assert_eq!(json(&repo.read().unwrap()), before);
    drop(repo);
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    assert_eq!(json(&repo.read().unwrap()), before, "the last committed state survives the crash");
    integrity_ok(&scratch.db());

    let begun = Instant::now();
    repo.transaction(&mut |store| {
        store.settings.user_name = "after-crash".into();
        Ok(())
    })
    .unwrap();
    assert!(begun.elapsed() < Duration::from_secs(3), "write after a crash waited {:?}", begun.elapsed());
    assert_eq!(repo.read().unwrap().settings.user_name, "after-crash");
}

// ---------------------------------------------------------------------------
// Compatibility with the JSON file the app writes today.
// ---------------------------------------------------------------------------

/// Both adapters share one data directory, as store.json and store.sqlite will
/// during the migration: managed file references then resolve to the same paths.
fn shared_dir_pair() -> (Scratch, JsonStoreRepository, SqliteStoreRepository) {
    let scratch = Scratch::new();
    let json_repo = JsonStoreRepository::new(scratch.data().join("store.json"));
    let sqlite_repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    (scratch, json_repo, sqlite_repo)
}

fn serialized(store: &Store) -> Vec<u8> {
    serde_json::to_vec(store).unwrap()
}

#[test]
fn json_and_sqlite_load_the_same_store_as_equal_projections() {
    let (scratch, json_repo, sqlite_repo) = shared_dir_pair();
    for (index, store) in [full_store(&scratch.data()), sample_store(), Store::default()].into_iter().enumerate() {
        save(&sqlite_repo, &store);
        let through_sqlite = sqlite_repo.read().unwrap();
        json_repo
            .transaction(&mut |current| {
                *current = store.clone();
                Ok(())
            })
            .unwrap();
        let through_json = json_repo.read().unwrap();
        assert_eq!(json(&through_sqlite), json(&through_json), "projections of store #{index}");
        assert_eq!(serialized(&through_sqlite), serialized(&through_json), "byte-for-byte equal projections");
        // Apart from the maintenance marker both adapters derive from the change itself.
        let mut expected = persisted(&store);
        expected.storage_maintenance_pending = through_sqlite.storage_maintenance_pending;
        assert_eq!(serialized(&through_sqlite), serialized(&expected));
    }
}

#[test]
fn a_store_json_loaded_by_the_json_adapter_imports_unchanged_into_sqlite() {
    // The first start with an existing store.json (EX-006): what the old adapter
    // reads is what the new one must return after the import.
    let (scratch, json_repo, sqlite_repo) = shared_dir_pair();
    let store = full_store(&scratch.data());
    json_repo
        .transaction(&mut |current| {
            *current = store.clone();
            Ok(())
        })
        .unwrap();
    let from_file = json_repo.read().unwrap();
    assert!(from_file.emails.len() >= 9, "sanity: the imported mailbox is not trivial");

    save(&sqlite_repo, &from_file);
    assert_eq!(serialized(&sqlite_repo.read().unwrap()), serialized(&from_file));

    // Importing again changes nothing: same aggregate, same rows.
    let rows = |table: &str| -> i64 {
        Connection::open(scratch.db())
            .unwrap()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    };
    let counts: Vec<_> = ["accounts", "emails", "attachments", "drafts", "outbox"].map(rows).to_vec();
    save(&sqlite_repo, &from_file);
    assert_eq!(["accounts", "emails", "attachments", "drafts", "outbox"].map(rows).to_vec(), counts);
    assert_eq!(counts[1], from_file.emails.len() as i64);
    assert_eq!(counts[0], from_file.accounts.len() as i64);
}

#[test]
fn a_fresh_database_needs_no_migration() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    assert_eq!(
        serialized(&repo.read().unwrap()),
        serialized(&Store::default()),
        "a database created by open() reads as the empty store"
    );

    let schema = |path: &Path| -> Vec<(String, String)> {
        let raw = Connection::open(path).unwrap();
        let mut statement = raw
            .prepare("SELECT name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let created = schema(&scratch.db());
    assert!(!created.is_empty());
    drop(repo);
    let again = SqliteStoreRepository::open(scratch.db()).unwrap();
    assert_eq!(schema(&scratch.db()), created, "opening again changes nothing");
    let versions: i64 = Connection::open(scratch.db())
        .unwrap()
        .query_row("SELECT count(*) FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(versions, 1);
    drop(again);

    let mut files: Vec<_> = std::fs::read_dir(scratch.data())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.retain(|name| !name.starts_with("store.sqlite"));
    assert!(files.is_empty(), "nothing but the database is created: {files:?}");
}

#[test]
fn opening_does_not_queue_behind_a_running_writer() {
    let scratch = Scratch::new();
    let desktop = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&desktop, &sample_store());
    let write = HeldWrite::open(Arc::clone(&desktop), |s| s.emails[0].starred = !s.emails[0].starred);
    let begun = Instant::now();
    let mcp = SqliteStoreRepository::open(scratch.db()).unwrap(); // the MCP process starting up
    let opened = begun.elapsed();
    let seen = mcp.read().unwrap().emails.len();
    assert!(write.still_open(), "the open did not overlap the write");
    write.finish();
    assert!(opened < Duration::from_millis(450), "open waited {opened:?} for the writer");
    assert_eq!(seen, sample_store().emails.len());
}

// ---------------------------------------------------------------------------
// Writers queue in the process; maintenance (checkpoint, vacuum) is never part
// of a write and never blocks readers (ADR-0007 decisions 1 and 2).
// ---------------------------------------------------------------------------

fn bulky_store(emails: u64, body: usize) -> Store {
    let mut store = Store::default();
    for id in 1..=emails {
        store.emails.push(Email { id, account_id: 1, body: "b".repeat(body), ..Default::default() });
    }
    store
}

fn len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

#[test]
fn commits_do_not_checkpoint_and_checkpoint_runs_on_request() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &bulky_store(40, 200_000)); // about 8 MB, far beyond SQLite's default auto-checkpoint
    let wal = scratch.data().join("store.sqlite-wal");
    assert!(len(&wal) > 6_000_000, "the commit went to the log: {}", len(&wal));
    assert!(
        len(&scratch.db()) < 1_000_000,
        "a commit must not fold the log into the database file ({} bytes): that is maintenance",
        len(&scratch.db())
    );

    let report = repo.checkpoint().unwrap();
    assert!(!report.busy);
    assert!(report.log_frames > 1_000);
    assert_eq!(report.checkpointed_frames, report.log_frames, "everything was folded in");
    assert!(len(&scratch.db()) > 6_000_000, "the data now lives in the database file");
    assert_eq!(repo.read().unwrap().emails.len(), 40);
}

#[test]
fn checkpoint_does_not_block_readers_or_an_open_write() {
    let scratch = Scratch::new();
    let repo = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&repo, &sample_store());
    let write = HeldWrite::open(Arc::clone(&repo), |s| s.emails[0].starred = !s.emails[0].starred);
    let begun = Instant::now();
    let report = repo.checkpoint().unwrap();
    let took = begun.elapsed();
    let read = repo.read().unwrap();
    assert!(write.still_open(), "the checkpoint did not overlap the write");
    write.finish();
    assert!(took < Duration::from_millis(450), "checkpoint waited {took:?} for the writer");
    assert!(!report.busy);
    assert_eq!(read.emails.len(), sample_store().emails.len());
}

#[test]
fn reclaim_space_returns_freed_pages_on_request_only() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &bulky_store(20, 100_000));
    repo.checkpoint().unwrap();
    let grown = len(&scratch.db());
    repo.transaction(&mut |s| {
        s.emails.clear();
        Ok(())
    })
    .unwrap();
    repo.checkpoint().unwrap();

    let raw = Connection::open(scratch.db()).unwrap();
    let free = |raw: &Connection| -> i64 { raw.query_row("PRAGMA freelist_count", [], |row| row.get(0)).unwrap() };
    let mode: i64 = raw.query_row("PRAGMA auto_vacuum", [], |row| row.get(0)).unwrap();
    assert_eq!(mode, 2, "incremental auto-vacuum: deleting rows never shrinks the file inside a write");
    assert!(free(&raw) > 100, "deleted pages wait on the free list");
    assert_eq!(len(&scratch.db()), grown, "the delete did not touch the file size");

    let reclaimed = repo.reclaim_space(None).unwrap();
    repo.checkpoint().unwrap();
    assert!(reclaimed > 100, "reclaimed {reclaimed}");
    assert_eq!(free(&raw), 0);
    assert!(len(&scratch.db()) < grown / 2, "the file shrank: {} from {grown}", len(&scratch.db()));
}

#[test]
fn in_process_writers_queue_beyond_the_busy_timeout() {
    // Only another process is held to the busy timeout; writers of this process wait their turn.
    let scratch = Scratch::new();
    let repo = Arc::new(
        SqliteStoreRepository::open_with_busy_timeout(scratch.db(), Duration::from_millis(150)).unwrap(),
    );
    save(&repo, &sample_store());
    let inside = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let widest = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writers: Vec<_> = (0..6)
        .map(|n| {
            let (repo, inside, widest) = (Arc::clone(&repo), Arc::clone(&inside), Arc::clone(&widest));
            thread::spawn(move || {
                repo.transaction(&mut |s| {
                    use std::sync::atomic::Ordering::SeqCst;
                    widest.fetch_max(inside.fetch_add(1, SeqCst) + 1, SeqCst);
                    thread::sleep(Duration::from_millis(100));
                    s.emails[0].labels.push(format!("queued-{n}"));
                    inside.fetch_sub(1, SeqCst);
                    Ok(())
                })
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap().expect("a queued writer must not time out");
    }
    assert_eq!(widest.load(std::sync::atomic::Ordering::SeqCst), 1, "writes never overlap");
    let labels = repo.read().unwrap().emails[0].labels.clone();
    for n in 0..6 {
        assert!(labels.contains(&format!("queued-{n}")), "lost update queued-{n}");
    }
}

#[test]
fn a_nested_write_is_refused_instead_of_deadlocking() {
    let scratch = Scratch::new();
    let repo = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&repo, &sample_store());
    let (done, outcome) = mpsc::channel();
    let inner = Arc::clone(&repo);
    thread::spawn(move || {
        let result = inner.transaction(&mut |_| {
            let nested = inner.transaction(&mut |s| {
                s.settings.user_name = "nested".into();
                Ok(())
            });
            assert!(nested.is_err(), "a write inside a write must be refused");
            Ok(())
        });
        let _ = done.send(result.is_ok());
    });
    assert!(outcome.recv_timeout(Duration::from_secs(10)).expect("the nested write hung"));
    assert_ne!(repo.read().unwrap().settings.user_name, "nested");
}

// ---------------------------------------------------------------------------
// Task 4.2: maintenance lives outside the write transaction (ADR-0007
// decision 2) and a cross-process `SQLITE_BUSY` is retried, never reported
// to the user (Review Focus 4).
// ---------------------------------------------------------------------------

use qamille::application::{
    AppResult, ServiceContext,
    maintenance::MaintenanceService,
    ports::{
        AttachmentGateway, Clock, MailGateway, NullSink, OutgoingMessage, RemoteAction, SecretStore,
        SyncBatch,
    },
};
use qamille::infrastructure::repository::MemoryRepository;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

/// Two repositories on one file are two processes for SQLite: separate write
/// slots, separate pools. With `busy_timeout` at zero every collision surfaces
/// as an immediate `SQLITE_BUSY`, so only the retry loop can bring both sides
/// through: 50 writes each, no error, 100 committed rows.
#[test]
fn concurrent_writers_both_commit() {
    let scratch = Scratch::new();
    let seed = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&seed, &sample_store());
    let baseline = seed.read().unwrap().emails[0].labels.len();
    drop(seed);

    const ROUNDS: u32 = 50;
    let writer = |tag: &'static str| {
        let repo = SqliteStoreRepository::open_with_busy_timeout(scratch.db(), Duration::ZERO).unwrap();
        thread::spawn(move || {
            for round in 0..ROUNDS {
                // A small gap keeps the two writers from pinning the lock at a
                // near-100% duty cycle; collisions stay frequent (they must be,
                // or the test proves nothing) while a retry round always finds
                // a free moment.
                thread::sleep(Duration::from_millis(50));
                repo.transaction(&mut |s| {
                    s.emails[0].labels.push(format!("{tag}-{round}"));
                    Ok(())
                })
                .unwrap_or_else(|error| panic!("{tag}-{round} failed: {}", error.message));
            }
        })
    };
    let first = writer("a");
    let second = writer("b");
    first.join().unwrap();
    second.join().unwrap();

    let labels = SqliteStoreRepository::open(scratch.db()).unwrap().read().unwrap().emails[0].labels.clone();
    for tag in ["a", "b"] {
        for round in 0..ROUNDS {
            assert!(labels.contains(&format!("{tag}-{round}")), "lost write {tag}-{round}");
        }
    }
    assert_eq!(labels.len(), baseline + 2 * ROUNDS as usize, "every write committed exactly once");
}

/// The deterministic shape of the same contract: while another process holds
/// the write lock, a write with no busy timeout keeps retrying on its own and
/// commits on the first retry after the lock frees — the user sees no error.
#[test]
fn a_busy_write_retries_until_the_other_process_commits() {
    let scratch = Scratch::new();
    let holder = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&holder, &sample_store());
    // The other "process" holds the write lock from its `opened` signal until
    // 150 ms after the writer announced its first attempt: the first attempts
    // are guaranteed to find the lock taken (proving the retry loop is what
    // gets the write through), and every later attempt finds it free with
    // more than a second of margin to the final retry.
    let (opened_tx, opened_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let holding = {
        let holder = Arc::clone(&holder);
        thread::spawn(move || {
            holder.transaction(&mut |s| {
                s.settings.user_name = "held".into();
                opened_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(30));
                Ok(())
            })
        })
    };
    opened_rx.recv().unwrap();
    let waiting = SqliteStoreRepository::open_with_busy_timeout(scratch.db(), Duration::ZERO).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done, outcome) = mpsc::channel();
    thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = waiting.transaction(&mut |s| {
            s.settings.user_name = "second".into();
            Ok(())
        });
        let _ = done.send(result);
    });
    started_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(150)); // the attempts at ~0/50/150 ms meet SQLITE_BUSY
    let _ = release.send(());
    holding.join().unwrap().expect("the holding write failed");
    let result = outcome.recv_timeout(Duration::from_secs(60)).expect("the busy write never returned");
    result.unwrap_or_else(|error| panic!("a busy write reached the user: {}", error.message));
    assert_eq!(holder.read().unwrap().settings.user_name, "second");
}

/// A repository that counts maintenance calls and how many of them overlap.
struct CountingRepository {
    inner: MemoryRepository,
    reconciles: AtomicUsize,
    inside: AtomicUsize,
    widest: AtomicUsize,
}
impl CountingRepository {
    fn new(store: Store) -> Arc<Self> {
        Arc::new(Self {
            inner: MemoryRepository::new(store),
            reconciles: AtomicUsize::new(0),
            inside: AtomicUsize::new(0),
            widest: AtomicUsize::new(0),
        })
    }
}
impl StoreRepository for CountingRepository {
    fn read(&self) -> AppResult<Store> {
        self.inner.read()
    }
    fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.inner.transaction(op)
    }
    fn reconcile_storage(&self) -> AppResult<()> {
        self.reconciles.fetch_add(1, SeqCst);
        self.widest.fetch_max(self.inside.fetch_add(1, SeqCst) + 1, SeqCst);
        thread::sleep(Duration::from_millis(20)); // an overlap becomes observable
        let result = self.inner.reconcile_storage();
        self.inside.fetch_sub(1, SeqCst);
        result
    }
}

struct StubClock;
impl Clock for StubClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct StubSecrets;
impl SecretStore for StubSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct StubAttachments;
impl AttachmentGateway for StubAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, attachment: &Attachment) -> AppResult<String> {
        Ok(attachment.inline_text.clone().unwrap_or_default())
    }
}
struct StubMail;
impl MailGateway for StubMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: Vec::new(), archive_folder: None, trash_folder: None })
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

fn maintenance_context() -> (Arc<ServiceContext>, Arc<CountingRepository>) {
    let repository = CountingRepository::new(sample_store());
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        repository: Arc::clone(&repository) as Arc<dyn StoreRepository>,
        mail: Arc::new(StubMail),
        secrets: Arc::new(StubSecrets),
        clock: Arc::new(StubClock),
        attachments: Arc::new(StubAttachments),
        events: Arc::new(NullSink),
    });
    (context, repository)
}

#[test]
fn update_does_not_run_maintenance() {
    // ADR-0007 decision 2: no user write runs reconcile_storage in line,
    // neither before nor after the transaction.
    let (context, repository) = maintenance_context();
    context.update(|store| {
        store.settings.user_name = "saved".into();
        Ok(())
    }).unwrap();
    context.update_with_events(|store| Ok((store.emails.len(), Vec::new()))).unwrap();
    assert_eq!(repository.reconciles.load(SeqCst), 0, "update ran maintenance in the write path");
    assert_eq!(context.read().unwrap().settings.user_name, "saved", "the write itself still commits");
}

#[test]
fn maintenance_passes_run_reconcile_serialized_and_on_schedule() {
    let (context, repository) = maintenance_context();
    let maintenance = Arc::new(MaintenanceService::with_interval(context.clone(), Duration::from_millis(40)));
    assert!(maintenance.next_due().is_none(), "the first pass is due immediately");

    maintenance.run_once().unwrap();
    assert_eq!(repository.reconciles.load(SeqCst), 1, "run_once reconciles the storage once");
    let due = maintenance.next_due().expect("a fresh pass schedules the next one");
    assert!(due > Instant::now());

    // The job thread and an on-demand pass (eviction notice) never overlap:
    // maintenance has its own lock.
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let maintenance = Arc::clone(&maintenance);
            thread::spawn(move || maintenance.run_once().unwrap())
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(repository.widest.load(SeqCst), 1, "two maintenance passes overlapped");
    assert_eq!(repository.reconciles.load(SeqCst), 3);

    // The thread sleeps on the deadline instead of polling.
    maintenance.wait_until_due();
    assert!(maintenance.next_due().is_none(), "the interval elapsed: a pass is due");
}

/// Gate G1: the undo retry columns reached the schema file while the recorded
/// version still said 1, so a database created in that window lacks them. It
/// must gain the columns in place — rows kept — and work as a current database.
#[test]
fn a_v1_database_gains_the_undo_retry_columns() {
    let scratch = Scratch::new();
    {
        // The undo_actions table exactly as version 1 created it, with one row
        // written by the old code. The remaining tables are created by open().
        let raw = Connection::open(scratch.db()).unwrap();
        raw.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE meta (name TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta (name, value) VALUES ('schema_version', '1');
             CREATE TABLE undo_actions (
                 seq           INTEGER PRIMARY KEY,
                 id            INTEGER NOT NULL UNIQUE,
                 operation     TEXT NOT NULL,
                 subject       TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 expires_at_ms INTEGER NOT NULL,
                 status        TEXT NOT NULL,
                 error         TEXT
             );
             INSERT INTO undo_actions (seq, id, operation, subject, created_at_ms, expires_at_ms, status, error)
             VALUES (1, 401, '{\"kind\":\"mail\",\"email_id\":101,\"action\":\"star\",\"value\":\"on\"}',
                     'Starred', 1800000000000, 1800000005000, 'committing', NULL);",
        )
        .unwrap();
    }

    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let actions = repo.read().unwrap().undo_actions;
    assert_eq!(actions.len(), 1, "the migrated table keeps its row");
    assert_eq!(actions[0].id, 401);
    assert_eq!(actions[0].subject, "Starred");
    assert_eq!(actions[0].status, UndoStatus::Committing);
    assert_eq!(actions[0].operation, UndoOperation::Mail { email_id: 101, action: "star".into(), value: Some("on".into()) });
    assert_eq!(actions[0].retry_at_ms, None, "the added column defaults to no retry");
    assert_eq!(actions[0].attempts, 0, "the added column defaults to zero attempts");

    // Retry fields can now be written and read back, on the old row and a new one.
    repo.transaction(&mut |s| {
        let queued = s.undo_actions.first_mut().unwrap();
        queued.retry_at_ms = Some(1_800_000_060_000);
        queued.attempts = 3;
        s.undo_actions.push(UndoAction {
            id: 402,
            operation: UndoOperation::SendDraft { draft_id: 7 },
            subject: "Send".into(),
            created_at_ms: 2,
            expires_at_ms: 3,
            status: UndoStatus::Pending,
            error: None,
            retry_at_ms: None,
            attempts: 0,
        });
        Ok(())
    })
    .unwrap();
    let actions = repo.read().unwrap().undo_actions;
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].retry_at_ms, Some(1_800_000_060_000));
    assert_eq!(actions[0].attempts, 3);

    let version: String = Connection::open(scratch.db())
        .unwrap()
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "5", "the migration records the new version");

    // Reopening a migrated database is a no-op and the data survives.
    drop(repo);
    let again = SqliteStoreRepository::open(scratch.db()).unwrap();
    let actions = again.read().unwrap().undo_actions;
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].attempts, 3);
}

/// A database created while the retry columns were already in the schema file
/// but the version still read 1 (the other half of the G1 window): nothing to
/// alter, but the recorded version must still move to the current one.
#[test]
fn a_v1_database_that_already_has_the_columns_only_records_the_new_version() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let raw = Connection::open(scratch.db()).unwrap();
    raw.execute("UPDATE meta SET value = '1' WHERE name = 'schema_version'", []).unwrap();
    drop(raw);
    drop(repo);
    let again = SqliteStoreRepository::open(scratch.db()).unwrap();
    again
        .transaction(&mut |s| {
            s.undo_actions.push(UndoAction {
                id: 1,
                operation: UndoOperation::DiscardDraft { draft_id: 2 },
                subject: "Discard".into(),
                created_at_ms: 0,
                expires_at_ms: 0,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: Some(5),
                attempts: 1,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(again.read().unwrap().undo_actions[0].retry_at_ms, Some(5));
    let version: String = Connection::open(scratch.db())
        .unwrap()
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "5");
}

#[test]
fn in_reply_to_survives_a_save_and_a_read() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let mut store = sample_store();
    store.emails[0].in_reply_to = "parent@x".into();
    save(&repo, &store);
    assert_eq!(repo.read().unwrap().emails[0].in_reply_to, "parent@x");
}

/// ADR-0010 INV-006: version 4 adds the direct parent. Received mail already
/// carries it in `thread_id` (the In-Reply-To header), so it is filled from
/// there; a reply sent by Qamille copied its parent's thread, one level too
/// high, so its parent stays unknown instead of being guessed.
#[test]
fn a_v3_database_gains_in_reply_to_for_received_mail_only() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let mail = |id: u64, folder: &str, message_id: &str| Email {
        id,
        account_id: 1,
        folder: folder.into(),
        message_id: message_id.into(),
        thread_id: "<root@x>".into(),
        ..Default::default()
    };
    let mut store = sample_store();
    store.emails = vec![
        mail(1, "Inbox", "<r1@x>"),
        mail(2, "Inbox", "<root@x>"),
        mail(3, "Sent", "<s1@x>"),
    ];
    save(&repo, &store);
    drop(repo);

    // The table exactly as version 3 had it: no in_reply_to column.
    let raw = Connection::open(scratch.db()).unwrap();
    raw.execute("ALTER TABLE emails DROP COLUMN in_reply_to", []).unwrap();
    raw.execute("UPDATE meta SET value = '3' WHERE name = 'schema_version'", []).unwrap();
    drop(raw);

    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let emails = repo.read().unwrap().emails;
    let parent = |id: u64| {
        let found = emails.iter().find(|e| e.id == id);
        let ids: Vec<u64> = emails.iter().map(|e| e.id).collect();
        found.unwrap_or_else(|| panic!("email {id} was not read back; the database holds {ids:?}")).in_reply_to.clone()
    };
    assert_eq!(parent(1), "root@x", "a received reply takes its parent from thread_id");
    assert_eq!(parent(2), "", "a received root has no parent");
    assert_eq!(parent(3), "", "a sent reply's parent is unknown, never guessed");

    let version: String = Connection::open(scratch.db())
        .unwrap()
        .query_row("SELECT value FROM meta WHERE name = 'schema_version'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, "5");
}

#[test]
fn the_shared_snapshot_is_reused_until_any_process_writes() {
    let scratch = Scratch::new();
    let first = SqliteStoreRepository::open(scratch.db()).unwrap();
    first.transaction(&mut |store| { *store = sample_store(); Ok(()) }).unwrap();
    let a = first.read_shared().unwrap();
    let b = first.read_shared().unwrap();
    assert!(Arc::ptr_eq(&a, &b), "no write in between: the same snapshot");
    let id = a.emails[0].id;
    first.transaction(&mut |store| { store.emails[0].starred = !store.emails[0].starred; Ok(()) }).unwrap();
    let c = first.read_shared().unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_ne!(c.email(id).unwrap().starred, a.email(id).unwrap().starred, "this process's own write");
    // Another process (a second repository on the same file) writes: the snapshot is stale.
    let second = SqliteStoreRepository::open(scratch.db()).unwrap();
    second.transaction(&mut |store| { store.emails[0].subject = "From the other process".into(); Ok(()) }).unwrap();
    assert_eq!(first.read_shared().unwrap().email(id).unwrap().subject, "From the other process");
    // A write of the first process now starts from the other process's commit (INV-8).
    first.transaction(&mut |store| { store.emails[0].pinned = true; Ok(()) }).unwrap();
    let after = second.read().unwrap();
    assert_eq!(after.email(id).unwrap().subject, "From the other process");
    assert!(after.email(id).unwrap().pinned);
    let unchanged = first.read_shared().unwrap();
    first.transaction(&mut |_| Ok(())).unwrap();
    assert!(Arc::ptr_eq(&unchanged, &first.read_shared().unwrap()), "an empty write moves nothing");
}

#[test]
fn derived_quota_usage_matches_the_measured_one() {
    use qamille::domain::storage::{derived_usage, diff_emails, usage};
    let mut before = sample_store();
    for account in &mut before.accounts {
        account.storage_policy = AccountStoragePolicy::default();
    }
    let previous = before.accounts.iter().map(|a| (a.id, usage(&before, a.id).unwrap().used_bytes)).collect();
    let mut after = before.clone();
    after.emails[0].read = !after.emails[0].read;
    after.emails[1].labels.push("Nuova etichetta con un nome lungo".into());
    let mut added = after.emails[2].clone();
    added.id = 9_999;
    added.attachments.iter_mut().for_each(|a| a.path = None);
    after.emails.push(added);
    let diff = diff_emails(&before.emails, &after.emails);
    assert_eq!(diff.changed.len(), 3);
    let derived = derived_usage(&before, &after, &diff, &previous).expect("a local change is derived");
    for account in &after.accounts {
        assert_eq!(derived[&account.id], usage(&after, account.id).unwrap().used_bytes, "account {}", account.id);
    }
    let mut removed = before.clone();
    removed.emails.remove(0);
    assert!(derived_usage(&before, &removed, &diff_emails(&before.emails, &removed.emails), &previous).is_none(), "removals are measured");
}

#[test]
fn remote_metadata_upserts_insert_once_and_keep_local_state() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    repo.transaction(&mut |store| {
        store.accounts.push(Account { id: 1, email: "me@example.com".into(), ..Default::default() });
        store.emails.push(Email { id: 1, account_id: 1, remote_global_id: Some(7), pinned: true, subject: "Known".into(), ..Default::default() });
        store.next_id = 1;
        Ok(())
    })
    .unwrap();
    let row = |n: u64| Email { remote_global_id: Some(n), uid: Some(n as u32), remote_folder: Some("[Gmail]/All".into()), remote_uid_validity: Some(3), subject: format!("Old {n}"), labels: vec!["Clienti".into()], ..Default::default() };
    let rows: Vec<Email> = (7..507).map(row).collect();
    let first = repo.upsert_remote_metadata(1, rows.clone()).unwrap();
    assert_eq!((first.inserted.len(), first.enriched), (499, vec![1]));
    let generation = repo.read_shared().unwrap();
    let again = repo.upsert_remote_metadata(1, rows).unwrap();
    assert_eq!((again.inserted.len(), again.enriched.len()), (0, 500), "the same batch adds nothing");
    assert!(Arc::ptr_eq(&generation, &repo.read_shared().unwrap()), "and writes nothing");
    let store = repo.read().unwrap();
    assert_eq!(store.emails.len(), 500);
    let known = store.email(1).unwrap();
    assert!(known.pinned && known.subject == "Known", "local state survives");
    assert_eq!(known.labels, ["Clienti"], "labels it never had are filled");
    assert!(store.emails.iter().all(|e| e.remote_only || e.id == 1));
}

#[test]
fn a_v4_database_gains_the_history_cursor_of_its_accounts() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &sample_store());
    drop(repo);
    let raw = Connection::open(scratch.db()).unwrap();
    raw.execute("ALTER TABLE accounts DROP COLUMN history", []).unwrap();
    raw.execute("UPDATE meta SET value = '4' WHERE name = 'schema_version'", []).unwrap();
    drop(raw);
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    let store = repo.read().unwrap();
    assert!(!store.accounts.is_empty());
    assert!(store.accounts.iter().all(|a| a.history == Default::default()), "not started");
    repo.transaction(&mut |store| { store.accounts[0].history.fetched = 12; Ok(()) }).unwrap();
    assert_eq!(repo.read().unwrap().accounts[0].history.fetched, 12);
}

fn stored_generation(scratch: &Scratch) -> i64 {
    Connection::open(scratch.db())
        .unwrap()
        .query_row("SELECT value FROM meta WHERE name = 'generation'", [], |row| row.get::<_, String>(0))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn a_write_on_listed_emails_commits_like_any_write_and_rolls_back_in_memory() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));
    // Another process on the same file: it sees only what was committed.
    let other = SqliteStoreRepository::open(scratch.db()).unwrap();
    let head = repo.change_feed_head().unwrap();

    // One message and a row of the queue change: committed and announced.
    repo.transaction_on_emails(&[103], &mut |s| {
        s.email_mut(103).unwrap().starred = true;
        s.undo_actions.retain(|a| a.id != 401);
        Ok(())
    })
    .unwrap();
    let committed = other.read().unwrap();
    assert!(committed.email(103).unwrap().starred);
    assert!(committed.undo_actions.iter().all(|a| a.id != 401));
    assert_eq!(json(&repo.read().unwrap()), json(&committed), "the snapshot is what was committed");
    let feed = repo.poll_change_feed_from(head, 0).unwrap();
    assert!(feed.iter().any(|row| row.kind == "email-changed" && row.ids == [103]), "{feed:?}");
    assert!(feed.iter().any(|row| row.kind == "undo-changed" && row.ids == [401]), "{feed:?}");

    // A refused operation leaves no trace, in the database or in memory.
    let before = json(&repo.read().unwrap());
    let error = repo
        .transaction_on_emails(&[103, 101], &mut |s| {
            s.email_mut(101).unwrap().subject = "Refused".into();
            s.emails.retain(|e| e.id != 103);
            s.settings.user_name = "Refused".into();
            s.undo_actions.clear();
            s.next_id += 5;
            Err(qamille::application::AppError::validation("refused"))
        })
        .unwrap_err();
    assert_eq!(error.message, "refused");
    assert_eq!(json(&repo.read().unwrap()), before, "the snapshot was restored");
    assert_eq!(json(&other.read().unwrap()), before);

    // Reader-only HTML is not a change: nothing written, nothing kept.
    let generation = stored_generation(&scratch);
    repo.transaction_on_emails(&[103], &mut |s| {
        s.email_mut(103).unwrap().body_html = "<p>reader only</p>".into();
        Ok(())
    })
    .unwrap();
    assert_eq!(stored_generation(&scratch), generation, "an empty write moves nothing");
    assert!(repo.read().unwrap().email(103).unwrap().body_html.is_empty());

    // Removing the listed message, then adding one: written like a full write.
    repo.transaction_on_emails(&[102], &mut |s| {
        s.emails.retain(|e| e.id != 102);
        Ok(())
    })
    .unwrap();
    repo.transaction_on_emails(&[], &mut |s| {
        s.emails.push(Email { id: 9_000, account_id: 1, subject: "Added".into(), ..Default::default() });
        Ok(())
    })
    .unwrap();
    let committed = other.read().unwrap();
    assert!(committed.email(102).is_none());
    assert_eq!(committed.email(9_000).unwrap().subject, "Added");
    assert_eq!(json(&repo.read().unwrap()), json(&committed));
}

#[test]
fn a_write_on_listed_emails_starts_from_another_process_commit() {
    let scratch = Scratch::new();
    let first = SqliteStoreRepository::open(scratch.db()).unwrap();
    first.transaction(&mut |store| { *store = sample_store(); Ok(()) }).unwrap();
    let id = first.read_shared().unwrap().emails[0].id;
    let second = SqliteStoreRepository::open(scratch.db()).unwrap();
    second.transaction(&mut |store| { store.emails[0].subject = "From the other process".into(); Ok(()) }).unwrap();
    // INV-8: the snapshot of the first process is stale, so the write starts
    // from the database and loses nothing the other process committed.
    first.transaction_on_emails(&[id], &mut |store| { store.email_mut(id).unwrap().pinned = true; Ok(()) }).unwrap();
    let after = second.read().unwrap();
    assert_eq!(after.email(id).unwrap().subject, "From the other process");
    assert!(after.email(id).unwrap().pinned);
    assert_eq!(json(&first.read().unwrap()), json(&after));
}

#[test]
fn a_write_on_listed_emails_never_shows_a_reader_half_a_change() {
    let scratch = Scratch::new();
    let repo = Arc::new(SqliteStoreRepository::open(scratch.db()).unwrap());
    save(&repo, &full_store(&scratch.data()));
    // A reader that holds the snapshot keeps it as it was: the write copies.
    let held = repo.read_shared().unwrap();
    repo.transaction_on_emails(&[103], &mut |s| { s.email_mut(103).unwrap().subject = "First".into(); Ok(()) }).unwrap();
    assert_ne!(held.email(103).unwrap().subject, "First");
    assert_eq!(repo.read_shared().unwrap().email(103).unwrap().subject, "First");
    drop(held);
    // While an operation changes the snapshot in place, a reader on another
    // thread waits for the write instead of seeing it half done.
    let (started_tx, started_rx) = mpsc::channel();
    let writer = {
        let repo = Arc::clone(&repo);
        thread::spawn(move || {
            repo.transaction_on_emails(&[103], &mut |s| {
                s.email_mut(103).unwrap().subject = "Half".into();
                started_tx.send(()).unwrap();
                thread::sleep(Duration::from_millis(300));
                s.email_mut(103).unwrap().subject = "Second".into();
                Ok(())
            })
            .unwrap();
        })
    };
    started_rx.recv().unwrap();
    let seen = repo.read_shared().unwrap().email(103).unwrap().subject.clone();
    writer.join().unwrap();
    assert!(seen == "First" || seen == "Second", "never the half-done change: {seen}");
    assert_eq!(repo.read().unwrap().email(103).unwrap().subject, "Second");
}

#[test]
fn a_write_on_listed_emails_enforces_the_quota_like_any_write() {
    let run = |listed: bool| {
        let scratch = Scratch::new();
        let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
        let write = |op: &mut dyn FnMut(&mut Store) -> qamille::application::AppResult<()>| {
            if listed { repo.transaction_on_emails(&[10], op) } else { repo.transaction(op) }
        };
        repo.transaction(&mut |s| { *s = quota_store(); Ok(()) }).unwrap();
        repo.transaction(&mut |s| {
            s.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 8_000 };
            Ok(())
        })
        .unwrap();
        // The content back on the one message: over the limit.
        let refilled = write(&mut |s| {
            s.emails[0].body = "y".repeat(20_000);
            s.emails[0].remote_only = false;
            s.emails[0].storage_evicted = false;
            Ok(())
        })
        .err()
        .map(|error| error.code.to_string());
        let refilled_state = json(&repo.read().unwrap());
        // A change that protected data cannot fit in is refused.
        let refused = write(&mut |s| {
            s.accounts[0].storage_policy.limit = StorageLimit::Limited { max_bytes: 100 };
            s.emails[0].body = "z".repeat(50_000);
            s.emails[0].remote_only = false;
            s.emails[0].storage_evicted = false;
            s.drafts.push(Draft { id: 99, account_id: 1, reply_to: Some(10), body: "d".repeat(20_000), ..Default::default() });
            Ok(())
        })
        .err()
        .map(|error| error.code.to_string());
        let after = json(&repo.read().unwrap());
        let reopened = json(&SqliteStoreRepository::open(scratch.db()).unwrap().read().unwrap());
        assert_eq!(after, reopened, "the snapshot is what was committed");
        (refilled, refilled_state, refused, after)
    };
    let in_place = run(true);
    assert!(in_place.2.is_some(), "the limit is enforced");
    assert_eq!(in_place, run(false));
}

#[cfg(debug_assertions)]
#[test]
fn a_write_that_changes_an_unlisted_email_panics_in_debug_builds_and_stores_nothing() {
    let scratch = Scratch::new();
    let repo = SqliteStoreRepository::open(scratch.db()).unwrap();
    save(&repo, &full_store(&scratch.data()));
    let before = json(&repo.read().unwrap());
    let broken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        repo.transaction_on_emails(&[103], &mut |s| {
            s.email_mut(101).unwrap().subject = "Not listed".into();
            Ok(())
        })
    }));
    let panic = broken.expect_err("the broken promise is caught");
    assert!(panic.downcast_ref::<String>().is_some_and(|message| message.contains("did not list")));
    assert_eq!(json(&repo.read().unwrap()), before, "nothing committed, the changed snapshot dropped");
    repo.transaction_on_emails(&[103], &mut |s| { s.email_mut(103).unwrap().pinned = true; Ok(()) }).unwrap();
    assert!(repo.read().unwrap().email(103).unwrap().pinned, "and the store still writes");
}
