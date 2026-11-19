//! Compute routes of D2 with a fake backend (no model): shapes, closed error codes, the
//! per-launch token, the queue and the body limits.
mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use serde_json::{Value, json};
use qamille_server::compute::{
    ComputeAuth, ComputeBackend, ComputeError, ComputeState, DIM, MODEL_ID, ModelStatus, fixed_router,
};
use axum::body::Body;
use axum::http::Request;
use support::{TestResponse, bearer};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// A backend that answers with deterministic vectors, or with the error it is given.
#[derive(Default)]
struct Fake {
    fail: Mutex<Option<ComputeError>>,
    delay: Mutex<Duration>,
    calls: AtomicUsize,
    installs: AtomicUsize,
}

fn vector(seed: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|i| ((i + seed) % 7) as f32 + 1.0).collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

impl Fake {
    fn step(&self) -> Result<(), ComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        match self.fail.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl ComputeBackend for Fake {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        self.step()?;
        Ok(text.split(". ").map(str::to_owned).collect())
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        self.step()?;
        Ok(inputs.iter().map(|input| vector(input.len())).collect())
    }
    fn embed_image(&self, bytes: &[u8]) -> Result<Vec<f32>, ComputeError> {
        self.step()?;
        Ok(vector(bytes.len()))
    }
    fn extract(&self, name: &str, bytes: &[u8]) -> Result<String, ComputeError> {
        self.step()?;
        Ok(format!("{name}:{}", String::from_utf8_lossy(bytes)))
    }
    fn models(&self) -> Vec<ModelStatus> {
        let ready = self.installs.load(Ordering::SeqCst) > 0;
        vec![ModelStatus {
            id: MODEL_ID.to_owned(),
            kind: "embedding".to_owned(),
            state: if ready { "ready" } else { "missing" }.to_owned(),
            percent: ready.then_some(100),
            done_bytes: 0,
            total_bytes: 10,
            error: None,
            loaded: false,
            dimensions: DIM,
            quantization: "q4f16".to_owned(),
        }]
    }
    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        if id != MODEL_ID {
            return Err(ComputeError::ModelMissing("unknown".to_owned()));
        }
        let mut status = self.models().remove(0);
        if self.installs.fetch_add(1, Ordering::SeqCst) == 0 {
            status.state = "downloading".to_owned();
        }
        Ok(status)
    }
}

fn router(fake: Arc<Fake>) -> Router {
    fixed_router(ComputeState::new(fake, ComputeAuth::fixed(TOKEN), "fixed"))
}

