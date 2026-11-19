//! Compute routes of `serve` (remote mode, wave 3 of the compute plan): behind the device keys of
//! the user database, `/health` anonymous and with a key, the administrator key only for the
//! models, at most N requests in flight per device key, per-user counters without content.
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use qamille_compute::speech::Segment;
use qamille_server::compute::speech::{SpeechBackend, TranscriptionInput};
use qamille_server::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus};
use support::{ADMIN_KEY, TestServer, bearer, keys, loopback, remote, request, request_raw, test_config, user_with_device};

#[derive(Default)]
struct Fake {
    calls: AtomicUsize,
    delay_ms: AtomicUsize,
    running: AtomicUsize,
    peak: AtomicUsize,
    installs: AtomicUsize,
}

fn unit() -> Vec<f32> {
    let mut v = vec![1.0f32; DIM];
    let norm = (DIM as f32).sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

impl ComputeBackend for Fake {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        Ok(vec![text.to_owned()])
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(self.delay_ms.load(Ordering::SeqCst) as u64));
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(inputs.iter().map(|_| unit()).collect())
    }
    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        let tokens = inputs.iter().map(|input| input.split_whitespace().count() as u64).sum();
        self.embed(inputs).map(|vectors| (vectors, tokens))
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Ok(unit())
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Ok(String::new())
    }
    fn models(&self) -> Vec<ModelStatus> {
        vec![ModelStatus {
            id: MODEL_ID.to_owned(),
            kind: "embedding".to_owned(),
            state: "ready".to_owned(),
            percent: Some(100),
            done_bytes: 10,
            total_bytes: 10,
            error: None,
            loaded: false,
            dimensions: DIM,
            quantization: "q4f16".to_owned(),
        }]
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        Ok(self.models().remove(0))
    }
}

struct Speech;

impl SpeechBackend for Speech {
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
    fn transcribe(&self, _: &TranscriptionInput, _: &AtomicBool) -> Result<Vec<Segment>, ComputeError> {
        Ok(vec![Segment { start_ms: 0, end_ms: 2000, text: "ciao".to_owned() }])
    }
}

fn server(fake: Arc<Fake>, max_in_flight: u32) -> TestServer {
    server_with(fake, max_in_flight, 120)
}

fn server_with(fake: Arc<Fake>, max_in_flight: u32, max_auth_failures: u32) -> TestServer {
    let mut config = test_config();
    config.compute_max_in_flight_per_user = max_in_flight;
    config.compute_max_auth_failures_per_origin = max_auth_failures;
    // The test origins are private addresses that may use plain text, as behind a Docker gateway.
    config.plaintext_from = vec![[10, 0, 0, 9].into(), [10, 0, 0, 10].into()];
    let TestServer { server, clock, dir, .. } = TestServer::new(config);
    let server = server.with_compute(fake, Arc::new(Speech), 2);
    TestServer { public: server.public_router(), admin: server.admin_router(), server, clock, dir }
}

fn embedding(text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"model": MODEL_ID, "input": [text], "dimensions": DIM})).unwrap()
}

