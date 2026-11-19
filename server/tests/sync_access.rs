//! Chi può usare la sincronizzazione: connessione sicura, chiave, diritto d'uso, richieste in
//! volo, chiavi sbagliate per origine, revoche (INV-36, INV-37).
#![allow(dead_code)]

mod support;

use std::sync::{Arc, Mutex};

use qamille_server::ServerError;
use qamille_server::ext::{BoxFut, Entitlement, Resource};
use serde_json::json;
use serde_json::value::RawValue;
use support::sync_client::{SyncClient, account_item, item_id};
use support::{ADMIN_KEY, TestServer, bearer, ip, loopback, remote, request, sync_config, user_with_device};

const PASSWORD: &str = "una password lunga";

type Seen = Arc<Mutex<Vec<(String, Resource)>>>;

struct Decide {
    seen: Seen,
    allow: bool,
}

impl Entitlement for Decide {
    fn require<'a>(&'a self, user_id: &'a str, resource: Resource) -> BoxFut<'a, Result<(), ServerError>> {
        self.seen.lock().unwrap().push((user_id.to_owned(), resource));
        let allow = self.allow;
        Box::pin(async move { if allow { Ok(()) } else { Err(ServerError::SubscriptionRequired) } })
    }
}

/// Un server con un `Entitlement` che registra le domande.
fn with_entitlement(allow: bool) -> (TestServer, Seen) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let TestServer { server, clock, dir, .. } = TestServer::new(sync_config());
    let server = server.with_entitlement(Arc::new(Decide { seen: seen.clone(), allow }));
    let public = server.public_router();
    let admin = server.admin_router();
    (TestServer { server, public, admin, clock, dir }, seen)
}

async fn count(ts: &TestServer, sql: &'static str) -> i64 {
    ts.accounts().db().blocking(move |c| Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?)).await.unwrap()
}

// --- Connessione, chiave, origine -------------------------------------------------------

#[tokio::test]
async fn plaintext_from_a_remote_address_is_refused() {
    let mut config = sync_config();
    config.trusted_proxies = vec![ip(remote([10, 0, 0, 1]))];
    config.plaintext_from = vec![ip(remote([192, 168, 1, 50]))];
    let ts = TestServer::new(config);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let plain = SyncClient::new(&ts.public, &mario.device_key).from(remote([203, 0, 113, 9]));
    for response in [plain.list(0).await, plain.put(&item_id(1), 0, account_item("x")).await, plain.delete(&item_id(1), None).await] {
        assert_eq!((response.status.as_u16(), response.code()), (400, Some("tls_required")), "{}", response.text);
    }
    // Nulla è stato scritto: non è stata nemmeno letta la chiave.
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 0);
    // Da un indirizzo di `plaintext_from` e dal loopback si può.
    let local = SyncClient::new(&ts.public, &mario.device_key).from(remote([192, 168, 1, 50]));
    assert_eq!(local.put(&item_id(1), 0, account_item("x")).await.status, 200);
    // Dal proxy fidato conta `X-Forwarded-Proto`.
    let headers = [("authorization", &bearer(&mario.device_key)[..]), ("x-forwarded-for", "203.0.113.9"), ("x-forwarded-proto", "https")];
    let via_proxy = request(&ts.public, "GET", "/v1/sync/items", None, remote([10, 0, 0, 1]), &headers).await;
    assert_eq!(via_proxy.status, 200, "{}", via_proxy.text);
    let headers = [("authorization", &bearer(&mario.device_key)[..]), ("x-forwarded-for", "203.0.113.9"), ("x-forwarded-proto", "http")];
    let insecure = request(&ts.public, "GET", "/v1/sync/items", None, remote([10, 0, 0, 1]), &headers).await;
    assert_eq!(insecure.code(), Some("tls_required"));
}

