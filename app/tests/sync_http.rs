//! The HTTP adapter of the mailbox synchronization against a scripted server, and the state file.
#[path = "support/http_fake.rs"]
mod http_fake;

use http_fake::{Fake, Reply, reply};
use qamille::{
    application::{
        server_account::RemoteEndpoint,
        sync::{PutOutcome, SyncServer, SyncState, SyncStateStore},
    },
    domain::sync::{CREDENTIAL_GOOGLE, SyncAccount, SyncCredential, SyncItem, item_id},
    infrastructure::{sync_http::HttpSyncServer, sync_state::FileSyncState},
};
use std::time::Duration;

fn endpoint(url: &str) -> RemoteEndpoint {
    RemoteEndpoint { url: url.into(), device_key: "spk_dev_secret-device-key".into(), ca_pem: None, timeout: Duration::from_secs(5) }
}
fn item() -> SyncItem {
    SyncItem {
        schema: 1,
        kind: "mail_account".into(),
        updated_at: 10,
        credential_updated_at: 11,
        account: SyncAccount {
            provider_id: "gmail".into(),
            name: "Giulia".into(),
            email: "giulia@gmail.com".into(),
            color: [1, 2, 3],
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            username: "giulia@gmail.com".into(),
            oauth_client_id: "c.apps.googleusercontent.com".into(),
            archive_folder: "[Gmail]/All Mail".into(),
            trash_folder: "[Gmail]/Trash".into(),
        },
        credential: SyncCredential { kind: CREDENTIAL_GOOGLE.into(), value: "1//refresh-secret".into(), client_secret: None },
        mail_signature: String::new(),
    }
}
fn id() -> String {
    item_id("gmail", "giulia@gmail.com")
}
fn item_json(version: u64, deleted: bool) -> String {
    if deleted {
        format!(r#"{{"id":"{}","version":{version},"deleted":true,"updated_at":"x","updated_by_device":"d"}}"#, id())
    } else {
        format!(r#"{{"id":"{}","version":{version},"deleted":false,"updated_at":"x","updated_by_device":"d","item":{}}}"#, id(), serde_json::to_string(&item()).unwrap())
    }
}

#[test]
fn the_requests_go_to_the_endpoint_with_the_bearer_key_and_the_exact_body() {
    let fake = Fake::start(vec![
        reply(200, &format!(r#"{{"version":5,"epoch":2,"items":[{},{}]}}"#, item_json(4, false), item_json(5, true))),
        reply(200, &format!(r#"{{"id":"{}","version":6,"updated_at":"x"}}"#, id())),
        reply(200, &format!(r#"{{"id":"{}","version":7,"deleted":true}}"#, id())),
    ]);
    let ep = endpoint(&fake.url);
    let listing = HttpSyncServer.list(&ep, 3).unwrap();
    assert_eq!((listing.version, listing.epoch, listing.items.len()), (5, 2, 2));
    assert_eq!(listing.items[0].item.as_ref().unwrap().credential.value, "1//refresh-secret");
    assert!(listing.items[1].deleted && listing.items[1].item.is_none());
    assert_eq!(HttpSyncServer.put(&ep, &id(), 4, &item()).unwrap(), PutOutcome::Written { version: 6 });
    HttpSyncServer.delete(&ep, &id(), Some(6)).unwrap();
    let seen = fake.requests();
    assert_eq!(seen[0].request_line, "GET /v1/sync/items?since=3 HTTP/1.1");
    assert_eq!(seen[1].request_line, format!("PUT /v1/sync/items/{} HTTP/1.1", id()));
    assert_eq!(seen[2].request_line, format!("DELETE /v1/sync/items/{}?base_version=6 HTTP/1.1", id()));
    for request in &seen {
        assert_eq!(request.header("authorization"), Some("Bearer spk_dev_secret-device-key"));
    }
    let body: serde_json::Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(body["base_version"], 4);
    assert_eq!(body["item"]["credential"]["value"], "1//refresh-secret");
    assert_eq!(body.as_object().unwrap().len(), 2, "only base_version and item");
}

#[test]
fn a_redirect_is_not_followed_and_nothing_reaches_the_other_address() {
    let other = Fake::start(vec![reply(200, "{}")]);
    let redirect = Reply { status: 307, headers: vec![("Location".into(), format!("{}/v1/sync/items/x", other.url))], body: String::new() };
    let fake = Fake::start(vec![redirect]);
    let error = HttpSyncServer.put(&endpoint(&fake.url), &id(), 0, &item()).unwrap_err();
    assert_eq!(error.code, "server_error");
    assert!(other.requests().is_empty(), "the credentials did not follow the redirect");
    assert!(!format!("{error:?}").contains("refresh-secret") && !format!("{error:?}").contains("spk_dev_"));
}

#[test]
fn errors_map_to_fixed_codes_without_echoing_the_body() {
    let cases = [
        (401, "unauthorized", "unauthorized"),
        (402, "subscription_required", "subscription_required"),
        (409, "sync_limit", "sync_limit"),
        (413, "payload_too_large", "payload_too_large"),
        (429, "rate_limited", "rate_limited"),
        (404, "not_found", "not_found"),
        (400, "tls_required", "tls_required"),
        (400, "invalid_request", "invalid_request"),
        (500, "internal", "server_error"),
    ];
    for (status, code, expected) in cases {
        let body = format!(r#"{{"error":{{"code":"{code}","message":"messaggio con 1//refresh-secret","retry_after_s":null}}}}"#);
        let fake = Fake::start(vec![reply(status, &body)]);
        let error = HttpSyncServer.put(&endpoint(&fake.url), &id(), 0, &item()).unwrap_err();
        assert_eq!(error.code, expected, "{status} {code}");
        assert!(!error.message.contains("refresh-secret"), "{}", error.message);
    }
    // Nothing listens.
    let error = HttpSyncServer.list(&endpoint("http://127.0.0.1:1"), 0).unwrap_err();
    assert_eq!(error.code, "unreachable");
    // JSON that is not an answer.
    let fake = Fake::start(vec![reply(200, "not json")]);
    assert_eq!(HttpSyncServer.list(&endpoint(&fake.url), 0).unwrap_err().code, "server_error");
}

#[test]
fn a_409_conflict_carries_the_current_item_or_null() {
    let with = format!(r#"{{"error":{{"code":"sync_conflict","message":"x","retry_after_s":null,"current":{}}}}}"#, item_json(9, false));
    let fake = Fake::start(vec![reply(409, &with), reply(409, r#"{"error":{"code":"sync_conflict","message":"x","retry_after_s":null,"current":null}}"#)]);
    let ep = endpoint(&fake.url);
    match HttpSyncServer.put(&ep, &id(), 1, &item()).unwrap() {
        PutOutcome::Conflict(Some(current)) => assert_eq!((current.version, current.deleted), (9, false)),
        other => panic!("{other:?}"),
    }
    assert_eq!(HttpSyncServer.put(&ep, &id(), 1, &item()).unwrap(), PutOutcome::Conflict(None));
}

#[test]
fn an_id_that_is_not_an_item_id_never_reaches_the_url() {
    let fake = Fake::start(vec![]);
    for bad in ["../x", "ABC", "", &"a".repeat(65), "a/b"] {
        assert_eq!(HttpSyncServer.put(&endpoint(&fake.url), bad, 0, &item()).unwrap_err().code, "invalid_request");
        assert_eq!(HttpSyncServer.delete(&endpoint(&fake.url), bad, None).unwrap_err().code, "invalid_request");
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn a_listing_larger_than_one_mib_is_read() {
    let big = "x".repeat(2 << 20);
    let body = format!(r#"{{"version":1,"epoch":0,"items":[{}],"padding":"{big}"}}"#, item_json(1, false));
    let fake = Fake::start(vec![reply(200, &body)]);
    assert_eq!(HttpSyncServer.list(&endpoint(&fake.url), 0).unwrap().items.len(), 1);
}

#[test]
fn an_item_of_a_future_schema_is_kept_as_no_content() {
    let body = format!(r#"{{"version":1,"epoch":0,"items":[{{"id":"{}","version":1,"deleted":false,"item":{{"schema":99}}}}]}}"#, id());
    let fake = Fake::start(vec![reply(200, &body)]);
    let listing = HttpSyncServer.list(&endpoint(&fake.url), 0).unwrap();
    assert!(listing.items[0].item.is_none() && !listing.items[0].deleted);
}

#[test]
fn the_state_file_is_written_atomically_and_a_damaged_one_reads_as_empty() {
    let dir = std::env::temp_dir().join(format!("qamille-sync-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = FileSyncState::new(&dir);
    assert_eq!(store.load().unwrap(), SyncState::default());
    let mut state = SyncState { url: "http://x".into(), since: 9, ..Default::default() };
    state.disabled.insert(id());
    store.save(&state).unwrap();
    assert_eq!(store.load().unwrap(), state);
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    assert_eq!(names, ["mailbox-sync.json"], "no temporary file is left");
    for damaged in ["", "{", "[1,2]", "\u{0}\u{0}"] {
        std::fs::write(dir.join("mailbox-sync.json"), damaged).unwrap();
        assert_eq!(store.load().unwrap(), SyncState::default(), "{damaged:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_state_file_gives_back_the_previous_save_and_an_unreadable_one_is_an_error() {
    let dir = std::env::temp_dir().join(format!("qamille-sync-state-bak-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = FileSyncState::new(&dir);
    let mut first = SyncState { url: "http://x".into(), since: 1, ..Default::default() };
    first.disabled.insert(id());
    store.save(&first).unwrap();
    let second = SyncState { since: 2, ..first.clone() };
    store.save(&second).unwrap();
    assert_eq!(store.load().unwrap(), second);
    // The newest file is damaged: the switches of the person are not lost with it.
    std::fs::write(dir.join("mailbox-sync.json"), "{\"url\":").unwrap();
    assert_eq!(store.load().unwrap(), first);
    // A file that cannot be read is an error, not «empty» (the next save would erase the switches).
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("mailbox-sync.json")).unwrap();
    assert!(store.load().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