#[tokio::test]
async fn compute_routes_take_a_device_key_and_health_says_more_with_one() {
    let ts = server(Arc::default(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let key = bearer(&login.device_key);

    let anonymous = request(&ts.public, "GET", "/health", None, remote([10, 0, 0, 9]), &[]).await;
    assert_eq!(anonymous.status, 200);
    assert_eq!(keys(&anonymous.body), ["status", "version"]);
    let known = request(&ts.public, "GET", "/health", None, remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
    assert_eq!(known.body["mode"], "remote");
    assert_eq!(known.body["models"][0]["id"], MODEL_ID);
    assert_eq!(known.body["embedding"]["model"], MODEL_ID);
    assert_eq!(known.body["embedding"]["dimensions"], DIM);
    assert_eq!(known.body["capabilities"].get("install_models"), None, "the key is gone with the public installation");
    assert_eq!(known.body["limits"]["max_in_flight_per_user"], 2);

    for headers in [vec![], vec![("authorization", "Bearer spk_dev_0000000000000000000000000000000000000000000000000000000000000000")]] {
        let refused = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 9]), &headers).await;
        assert_eq!(refused.status, 401);
        assert_eq!(refused.code(), Some("unauthorized"));
    }
    let ok = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("fattura di ottobre")), remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
    assert_eq!(ok.status, 200, "{}", ok.text);
    assert_eq!(ok.body["data"][0]["embedding"].as_array().unwrap().len(), DIM);
    assert_eq!(ok.body["usage"]["prompt_tokens"], 3);

    // The account routes are still there, next to the compute ones.
    let info = request(&ts.public, "GET", "/v1/info", None, remote([10, 0, 0, 9]), &[]).await;
    assert_eq!(info.status, 200);

    // A device revoked by the administrator gets 401 on the next request (EX-15).
    ts.accounts().revoke_device_admin(&login.user_id, &login.device_id).await.unwrap();
    let revoked = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
    assert_eq!(revoked.status, 401);
    assert_eq!(revoked.code(), Some("unauthorized"));
    let health = request(&ts.public, "GET", "/health", None, remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
    assert_eq!(keys(&health.body), ["status", "version"], "a revoked key sees only the anonymous health");
}

