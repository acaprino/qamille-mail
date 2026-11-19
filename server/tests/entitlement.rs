//! L'`Entitlement` del calcolo remoto: il calcolo chiede, non conosce gli abbonamenti.
mod support;

use std::sync::{Arc, Mutex};

use qamille_server::compute::{
    ComputeAuth, ComputeBackend, ComputeError, ComputeState, DEVICE_AUTH_WINDOW, DIM, DeviceGate,
    MODEL_ID, ModelStatus, remote_router,
};
use qamille_server::ext::{BoxFut, Entitlement, Resource};
use qamille_server::ServerError;
use serde_json::json;
use support::{TestServer, bearer, loopback, request_raw, test_config, user_with_device};

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

struct Decide {
    seen: Arc<Mutex<Vec<(String, Resource)>>>,
    answer: fn() -> Result<(), ServerError>,
}

impl Entitlement for Decide {
    fn require<'a>(&'a self, user_id: &'a str, resource: Resource) -> BoxFut<'a, Result<(), ServerError>> {
        self.seen.lock().unwrap().push((user_id.to_owned(), resource));
        let answer = (self.answer)();
        Box::pin(async move { answer })
    }
}

async fn post_embeddings(
    entitlement: Decide,
) -> (support::TestResponse, String) {
    let ts = TestServer::new(test_config());
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let user_id = ts.accounts().user_id_by_username("mario").await.unwrap();
    let gate = DeviceGate::with_failures(
        Arc::clone(ts.accounts()),
        Arc::from(Vec::new()),
        2,
        120,
        DEVICE_AUTH_WINDOW,
    )
    .with_entitlement(Some(Arc::new(entitlement)));
    let state = ComputeState::new(Arc::new(Fake), ComputeAuth::Devices(Arc::new(gate)), "remote");
    let router = remote_router(state);
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "input": ["a"], "dimensions": DIM})).unwrap();
    let key = bearer(&login.device_key);
    let response = request_raw(&router, "POST", "/v1/embeddings", Some(body), loopback(), &[("authorization", &key)]).await;
    (response, user_id)
}

#[tokio::test]
async fn compute_asks_the_entitlement_for_the_compute_resource() {
    let seen = Arc::new(Mutex::new(vec![]));
    let (response, user_id) = post_embeddings(Decide {
        seen: seen.clone(),
        answer: || Err(ServerError::SubscriptionRequired),
    })
    .await;
    assert_eq!(response.status, 402);
    assert_eq!(response.code(), Some("subscription_required"));
    assert_eq!(*seen.lock().unwrap(), [(user_id, Resource::Compute)]);
}

#[tokio::test]
async fn an_entitled_user_computes() {
    let seen = Arc::new(Mutex::new(vec![]));
    let (response, _) = post_embeddings(Decide { seen: seen.clone(), answer: || Ok(()) }).await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

fn quota_exceeded() -> ServerError {
    ServerError::Extension {
        status: axum::http::StatusCode::TOO_MANY_REQUESTS,
        code: "quota_exceeded",
        message: "La quota del mese è finita.".to_owned(),
        retry_after_s: Some(3600),
        extra: Some(json!({"resource": "compute", "limit": 10, "used": 10, "renews_at": "2026-12-01T00:00:00.000Z"})),
    }
}

#[tokio::test]
async fn an_entitlement_error_with_a_status_reaches_the_client() {
    let (response, _) = post_embeddings(Decide { seen: Arc::default(), answer: || Err(quota_exceeded()) }).await;
    assert_eq!(response.status, 429, "{}", response.text);
    assert_eq!(response.code(), Some("quota_exceeded"));
    let error = &response.body["error"];
    assert_eq!(error["message"], "La quota del mese è finita.");
    assert_eq!(error["retry_after_s"], 3600);
    assert_eq!(error["resource"], "compute");
    assert_eq!(error["limit"], 10);
    assert_eq!(error["used"], 10);
    assert_eq!(error["renews_at"], "2026-12-01T00:00:00.000Z");
    assert_eq!(response.headers.get("retry-after").and_then(|v| v.to_str().ok()), Some("3600"));
}

#[tokio::test]
async fn a_plain_internal_error_from_the_entitlement_is_still_a_500() {
    let (response, _) = post_embeddings(Decide {
        seen: Arc::default(),
        answer: || Err(ServerError::Internal("dettaglio riservato".to_owned())),
    })
    .await;
    assert_eq!(response.status, 500);
    assert_eq!(response.code(), Some("internal"));
}

#[tokio::test]
async fn the_old_402_is_unchanged() {
    let (response, _) = post_embeddings(Decide { seen: Arc::default(), answer: || Err(ServerError::SubscriptionRequired) }).await;
    assert_eq!(response.status, 402);
    assert_eq!(response.code(), Some("subscription_required"));
    assert!(response.body["error"].get("resource").is_none());
}

#[tokio::test]
async fn transcription_asks_for_its_own_resource() {
    let seen = Arc::new(Mutex::new(vec![]));
    let ts = TestServer::new(test_config());
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let user_id = ts.accounts().user_id_by_username("mario").await.unwrap();
    let gate = DeviceGate::with_failures(Arc::clone(ts.accounts()), Arc::from(Vec::new()), 2, 120, DEVICE_AUTH_WINDOW)
        .with_entitlement(Some(Arc::new(Decide { seen: seen.clone(), answer: || Ok(()) })));
    let router = remote_router(ComputeState::new(Arc::new(Fake), ComputeAuth::Devices(Arc::new(gate)), "remote"));
    let key = bearer(&login.device_key);
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "input": ["a"], "dimensions": DIM})).unwrap();
    request_raw(&router, "POST", "/v1/embeddings", Some(body), loopback(), &[("authorization", &key)]).await;
    // The body is not a valid upload: the entitlement is asked first, whatever follows.
    request_raw(&router, "POST", "/v1/audio/transcriptions", Some(b"x".to_vec()), loopback(), &[("authorization", &key)]).await;
    assert_eq!(
        *seen.lock().unwrap(),
        [(user_id.clone(), Resource::Compute), (user_id, Resource::Transcription)]
    );
}
