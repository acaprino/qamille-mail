//! `GET /v1/sync/items`, `PUT` e `DELETE /v1/sync/items/{id}`.
//!
//! Il router ha il suo limite del corpo (`sync_max_item_bytes` più un margine) e i suoi strati,
//! gli stessi del core per la forma degli errori e il log di accesso: nel log c'è la rotta come
//! schema (`/v1/sync/items/{id}`), mai l'id né il contenuto (INV-38).

use std::fmt;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRef, FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::time::Instant;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use super::store::{DeleteOutcome, PutOutcome, StoredItem};
use super::{BODY_MARGIN, BUSY_RETRY_S, Slot, SyncService};
use crate::accounts::AuthContext;
use crate::error::ServerError;
use crate::ext::Resource;
use crate::http::{AppState, REQUEST_TIMEOUT, SecureOrigin, bearer, finish_response};

/// Lo stato delle rotte: quello del core e il servizio.
#[derive(Clone)]
pub struct SyncState {
    pub app: AppState,
    pub service: Arc<SyncService>,
}

impl FromRef<SyncState> for AppState {
    fn from_ref(state: &SyncState) -> AppState {
        state.app.clone()
    }
}

/// Chi può usare la sincronizzazione, nell'ordine: connessione sicura, limite per origine sulle
/// chiavi sbagliate, chiave di un dispositivo attivo, diritto d'uso (`Resource::Sync`), posto
/// tra le richieste in volo dell'utente. Il corpo si legge dopo, nel gestore.
pub struct SyncAccess {
    pub auth: AuthContext,
    _slot: Slot,
}

impl fmt::Debug for SyncAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncAccess").field("auth", &self.auth).finish_non_exhaustive()
    }
}

impl FromRequestParts<SyncState> for SyncAccess {
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &SyncState) -> Result<Self, ServerError> {
        let SecureOrigin(origin) = SecureOrigin::from_request_parts(parts, state).await?;
        let gate = state.service.gate();
        gate.failures.check(origin.ip, Instant::now())?;
        let key = bearer(parts).ok_or(ServerError::Unauthorized)?;
        let auth = match state.app.accounts.authenticate(key).await {
            Ok(auth) => auth,
            Err(error) => {
                if matches!(error, ServerError::Unauthorized) {
                    gate.failures.record_failure(origin.ip, Instant::now());
                }
                return Err(error);
            }
        };
        if let Some(entitlement) = &state.app.entitlement {
            entitlement.require(&auth.user_id, Resource::Sync).await?;
        }
        let slot = gate
            .enter(&auth.user_id)
            .ok_or(ServerError::RateLimited { retry_after_s: BUSY_RETRY_S })?;
        Ok(SyncAccess { auth, _slot: slot })
    }
}

/// Le rotte, con il loro stato già applicato e senza fallback (lo ha il router pubblico).
pub fn routes(app: AppState, service: Arc<SyncService>) -> Router {
    let limit = service.limits().max_item_bytes + BODY_MARGIN;
    Router::new()
        .route("/v1/sync/items", get(list_items))
        .route("/v1/sync/items/{id}", axum::routing::put(put_item).delete(delete_item))
        .layer(DefaultBodyLimit::max(limit))
        .layer(RequestBodyLimitLayer::new(limit))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, REQUEST_TIMEOUT))
        .layer(middleware::from_fn(finish_response))
        .with_state(SyncState { app, service })
}

#[derive(Deserialize)]
struct ListQuery {
    since: Option<u64>,
}

#[derive(Deserialize)]
struct DeleteQuery {
    base_version: Option<u64>,
}

/// Il corpo di `PUT`: la versione da cui parte il dispositivo e l'oggetto, che il server
/// conserva così com'è.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutBody {
    base_version: u64,
    item: Box<RawValue>,
}

impl fmt::Debug for PutBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PutBody").field("base_version", &self.base_version).finish_non_exhaustive()
    }
}

/// `409 sync_conflict`, con l'elemento attuale (o `null` se non c'è più) in `error.current`.
fn conflict(current: Option<StoredItem>) -> ServerError {
    let current = current
        .and_then(|item| serde_json::to_value(&item).ok())
        .unwrap_or(serde_json::Value::Null);
    ServerError::Extension {
        status: StatusCode::CONFLICT,
        code: "sync_conflict",
        message: "L'elemento è cambiato su un altro computer.".to_owned(),
        retry_after_s: None,
        extra: Some(serde_json::json!({ "current": current })),
    }
}

async fn list_items(
    State(state): State<SyncState>,
    access: SyncAccess,
    Query(query): Query<ListQuery>,
) -> Result<Response, ServerError> {
    let listing = state.service.list(&access.auth, query.since.unwrap_or(0)).await?;
    Ok(Json(listing).into_response())
}

async fn put_item(
    State(state): State<SyncState>,
    access: SyncAccess,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Response, ServerError> {
    let request: PutBody = serde_json::from_slice(&body).map_err(|_| {
        ServerError::InvalidRequest(
            "Corpo della richiesta non valido: servono base_version e item.".to_owned(),
        )
    })?;
    match state.service.put(&access.auth, &id, request.base_version, request.item).await? {
        PutOutcome::Written { version, updated_at } => Ok(Json(serde_json::json!({
            "id": id,
            "version": version,
            "updated_at": updated_at,
        }))
        .into_response()),
        PutOutcome::Conflict(current) => Err(conflict(current)),
    }
}

async fn delete_item(
    State(state): State<SyncState>,
    access: SyncAccess,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> Result<Response, ServerError> {
    match state.service.delete(&access.auth, &id, query.base_version).await? {
        DeleteOutcome::Deleted { version } | DeleteOutcome::AlreadyDeleted { version } => {
            Ok(Json(serde_json::json!({"id": id, "version": version, "deleted": true})).into_response())
        }
        DeleteOutcome::Conflict(current) => Err(conflict(Some(current))),
        DeleteOutcome::Missing => Err(ServerError::not_found()),
    }
}
