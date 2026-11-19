//! Le rotte `/v1/sync/items` con client simulati: due dispositivi dello stesso utente e uno di
//! un altro.
#![allow(dead_code)]

mod support;

use qamille_server::accounts::LoginOk;
use serde_json::{Value, json};
use support::sync_client::{SyncClient, account_item, item_id};
use support::{TestServer, ip, loopback, request, sync_config, test_config, user_with_device};

const PASSWORD: &str = "una password lunga";

async fn second_device(ts: &TestServer, username: &str) -> LoginOk {
    ts.accounts().login(ip(loopback()), username, PASSWORD, "Portatile").await.expect("secondo dispositivo")
}

#[tokio::test]
async fn without_the_key_the_routes_do_not_exist() {
    let ts = TestServer::new(test_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    for response in [
        client.list(0).await,
        client.put(&item_id(1), 0, account_item("x")).await,
        client.delete(&item_id(1), None).await,
    ] {
        assert_eq!(response.status, 404, "{}", response.text);
        assert_eq!(response.code(), Some("not_found"));
    }
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert!(info.body.get("sync").is_none(), "{}", info.text);
}

#[tokio::test]
async fn info_declares_sync_only_with_the_key() {
    let ts = TestServer::new(sync_config());
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert_eq!(info.status, 200);
    assert_eq!(info.body["sync"], json!({"available": true}));
    assert_eq!(support::keys(&info.body), ["api", "edition", "sync", "version"]);
}

#[tokio::test]
async fn an_item_written_by_one_device_is_read_by_another_of_the_same_user() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = second_device(&ts, "mario").await;
    let (client_a, client_b) = (SyncClient::new(&ts.public, &a.device_key), SyncClient::new(&ts.public, &b.device_key));
    let item = account_item("token-1");
    let put = client_a.put(&item_id(1), 0, item.clone()).await;
    assert_eq!(put.status, 200, "{}", put.text);
    assert_eq!(put.body["version"], 1);
    assert_eq!(put.body["id"], item_id(1));
    let list = client_b.list(0).await;
    assert_eq!(list.status, 200, "{}", list.text);
    assert_eq!((list.body["version"].clone(), list.body["epoch"].clone()), (json!(1), json!(0)));
    let items = list.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["item"], item);
    assert_eq!(items[0]["deleted"], false);
    assert_eq!(items[0]["updated_by_device"], a.device_id);
}

#[tokio::test]
async fn a_conflict_returns_the_current_item_and_the_app_can_retry() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = second_device(&ts, "mario").await;
    let (client_a, client_b) = (SyncClient::new(&ts.public, &a.device_key), SyncClient::new(&ts.public, &b.device_key));
    assert_eq!(client_a.put(&item_id(1), 0, account_item("v1")).await.body["version"], 1);
    assert_eq!(client_a.put(&item_id(1), 1, account_item("v2")).await.body["version"], 2);
    let conflict = client_b.put(&item_id(1), 1, account_item("v-di-b")).await;
    assert_eq!(conflict.status, 409, "{}", conflict.text);
    assert_eq!(conflict.code(), Some("sync_conflict"));
    assert_eq!(conflict.body["error"]["current"]["version"], 2);
    assert_eq!(conflict.body["error"]["current"]["item"], account_item("v2"));
    let retry = client_b.put(&item_id(1), 2, account_item("v-di-b")).await;
    assert_eq!((retry.status.as_u16(), retry.body["version"].clone()), (200, json!(3)));
}

#[tokio::test]
async fn a_new_item_over_an_existing_one_conflicts() {
    // EX-39: la stessa casella collegata su due PC prima del login sul secondo.
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = second_device(&ts, "mario").await;
    SyncClient::new(&ts.public, &a.device_key).put(&item_id(1), 0, account_item("v1")).await;
    let conflict = SyncClient::new(&ts.public, &b.device_key).put(&item_id(1), 0, account_item("v-di-b")).await;
    assert_eq!(conflict.status, 409);
    assert_eq!(conflict.body["error"]["current"]["item"], account_item("v1"));
}

