//! L'archivio degli elementi di sincronizzazione: migrazione, versioni, conflitti, limiti,
//! segnaposto, isolamento tra utenti.
#![allow(dead_code)]

mod support;

use chrono::Duration;
use qamille_server::accounts::LoginOk;
use qamille_server::db::{CORE_MIGRATIONS, Db, SCHEMA_V1, SCHEMA_FORMAT};
use qamille_server::sync::SyncKey;
use qamille_server::sync::store::{self, DeleteOutcome, Limits, PutOutcome, StoreError, Who};
use rusqlite::Connection;
use serde_json::value::RawValue;
use support::sync_client;
use support::{TempDir, TestServer, test_config, user_with_device};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_KEY: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn id(n: u32) -> String {
    format!("{n:064x}")
}

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).unwrap()
}

fn limits() -> Limits {
    Limits { max_items: 200, max_item_bytes: 64 * 1024 }
}

struct Fx {
    ts: TestServer,
    key: SyncKey,
    mario: LoginOk,
    anna: LoginOk,
}

async fn fixture() -> Fx {
    let ts = TestServer::new(test_config());
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let anna = user_with_device(&ts, "anna", "una password lunga").await;
    Fx { ts, key: SyncKey::parse(KEY).unwrap(), mario, anna }
}

impl Fx {
    async fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> T + Send + 'static) -> T {
        self.ts.accounts().db().blocking(move |conn| Ok(f(conn))).await.unwrap()
    }

    async fn put(&self, who: &LoginOk, item_id: u32, base: u64, text: &str) -> Result<PutOutcome, StoreError> {
        self.put_with(who, item_id, base, text, limits()).await
    }

    async fn put_with(&self, who: &LoginOk, item_id: u32, base: u64, text: &str, limits: Limits) -> Result<PutOutcome, StoreError> {
        let (key, user, device, dk, now) = (self.key.clone(), who.user_id.clone(), who.device_id.clone(), sync_client::key_sha256(&who.device_key), self.ts.accounts().now());
        let (item_id, item) = (id(item_id), raw(text));
        self.run(move |c| store::put(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, &item_id, base, &item, limits, now)).await
    }

    async fn delete(&self, who: &LoginOk, item_id: u32, base: Option<u64>) -> Result<DeleteOutcome, StoreError> {
        self.delete_with(who, item_id, base, limits()).await
    }

    async fn delete_with(&self, who: &LoginOk, item_id: u32, base: Option<u64>, limits: Limits) -> Result<DeleteOutcome, StoreError> {
        let (key, user, device, dk, now) = (self.key.clone(), who.user_id.clone(), who.device_id.clone(), sync_client::key_sha256(&who.device_key), self.ts.accounts().now());
        let item_id = id(item_id);
        self.run(move |c| store::delete(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, &item_id, base, limits, now)).await
    }

    async fn list(&self, who: &LoginOk, since: u64) -> Result<store::Listing, StoreError> {
        let (key, user, device, dk) = (self.key.clone(), who.user_id.clone(), who.device_id.clone(), sync_client::key_sha256(&who.device_key));
        self.run(move |c| store::list(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, since)).await
    }
}

fn written(outcome: Result<PutOutcome, StoreError>) -> u64 {
    match outcome.expect("scrittura") {
        PutOutcome::Written { version, .. } => version,
        PutOutcome::Conflict(_) => panic!("conflitto inatteso"),
    }
}

// --- Migrazione ---------------------------------------------------------------------