#[tokio::test]
async fn no_key_a_wrong_key_and_a_malformed_header_are_401() {
    let ts = TestServer::new(sync_config());
    user_with_device(&ts, "mario", PASSWORD).await;
    let cases: Vec<Vec<(&str, &str)>> = vec![
        vec![],
        vec![("authorization", "Bearer spk_dev_sbagliata")],
        vec![("authorization", "Basic bWFyaW86cGFzcw==")],
        vec![("authorization", "Bearer")],
    ];
    for headers in cases {
        let r = request(&ts.public, "GET", "/v1/sync/items", None, loopback(), &headers).await;
        assert_eq!((r.status.as_u16(), r.code()), (401, Some("unauthorized")), "{headers:?}");
        assert_eq!(r.headers["www-authenticate"], "Bearer");
    }
    // La chiave amministratore non vale come chiave di un dispositivo.
    let admin = bearer(ADMIN_KEY);
    let r = request(&ts.public, "GET", "/v1/sync/items", None, loopback(), &[("authorization", &admin)]).await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn wrong_keys_from_one_origin_are_limited_with_429() {
    let mut config = sync_config();
    config.compute_max_auth_failures_per_origin = 3;
    config.plaintext_from = vec![ip(remote([192, 168, 1, 5])), ip(remote([192, 168, 1, 6]))];
    let ts = TestServer::new(config);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let wrong = SyncClient::new(&ts.public, "spk_dev_sbagliata").from(remote([192, 168, 1, 5]));
    for _ in 0..3 {
        assert_eq!(wrong.list(0).await.status, 401);
    }
    let blocked = wrong.list(0).await;
    assert_eq!((blocked.status.as_u16(), blocked.code()), (429, Some("rate_limited")), "{}", blocked.text);
    assert!(blocked.body["error"]["retry_after_s"].as_u64().is_some());
    // Anche la chiave giusta, dalla stessa origine, finché dura il blocco.
    let right_same = SyncClient::new(&ts.public, &mario.device_key).from(remote([192, 168, 1, 5]));
    assert_eq!(right_same.list(0).await.status, 429);
    // Da un'altra origine passa.
    let right_other = SyncClient::new(&ts.public, &mario.device_key).from(remote([192, 168, 1, 6]));
    assert_eq!(right_other.list(0).await.status, 200);
}

// --- Entitlement --------------------------------------------------------------------------

#[tokio::test]
async fn the_entitlement_is_asked_for_the_sync_resource() {
    let (ts, seen) = with_entitlement(false);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    for response in [client.list(0).await, client.put(&item_id(1), 0, account_item("x")).await, client.delete(&item_id(1), None).await] {
        assert_eq!((response.status.as_u16(), response.code()), (402, Some("subscription_required")), "{}", response.text);
    }
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|(user, resource)| user == &mario.user_id && *resource == Resource::Sync));
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 0);
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_state").await, 0);
}

#[tokio::test]
async fn the_entitlement_comes_after_the_key_and_before_the_data() {
    let (ts, seen) = with_entitlement(false);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let wrong = SyncClient::new(&ts.public, "spk_dev_sbagliata");
    assert_eq!(wrong.list(0).await.status, 401);
    assert!(seen.lock().unwrap().is_empty(), "una chiave sbagliata non arriva al diritto d'uso");
    // Diritto negato e id non valido: 402, non 400; e il corpo rotto non viene letto.
    let client = SyncClient::new(&ts.public, &mario.device_key);
    assert_eq!(client.put("zz", 0, account_item("x")).await.status, 402);
    assert_eq!(client.put_raw(&item_id(1), b"non json".to_vec()).await.status, 402);
}

#[tokio::test]
async fn an_allowing_entitlement_lets_everything_through() {
    let (ts, seen) = with_entitlement(true);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    assert_eq!(client.put(&item_id(1), 0, account_item("x")).await.status, 200);
    assert_eq!(client.list(0).await.status, 200);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn without_an_entitlement_every_active_user_may_sync() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    assert_eq!(SyncClient::new(&ts.public, &mario.device_key).list(0).await.status, 200);
}

// --- Richieste in volo ---------------------------------------------------------------------

#[tokio::test]
async fn at_most_two_requests_per_user_in_flight() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let anna = user_with_device(&ts, "anna", PASSWORD).await;
    let gate = Arc::clone(ts.server.sync_service().unwrap().gate());
    let (first, second) = (gate.enter(&mario.user_id).unwrap(), gate.enter(&mario.user_id).unwrap());
    assert_eq!(gate.in_flight(&mario.user_id), 2);
    let busy = SyncClient::new(&ts.public, &mario.device_key).list(0).await;
    assert_eq!((busy.status.as_u16(), busy.code()), (429, Some("rate_limited")), "{}", busy.text);
    assert_eq!(busy.body["error"]["retry_after_s"], 2);
    assert_eq!(busy.headers["retry-after"], "2");
    // Un altro utente non ne risente.
    assert_eq!(SyncClient::new(&ts.public, &anna.device_key).list(0).await.status, 200);
    drop(first);
    assert_eq!(SyncClient::new(&ts.public, &mario.device_key).list(0).await.status, 200);
    drop(second);
    assert_eq!(gate.in_flight(&mario.user_id), 0);
}

#[tokio::test]
async fn the_slot_is_released_after_every_kind_of_answer() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let gate = Arc::clone(ts.server.sync_service().unwrap().gate());
    let client = SyncClient::new(&ts.public, &mario.device_key);
    client.put(&item_id(1), 0, account_item("a")).await;
    client.put(&item_id(1), 0, account_item("b")).await; // 409
    client.put("zz", 0, account_item("c")).await; // 400
    client.delete(&item_id(5), None).await; // 404
    client.put_raw(&item_id(1), b"{".to_vec()).await; // 400
    assert_eq!(gate.in_flight(&mario.user_id), 0);
}

// --- Revoche (INV-37) ----------------------------------------------------------------------

