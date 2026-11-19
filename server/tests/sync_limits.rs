//! I limiti sul filo: dimensione dell'elemento e del corpo, numero degli elementi (INV-38).
#![allow(dead_code)]

mod support;

use serde_json::json;
use support::sync_client::{SyncClient, account_item, item_id};
use support::{TestServer, loopback, request, sync_config, user_with_device};

const PASSWORD: &str = "una password lunga";

/// Un elemento di esattamente `len` byte.
fn item_of(len: usize) -> String {
    let overhead = r#"{"x":""}"#.len();
    format!(r#"{{"x":"{}"}}"#, "a".repeat(len - overhead))
}

fn body_of(base: u64, item: &str) -> Vec<u8> {
    format!(r#"{{"base_version":{base},"item":{item}}}"#).into_bytes()
}

async fn server_with(max_items: u32, max_item_bytes: usize) -> (TestServer, String) {
    let mut config = sync_config();
    config.sync.max_items = max_items;
    config.sync.max_item_bytes = max_item_bytes;
    let ts = TestServer::new(config);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    (ts, mario.device_key)
}

#[tokio::test]
async fn an_item_of_exactly_the_limit_is_accepted_and_one_byte_more_is_413() {
    let (ts, key) = server_with(200, 4096).await;
    let client = SyncClient::new(&ts.public, &key);
    let exact = client.put_raw(&item_id(1), body_of(0, &item_of(4096))).await;
    assert_eq!(exact.status, 200, "{}", exact.text);
    let over = client.put_raw(&item_id(2), body_of(0, &item_of(4097))).await;
    assert_eq!((over.status.as_u16(), over.code()), (413, Some("payload_too_large")), "{}", over.text);
    assert_eq!(client.list(0).await.body["version"], 1, "il rifiuto non muove il contatore");
}

#[tokio::test]
async fn a_body_over_the_margin_is_413() {
    let (ts, key) = server_with(200, 4096).await;
    let client = SyncClient::new(&ts.public, &key);
    let huge = body_of(0, &item_of(4096 + 1024 + 100));
    let r = client.put_raw(&item_id(1), huge).await;
    assert_eq!((r.status.as_u16(), r.code()), (413, Some("payload_too_large")), "{}", r.text);
    assert_eq!(client.list(0).await.body["items"], json!([]));
}

#[tokio::test]
async fn the_default_limit_is_64_kib_and_the_routes_are_not_capped_at_64_kib_of_the_body() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let exact = body_of(0, &item_of(64 * 1024));
    assert!(exact.len() > 64 * 1024, "il corpo supera i 64 KiB del core");
    let ok = client.put_raw(&item_id(1), exact).await;
    assert_eq!(ok.status, 200, "{}", ok.text);
    let over = client.put_raw(&item_id(2), body_of(0, &item_of(64 * 1024 + 1))).await;
    assert_eq!(over.status, 413);
    // E si rilegge intero.
    let list = client.list(0).await;
    assert_eq!(list.status, 200);
    assert!(list.text.len() > 64 * 1024);
}

#[tokio::test]
async fn the_next_item_over_the_count_is_409_sync_limit_but_an_update_passes() {
    let (ts, key) = server_with(3, 4096).await;
    let client = SyncClient::new(&ts.public, &key);
    for n in 1..=3 {
        assert_eq!(client.put(&item_id(n), 0, account_item("x")).await.status, 200);
    }
    let full = client.put(&item_id(4), 0, account_item("x")).await;
    assert_eq!((full.status.as_u16(), full.code()), (409, Some("sync_limit")), "{}", full.text);
    assert!(full.body["error"]["message"].as_str().unwrap().contains("massimo"));
    assert_eq!(client.put(&item_id(1), 1, account_item("y")).await.status, 200);
    assert_eq!(client.delete(&item_id(2), None).await.status, 200);
    assert_eq!(client.put(&item_id(4), 0, account_item("x")).await.status, 200);
}

#[tokio::test]
async fn tombstones_do_not_count_as_items_but_are_capped() {
    let (ts, key) = server_with(3, 4096).await;
    let client = SyncClient::new(&ts.public, &key);
    for n in 1..=10 {
        assert_eq!(client.put(&item_id(n), 0, account_item("x")).await.status, 200, "{n}");
        assert_eq!(client.delete(&item_id(n), None).await.status, 200, "{n}");
    }
    let items = client.list(0).await.body["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i["deleted"] == true));
}

#[tokio::test]
async fn the_other_core_routes_keep_their_64_kib_limit() {
    let (ts, _) = server_with(10, 1024 * 1024).await;
    let body = json!({"identifier": "x".repeat(70 * 1024), "password": "y", "device_name": "z"});
    let r = request(&ts.public, "POST", "/v1/login", Some(body), loopback(), &[]).await;
    assert_eq!((r.status.as_u16(), r.code()), (413, Some("payload_too_large")), "{}", r.text);
}
