//! Supporto dei test: server su una cartella temporanea e richieste con
//! `ConnectInfo` iniettato (senza, tutte le origini finirebbero in una voce sola).
#![allow(dead_code)]

pub mod fake_hf;
pub mod fake_llama;
pub mod fake_openai;
pub mod llm_env;
pub mod sync_client;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::routing::get;
use chrono::{TimeZone, Utc};
use serde_json::Value;
use qamille_server::db::CORE_MIGRATIONS;
use qamille_server::{AdminKey, Config, DeviceAuth, ManualClock, Server};
use tower::ServiceExt;

/// Chiave amministratore dei test: 32 caratteri, 16 distinti.
pub const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef";

/// Cartella temporanea cancellata alla fine del test.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("qamille-server-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("cartella temporanea");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Configurazione dei test: valori predefiniti con la chiave amministratore.
pub fn test_config() -> Config {
    let mut config = Config::with_data_dir("unused");
    config.admin_key = Some(AdminKey::new(ADMIN_KEY).expect("chiave dei test valida"));
    config
}

/// La chiave di sincronizzazione dei test: 64 cifre, 16 distinte.
pub const TEST_SYNC_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// Come [`test_config`], con la sincronizzazione accesa.
pub fn sync_config() -> Config {
    let mut config = test_config();
    config.sync.key = Some(qamille_server::sync::SyncKey::parse(TEST_SYNC_KEY).expect("chiave dei test valida"));
    config
}

pub struct TestServer {
    pub server: Server,
    pub public: Router,
    pub admin: Option<Router>,
    pub clock: Arc<ManualClock>,
    pub dir: TempDir,
}

async fn whoami(DeviceAuth(auth): DeviceAuth) -> axum::Json<Value> {
    axum::Json(serde_json::json!({
        "user_id": auth.user_id,
        "device_id": auth.device_id,
    }))
}

impl TestServer {
    /// Server sul database di una cartella temporanea nuova (la `data_dir` della
    /// configurazione viene sostituita), con `GET /test/whoami` sul router pubblico.
    pub fn new(config: Config) -> Self {
        Self::build(config)
    }

    fn build(mut config: Config) -> Self {
        let dir = TempDir::new();
        config.data_dir = dir.path().to_path_buf();
        let clock = Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap(),
        ));
        let server = Server::open_with_migrations(config, clock.clone(), CORE_MIGRATIONS)
            .expect("server dei test");
        let whoami = Router::new()
            .route("/test/whoami", get(whoami))
            .with_state(server.state());
        let server = server.with_public(whoami);
        let public = server.public_router();
        let admin = server.admin_router();
        TestServer {
            server,
            public,
            admin,
            clock,
            dir,
        }
    }

    pub fn standard() -> Self {
        Self::new(test_config())
    }

    pub fn accounts(&self) -> &Arc<qamille_server::Accounts> {
        self.server.accounts()
    }

    pub fn admin(&self) -> &Router {
        self.admin.as_ref().expect("router amministrativo")
    }
}

/// Un'origine sul loopback (connessione considerata sicura).
pub fn loopback() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 50_000))
}

/// Un'origine remota qualsiasi.
pub fn remote(ip: [u8; 4]) -> SocketAddr {
    SocketAddr::from((ip, 50_000))
}

pub fn ip(addr: SocketAddr) -> IpAddr {
    addr.ip()
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
    pub text: String,
}

impl TestResponse {
    pub fn code(&self) -> Option<&str> {
        self.body["error"]["code"].as_str()
    }
}

/// Manda una richiesta al router con `ConnectInfo(origin)` nelle estensioni.
pub async fn request(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    origin: SocketAddr,
    headers: &[(&str, &str)],
) -> TestResponse {
    let raw = body.map(|b| serde_json::to_vec(&b).unwrap());
    request_raw(router, method, path, raw, origin, headers).await
}

/// Come [`request`], con un corpo già serializzato (anche JSON non valido).
pub async fn request_raw(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<Vec<u8>>,
    origin: SocketAddr,
    headers: &[(&str, &str)],
) -> TestResponse {
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut req = builder
        .body(body.map(Body::from).unwrap_or_else(Body::empty))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(origin));
    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    TestResponse {
        status,
        headers,
        body,
        text,
    }
}

/// Le chiavi di un oggetto JSON, ordinate.
pub fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("oggetto JSON")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

pub fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

/// Crea un utente e completa il cambio della provvisoria: restituisce la chiave
/// del primo dispositivo.
pub async fn user_with_device(
    ts: &TestServer,
    username: &str,
    password: &str,
) -> qamille_server::accounts::LoginOk {
    let created = ts
        .accounts()
        .create_user(username, None, None)
        .await
        .expect("utente creato");
    ts.accounts()
        .change_temporary_password(
            ip(loopback()),
            username,
            &created.temporary_password,
            password,
            "Qamille su Windows",
        )
        .await
        .expect("cambio della provvisoria")
}