#[tokio::test]
async fn a_version_claimed_for_a_missing_item_conflicts_with_a_null_current() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let conflict = SyncClient::new(&ts.public, &a.device_key).put(&item_id(1), 4, account_item("x")).await;
    assert_eq!(conflict.status, 409, "{}", conflict.text);
    assert_eq!(conflict.code(), Some("sync_conflict"));
    assert!(conflict.body["error"]["current"].is_null());
}

#[tokio::test]
async fn delete_leaves_a_tombstone_that_the_other_device_sees() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = second_device(&ts, "mario").await;
    let (client_a, client_b) = (SyncClient::new(&ts.public, &a.device_key), SyncClient::new(&ts.public, &b.device_key));
    client_a.put(&item_id(1), 0, account_item("v1")).await;
    let seen = client_b.list(0).await.body["version"].as_u64().unwrap();
    let deleted = client_a.delete(&item_id(1), None).await;
    assert_eq!(deleted.status, 200, "{}", deleted.text);
    assert_eq!(deleted.body, json!({"id": item_id(1), "version": 2, "deleted": true}));
    let list = client_b.list(seen).await;
    let items = list.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["deleted"], true);
    assert!(items[0].get("item").is_none());
    // Idempotente; un id mai scritto è 404.
    assert_eq!(client_a.delete(&item_id(1), None).await.body["version"], 2);
    let missing = client_a.delete(&item_id(2), None).await;
    assert_eq!((missing.status.as_u16(), missing.code()), (404, Some("not_found")));
    // Con una versione che non è quella attuale: conflitto.
    client_a.put(&item_id(3), 0, account_item("x")).await;
    let stale = client_a.delete(&item_id(3), Some(1)).await;
    assert_eq!((stale.status.as_u16(), stale.code()), (409, Some("sync_conflict")));
}

#[tokio::test]
async fn a_deleted_item_is_recreated_with_the_tombstone_version() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &a.device_key);
    client.put(&item_id(1), 0, account_item("v1")).await;
    client.delete(&item_id(1), None).await;
    let conflict = client.put(&item_id(1), 0, account_item("v2")).await;
    assert_eq!(conflict.status, 409);
    assert_eq!(conflict.body["error"]["current"]["deleted"], true);
    let version = conflict.body["error"]["current"]["version"].as_u64().unwrap();
    assert_eq!(client.put(&item_id(1), version, account_item("v2")).await.status, 200);
}

#[tokio::test]
async fn another_user_cannot_read_write_or_delete_an_item_of_mario() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let anna = user_with_device(&ts, "anna", PASSWORD).await;
    let (cm, ca) = (SyncClient::new(&ts.public, &mario.device_key), SyncClient::new(&ts.public, &anna.device_key));
    cm.put(&item_id(1), 0, account_item("di-mario")).await;
    assert_eq!(ca.list(0).await.body["items"], json!([]));
    // Lo stesso id presso anna è un altro elemento, con il suo contatore.
    let put = ca.put(&item_id(1), 0, account_item("di-anna")).await;
    assert_eq!((put.status.as_u16(), put.body["version"].clone()), (200, json!(1)));
    // Anna non sa la versione di mario e non può leggerla: una versione a caso è un suo conflitto.
    assert_eq!(ca.put(&item_id(1), 99, account_item("x")).await.status, 409);
    let theirs = cm.list(0).await;
    assert_eq!(theirs.body["items"][0]["item"], account_item("di-mario"));
    ca.delete(&item_id(1), None).await;
    assert_eq!(cm.list(0).await.body["items"][0]["deleted"], false);
    // Un id che anna non ha scritto è 404 per lei, anche se mario ce l'ha.
    cm.put(&item_id(2), 0, account_item("solo-mario")).await;
    assert_eq!(ca.delete(&item_id(2), None).await.status, 404);
    assert_eq!(cm.list(0).await.body["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_invalid_id_is_400_and_never_reaches_the_database() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let bad = [
        "ab".repeat(31),
        "AB".repeat(32),
        "..%2f..%2fx".to_owned(),
        "%2e%2e".to_owned(),
        "ab".repeat(33),
        "zz".repeat(32),
    ];
    for id in bad {
        let put = client.put(&id, 0, account_item("x")).await;
        assert_eq!((put.status.as_u16(), put.code()), (400, Some("invalid_request")), "{id}: {}", put.text);
        let del = client.delete(&id, None).await;
        assert_eq!(del.status, 400, "{id}");
    }
    assert_eq!(client.list(0).await.body["version"], 0);
}

