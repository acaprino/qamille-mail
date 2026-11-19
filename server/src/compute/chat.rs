//! `POST /v1/chat/completions`: l'assistente (gateway LLM). Dietro la chiave del dispositivo, il
//! diritto d'uso (`Resource::Llm`), il consenso al provider attuale, il tetto giornaliero e un
//! cancello suo per le richieste in volo, separato da quello del calcolo. Mai il contenuto nei
//! log (INV-33): la riga `llm` la scrive il gateway, con modello, token e durata.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use tower_http::timeout::TimeoutLayer;

use super::{Caller, ComputeError, ComputeState, socket_of};
use crate::ext::Resource;
use crate::llm::{BUSY_RETRY_S, LlmGateway, LlmSlot, ROUTE_EXTRA_SECS};

/// L'intestazione con cui l'app dichiara a quale provider ha dato il consenso:
/// `<external>;<label>`.
pub const CONSENT_HEADER: &str = "x-qamille-llm-consent";

/// Chi è, per i contatori, il chiamante senza utente (la credenziale fissa delle prove).
const FIXED_USER: &str = "(fixed)";

pub(super) fn routes(state: &ComputeState, gateway: &Arc<LlmGateway>) -> Router<ComputeState> {
    let config = gateway.config();
    let limit = DefaultBodyLimit::max(config.max_request_bytes);
    let timeout = TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(config.timeout_secs + ROUTE_EXTRA_SECS),
    );
    Router::new()
        .route("/v1/chat/completions", post(chat_completions)
                .layer::<_, std::convert::Infallible>(limit)
                .layer::<_, std::convert::Infallible>(timeout))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_llm))
}

fn user_of(caller: &Caller) -> &str {
    match caller {
        Caller::Device { user_id, .. } => user_id,
        Caller::Fixed => FIXED_USER,
    }
}

/// L'ordine dei controlli: chiave e diritto (`401`, `402`), consenso (`409`), tetto giornaliero
/// (`429`), posto tra le richieste in volo (`429`); poi la lettura del corpo e la validazione.
async fn require_llm(State(state): State<ComputeState>, mut request: Request, next: Next) -> Response {
    let Some(gateway) = state.llm.clone() else {
        return ComputeError::NotFound.into_response();
    };
    let caller = match super::authorize(&state, request.headers(), socket_of(&request), Resource::Llm).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response(),
    };
    let consent = request.headers().get(CONSENT_HEADER).map(|value| value.as_bytes());
    if !gateway.consent_matches(consent) {
        return ComputeError::LlmConsentRequired.into_response();
    }
    let user_id = user_of(&caller).to_owned();
    if let Err(error) = gateway.within_daily_cap(&user_id).await {
        return ComputeError::from(error).into_response();
    }
    let Some(slot) = gateway.enter(&user_id) else {
        return ComputeError::RateLimited { retry_after_s: BUSY_RETRY_S }.into_response();
    };
    request.extensions_mut().insert(caller.clone());
    request.extensions_mut().insert(Arc::new(slot));
    let response = next.run(request).await;
    if let (Some(sink), Caller::Device { user_id, .. }) = (&state.usage, &caller) {
        sink.record(user_id, super::Usage::new(1, 0.0, 0));
    }
    response
}

async fn chat_completions(
    State(state): State<ComputeState>,
    caller: Option<axum::Extension<Caller>>,
    slot: Option<axum::Extension<Arc<LlmSlot>>>,
    body: Bytes,
) -> Result<Response, ComputeError> {
    let (Some(gateway), Some(axum::Extension(caller)), Some(axum::Extension(slot))) = (&state.llm, caller, slot)
    else {
        return Err(ComputeError::NotFound);
    };
    let answer = gateway.complete(user_of(&caller), slot, body.to_vec()).await?;
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], answer).into_response())
}