#[tokio::test]
async fn the_migration_is_named_and_creates_the_sync_tables() {
    let fx = fixture().await;
    let names: Vec<String> = fx
        .run(|c| {
            let mut stmt = c.prepare("SELECT name FROM schema_migrations ORDER BY name").unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        })
        .await;
    assert_eq!(names, ["core-001-users", "core-002-usage", "core-003-llm-usage", "core-004-sync"]);
    let tables: i64 = fx
        .run(|c| c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('sync_state', 'sync_items')", [], |r| r.get(0)).unwrap())
        .await;
    assert_eq!(tables, 2);
    let version: i64 = fx.run(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap()).await;
    assert_eq!(version, SCHEMA_FORMAT);
    assert_eq!(SCHEMA_FORMAT, 6);
}

#[test]
fn an_old_file_gets_the_sync_tables_and_keeps_its_users() {
    let dir = TempDir::new();
    let path = dir.path().join("server.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute("INSERT INTO users (id, username, created_at) VALUES ('u1', 'mario', '2026-11-01T00:00:00.000Z')", []).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let db = Db::open(&path, CORE_MIGRATIONS).unwrap();
    let conn = Connection::open(&path).unwrap();
    drop(db);
    let users: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)).unwrap();
    let last: String = conn.query_row("SELECT name FROM schema_migrations ORDER BY name DESC LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!((users, last.as_str()), (1, "core-004-sync"));
    conn.execute("INSERT INTO sync_state (user_id) VALUES ('u1')", []).unwrap();
}

#[tokio::test]
async fn the_item_id_check_and_the_ciphertext_check_are_in_the_schema() {
    let fx = fixture().await;
    let user = fx.mario.user_id.clone();
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>, i64)> = vec![
        ("ab".repeat(31), Some(vec![0; 12]), Some(vec![1; 20]), 0),
        ("AB".repeat(32), Some(vec![0; 12]), Some(vec![1; 20]), 0),
        ("ab".repeat(32), Some(vec![0; 5]), Some(vec![1; 20]), 0),
        ("ab".repeat(32), None, None, 0),
        ("ab".repeat(32), Some(vec![0; 12]), Some(vec![1; 20]), 1),
    ];
    for (item_id, nonce, ciphertext, deleted) in rows {
        let user = user.clone();
        let result = fx
            .run(move |c| {
                c.execute(
                    "INSERT INTO sync_items (user_id, item_id, version, nonce, ciphertext, deleted, updated_at, updated_by_device)
                     VALUES (?1, ?2, 1, ?3, ?4, ?5, 'x', 'd')",
                    rusqlite::params![user, item_id, nonce, ciphertext, deleted],
                )
            })
            .await;
        assert!(result.is_err(), "il CHECK doveva rifiutare la riga");
    }
}

// --- Versioni, conflitti, limiti ----------------------------------------------------