#[tokio::test]
async fn the_body_must_be_the_expected_json() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let id = item_id(1);
    let bodies: Vec<Vec<u8>> = vec![
        br#"{"base_version":0}"#.to_vec(),
        br#"{"base_version":0,"item":[1]}"#.to_vec(),
        br#"{"base_version":0,"item":"testo"}"#.to_vec(),
        br#"{"base_version":0,"item":null}"#.to_vec(),
        br#"{"base_version":0,"item":{},"extra":1}"#.to_vec(),
        br#"{"base_version":-1,"item":{}}"#.to_vec(),
        br#"{"base_version":"0","item":{}}"#.to_vec(),
        br#"{"item":{}}"#.to_vec(),
        br#"{"base_version":0,"item":{"a":CONTENUTO-SEGRETO"#.to_vec(),
        Vec::new(),
    ];
    for body in bodies {
        let r = client.put_raw(&id, body.clone()).await;
        assert_eq!((r.status.as_u16(), r.code()), (400, Some("invalid_request")), "{}", String::from_utf8_lossy(&body));
        assert!(!r.text.contains("CONTENUTO-SEGRETO") && !r.text.contains(&id), "{}", r.text);
    }
    assert_eq!(client.list(0).await.body["version"], 0);
}

#[tokio::test]
async fn the_item_is_returned_byte_for_byte() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let item = r#"{ "z": 1,   "a": [1, 2],"m":{"k":"v"} }"#;
    let body = format!(r#"{{"base_version":0,"item":{item}}}"#).into_bytes();
    assert_eq!(client.put_raw(&item_id(1), body).await.status, 200);
    let list = client.list(0).await;
    assert!(list.text.contains(item), "{}", list.text);
}

#[tokio::test]
async fn a_failed_conflict_or_validation_does_not_move_the_counter() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    assert_eq!(client.put(&item_id(1), 0, account_item("a")).await.body["version"], 1);
    assert_eq!(client.put(&item_id(1), 0, account_item("b")).await.status, 409);
    assert_eq!(client.put_raw(&item_id(1), b"{}".to_vec()).await.status, 400);
    assert_eq!(client.put("zz", 0, account_item("c")).await.status, 400);
    assert_eq!(client.put(&item_id(2), 0, account_item("d")).await.body["version"], 2);
}

#[tokio::test]
async fn every_response_is_json_and_not_cacheable() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let wrong = SyncClient::new(&ts.public, "spk_dev_sbagliata");
    let responses = [
        client.put(&item_id(1), 0, account_item("a")).await,
        client.list(0).await,
        client.put(&item_id(1), 0, account_item("b")).await,
        client.delete(&item_id(9), None).await,
        client.put("zz", 0, account_item("c")).await,
        wrong.list(0).await,
        support::request(&ts.public, "POST", "/v1/sync/items", Some(json!({})), loopback(), &[]).await,
        support::request(&ts.public, "GET", "/v1/sync/nulla", None, loopback(), &[]).await,
    ];
    let statuses: Vec<u16> = responses.iter().map(|r| r.status.as_u16()).collect();
    assert_eq!(statuses, [200, 200, 409, 404, 400, 401, 405, 404]);
    for r in &responses {
        assert_eq!(r.headers["cache-control"], "no-store");
        assert!(r.headers["content-type"].to_str().unwrap().starts_with("application/json"), "{:?}", r.headers);
    }
    assert_eq!(responses[6].code(), Some("method_not_allowed"));
}

#[tokio::test]
async fn the_other_core_routes_are_unchanged_by_the_merge() {
    let ts = TestServer::new(sync_config());
    let login = request(&ts.public, "POST", "/v1/login", Some(json!({"identifier": "x", "password": "y", "device_name": "z"})), loopback(), &[]).await;
    assert_eq!(login.status, 401);
    let whoami = request(&ts.public, "GET", "/test/whoami", None, loopback(), &[]).await;
    assert_eq!(whoami.status, 401);
    let health = request(&ts.public, "GET", "/health", None, loopback(), &[]).await;
    assert_eq!(health.status, 200);
    let _: Value = health.body;
}

