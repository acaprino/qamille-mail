//! INV-30: nessuna rotta pubblica del core crea un utente.
mod support;

use serde_json::json;
use support::{TestServer, loopback, request};

#[tokio::test]
async fn no_public_route_of_the_core_creates_a_user() {
    let core = TestServer::standard();
    let body = json!({"username":"nuovo","email":"n@e.it","password":"Una-password-lunga-1","device_name":"PC"});
    for (method, path) in [
        ("POST", "/v1/login"),
        ("POST", "/v1/login/password"),
        ("POST", "/v1/signup"),
        ("POST", "/v1/signup/confirm"),
        ("POST", "/v1/password-reset"),
        ("POST", "/v1/password-reset/confirm"),
        ("GET", "/v1/auth/google/start"),
        ("POST", "/v1/login/browser"),
    ] {
        request(&core.public, method, path, Some(body.clone()), loopback(), &[]).await;
    }
    assert!(core.accounts().list_users().await.unwrap().is_empty());
}
