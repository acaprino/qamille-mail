//! Il core da solo e le estensioni montate sopra: router uniti una volta sola.
mod support;

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use qamille_server::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus};
use qamille_server::ext::{
    AccountExtension, AdminPage, AdminUsersExtension, AdminUsersExtra, BoxFut, Entitlement,
    InfoExtension, JsonMap, Resource,
};
use qamille_server::{Server, ServerError, StartError};
use serde_json::{Value, json};
use support::{
    ADMIN_KEY, TempDir, TestResponse, bearer, ip, loopback, remote, request, request_raw,
    test_config,
};

struct Fake;

impl ComputeBackend for Fake {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        Ok(vec![text.to_owned()])
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        Ok(inputs.iter().map(|_| vec![0.0; DIM]).collect())
    }
    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        self.embed(inputs).map(|vectors| (vectors, 1))
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Ok(vec![0.0; DIM])
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Ok(String::new())
    }
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
}

struct RefuseAll;

impl Entitlement for RefuseAll {
    fn require<'a>(&'a self, _: &'a str, _: Resource) -> BoxFut<'a, Result<(), ServerError>> {
        Box::pin(async { Err(ServerError::SubscriptionRequired) })
    }
}

/// Il core da solo, su una cartella nuova (la cartella vive quanto il test).
fn core_server() -> (Server, TempDir) {
    let dir = TempDir::new();
    let mut config = test_config();
    config.data_dir = dir.path().to_path_buf();
    (Server::open(config).expect("server core"), dir)
}

async fn get_path(router: &Router, path: &str, headers: &[(&str, &str)]) -> TestResponse {
    request(router, "GET", path, None, loopback(), headers).await
}

async fn post(router: &Router, path: &str, body: &str) -> TestResponse {
    request_raw(
        router,
        "POST",
        path,
        Some(body.as_bytes().to_vec()),
        loopback(),
        &[],
    )
    .await
}

async fn get_admin(router: &Router, path: &str) -> TestResponse {
    get_path(router, path, &[("authorization", &bearer(ADMIN_KEY))]).await
}

/// Crea un utente con la sua prima chiave di dispositivo.
async fn device_key(server: &Server, username: &str) -> String {
    let created = server
        .accounts()
        .create_user(username, None, None)
        .await
        .unwrap();
    server
        .accounts()
        .change_temporary_password(
            ip(loopback()),
            username,
            &created.temporary_password,
            "una password lunga",
            "Qamille su Windows",
        )
        .await
        .unwrap()
        .device_key
}

async fn post_embeddings(router: &Router, key: &str) -> TestResponse {
    let body =
        serde_json::to_vec(&json!({"model": MODEL_ID, "input": ["a"], "dimensions": DIM})).unwrap();
    request_raw(
        router,
        "POST",
        "/v1/embeddings",
        Some(body),
        remote([10, 0, 0, 9]),
        &[("authorization", &bearer(key))],
    )
    .await
}

#[tokio::test]
async fn core_alone_answers_404_in_the_server_shape_on_enterprise_routes() {
    let (core, _dir) = core_server();
    let router = core.try_public_router().unwrap();
    for path in [
        "/v1/signup",
        "/v1/password-reset",
        "/v1/login/browser",
        "/v1/billing/plans",
    ] {
        let r = post(&router, path, "{}").await;
        assert_eq!((r.status.as_u16(), r.code()), (404, Some("not_found")), "{path}");
    }
}

#[tokio::test]
async fn a_duplicate_route_is_a_start_error() {
    let (core, _dir) = core_server();
    let extra = Router::new().route("/v1/account", get(|| async { "x" }));
    assert!(matches!(
        core.with_public(extra).try_public_router(),
        Err(StartError::DuplicateRoute(_))
    ));
}

#[tokio::test]
async fn the_extension_page_replaces_the_core_page_but_not_its_api() {
    let (core, _dir) = core_server();
    let server = core.admin_page(AdminPage::Extension);
    let admin = server.try_admin_router().unwrap().unwrap();
    assert_eq!(get_admin(&admin, "/admin").await.status, 404);
    assert_eq!(get_admin(&admin, "/admin/app.js").await.status, 404);
    assert_eq!(get_admin(&admin, "/admin/api/users").await.status, 200);
}

#[tokio::test]
async fn the_core_page_is_there_by_default() {
    let (core, _dir) = core_server();
    let admin = core.try_admin_router().unwrap().unwrap();
    assert_eq!(get_admin(&admin, "/admin").await.status, 200);
}

#[tokio::test]
async fn the_order_of_with_calls_does_not_matter() {
    let (core, _dir) = core_server();
    let server = core
        .with_entitlement(Arc::new(RefuseAll))
        .with_compute(Arc::new(Fake), Arc::new(qamille_server::compute::speech::NoSpeech), 1);
    let key = device_key(&server, "mario").await;
    let router = server.try_public_router().unwrap();
    assert_eq!(post_embeddings(&router, &key).await.status, 402);

    let (core, _dir) = core_server();
    let server = core
        .with_compute(Arc::new(Fake), Arc::new(qamille_server::compute::speech::NoSpeech), 1)
        .with_entitlement(Arc::new(RefuseAll));
    let key = device_key(&server, "mario").await;
    let router = server.try_public_router().unwrap();
    assert_eq!(post_embeddings(&router, &key).await.status, 402);
}