#[tokio::test]
async fn a_revoked_device_gets_401_from_the_next_request() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = ts.accounts().login(ip(loopback()), "mario", PASSWORD, "Portatile").await.unwrap();
    let (ca, cb) = (SyncClient::new(&ts.public, &a.device_key), SyncClient::new(&ts.public, &b.device_key));
    assert_eq!(cb.put(&item_id(1), 0, account_item("x")).await.status, 200);
    let auth = bearer(&a.device_key);
    let revoke = request(&ts.public, "DELETE", &format!("/v1/account/devices/{}", b.device_id), None, loopback(), &[("authorization", &auth)]).await;
    assert_eq!(revoke.status, 204, "{}", revoke.text);
    for r in [cb.list(0).await, cb.put(&item_id(2), 0, account_item("y")).await, cb.delete(&item_id(1), None).await] {
        assert_eq!((r.status.as_u16(), r.code()), (401, Some("unauthorized")), "{}", r.text);
    }
    // L'altro dispositivo continua, e vede ciò che b aveva scritto.
    assert_eq!(ca.list(0).await.body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_admin_revoking_a_device_has_the_same_effect() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let admin = bearer(ADMIN_KEY);
    let path = format!("/admin/api/users/{}/devices/{}", a.user_id, a.device_id);
    let r = request(ts.admin(), "DELETE", &path, None, loopback(), &[("authorization", &admin)]).await;
    assert!(r.status.is_success(), "{}", r.text);
    assert_eq!(SyncClient::new(&ts.public, &a.device_key).list(0).await.status, 401);
}

#[tokio::test]
async fn a_disabled_user_gets_401_and_the_items_are_kept() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &a.device_key);
    client.put(&item_id(1), 0, account_item("x")).await;
    let admin = bearer(ADMIN_KEY);
    let off = request(ts.admin(), "POST", &format!("/admin/api/users/{}/disable", a.user_id), None, loopback(), &[("authorization", &admin)]).await;
    assert!(off.status.is_success(), "{}", off.text);
    assert_eq!(client.list(0).await.status, 401);
    assert_eq!(client.put(&item_id(2), 0, account_item("y")).await.status, 401);
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 1);
    let on = request(ts.admin(), "POST", &format!("/admin/api/users/{}/enable", a.user_id), None, loopback(), &[("authorization", &admin)]).await;
    assert!(on.status.is_success());
    // La disattivazione ha revocato i dispositivi: la chiave di prima non torna a valere, ma
    // le caselle ci sono ancora e si leggono dopo un nuovo accesso.
    assert_eq!(client.list(0).await.status, 401);
    let again = ts.accounts().login(support::ip(loopback()), "mario", PASSWORD, "PC").await.unwrap();
    let client = SyncClient::new(&ts.public, &again.device_key);
    assert_eq!(client.list(0).await.body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_deleted_user_loses_the_items() {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let anna = user_with_device(&ts, "anna", PASSWORD).await;
    let giulia = user_with_device(&ts, "giulia", PASSWORD).await;
    for login in [&mario, &anna, &giulia] {
        SyncClient::new(&ts.public, &login.device_key).put(&item_id(1), 0, account_item("x")).await;
    }
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 3);
    // L'utente cancella il suo account con la password.
    let auth = bearer(&mario.device_key);
    let r = request(&ts.public, "DELETE", "/v1/account", Some(json!({"password": PASSWORD})), loopback(), &[("authorization", &auth)]).await;
    assert!(r.status.is_success(), "{}", r.text);
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 2);
    // L'amministratore cancella anna.
    let admin = bearer(ADMIN_KEY);
    let r = request(ts.admin(), "DELETE", &format!("/admin/api/users/{}", anna.user_id), None, loopback(), &[("authorization", &admin)]).await;
    assert!(r.status.is_success(), "{}", r.text);
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 1);
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_state").await, 1);
    assert_eq!(SyncClient::new(&ts.public, &mario.device_key).list(0).await.status, 401);
    // Giulia non ha perso nulla.
    assert_eq!(SyncClient::new(&ts.public, &giulia.device_key).list(0).await.body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_device_revoked_after_authentication_cannot_write_read_or_delete() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let service = ts.server.sync_service().unwrap().clone();
    // La richiesta si autentica...
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    // ...e un attimo dopo il dispositivo viene revocato.
    ts.accounts().revoke_device_admin(&a.user_id, &a.device_id).await.unwrap();
    let item = RawValue::from_string(r#"{"a":1}"#.to_owned()).unwrap();
    assert!(matches!(service.put(&auth, &item_id(1), 0, item).await, Err(ServerError::Unauthorized)));
    assert!(matches!(service.delete(&auth, &item_id(1), None).await, Err(ServerError::Unauthorized)));
    assert!(matches!(service.list(&auth, 0).await, Err(ServerError::Unauthorized)));
    assert_eq!(count(&ts, "SELECT COUNT(*) FROM sync_items").await, 0);
}

#[tokio::test]
async fn the_password_change_does_not_touch_the_items() {
    let ts = TestServer::new(sync_config());
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &a.device_key);
    client.put(&item_id(1), 0, account_item("x")).await;
    let auth = bearer(&a.device_key);
    let body = json!({"current_password": PASSWORD, "new_password": "un'altra password lunga", "sign_out_other_devices": false});
    let r = request(&ts.public, "PUT", "/v1/account/password", Some(body), loopback(), &[("authorization", &auth)]).await;
    assert_eq!(r.status, 204, "{}", r.text);
    assert_eq!(client.list(0).await.body["items"].as_array().unwrap().len(), 1);
}
