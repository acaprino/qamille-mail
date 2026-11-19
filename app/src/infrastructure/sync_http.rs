//! The mailbox synchronization routes of a Qamille server over HTTP (`/v1/sync/items`, sync spec
//! S7) with `ureq`. The mailboxes' credentials travel in the bodies of these requests, so the
//! address is only the session's, a redirect is never followed, and neither bodies nor answers
//! appear in an error or a log: the errors are fixed texts.
use super::tls;
use crate::application::{
    AppError, AppResult,
    server_account::RemoteEndpoint,
    sync::{Listing, PutOutcome, RemoteItem, SyncServer},
};
use crate::domain::sync::{SyncItem, valid_item_id};
use serde_json::{Value, json};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The server caps a listing at 64 MiB (`MAX_TOTAL_BYTES`); a larger answer is not its answer.
const MAX_BODY: u64 = 72 << 20;

pub struct HttpSyncServer;

fn agent(endpoint: &RemoteEndpoint) -> AppResult<ureq::Agent> {
    Ok(ureq::Agent::config_builder()
        .timeout_global(Some(endpoint.timeout.max(REQUEST_TIMEOUT)))
        // A redirect would carry the credentials of the mailboxes to another address.
        .max_redirects(0)
        .http_status_as_error(false)
        .tls_config(tls::config(endpoint.ca_pem.as_deref())?)
        .build()
        .into())
}

fn bearer(endpoint: &RemoteEndpoint) -> String {
    format!("Bearer {}", endpoint.device_key)
}

fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> AppResult<(u16, Value)> {
    let mut response = response.map_err(|_| AppError::new("unreachable", "The Qamille server does not answer"))?;
    let status = response.status().as_u16();
    let text = response.body_mut().with_config().limit(MAX_BODY).read_to_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// The application error for a server answer: fixed English texts, the interface translates them.
fn server_error(status: u16, body: &Value) -> AppError {
    let code = body["error"]["code"].as_str().unwrap_or_default();
    match (code, status) {
        ("unauthorized", _) | (_, 401) => AppError::new("unauthorized", "This device is no longer connected to the server"),
        ("subscription_required", _) | (_, 402) => AppError::new("subscription_required", "Mailbox synchronization needs an active Qamille subscription"),
        ("sync_limit", _) => AppError::new("sync_limit", "The server has reached its maximum of synchronized mailboxes"),
        ("payload_too_large", _) | (_, 413) => AppError::new("payload_too_large", "The mailbox is too large for the server"),
        // Sync has no plan quota; if the core ever sends one it is a pause, like a missing subscription.
        ("quota_exceeded", _) => AppError::new("subscription_required", "Mailbox synchronization is paused: the monthly limit of your plan is used up"),
        ("rate_limited", _) | (_, 429) => AppError::new("rate_limited", "The server is busy. Try again in a moment"),
        ("tls_required", _) => AppError::new("tls_required", "The server accepts mailboxes only over HTTPS"),
        ("invalid_request", _) | (_, 400) => AppError::new("invalid_request", "The server refused the mailbox item"),
        ("not_found", _) | (_, 404) => AppError::new("not_found", "This server does not synchronize mailboxes"),
        _ => AppError::new("server_error", "The Qamille server answered with an error"),
    }
}

fn remote_item(value: &Value) -> Option<RemoteItem> {
    let id = value["id"].as_str()?.to_string();
    if !valid_item_id(&id) {
        return None;
    }
    let deleted = value["deleted"].as_bool().unwrap_or(false);
    // An item this app cannot read (a future schema) is kept as «no content»: it is never applied.
    let item = value.get("item").and_then(|item| serde_json::from_value::<SyncItem>(item.clone()).ok());
    Some(RemoteItem { id, version: value["version"].as_u64()?, deleted, item })
}

fn check_id(id: &str) -> AppResult<()> {
    if valid_item_id(id) { Ok(()) } else { Err(AppError::new("invalid_request", "The server refused the mailbox item")) }
}

impl SyncServer for HttpSyncServer {
    fn list(&self, endpoint: &RemoteEndpoint, since: u64) -> AppResult<Listing> {
        let agent = agent(endpoint)?;
        let (status, body) = read(agent.get(format!("{}/v1/sync/items?since={since}", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        if status != 200 {
            return Err(server_error(status, &body));
        }
        let bad = || AppError::new("server_error", "The Qamille server answered with an error");
        let items = body["items"].as_array().ok_or_else(bad)?;
        Ok(Listing {
            version: body["version"].as_u64().ok_or_else(bad)?,
            epoch: body["epoch"].as_u64().unwrap_or(0),
            items: items.iter().filter_map(remote_item).collect(),
        })
    }

    fn put(&self, endpoint: &RemoteEndpoint, id: &str, base_version: u64, item: &SyncItem) -> AppResult<PutOutcome> {
        check_id(id)?;
        let agent = agent(endpoint)?;
        let body = json!({ "base_version": base_version, "item": item });
        let (status, answer) = read(agent.put(format!("{}/v1/sync/items/{id}", endpoint.url)).header("Authorization", bearer(endpoint)).send_json(&body))?;
        match status {
            200 => Ok(PutOutcome::Written { version: answer["version"].as_u64().ok_or_else(|| AppError::new("server_error", "The Qamille server answered with an error"))? }),
            409 if answer["error"]["code"] == "sync_conflict" => Ok(PutOutcome::Conflict(remote_item(&answer["error"]["current"]))),
            _ => Err(server_error(status, &answer)),
        }
    }

    fn delete(&self, endpoint: &RemoteEndpoint, id: &str, base_version: Option<u64>) -> AppResult<()> {
        check_id(id)?;
        let agent = agent(endpoint)?;
        let query = base_version.map(|v| format!("?base_version={v}")).unwrap_or_default();
        let (status, body) = read(agent.delete(format!("{}/v1/sync/items/{id}{query}", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        match status {
            200 | 404 => Ok(()),
            // Changed elsewhere after we saw it: the removal is not blind, the next pass looks again.
            409 => Err(AppError::new("sync_conflict", "The mailbox changed on another computer; it will be retried")),
            _ => Err(server_error(status, &body)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_used_up_quota_pauses_like_a_missing_subscription() {
        let body = serde_json::json!({"error": {"code": "quota_exceeded", "message": "x", "retry_after_s": 3600}});
        assert_eq!(server_error(429, &body).code, "subscription_required");
        let busy = serde_json::json!({"error": {"code": "rate_limited", "message": "x"}});
        assert_eq!(server_error(429, &busy).code, "rate_limited");
    }
}