#[tokio::test]
async fn models_are_installed_only_on_the_administrative_listener() {
    let fake: Arc<Fake> = Arc::default();
    let ts = server(fake.clone(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    // On the public listener the administrator key is just a wrong key: the same 401 over
    // plain HTTP and over loopback, nothing tells a right guess from a wrong one.
    let wrong = request(&ts.public, "GET", "/v1/models", None, loopback(), &[("authorization", "Bearer 0000000000000000000000000000000000000000")]).await;
    for origin in [remote([10, 0, 0, 9]), loopback()] {
        let admin = request(&ts.public, "GET", "/v1/models", None, origin, &[("authorization", &bearer(ADMIN_KEY))]).await;
        assert_eq!((admin.status, admin.body.clone()), (wrong.status, wrong.body.clone()));
        assert_eq!(admin.status, 401);
        let health = request(&ts.public, "GET", "/health", None, origin, &[("authorization", &bearer(ADMIN_KEY))]).await;
        assert_eq!(keys(&health.body), ["status", "version"]);
    }
    let content = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(content.status, 401);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    // The administrative listener: its TLS rule, its limit per origin, then the key.
    let admin_path = format!("/admin/api/models/{MODEL_ID}/install");
    let plain = request(ts.admin(), "POST", &admin_path, None, remote([10, 0, 0, 9]), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(plain.code(), Some("tls_required"), "checked before the key");
    let wrong = request(ts.admin(), "POST", &admin_path, None, loopback(), &[("authorization", &bearer("x"))]).await;
    assert_eq!(wrong.status, 401);
    let installed = request(ts.admin(), "POST", &admin_path, None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(installed.status, 200, "{}", installed.text);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
    let models = request(ts.admin(), "GET", "/admin/api/models", None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(models.body["models"][0]["id"], MODEL_ID);
    // A device may read the state of a model.
    let state = request(&ts.public, "GET", &format!("/qamille/v1/models/{MODEL_ID}"), None, remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(state.status, 200);
    assert_eq!(state.body["state"], "ready");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_has_at_most_its_requests_in_flight_whatever_the_device() {
    let fake: Arc<Fake> = Arc::default();
    fake.delay_ms.store(400, Ordering::SeqCst);
    let ts = server(fake.clone(), 2);
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let laptop = ts.accounts().login(support::ip(loopback()), "mario", "una password lunga", "Portatile").await.unwrap();
    let anna = user_with_device(&ts, "anna", "un'altra password").await;
    // Three requests of the same user at once, from two devices: the third is refused at once
    // (A2: the devices of a user share the limit).
    let embed = |key: String| {
        let router = ts.public.clone();
        tokio::spawn(async move {
            request_raw(&router, "POST", "/v1/embeddings", Some(embedding("x")), remote([10, 0, 0, 9]), &[("authorization", &bearer(&key))]).await
        })
    };
    let first = embed(mario.device_key.clone());
    let second = embed(mario.device_key.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let third = embed(laptop.device_key.clone()).await.unwrap();
    assert_eq!(third.status, 429);
    assert_eq!(third.code(), Some("rate_limited"));
    assert_eq!(third.body["error"]["retry_after_s"], 1);
    // Another user is not held back by Mario's limit.
    let other = embed(anna.device_key.clone()).await.unwrap();
    assert_eq!(other.status, 200);
    assert_eq!(first.await.unwrap().status, 200);
    assert_eq!(second.await.unwrap().status, 200);
    let fourth = embed(mario.device_key.clone()).await.unwrap();
    assert_eq!(fourth.status, 200, "the slots are given back");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_user_counters_hold_numbers_only() {
    let ts = server(Arc::default(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let key = bearer(&login.device_key);
    for text in ["fattura di ottobre", "treno per Cefalù"] {
        let ok = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding(text)), remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
        assert_eq!(ok.status, 200);
    }
    // Two seconds of 16 kHz mono 16-bit audio.
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36u32 + 64_000).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&32_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&64_000u32.to_le_bytes());
    wav.extend(std::iter::repeat_n(0u8, 64_000));
    let boundary = "qamille-test";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-base\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(&wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let mut spoken = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("authorization", &key)
        .header("content-type", &content_type)
        .body(axum::body::Body::from(body))
        .unwrap();
    spoken.extensions_mut().insert(axum::extract::ConnectInfo(remote([10, 0, 0, 9])));
    let spoken = tower::ServiceExt::oneshot(ts.public.clone(), spoken).await.unwrap();
    assert_eq!(spoken.status(), 200);
    // The counters are written outside the request.
    let mut usage = ts.accounts().usage(&login.user_id).await.unwrap();
    for _ in 0..50 {
        if usage.requests >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        usage = ts.accounts().usage(&login.user_id).await.unwrap();
    }
    assert_eq!(usage.requests, 3);
    assert_eq!(usage.embedded_tokens, 3 + 3);
    assert!((usage.audio_seconds - 2.0).abs() < 0.01, "{usage:?}");
    let users = ts.accounts().list_users().await.unwrap();
    let mario = users.iter().find(|u| u.username == "mario").unwrap();
    assert_eq!(mario.usage.requests, 3);
    // Only numbers: the table has no column for a text.
    let columns: Vec<String> = {
        let conn = rusqlite::Connection::open(ts.dir.path().join("server.db")).unwrap();
        let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('compute_usage')").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(
        columns,
        ["user_id", "requests", "audio_seconds", "embedded_tokens", "updated_at", "llm_prompt_tokens", "llm_completion_tokens"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_revoked_while_it_waits_in_the_queue_is_dropped() {
    let fake: Arc<Fake> = Arc::default();
    fake.delay_ms.store(500, Ordering::SeqCst);
    let ts = server(fake.clone(), 2);
    let anna = user_with_device(&ts, "anna", "un'altra password").await;
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let embed = |key: String| {
        let router = ts.public.clone();
        tokio::spawn(async move {
            request_raw(&router, "POST", "/v1/embeddings", Some(embedding("x")), remote([10, 0, 0, 9]), &[("authorization", &bearer(&key))]).await
        })
    };
    // Anna holds the model; Mario's request waits for it; the administrator revokes Mario.
    let running = embed(anna.device_key.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued = embed(mario.device_key.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    ts.accounts().revoke_device_admin(&mario.user_id, &mario.device_id).await.unwrap();
    assert_eq!(running.await.unwrap().status, 200);
    let dropped = queued.await.unwrap();
    assert_eq!(dropped.status, 401);
    assert_eq!(dropped.code(), Some("unauthorized"));
    assert_eq!(fake.peak.load(Ordering::SeqCst), 1);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1, "the revoked request never reached the model");
}

#[tokio::test]
async fn an_input_longer_than_the_model_takes_is_refused_before_the_model() {
    let fake: Arc<Fake> = Arc::default();
    let ts = server(fake.clone(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let huge = "parola ".repeat(qamille_server::compute::MAX_INPUT_BYTES / 7 + 10);
    let refused = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding(&huge)), remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(refused.status, 413);
    assert_eq!(refused.code(), Some("payload_too_large"));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    // A passage of the app (256 tokens, about 1 KiB) passes.
    let passage = "title: Fattura | text: ".to_owned() + &"la fattura di ottobre è pagata ".repeat(40);
    let ok = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding(&passage)), remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(ok.status, 200);
}

#[tokio::test]
async fn wrong_keys_are_limited_per_origin() {
    let ts = server_with(Arc::default(), 2, 3);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let wrong = "Bearer spk_dev_0000000000000000000000000000000000000000000000000000000000000000";
    for _ in 0..3 {
        let refused = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 9]), &[("authorization", wrong)]).await;
        assert_eq!(refused.status, 401);
    }
    // Over the limit: refused before any query, even with a right key, from that origin only.
    let limited = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(limited.status, 429);
    assert_eq!(limited.code(), Some("rate_limited"));
    let health = request(&ts.public, "GET", "/health", None, remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(keys(&health.body), ["status", "version"]);
    let elsewhere = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 10]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(elsewhere.status, 200);
}

#[tokio::test]
async fn the_public_install_routes_are_gone() {
    let fake: Arc<Fake> = Arc::default();
    let ts = server(fake.clone(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    for prefix in ["/qamille", "/qemille", "/sparklone"] {
        let path = format!("{prefix}/v1/models/{MODEL_ID}/install");
        for headers in [
            vec![],
            vec![("authorization", bearer(&login.device_key))],
            vec![("authorization", bearer(ADMIN_KEY))],
        ] {
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let r = request(&ts.public, "POST", &path, None, loopback(), &headers).await;
            assert_eq!((r.status.as_u16(), r.code()), (404, Some("not_found")), "{path}");
        }
    }
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    let known = request(&ts.public, "GET", "/health", None, remote([10, 0, 0, 9]), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(known.body["capabilities"].get("install_models"), None);
}

#[tokio::test]
async fn a_device_key_over_a_connection_that_is_not_secure_is_refused() {
    let ts = server(Arc::default(), 2);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let key = bearer(&login.device_key);
    // A public address that is not in `plaintext_from`, without a trusted proxy: plain text.
    let plain = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([203, 0, 113, 5]), &[("authorization", &key)]).await;
    assert_eq!(plain.status, 400);
    assert_eq!(plain.code(), Some("tls_required"));
    // The key is not even looked at: a wrong one gets the same answer and does not count as a failure.
    let wrong = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([203, 0, 113, 5]), &[("authorization", "Bearer spk_dev_0000000000000000000000000000000000000000000000000000000000000000")]).await;
    assert_eq!(wrong.code(), Some("tls_required"));
    // `/health` keeps answering anonymously.
    let health = request(&ts.public, "GET", "/health", None, remote([203, 0, 113, 5]), &[("authorization", &key)]).await;
    assert_eq!(health.status, 200);
    assert_eq!(keys(&health.body), ["status", "version"]);
    // Loopback and the addresses of `plaintext_from` are fine.
    let loop_ok = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), loopback(), &[("authorization", &key)]).await;
    assert_eq!(loop_ok.status, 200);
    let listed = request_raw(&ts.public, "POST", "/v1/embeddings", Some(embedding("a")), remote([10, 0, 0, 9]), &[("authorization", &key)]).await;
    assert_eq!(listed.status, 200);
}