async fn call(router: &Router, method: &str, path: &str, body: Option<Vec<u8>>, headers: &[(&str, &str)]) -> TestResponse {
    let mut builder = Request::builder().method(method).uri(path);
    if body.is_some() && !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type")) {
        builder = builder.header("content-type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(body.map(Body::from).unwrap_or_else(Body::empty)).unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    TestResponse { status, headers, body, text }
}

async fn post_json(router: &Router, path: &str, body: Value) -> TestResponse {
    let auth = bearer(TOKEN);
    call(router, "POST", path, Some(serde_json::to_vec(&body).unwrap()), &[("authorization", &auth)]).await
}

fn multipart(name: &str, file: &[u8]) -> (String, Vec<u8>) {
    let boundary = "qamille-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"upload\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn assert_error_shape(response: &TestResponse, code: &str) {
    assert_eq!(response.code(), Some(code), "{}", response.text);
    let error = &response.body["error"];
    assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(error.get("retry_after_s").is_some(), "retry_after_s is always present");
}

#[tokio::test]
async fn health_shows_models_only_with_the_token() {
    let router = router(Arc::new(Fake::default()));
    let anonymous = call(&router, "GET", "/health", None, &[]).await;
    assert_eq!(anonymous.status, 200);
    assert_eq!(support::keys(&anonymous.body), vec!["status", "version"]);
    let wrong = call(&router, "GET", "/health", None, &[("authorization", "Bearer not-the-token")]).await;
    assert_eq!(support::keys(&wrong.body), vec!["status", "version"]);
    let auth = bearer(TOKEN);
    let full = call(&router, "GET", "/health", None, &[("authorization", &auth)]).await;
    assert_eq!(full.body["mode"], "fixed");
    assert_eq!(full.body["models"][0]["id"], MODEL_ID);
    assert_eq!(full.body["models"][0]["state"], "missing");
    assert_eq!(full.body["embedding"], json!({"model": MODEL_ID, "dimensions": 256, "quantization": "q4f16"}));
    assert_eq!(full.headers["cache-control"], "no-store");
}

#[tokio::test]
async fn every_compute_route_wants_the_fixed_token() {
    let fake = Arc::new(Fake::default());
    let router = router(fake.clone());
    let routes: [(&str, &str); 14] = [
        ("GET", "/v1/models"),
        ("POST", "/v1/embeddings"),
        ("POST", "/qamille/v1/chunks"),
        ("POST", "/qamille/v1/embeddings/image"),
        ("POST", "/qamille/v1/extract"),
        ("GET", "/qamille/v1/models/embeddinggemma-2"),
        // The names from before the rebrandings, kept for apps not yet updated.
        ("POST", "/qemille/v1/chunks"),
        ("POST", "/qemille/v1/embeddings/image"),
        ("POST", "/qemille/v1/extract"),
        ("GET", "/qemille/v1/models/embeddinggemma-2"),
        ("POST", "/sparklone/v1/chunks"),
        ("POST", "/sparklone/v1/embeddings/image"),
        ("POST", "/sparklone/v1/extract"),
        ("GET", "/sparklone/v1/models/embeddinggemma-2"),
    ];
    for (method, path) in routes {
        for headers in [vec![], vec![("authorization", "Bearer 0123456789abcdef0123456789abcdef0123456789abcdeX")], vec![("authorization", "Basic abc")]] {
            let response = call(&router, method, path, Some(b"{}".to_vec()), &headers).await;
            assert_eq!(response.status, 401, "{method} {path}");
            assert_error_shape(&response, "unauthorized");
            assert_eq!(response.headers["www-authenticate"], "Bearer");
        }
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0, "nothing reached the backend");
}

#[tokio::test]
async fn embeddings_follow_the_openai_shape_in_batches() {
    let router = router(Arc::new(Fake::default()));
    let one = post_json(&router, "/v1/embeddings", json!({"model": MODEL_ID, "input": "abc", "dimensions": 256})).await;
    assert_eq!(one.status, 200, "{}", one.text);
    assert_eq!(one.body["object"], "list");
    assert_eq!(one.body["model"], MODEL_ID);
    assert_eq!(one.body["data"][0]["object"], "embedding");
    assert_eq!(one.body["data"][0]["embedding"].as_array().unwrap().len(), DIM);
    let batch = post_json(&router, "/v1/embeddings", json!({"input": ["a", "bb", "ccc"]})).await;
    let data = batch.body["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);
    assert_eq!(data.iter().map(|d| d["index"].as_u64().unwrap()).collect::<Vec<_>>(), vec![0, 1, 2]);
    let other = post_json(&router, "/v1/embeddings", json!({"model": "text-embedding-3-small", "input": "a"})).await;
    assert_eq!(other.status, 503);
    assert_error_shape(&other, "model_missing");
    let dims = post_json(&router, "/v1/embeddings", json!({"input": "a", "dimensions": 768})).await;
    assert_error_shape(&dims, "invalid_request");
    let empty = post_json(&router, "/v1/embeddings", json!({"input": []})).await;
    assert_error_shape(&empty, "invalid_request");
    let too_many = post_json(&router, "/v1/embeddings", json!({"input": vec!["x"; 65]})).await;
    assert_error_shape(&too_many, "invalid_request");
    let auth = bearer(TOKEN);
    let broken = call(&router, "POST", "/v1/embeddings", Some(b"{\"input\": secret text".to_vec()), &[("authorization", &auth)]).await;
    assert_error_shape(&broken, "invalid_request");
    assert!(!broken.text.contains("secret"), "the body is never echoed");
}

#[tokio::test]
async fn chunks_and_models_routes() {
    let fake = Arc::new(Fake::default());
    let router = router(fake.clone());
    let chunks = post_json(&router, "/qamille/v1/chunks", json!({"model": MODEL_ID, "title": "t", "text": "uno. due. tre"})).await;
    assert_eq!(chunks.body, json!({"chunks": ["uno", "due", "tre"]}));
    let legacy = post_json(&router, "/sparklone/v1/chunks", json!({"model": MODEL_ID, "title": "t", "text": "uno. due. tre"})).await;
    assert_eq!(legacy.body, chunks.body, "an app from before the rebranding gets the same answer");
    let auth = bearer(TOKEN);
    let models = call(&router, "GET", "/v1/models", None, &[("authorization", &auth)]).await;
    assert_eq!(models.body["object"], "list");
    assert_eq!(models.body["data"][0]["id"], MODEL_ID);
    let status = call(&router, "GET", "/qamille/v1/models/embeddinggemma-2", None, &[("authorization", &auth)]).await;
    assert_eq!(status.body["state"], "missing");
    let unknown = call(&router, "GET", "/qamille/v1/models/whisper-small", None, &[("authorization", &auth)]).await;
    assert_error_shape(&unknown, "model_missing");
}

#[tokio::test]
async fn extraction_and_images_take_files_up_to_25_mib() {
    let router = router(Arc::new(Fake::default()));
    let auth = bearer(TOKEN);
    let (content_type, body) = multipart("fattura.pdf", b"testo");
    let extracted = call(&router, "POST", "/qamille/v1/extract", Some(body), &[("authorization", &auth), ("content-type", &content_type)]).await;
    assert_eq!(extracted.status, 200, "{}", extracted.text);
    assert_eq!(extracted.body, json!({"text": "fattura.pdf:testo"}));
    let (content_type, body) = multipart("big.pdf", &vec![b'x'; 25 * 1024 * 1024 + 1]);
    let big = call(&router, "POST", "/qamille/v1/extract", Some(body), &[("authorization", &auth), ("content-type", &content_type)]).await;
    assert_eq!(big.status, 413);
    assert_error_shape(&big, "payload_too_large");
    let missing = call(&router, "POST", "/qamille/v1/extract", Some(b"no multipart".to_vec()), &[("authorization", &auth), ("content-type", "text/plain")]).await;
    assert_error_shape(&missing, "invalid_request");
    let image = call(&router, "POST", "/qamille/v1/embeddings/image", Some(vec![1, 2, 3]), &[("authorization", &auth), ("content-type", "application/octet-stream")]).await;
    assert_eq!(image.status, 200);
    assert_eq!(image.body["embedding"].as_array().unwrap().len(), DIM);
    let huge = call(&router, "POST", "/qamille/v1/embeddings/image", Some(vec![0; 26 * 1024 * 1024]), &[("authorization", &auth), ("content-type", "application/octet-stream")]).await;
    assert_eq!(huge.status, 413);
    assert_error_shape(&huge, "payload_too_large");
}

#[tokio::test]
async fn backend_errors_keep_their_closed_codes_statuses_and_pauses() {
    let cases = [
        (ComputeError::ModelMissing("missing".to_owned()), 503, "model_missing", None),
        (ComputeError::ModelDownloading { percent: Some(40) }, 503, "model_downloading", Some(10)),
        (ComputeError::PausedForMemory { retry_after_s: 60 }, 503, "paused_for_memory", Some(60)),
        (ComputeError::PausedAfterFailure { retry_after_s: 42 }, 503, "paused_after_failure", Some(42)),
        (ComputeError::DocumentUnreadable("bad".to_owned()), 422, "document_unreadable", None),
        (ComputeError::PayloadTooLarge, 413, "payload_too_large", None),
        (ComputeError::RateLimited { retry_after_s: 3 }, 429, "rate_limited", Some(3)),
        (ComputeError::Internal("Embedding failed: tensor".to_owned()), 500, "internal", None),
    ];
    for (error, status, code, retry) in cases {
        let fake = Arc::new(Fake::default());
        *fake.fail.lock().unwrap() = Some(error);
        let router = router(fake);
        let response = post_json(&router, "/v1/embeddings", json!({"input": "x"})).await;
        assert_eq!(response.status, status, "{code}");
        assert_error_shape(&response, code);
        assert_eq!(response.body["error"]["retry_after_s"].as_u64(), retry, "{code}");
        if let Some(seconds) = retry {
            assert_eq!(response.headers["retry-after"], seconds.to_string().as_str());
        }
    }
}

#[tokio::test]
async fn one_inference_at_a_time_and_a_bounded_queue() {
    let fake = Arc::new(Fake::default());
    *fake.delay.lock().unwrap() = Duration::from_millis(400);
    let router = fixed_router(ComputeState::with_queue(fake.clone(), ComputeAuth::fixed(TOKEN), "fixed", 1));
    let send = |router: Router| async move { post_json(&router, "/v1/embeddings", json!({"input": "x"})).await };
    let (a, b, c) = tokio::join!(send(router.clone()), send(router.clone()), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        send(router.clone()).await
    });
    let mut statuses = vec![a.status.as_u16(), b.status.as_u16(), c.status.as_u16()];
    statuses.sort();
    assert_eq!(statuses, vec![200, 200, 429], "one runs, one waits, the third is refused");
    let refused = [a, b, c].into_iter().find(|r| r.status == 429).unwrap();
    assert_error_shape(&refused, "rate_limited");
    assert!(refused.body["error"]["retry_after_s"].as_u64().is_some_and(|s| s > 0));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unknown_routes_answer_in_the_same_error_form() {
    let router = router(Arc::new(Fake::default()));
    let missing = call(&router, "GET", "/v1/login", None, &[]).await;
    assert_eq!(missing.status, 404);
    assert_error_shape(&missing, "not_found");
    let auth = bearer(TOKEN);
    let method = call(&router, "GET", "/v1/embeddings", None, &[("authorization", &auth)]).await;
    assert_eq!(method.status, 405);
    assert_error_shape(&method, "invalid_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_upload_holds_no_model_permit_and_ends_at_its_deadline() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fake = Arc::new(Fake::default());
    let state = ComputeState::new(fake.clone(), ComputeAuth::fixed(TOKEN), "fixed").with_upload_deadline(Duration::from_millis(600));
    let router = fixed_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = router.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    // An image whose body trickles: 3 bytes of 1000, then nothing.
    let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
    let head = format!("POST /qamille/v1/embeddings/image HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 1000\r\n\r\nabc");
    slow.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Meanwhile the model is free: an embedding goes through at once.
    let started = std::time::Instant::now();
    let other = post_json(&router, "/v1/embeddings", json!({"input": "x"})).await;
    assert_eq!(other.status, 200);
    assert!(started.elapsed() < Duration::from_millis(400), "{:?}", started.elapsed());
    // The slow upload is refused once its deadline passes; nothing reached the model for it.
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), slow.read_to_end(&mut answer)).await;
    let answer = String::from_utf8_lossy(&answer);
    assert!(answer.starts_with("HTTP/1.1 400"), "{answer}");
    assert!(answer.contains("The upload did not arrive in time."), "{answer}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
}
