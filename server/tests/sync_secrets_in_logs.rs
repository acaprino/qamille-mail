//! La sincronizzazione non scrive segreti nei log: né il contenuto degli elementi, né la loro
//! id, né la chiave del server, né quella del dispositivo (INV-35, INV-38), nemmeno nei
//! percorsi d'errore. Nessun `Debug` li stampa.
#![allow(dead_code)]

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use qamille_server::ServerError;
use qamille_server::ext::{BoxFut, Entitlement, Resource};
use qamille_server::sync::SyncKey;
use qamille_server::sync::store::{self, Limits, Who};
use serde_json::json;
use support::sync_client::{SyncClient, account_item, item_id};
use support::{TEST_SYNC_KEY, TestServer, remote, sync_config, user_with_device};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .init();
        capture
    })
}

const MARKER: &str = "MARKER-TOKEN-NEL-LOG";
const OTHER_KEY: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const PASSWORD: &str = "una password lunga";

struct Deny;

impl Entitlement for Deny {
    fn require<'a>(&'a self, _: &'a str, _: Resource) -> BoxFut<'a, Result<(), ServerError>> {
        Box::pin(async { Err(ServerError::SubscriptionRequired) })
    }
}

#[tokio::test]
async fn secrets_in_logs() {
    let log = capture();
    let mut config = sync_config();
    config.sync.max_items = 2;
    config.sync.max_item_bytes = 2048;
    config.plaintext_from = vec![];
    let ts = TestServer::new(config);
    let mario = user_with_device(&ts, "mario-marker", PASSWORD).await;
    let key = mario.device_key.clone();
    let client = SyncClient::new(&ts.public, &key);
    let id = |n| item_id(n);
    let secret_item = account_item(MARKER);

    // 200: scrittura, lettura, cancellazione.
    assert_eq!(client.put(&id(1), 0, secret_item.clone()).await.status, 200);
    assert_eq!(client.put(&id(2), 0, secret_item.clone()).await.status, 200);
    assert_eq!(client.list(0).await.status, 200);
    // 409 conflitto e 409 limite.
    assert_eq!(client.put(&id(1), 0, secret_item.clone()).await.status, 409);
    assert_eq!(client.put(&id(3), 0, secret_item.clone()).await.status, 409);
    // 400: id non valido, corpo rotto con il marker dentro.
    assert_eq!(client.put(MARKER, 0, secret_item.clone()).await.status, 400);
    let broken = format!(r#"{{"base_version":0,"item":{{"a":"{MARKER}"#).into_bytes();
    assert_eq!(client.put_raw(&id(4), broken).await.status, 400);
    // 413: troppo grande, con il marker dentro.
    let big = json!({"x": format!("{MARKER}{}", "a".repeat(4000))});
    assert_eq!(client.put(&id(5), 0, big).await.status, 413);
    // 404.
    assert_eq!(client.delete(&id(9), None).await.status, 404);
    assert_eq!(client.delete(&id(1), None).await.status, 200);
    // 401 con una chiave sbagliata che contiene il marker.
    let wrong = format!("spk_dev_{MARKER}");
    assert_eq!(SyncClient::new(&ts.public, &wrong).list(0).await.status, 401);
    // Connessione in chiaro da un indirizzo remoto.
    let plain = SyncClient::new(&ts.public, &key).from(remote([203, 0, 113, 9]));
    assert_eq!(plain.put(&id(6), 0, secret_item.clone()).await.status, 400);
    // 500: elementi che non si aprono con un'altra chiave.
    let other = SyncKey::parse(OTHER_KEY).unwrap();
    let (user, device, dk) = (mario.user_id.clone(), mario.device_id.clone(), support::sync_client::key_sha256(&mario.device_key));
    let unreadable = ts
        .accounts()
        .db()
        .blocking(move |c| Ok(store::list(c, &other, &Who { user_id: &user, device_id: &device, key_sha256: &dk }, 0).is_err()))
        .await
        .unwrap();
    assert!(unreadable);
    // Operazioni di servizio: rekey, reset, prova, pulizia, avvio.
    let (old, new) = (SyncKey::parse(TEST_SYNC_KEY).unwrap(), SyncKey::parse(OTHER_KEY).unwrap());
    ts.accounts().db().blocking(move |c| Ok(store::rekey(c, &old, &new).unwrap())).await.unwrap();
    let service = ts.server.sync_service().unwrap().clone();
    service.log_startup(true).await; // la chiave del server non apre più: errore nel log, senza segreti
    service.purge_expired().await.unwrap();
    ts.accounts().db().blocking(|c| Ok(store::reset(c).unwrap())).await.unwrap();
    let _ = Limits { max_items: 1, max_item_bytes: 1024 };

    // 402 con un Entitlement che nega.
    let TestServer { server, clock, dir, .. } = ts;
    let server = server.with_entitlement(Arc::new(Deny));
    let (public, admin) = (server.public_router(), server.admin_router());
    let denied = TestServer { server, public, admin, clock, dir };
    assert_eq!(SyncClient::new(&denied.public, &key).put(&id(7), 0, secret_item).await.status, 402);

    let text = log.text();
    assert!(text.contains("richiesta") || text.contains("/v1/sync/items"), "il log di accesso c'è: {text}");
    assert!(text.contains("/v1/sync/items/{id}"), "la rotta si logga come schema: {text}");
    for secret in [MARKER, TEST_SYNC_KEY, &TEST_SYNC_KEY[..16], OTHER_KEY, &OTHER_KEY[..16], key.as_str(), PASSWORD, "Bearer"] {
        assert!(!text.contains(secret), "il log contiene «{secret}»:\n{text}");
    }
    for n in 1..=9 {
        assert!(!text.contains(&id(n)), "il log contiene l'id di un elemento ({n})");
    }
    assert!(!text.contains("account_item") && !text.contains("refresh_token"));
}

#[test]
fn the_debug_of_every_sync_type_hides_secrets() {
    let key = SyncKey::parse(TEST_SYNC_KEY).unwrap();
    let config = qamille_server::sync::SyncConfig { key: Some(key.clone()), old_key: Some(key.clone()), ..Default::default() };
    let item = serde_json::value::RawValue::from_string(format!(r#"{{"t":"{MARKER}"}}"#)).unwrap();
    let stored = store::StoredItem {
        id: item_id(1),
        version: 3,
        deleted: false,
        updated_at: "2026-11-17T10:00:00.000Z".to_owned(),
        updated_by_device: "dev_1".to_owned(),
        item: Some(item),
    };
    let listing = store::Listing { version: 3, epoch: 0, items: vec![stored] };
    let texts = [
        format!("{key:?}"),
        format!("{key:#?}"),
        format!("{config:?}"),
        format!("{config:#?}"),
        format!("{listing:?}"),
        format!("{listing:#?}"),
        format!("{:?}", store::PutOutcome::Conflict(None)),
        format!("{:?}", store::StoreError::Unreadable),
        format!("{:?}", qamille_server::sync::OpenError),
    ];
    for text in texts {
        assert!(!text.contains(MARKER) && !text.contains(&item_id(1)), "{text}");
        assert!(!text.contains(TEST_SYNC_KEY) && !text.contains(&TEST_SYNC_KEY[..16]), "{text}");
    }
}

#[tokio::test]
async fn the_error_bodies_do_not_echo_the_request() {
    let mut config = sync_config();
    config.sync.max_item_bytes = 1024;
    let ts = TestServer::new(config);
    let mario = user_with_device(&ts, "mario", PASSWORD).await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    let id = item_id(1);
    let big = json!({"x": format!("{MARKER}{}", "a".repeat(2000))});
    let too_big = client.put(&id, 0, big).await;
    let broken = client.put_raw(&id, format!(r#"{{"item":"{MARKER}"#).into_bytes()).await;
    let bad_id = client.put(MARKER, 0, account_item(MARKER)).await;
    for r in [too_big, broken, bad_id] {
        assert!(!r.text.contains(MARKER) && !r.text.contains(&id), "{}", r.text);
    }
}