// --- Pulizia oraria, prova della chiave ---------------------------------------------------

use chrono::TimeDelta;
use qamille_server::Server;
use qamille_server::sync::SyncKey;

#[tokio::test]
async fn the_purge_removes_only_the_expired_tombstones() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    client.put(&item_id(1), 0, account_item("x")).await;
    client.delete(&item_id(1), None).await;
    client.put(&item_id(2), 0, account_item("y")).await;
    let service = ts.server.sync_service().unwrap().clone();
    ts.clock.advance(TimeDelta::days(89));
    assert_eq!(service.purge_expired().await.unwrap(), 0);
    ts.clock.advance(TimeDelta::days(2));
    assert_eq!(service.purge_expired().await.unwrap(), 1);
    let list = client.list(0).await;
    let items = list.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], item_id(2));
}

#[tokio::test(start_paused = true)]
async fn the_hourly_task_purges() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    client.put(&item_id(1), 0, account_item("x")).await;
    client.delete(&item_id(1), None).await;
    let service = ts.server.sync_service().unwrap().clone();
    service.spawn_purge();
    ts.clock.advance(TimeDelta::days(91));
    // Prima dell'ora il segnaposto c'è ancora.
    tokio::time::sleep(std::time::Duration::from_secs(3000)).await;
    assert_eq!(client.list(0).await.body["items"].as_array().unwrap().len(), 1);
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        if client.list(0).await.body["items"].as_array().unwrap().is_empty() {
            return;
        }
    }
    panic!("il segnaposto scaduto non è stato tolto dall'attività oraria");
}

#[tokio::test]
async fn a_wrong_key_does_not_stop_the_server_and_answers_500_without_ids() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    SyncClient::new(&ts.public, &mario.device_key).put(&item_id(1), 0, account_item("MARKER-SEGRETO")).await;
    // Lo stesso database, aperto con un'altra chiave.
    let mut config = sync_config();
    config.sync.key = Some(SyncKey::parse(&"fedcba9876543210".repeat(4)).unwrap());
    config.data_dir = ts.dir.path().to_path_buf();
    let other = Server::open(config).expect("il server parte anche con la chiave sbagliata");
    other.sync_service().unwrap().log_startup(false).await;
    let router = other.public_router();
    let client = SyncClient::new(&router, &mario.device_key);
    let health = request(&router, "GET", "/health", None, loopback(), &[]).await;
    assert_eq!(health.status, 200);
    let list = client.list(0).await;
    assert_eq!((list.status.as_u16(), list.code()), (500, Some("internal")), "{}", list.text);
    assert!(!list.text.contains(&item_id(1)) && !list.text.contains("MARKER-SEGRETO"), "{}", list.text);
    // Un elemento nuovo si scrive: non legge le altre righe.
    assert_eq!(client.put(&item_id(2), 0, account_item("z")).await.status, 200);
    // Il rimedio da riga di comando: `sync reset`.
    other.accounts().db().blocking(|c| Ok(qamille_server::sync::store::reset(c).unwrap())).await.unwrap();
    assert_eq!(client.list(0).await.status, 200);
}

#[tokio::test]
async fn a_bad_query_is_a_json_400_without_echoing_the_value() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let id = item_id(1);
    let responses = [
        support::request(&ts.public, "GET", "/v1/sync/items?since=MARKER-NEL-QUERY", None, loopback(), &[("authorization", &support::bearer(&mario.device_key))]).await,
        support::request(&ts.public, "GET", "/v1/sync/items?since=-1", None, loopback(), &[("authorization", &support::bearer(&mario.device_key))]).await,
        support::request(&ts.public, "DELETE", &format!("/v1/sync/items/{id}?base_version=MARKER-NEL-QUERY"), None, loopback(), &[("authorization", &support::bearer(&mario.device_key))]).await,
    ];
    for r in responses {
        assert_eq!((r.status.as_u16(), r.code()), (400, Some("invalid_request")), "{}", r.text);
        assert!(!r.text.contains("MARKER-NEL-QUERY"), "{}", r.text);
        assert_eq!(r.headers["cache-control"], "no-store");
    }
    assert_eq!(client.list(0).await.status, 200);
}
