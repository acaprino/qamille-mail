//! Un client simulato della sincronizzazione: parla con il router pubblico come l'app, con la
//! chiave di un dispositivo. Usato dai test del core e dell'enterprise (`#[path]`).
#![allow(dead_code)]

use std::net::SocketAddr;

use axum::Router;
use serde_json::{Value, json};

use crate::support::{TestResponse, bearer, loopback, request, request_raw};

pub struct SyncClient<'a> {
    pub router: &'a Router,
    pub device_key: String,
    pub origin: SocketAddr,
}

/// Un id di elemento: 64 cifre esadecimali minuscole.
pub fn item_id(n: u32) -> String {
    format!("{n:064x}")
}

/// Un elemento verosimile (il contenuto che manderà l'app), con `marker` nel token.
pub fn account_item(marker: &str) -> Value {
    json!({
        "schema": 1,
        "kind": "mail_account",
        "updated_at": "2026-11-17T10:00:00Z",
        "credential_updated_at": "2026-11-17T10:00:00Z",
        "account": {"address": "giulia@example.com", "provider_id": "gmail", "oauth_client_id": "client-1"},
        "credential": {"refresh_token": marker},
        "mail_signature": "Giulia",
    })
}

impl<'a> SyncClient<'a> {
    pub fn new(router: &'a Router, device_key: &str) -> Self {
        SyncClient { router, device_key: device_key.to_owned(), origin: loopback() }
    }

    pub fn from(mut self, origin: SocketAddr) -> Self {
        self.origin = origin;
        self
    }

    fn headers(&self) -> Vec<(String, String)> {
        vec![("authorization".to_owned(), bearer(&self.device_key))]
    }

    async fn call(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> TestResponse {
        let headers = self.headers();
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        request_raw(self.router, method, path, body, self.origin, &refs).await
    }

    pub async fn list(&self, since: u64) -> TestResponse {
        self.call("GET", &format!("/v1/sync/items?since={since}"), None).await
    }

    pub async fn put(&self, id: &str, base_version: u64, item: Value) -> TestResponse {
        let body = serde_json::to_vec(&json!({"base_version": base_version, "item": item})).unwrap();
        self.put_raw(id, body).await
    }

    pub async fn put_raw(&self, id: &str, body: Vec<u8>) -> TestResponse {
        self.call("PUT", &format!("/v1/sync/items/{id}"), Some(body)).await
    }

    pub async fn delete(&self, id: &str, base_version: Option<u64>) -> TestResponse {
        let query = base_version.map(|b| format!("?base_version={b}")).unwrap_or_default();
        self.call("DELETE", &format!("/v1/sync/items/{id}{query}"), None).await
    }
}

/// Una richiesta senza il client, per i casi che non passano da una chiave.
pub async fn bare(router: &Router, method: &str, path: &str, body: Option<Value>, origin: SocketAddr, headers: &[(&str, &str)]) -> TestResponse {
    request(router, method, path, body, origin, headers).await
}

/// Lo SHA-256 esadecimale di una chiave di dispositivo, come lo conserva il server.
pub fn key_sha256(key: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(key.as_bytes()))
}