#[tokio::test]
async fn a_new_item_has_version_one_and_each_write_bumps_the_user_counter() {
    let fx = fixture().await;
    assert_eq!(written(fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await), 1);
    assert_eq!(written(fx.put(&fx.mario, 2, 0, r#"{"b":1}"#).await), 2);
    assert_eq!(written(fx.put(&fx.mario, 1, 1, r#"{"a":2}"#).await), 3);
    let all = fx.list(&fx.mario, 0).await.unwrap();
    assert_eq!(all.version, 3);
    assert_eq!(all.epoch, 0);
    let versions: Vec<u64> = all.items.iter().map(|i| i.version).collect();
    assert_eq!(versions, [2, 3]);
    assert_eq!(all.items[1].item.as_ref().unwrap().get(), r#"{"a":2}"#);
}

#[tokio::test]
async fn a_stale_base_version_is_a_conflict_with_the_current_item_and_writes_nothing() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    fx.put(&fx.mario, 1, 1, r#"{"a":2}"#).await.unwrap();
    fx.put(&fx.mario, 1, 2, r#"{"a":3}"#).await.unwrap();
    match fx.put(&fx.mario, 1, 1, r#"{"a":9}"#).await.unwrap() {
        PutOutcome::Conflict(Some(current)) => {
            assert_eq!(current.version, 3);
            assert_eq!(current.item.unwrap().get(), r#"{"a":3}"#);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().version, 3);
}

#[tokio::test]
async fn a_new_item_over_an_existing_one_is_a_conflict() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    assert!(matches!(fx.put(&fx.mario, 1, 0, r#"{"a":2}"#).await.unwrap(), PutOutcome::Conflict(Some(_))));
}

#[tokio::test]
async fn a_version_claimed_for_an_item_that_does_not_exist_is_a_conflict_without_current() {
    let fx = fixture().await;
    assert!(matches!(fx.put(&fx.mario, 1, 7, r#"{"a":1}"#).await.unwrap(), PutOutcome::Conflict(None)));
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().version, 0);
}

#[tokio::test]
async fn list_since_returns_only_newer_versions_including_tombstones() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    fx.put(&fx.mario, 2, 0, r#"{"b":1}"#).await.unwrap();
    assert!(matches!(fx.delete(&fx.mario, 1, None).await.unwrap(), DeleteOutcome::Deleted { version: 3 }));
    let page = fx.list(&fx.mario, 2).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].deleted && page.items[0].item.is_none());
    assert_eq!(page.items[0].id, id(1));
    let empty = fx.list(&fx.mario, 3).await.unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.version, 3);
}

#[tokio::test]
async fn delete_is_idempotent_missing_is_404_and_a_stale_base_is_a_conflict() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    assert!(matches!(fx.delete(&fx.mario, 9, None).await.unwrap(), DeleteOutcome::Missing));
    assert!(matches!(fx.delete(&fx.mario, 1, Some(5)).await.unwrap(), DeleteOutcome::Conflict(_)));
    assert!(matches!(fx.delete(&fx.mario, 1, Some(1)).await.unwrap(), DeleteOutcome::Deleted { version: 2 }));
    assert!(matches!(fx.delete(&fx.mario, 1, None).await.unwrap(), DeleteOutcome::AlreadyDeleted { version: 2 }));
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().version, 2, "il secondo delete non scrive");
}

#[tokio::test]
async fn a_deleted_item_is_recreated_with_the_tombstone_version() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    fx.delete(&fx.mario, 1, None).await.unwrap();
    match fx.put(&fx.mario, 1, 0, r#"{"a":2}"#).await.unwrap() {
        PutOutcome::Conflict(Some(current)) => assert!(current.deleted && current.version == 2),
        other => panic!("{other:?}"),
    }
    assert_eq!(written(fx.put(&fx.mario, 1, 2, r#"{"a":2}"#).await), 3);
    let page = fx.list(&fx.mario, 0).await.unwrap();
    assert!(!page.items[0].deleted);
}

#[tokio::test]
async fn the_limit_counts_only_live_items_and_an_update_always_passes() {
    let fx = fixture().await;
    let small = Limits { max_items: 2, max_item_bytes: 4096 };
    fx.put_with(&fx.mario, 1, 0, "{}", small).await.unwrap();
    fx.put_with(&fx.mario, 2, 0, "{}", small).await.unwrap();
    assert!(matches!(fx.put_with(&fx.mario, 3, 0, "{}", small).await, Err(StoreError::LimitReached)));
    assert_eq!(written(fx.put_with(&fx.mario, 1, 1, r#"{"a":1}"#, small).await), 3);
    fx.delete_with(&fx.mario, 2, None, small).await.unwrap();
    assert_eq!(written(fx.put_with(&fx.mario, 3, 0, "{}", small).await), 5);
    // L'altro utente ha il suo limite.
    fx.put_with(&fx.anna, 1, 0, "{}", small).await.unwrap();
}

#[tokio::test]
async fn the_size_and_the_shape_of_the_item_are_checked_first() {
    let fx = fixture().await;
    let small = Limits { max_items: 10, max_item_bytes: 1024 };
    let exact = format!(r#"{{"x":"{}"}}"#, "a".repeat(1024 - 8));
    assert_eq!(exact.len(), 1024);
    assert_eq!(written(fx.put_with(&fx.mario, 1, 0, &exact, small).await), 1);
    let big = format!(r#"{{"x":"{}"}}"#, "a".repeat(1024 - 7));
    assert!(matches!(fx.put_with(&fx.mario, 2, 0, &big, small).await, Err(StoreError::TooLarge)));
    for not_object in ["[]", "7", r#""x""#, "null"] {
        assert!(matches!(fx.put(&fx.mario, 3, 0, not_object).await, Err(StoreError::NotObject)), "{not_object}");
    }
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().version, 1);
}

// --- Isolamento ---------------------------------------------------------------------

#[tokio::test]
async fn two_users_with_the_same_item_id_do_not_meet() {
    let fx = fixture().await;
    assert_eq!(written(fx.put(&fx.mario, 1, 0, r#"{"di":"mario"}"#).await), 1);
    assert_eq!(written(fx.put(&fx.anna, 1, 0, r#"{"di":"anna"}"#).await), 1);
    assert_eq!(written(fx.put(&fx.anna, 1, 1, r#"{"di":"anna2"}"#).await), 2);
    let mario = fx.list(&fx.mario, 0).await.unwrap();
    assert_eq!((mario.version, mario.items.len()), (1, 1));
    assert_eq!(mario.items[0].item.as_ref().unwrap().get(), r#"{"di":"mario"}"#);
    fx.delete(&fx.anna, 1, None).await.unwrap();
    assert!(!fx.list(&fx.mario, 0).await.unwrap().items[0].deleted);
    assert!(matches!(fx.delete(&fx.anna, 2, None).await.unwrap(), DeleteOutcome::Missing));
}

#[tokio::test]
async fn a_row_copied_to_another_user_is_unreadable() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"segreto":1}"#).await.unwrap();
    let anna = fx.anna.user_id.clone();
    fx.run(move |c| c.execute("UPDATE sync_items SET user_id = ?1", [anna]).unwrap()).await;
    assert!(matches!(fx.list(&fx.anna, 0).await, Err(StoreError::Unreadable)));
}

#[tokio::test]
async fn a_row_with_another_item_id_is_unreadable() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"segreto":1}"#).await.unwrap();
    let other = id(2);
    fx.run(move |c| c.execute("UPDATE sync_items SET item_id = ?1", [other]).unwrap()).await;
    assert!(matches!(fx.list(&fx.mario, 0).await, Err(StoreError::Unreadable)));
}

#[tokio::test]
async fn a_wrong_key_makes_the_items_unreadable_without_a_detail() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"segreto":1}"#).await.unwrap();
    let (other, user, device, dk) = (SyncKey::parse(OTHER_KEY).unwrap(), fx.mario.user_id.clone(), fx.mario.device_id.clone(), sync_client::key_sha256(&fx.mario.device_key));
    let result = fx.run(move |c| store::list(c, &other, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, 0)).await;
    let error = result.unwrap_err();
    assert!(matches!(error, StoreError::Unreadable));
    assert_eq!(format!("{error:?}"), "Unreadable");
}

#[tokio::test]
async fn a_device_of_another_user_or_a_revoked_one_is_inactive_inside_the_transaction() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    // Un dispositivo di anna con l'utente di mario.
    let (key, user, device, dk, now) = (fx.key.clone(), fx.mario.user_id.clone(), fx.anna.device_id.clone(), sync_client::key_sha256(&fx.anna.device_key), fx.ts.accounts().now());
    let item = raw("{}");
    let result = fx
        .run(move |c| store::put(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, &id(5), 0, &item, limits(), now))
        .await;
    assert!(matches!(result, Err(StoreError::Inactive)));

    fx.ts.accounts().revoke_device_admin(&fx.mario.user_id, &fx.mario.device_id).await.unwrap();
    assert!(matches!(fx.put(&fx.mario, 2, 0, "{}").await, Err(StoreError::Inactive)));
    assert!(matches!(fx.delete(&fx.mario, 1, None).await, Err(StoreError::Inactive)));
    assert!(matches!(fx.list(&fx.mario, 0).await, Err(StoreError::Inactive)));

    fx.ts.accounts().disable_user(&fx.anna.user_id).await.unwrap();
    assert!(matches!(fx.list(&fx.anna, 0).await, Err(StoreError::Inactive)));
}

#[tokio::test]
async fn the_item_bytes_are_kept_as_they_were_sent() {
    let fx = fixture().await;
    let text = r#"{ "z": 1,   "a": [1, 2],"m":{"k":"v"} }"#;
    fx.put(&fx.mario, 1, 0, text).await.unwrap();
    let page = fx.list(&fx.mario, 0).await.unwrap();
    assert_eq!(page.items[0].item.as_ref().unwrap().get(), text);
}

#[tokio::test]
async fn tombstones_are_capped_at_max_items_dropping_the_oldest() {
    let fx = fixture().await;
    let small = Limits { max_items: 3, max_item_bytes: 4096 };
    for n in 1..=5 {
        fx.put_with(&fx.mario, n, 0, "{}", small).await.unwrap();
        fx.delete_with(&fx.mario, n, None, small).await.unwrap();
    }
    let page = fx.list(&fx.mario, 0).await.unwrap();
    let ids: Vec<String> = page.items.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids, [id(3), id(4), id(5)]);
    assert!(page.items.iter().all(|i| i.deleted));
}

#[tokio::test]
async fn a_write_purges_the_expired_tombstones_of_its_user_only() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    fx.delete(&fx.mario, 1, None).await.unwrap();
    fx.put(&fx.anna, 1, 0, "{}").await.unwrap();
    fx.delete(&fx.anna, 1, None).await.unwrap();
    // 91 giorni dopo, mario scrive un altro elemento: il suo segnaposto sparisce, quello di anna no.
    let later = fx.ts.accounts().now() + Duration::days(91);
    let (key, user, device, dk) = (fx.key.clone(), fx.mario.user_id.clone(), fx.mario.device_id.clone(), sync_client::key_sha256(&fx.mario.device_key));
    let item = raw("{}");
    fx.run(move |c| store::put(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, &id(2), 0, &item, limits(), later).unwrap()).await;
    let mario = fx.list(&fx.mario, 0).await.unwrap();
    assert_eq!(mario.items.iter().filter(|i| i.deleted).count(), 0);
    assert_eq!(fx.list(&fx.anna, 0).await.unwrap().items.iter().filter(|i| i.deleted).count(), 1);
}

#[tokio::test]
async fn the_debug_of_a_stored_item_shows_neither_id_nor_content() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"token":"CONTENUTO-SEGRETO"}"#).await.unwrap();
    let page = fx.list(&fx.mario, 0).await.unwrap();
    for text in [format!("{:?}", page.items[0]), format!("{:#?}", page), format!("{:?}", page.items)] {
        assert!(!text.contains("CONTENUTO-SEGRETO"), "{text}");
        assert!(!text.contains(&id(1)), "{text}");
    }
}

// --- Pulizia, rekey, reset, prova della chiave, cifratura a riposo ----------------------

impl Fx {
    async fn tombstones(&self, who: &LoginOk) -> usize {
        self.list(who, 0).await.unwrap().items.iter().filter(|i| i.deleted).count()
    }
}

#[tokio::test]
async fn expired_tombstones_are_purged_and_recent_ones_stay() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    fx.delete(&fx.mario, 1, None).await.unwrap();
    fx.put(&fx.anna, 1, 0, "{}").await.unwrap();
    fx.put(&fx.mario, 2, 0, "{}").await.unwrap();
    let now = fx.ts.accounts().now();
    assert_eq!(fx.run(move |c| store::purge_expired(c, now + Duration::days(89)).unwrap()).await, 0);
    assert_eq!(fx.run(move |c| store::purge_expired(c, now + Duration::days(91)).unwrap()).await, 1);
    assert_eq!(fx.tombstones(&fx.mario).await, 0);
    // Gli elementi vivi non si toccano mai.
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().items.len(), 1);
    assert_eq!(fx.list(&fx.anna, 0).await.unwrap().items.len(), 1);
}

async fn nonces(fx: &Fx) -> Vec<Vec<u8>> {
    fx.run(|c| {
        let mut stmt = c.prepare("SELECT nonce FROM sync_items WHERE deleted = 0 ORDER BY user_id, item_id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    })
    .await
}

#[tokio::test]
async fn rekey_reencrypts_everything_and_keeps_versions() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    fx.put(&fx.mario, 2, 0, r#"{"b":2}"#).await.unwrap();
    fx.put(&fx.anna, 3, 0, r#"{"c":3}"#).await.unwrap();
    fx.delete(&fx.mario, 2, None).await.unwrap();
    let before = fx.list(&fx.mario, 0).await.unwrap();
    let nonces_before = nonces(&fx).await;
    let (old, new) = (SyncKey::parse(KEY).unwrap(), SyncKey::parse(OTHER_KEY).unwrap());
    let done = fx.run(move |c| store::rekey(c, &old, &new).unwrap()).await;
    assert_eq!(done, 2, "solo gli elementi non cancellati");
    assert_ne!(nonces(&fx).await, nonces_before);
    // Con la chiave nuova tutto è uguale a prima: stessi byte, stesse versioni, stesse date.
    let new_key = SyncKey::parse(OTHER_KEY).unwrap();
    let (user, device, dk) = (fx.mario.user_id.clone(), fx.mario.device_id.clone(), sync_client::key_sha256(&fx.mario.device_key));
    let after = fx.run(move |c| store::list(c, &new_key, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, 0).unwrap()).await;
    assert_eq!(serde_json::to_string(&before).unwrap(), serde_json::to_string(&after).unwrap());
    // Con la vecchia non si apre più.
    assert!(matches!(fx.list(&fx.mario, 0).await, Err(StoreError::Unreadable)));
}

#[tokio::test]
async fn rekey_is_all_or_nothing() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, r#"{"a":1}"#).await.unwrap();
    // Una riga cifrata con una terza chiave, di un utente che viene dopo nell'ordine.
    let third = SyncKey::parse(&"0123456789abcdef".repeat(4).chars().rev().collect::<String>()).unwrap();
    let anna = fx.anna.user_id.clone();
    fx.run(move |c| {
        let sealed = third.seal(&anna, &id(9), b"{}");
        c.execute(
            "INSERT INTO sync_items (user_id, item_id, version, nonce, ciphertext, deleted, updated_at, updated_by_device)
             VALUES (?1, ?2, 1, ?3, ?4, 0, 'x', 'd')",
            rusqlite::params![anna, id(9), &sealed.nonce[..], sealed.ciphertext],
        )
        .unwrap();
    })
    .await;
    let nonces_before = nonces(&fx).await;
    let (old, new) = (SyncKey::parse(KEY).unwrap(), SyncKey::parse(OTHER_KEY).unwrap());
    let result = fx.run(move |c| store::rekey(c, &old, &new)).await;
    assert!(matches!(result, Err(StoreError::Unreadable)));
    assert_eq!(nonces(&fx).await, nonces_before);
    assert!(fx.list(&fx.mario, 0).await.is_ok());
}

#[tokio::test]
async fn rekey_works_across_many_batches() {
    let fx = fixture().await;
    let big = Limits { max_items: 300, max_item_bytes: 4096 };
    for n in 1..=250 {
        fx.put_with(&fx.mario, n, 0, r#"{"k":1}"#, big).await.unwrap();
    }
    let (old, new) = (SyncKey::parse(KEY).unwrap(), SyncKey::parse(OTHER_KEY).unwrap());
    assert_eq!(fx.run(move |c| store::rekey(c, &old, &new).unwrap()).await, 250);
}

#[tokio::test]
async fn reset_deletes_every_item_keeps_the_counter_and_bumps_the_epoch() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    fx.put(&fx.mario, 2, 0, "{}").await.unwrap();
    fx.delete(&fx.mario, 2, None).await.unwrap();
    fx.put(&fx.anna, 1, 0, "{}").await.unwrap();
    assert_eq!(fx.run(|c| store::reset(c).unwrap()).await, 3);
    let page = fx.list(&fx.mario, 0).await.unwrap();
    assert!(page.items.is_empty());
    assert_eq!((page.version, page.epoch), (3, 1));
    assert_eq!(fx.list(&fx.anna, 0).await.unwrap().epoch, 1);
    // Il contatore non torna indietro: una scrittura nuova ha una versione più alta.
    assert_eq!(written(fx.put(&fx.mario, 1, 0, "{}").await), 4);
    fx.run(|c| store::reset(c).unwrap()).await;
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().epoch, 2);
}

#[tokio::test]
async fn a_user_without_sync_rows_has_epoch_zero_and_stays_so_after_a_reset() {
    let fx = fixture().await;
    fx.run(|c| store::reset(c).unwrap()).await;
    let page = fx.list(&fx.mario, 0).await.unwrap();
    assert_eq!((page.version, page.epoch, page.items.len()), (0, 0, 0));
}

#[tokio::test]
async fn purge_user_deletes_one_users_items_and_bumps_only_their_epoch() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    fx.put(&fx.mario, 2, 0, "{}").await.unwrap();
    fx.delete(&fx.mario, 2, None).await.unwrap();
    fx.put(&fx.anna, 1, 0, "{}").await.unwrap();
    let (db, mario) = (fx.ts.accounts().db().clone(), fx.mario.user_id.clone());
    assert_eq!(qamille_server::sync::purge_user(&db, &mario).await.unwrap(), 2);
    let page = fx.list(&fx.mario, 0).await.unwrap();
    assert!(page.items.is_empty());
    assert_eq!((page.version, page.epoch), (3, 1));
    let anna = fx.list(&fx.anna, 0).await.unwrap();
    assert_eq!((anna.items.len(), anna.epoch), (1, 0));
    // Ripetuta non fa nulla (e non incrementa di nuovo l'epoca); il contatore non torna indietro.
    assert_eq!(qamille_server::sync::purge_user(&db, &mario).await.unwrap(), 0);
    assert_eq!(fx.list(&fx.mario, 0).await.unwrap().epoch, 1);
    assert_eq!(written(fx.put(&fx.mario, 1, 0, "{}").await), 4);
    // Un utente senza righe: nulla, nessun errore.
    assert_eq!(qamille_server::sync::purge_user(&db, "nessuno").await.unwrap(), 0);
}

#[tokio::test]
async fn probe_says_when_the_key_does_not_open_the_items() {
    let fx = fixture().await;
    let (a, b) = (SyncKey::parse(KEY).unwrap(), SyncKey::parse(OTHER_KEY).unwrap());
    let probe = |key: SyncKey| fx.run(move |c| store::probe(c, &key).unwrap());
    assert_eq!(probe(a.clone()).await, store::Probe::Empty);
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    assert_eq!(probe(a.clone()).await, store::Probe::Opens);
    assert_eq!(probe(b.clone()).await, store::Probe::DoesNotOpen);
    fx.delete(&fx.mario, 1, None).await.unwrap();
    assert_eq!(probe(b).await, store::Probe::Empty, "i segnaposto non hanno nulla da aprire");
}

#[tokio::test]
async fn the_database_file_and_the_wal_do_not_contain_the_item() {
    let fx = fixture().await;
    let marker = "MARKER-IN-CHIARO-12345";
    fx.put(&fx.mario, 1, 0, &format!(r#"{{"password_per_app":"{marker}"}}"#)).await.unwrap();
    let mut bytes = Vec::new();
    for name in ["server.db", "server.db-wal", "server.db-shm"] {
        if let Ok(data) = std::fs::read(fx.ts.dir.path().join(name)) {
            bytes.extend_from_slice(&data);
        }
    }
    let contains = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!contains(marker), "il contenuto è nel file del database");
    assert!(!contains("password_per_app"));
    assert!(contains("mario"), "la scansione trova lo username: la prova non è vuota");
    assert!(contains(&id(1)), "l'id è in chiaro per scelta (è un hash)");
}

#[tokio::test]
async fn the_ciphertext_in_the_database_is_the_length_of_the_item_plus_the_tag() {
    let fx = fixture().await;
    let text = r#"{"a":"bcdefg"}"#;
    fx.put(&fx.mario, 1, 0, text).await.unwrap();
    let (nonce, ciphertext): (i64, i64) = fx
        .run(|c| c.query_row("SELECT length(nonce), length(ciphertext) FROM sync_items", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap())
        .await;
    assert_eq!((nonce, ciphertext), (12, text.len() as i64 + 16));
}

#[tokio::test]
async fn the_transaction_checks_the_key_of_the_request_not_just_the_device() {
    let fx = fixture().await;
    fx.put(&fx.mario, 1, 0, "{}").await.unwrap();
    // Il dispositivo e l'utente sono giusti, la chiave no (per esempio una chiave già revocata).
    let (key, user, device) = (fx.key.clone(), fx.mario.user_id.clone(), fx.mario.device_id.clone());
    let wrong = sync_client::key_sha256("spk_dev_un'altra");
    let result = fx.run(move |c| store::list(c, &key, &Who { user_id: &user, device_id: &device, key_sha256: &wrong }, 0)).await;
    assert!(matches!(result, Err(StoreError::Inactive)));
    // E quella giusta, calcolata come fa il server, passa: lo SHA-256 del test è quello vero.
    let auth = fx.ts.accounts().authenticate(&fx.mario.device_key).await.unwrap();
    assert_eq!(auth.key_sha256(), sync_client::key_sha256(&fx.mario.device_key));
}