#[tokio::test]
async fn info_and_account_extensions_add_their_fields_after_the_core_ones() {
    let (core, _dir) = core_server();
    let info: InfoExtension = Arc::new(|map: &mut JsonMap| {
        map.insert("signup".to_owned(), json!(true));
    });
    let account: AccountExtension = Arc::new(|_user_id: String| {
        Box::pin(async {
            let mut map = JsonMap::new();
            map.insert("identities".to_owned(), json!(["google"]));
            Ok(map)
        })
    });
    let server = core.with_info(info).with_account(account);
    let key = device_key(&server, "mario").await;
    let router = server.try_public_router().unwrap();
    let info = get_path(&router, "/v1/info", &[]).await;
    assert_eq!(info.body["signup"], true);
    assert_eq!(info.body["api"], 1);
    let account = get_path(&router, "/v1/account", &[("authorization", &bearer(&key))]).await;
    assert_eq!(account.status, 200);
    assert_eq!(account.body["identities"], json!(["google"]));
    assert_eq!(account.body["username"], "mario");
}

#[tokio::test]
async fn core_info_and_account_have_no_enterprise_fields() {
    let (core, _dir) = core_server();
    let key = device_key(&core, "mario").await;
    let router = core.try_public_router().unwrap();
    let info = get_path(&router, "/v1/info", &[]).await;
    for field in ["signup", "password_reset", "billing", "login_providers"] {
        assert!(info.body.get(field).is_none(), "{field}");
    }
    let account = get_path(&router, "/v1/account", &[("authorization", &bearer(&key))]).await;
    assert!(account.body.get("identities").is_none());
    assert!(account.body.get("subscription").is_none());
}

#[tokio::test]
async fn admin_users_extension_adds_per_user_and_top_level_fields() {
    let (core, _dir) = core_server();
    let created = core.accounts().create_user("mario", None, None).await.unwrap();
    let id = created.id.clone();
    let extension: AdminUsersExtension = Arc::new(move || {
        let id = id.clone();
        Box::pin(async move {
            let mut top = JsonMap::new();
            top.insert("billing".to_owned(), json!({"on": true}));
            let mut mine = JsonMap::new();
            mine.insert("subscription".to_owned(), json!("active"));
            let mut default = JsonMap::new();
            default.insert("subscription".to_owned(), json!("none"));
            Ok(AdminUsersExtra {
                top,
                per_user: HashMap::from([(id, mine)]),
                per_user_default: default,
            })
        })
    });
    let server = core.with_admin_users(extension);
    server.accounts().create_user("luigi", None, None).await.unwrap();
    let admin = server.try_admin_router().unwrap().unwrap();
    let r = get_admin(&admin, "/admin/api/users").await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["billing"], json!({"on": true}));
    let users = r.body["users"].as_array().unwrap();
    let find = |name: &str| -> &Value {
        users.iter().find(|u| u["username"] == name).unwrap()
    };
    assert_eq!(find("mario")["subscription"], "active");
    assert_eq!(find("luigi")["subscription"], "none");
}

#[tokio::test]
async fn with_entitlement_and_usage_sink_after_the_compute_state_was_built_still_apply() {
    let (core, _dir) = core_server();
    let server = core.with_compute(Arc::new(Fake), Arc::new(qamille_server::compute::speech::NoSpeech), 1);
    // Building the compute state (as a router would) must not freeze the gate without its entitlement.
    assert!(server.compute_state().is_some());
    let server = server.with_entitlement(Arc::new(RefuseAll));
    let key = device_key(&server, "mario").await;
    let router = server.try_public_router().unwrap();
    assert_eq!(post_embeddings(&router, &key).await.status, 402);
}

#[tokio::test]
async fn every_extension_admin_api_route_answers_401_without_the_admin_key() {
    let (core, _dir) = core_server();
    let extra = Router::new()
        .route("/admin/api/ext", get(|| async { "x" }).post(|| async { "x" }).delete(|| async { "x" }))
        .route("/admin/api/ext/{id}/thing", get(|| async { "x" }).put(|| async { "x" }));
    let admin = core.with_admin(extra).try_admin_router().unwrap().unwrap();
    for (method, path) in [
        ("GET", "/admin/api/ext"),
        ("POST", "/admin/api/ext"),
        ("DELETE", "/admin/api/ext"),
        ("GET", "/admin/api/ext/abc/thing"),
        ("PUT", "/admin/api/ext/abc/thing"),
    ] {
        for headers in [vec![], vec![("authorization", "Bearer wrong")]] {
            let r = request(&admin, method, path, None, loopback(), &headers).await;
            assert_eq!((r.status.as_u16(), r.code()), (401, Some("unauthorized")), "{method} {path}");
        }
        let ok = request(&admin, method, path, None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
        assert_eq!(ok.status, 200, "{method} {path} with the key");
    }
}
