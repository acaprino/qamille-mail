//! `POST /v1/audio/transcriptions` with a fake `whisper-cli` (a shell script written here) and a
//! synthetic model file: OpenAI-compatible answers, the closed error codes of D2, the end of the
//! recognizer when the client goes away, the queue, and the model state in `/health`.
#![cfg(unix)]
mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use qamille_compute::speech::SpeechModelSpec;
use qamille_server::compute::speech::{SpeechBackend, WhisperBackend};
use qamille_server::compute::{ComputeAuth, ComputeBackend, ComputeError, ComputeState, Gate, ModelStatus, fixed_router};
use support::{TempDir, TestResponse, bearer};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
const MODEL: &[u8] = b"fake ggml model for the transcription route tests\n";
static CATALOG: [SpeechModelSpec; 1] = [SpeechModelSpec {
    size: "base",
    bytes: 50,
    sha256: "033746ff9f4f98d5764227a5c85535f7290d9746f94f026661ec2a76eb7f3235",
}];

/// Reads the arguments like whisper-cli, records them, then behaves as the marker files say.
const SCRIPT: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
out=""; lang=""; threads=""; wave=""
while [ $# -gt 0 ]; do
  case "$1" in
    -of) out="$2"; shift 2;;
    -l) lang="$2"; shift 2;;
    -t) threads="$2"; shift 2;;
    -f) wave="$2"; shift 2;;
    -m) shift 2;;
    *) shift;;
  esac
done
echo "$$" > "$dir/pid"
printf 'lang=%s threads=%s wave=%s\n' "$lang" "$threads" "$wave" > "$dir/args"
if [ -f "$dir/slow" ]; then sleep 30; echo done > "$dir/finished"; fi
if [ -f "$dir/fail" ]; then exit 3; fi
cat > "$out.json" <<EOF
{"transcription":[{"text":" Buongiorno a tutti","offsets":{"from":120,"to":2400}},{"text":"[BLANK_AUDIO]","offsets":{"from":2400,"to":5000}},{"text":" iniziamo.","offsets":{"from":5000,"to":7300}}]}
EOF
"#;

/// No embedding model: only the speech side matters here.
struct NoEmbedding;
impl ComputeBackend for NoEmbedding {
    fn chunk(&self, _: &str) -> Result<Vec<String>, ComputeError> {
        Err(ComputeError::ModelMissing("none".into()))
    }
    fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        Err(ComputeError::ModelMissing("none".into()))
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Err(ComputeError::ModelMissing("none".into()))
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Err(ComputeError::ModelMissing("none".into()))
    }
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".into()))
    }
}

struct Setup {
    dir: TempDir,
    router: Router,
    speech: Arc<WhisperBackend>,
}

impl Setup {
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn mark(&self, name: &str) {
        std::fs::write(self.path().join(name), b"").unwrap();
    }
    fn unmark(&self, name: &str) {
        let _ = std::fs::remove_file(self.path().join(name));
    }
}

fn setup_with(runtime: bool, model: Option<&[u8]>, parallelism: usize, queue: usize) -> Setup {
    let dir = TempDir::new();
    if runtime {
        let program = dir.path().join("whisper-cli");
        std::fs::write(&program, SCRIPT).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    if let Some(bytes) = model {
        std::fs::write(dir.path().join("ggml-base.bin"), bytes).unwrap();
    }
    let speech = Arc::new(WhisperBackend::with_catalog(dir.path(), &CATALOG));
    let state = ComputeState::new(Arc::new(NoEmbedding), ComputeAuth::fixed(TOKEN), "fixed").with_speech_queue(speech.clone(), parallelism, queue);
    Setup { dir, router: fixed_router(state), speech }
}

fn setup() -> Setup {
    setup_with(true, Some(MODEL), 2, 4)
}

/// `seconds` of 16 kHz mono 16-bit silence-free samples.
fn wav(seconds: u32) -> Vec<u8> {
    let data = 16_000 * 2 * seconds;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.extend((0..data / 2).flat_map(|i| (((i % 64) as i16 - 32) * 100).to_le_bytes()));
    out
}

fn multipart(fields: &[(&str, &str)], file: Option<&[u8]>) -> (String, Vec<u8>) {
    let boundary = "qamille-speech-boundary";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    if let Some(file) = file {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(file);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn send(router: &Router, request: Request<Body>) -> TestResponse {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    TestResponse { status, headers, body, text }
}

fn transcription_request(fields: &[(&str, &str)], file: Option<&[u8]>, token: Option<&str>) -> Request<Body> {
    let (content_type, body) = multipart(fields, file);
    let mut builder = Request::builder().method("POST").uri("/v1/audio/transcriptions").header("content-type", content_type);
    if let Some(token) = token {
        builder = builder.header("authorization", bearer(token));
    }
    builder.body(Body::from(body)).unwrap()
}

async fn transcribe(router: &Router, fields: &[(&str, &str)], file: Option<&[u8]>) -> TestResponse {
    send(router, transcription_request(fields, file, Some(TOKEN))).await
}

const VERBOSE: &[(&str, &str)] = &[("model", "whisper-base"), ("language", "it"), ("response_format", "verbose_json")];

fn assert_error(response: &TestResponse, status: u16, code: &str) {
    assert_eq!(response.status.as_u16(), status, "{}", response.text);
    assert_eq!(response.code(), Some(code), "{}", response.text);
    assert!(response.body["error"]["message"].as_str().is_some_and(|m| !m.is_empty()));
}

fn args(setup: &Setup) -> String {
    std::fs::read_to_string(setup.path().join("args")).unwrap_or_default()
}

#[tokio::test]
async fn verbose_json_has_openai_segments_in_seconds() {
    let setup = setup();
    let response = transcribe(&setup.router, VERBOSE, Some(&wav(15))).await;
    assert_eq!(response.status.as_u16(), 200, "{}", response.text);
    let body = &response.body;
    assert_eq!(body["task"], "transcribe");
    assert_eq!(body["language"], "it");
    assert_eq!(body["duration"].as_f64(), Some(15.0));
    assert_eq!(body["text"], "Buongiorno a tutti iniziamo.");
    let segments = body["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2, "blank markers are left out");
    assert_eq!((segments[0]["id"].as_u64(), segments[0]["start"].as_f64(), segments[0]["end"].as_f64()), (Some(0), Some(0.12), Some(2.4)));
    assert_eq!(segments[0]["text"], "Buongiorno a tutti");
    assert_eq!((segments[1]["start"].as_f64(), segments[1]["end"].as_f64()), (Some(5.0), Some(7.3)));
    let recorded = args(&setup);
    assert!(recorded.contains("lang=it "), "{recorded}");
    // The audio lived only for the time of the request.
    let wave = recorded.split("wave=").nth(1).unwrap().trim();
    assert!(wave.ends_with("input.wav") && !Path::new(wave).exists(), "{wave}");
    assert!(!Path::new(wave).parent().unwrap().exists());
}

#[tokio::test]
async fn the_duration_comes_from_the_samples_not_from_the_byte_rate() {
    let setup = setup();
    let mut wave = wav(15);
    wave[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    let response = transcribe(&setup.router, VERBOSE, Some(&wave)).await;
    assert_eq!(response.status.as_u16(), 200, "{}", response.text);
    assert_eq!(response.body["duration"].as_f64(), Some(15.0), "{}", response.text);
}

#[tokio::test]
async fn json_text_and_auto_language() {
    let setup = setup();
    let json = transcribe(&setup.router, &[("model", "whisper-base")], Some(&wav(2))).await;
    assert_eq!(json.status.as_u16(), 200, "{}", json.text);
    assert_eq!(json.body, serde_json::json!({"text": "Buongiorno a tutti iniziamo."}), "json is the OpenAI default");
    assert!(args(&setup).contains("lang=auto "), "no language: whisper detects it");
    let text = transcribe(&setup.router, &[("model", "base"), ("language", "auto"), ("response_format", "text")], Some(&wav(2))).await;
    assert_eq!(text.status.as_u16(), 200);
    assert!(text.headers["content-type"].to_str().unwrap().starts_with("text/plain"));
    assert_eq!(text.text, "Buongiorno a tutti iniziamo.");
}

#[tokio::test]
async fn errors_use_the_closed_codes() {
    let setup = setup();
    let refused = send(&setup.router, transcription_request(VERBOSE, Some(&wav(1)), Some("wrong-token"))).await;
    assert_error(&refused, 401, "unauthorized");
    let anonymous = send(&setup.router, transcription_request(VERBOSE, Some(&wav(1)), None)).await;
    assert_error(&anonymous, 401, "unauthorized");
    let unknown = transcribe(&setup.router, &[("model", "whisper-huge")], Some(&wav(1))).await;
    assert_error(&unknown, 503, "model_missing");
    let not_wav = transcribe(&setup.router, VERBOSE, Some(b"ID3 an mp3 file is not read by this server, only WAV files")).await;
    assert_error(&not_wav, 422, "document_unreadable");
    // A WAV without `fmt ` before `data` gives no duration to charge: refused, whisper never runs.
    let mut no_fmt = wav(1);
    no_fmt[12..16].copy_from_slice(b"junk");
    assert_error(&transcribe(&setup.router, VERBOSE, Some(&no_fmt)).await, 400, "invalid_request");
    let no_file = transcribe(&setup.router, VERBOSE, None).await;
    assert_error(&no_file, 400, "invalid_request");
    let no_model = transcribe(&setup.router, &[("language", "it")], Some(&wav(1))).await;
    assert_error(&no_model, 400, "invalid_request");
    let language = transcribe(&setup.router, &[("model", "whisper-base"), ("language", "it;rm -rf")], Some(&wav(1))).await;
    assert_error(&language, 400, "invalid_request");
    let format = transcribe(&setup.router, &[("model", "whisper-base"), ("response_format", "srt")], Some(&wav(1))).await;
    assert_error(&format, 400, "invalid_request");
    let mut large = transcription_request(VERBOSE, Some(&wav(1)), Some(TOKEN));
    large.headers_mut().insert("content-length", (40u64 * 1024 * 1024).into());
    assert_error(&send(&setup.router, large).await, 413, "payload_too_large");
    setup.mark("fail");
    let failed = transcribe(&setup.router, VERBOSE, Some(&wav(1))).await;
    assert_error(&failed, 500, "internal");
    assert_eq!(failed.body["error"]["message"], "The speech recognizer did not complete this audio.");
    // No audio or text in the answers of errors.
    assert!(!failed.text.contains("input.wav"));
}

#[tokio::test]
async fn a_missing_or_altered_model_or_runtime_is_model_missing() {
    let without_model = setup_with(true, None, 1, 1);
    assert_error(&transcribe(&without_model.router, VERBOSE, Some(&wav(1))).await, 503, "model_missing");
    let altered = setup_with(true, Some(b"fake ggml model for the transcription route TESTS\n"), 1, 1);
    let response = transcribe(&altered.router, VERBOSE, Some(&wav(1))).await;
    assert_error(&response, 503, "model_missing");
    assert!(response.body["error"]["message"].as_str().unwrap().contains("Reinstall"), "{}", response.text);
    assert!(!altered.path().join("pid").exists(), "the recognizer never ran on an unchecked model");
    let without_runtime = setup_with(false, Some(MODEL), 1, 1);
    let response = transcribe(&without_runtime.router, VERBOSE, Some(&wav(1))).await;
    // On this machine `whisper-cli` may be on PATH; only without one is the code checked.
    if std::env::var_os("PATH").is_none_or(|path| !std::env::split_paths(&path).any(|dir| dir.join("whisper-cli").is_file())) {
        assert_error(&response, 503, "model_missing");
        assert_eq!(response.body["error"]["message"], qamille_compute::speech::RUNTIME_MISSING_MESSAGE);
    }
}

#[test]
fn install_without_the_runtime_is_model_missing() {
    let without_runtime = setup_with(false, Some(MODEL), 1, 1);
    // On this machine `whisper-cli` may be on PATH; only without one is the code checked.
    if std::env::var_os("PATH").is_none_or(|path| !std::env::split_paths(&path).any(|dir| dir.join("whisper-cli").is_file())) {
        assert!(matches!(without_runtime.speech.install("whisper-base"), Err(ComputeError::ModelMissing(_))));
    }
}

#[test]
fn an_installed_speech_model_is_not_downloaded_again() {
    let ready = setup();
    let status = ready.speech.install("whisper-base").expect("an installed model answers at once");
    assert_eq!(status.state, "ready", "an installed model is not downloaded again");
}

#[test]
fn an_unknown_speech_model_is_refused_by_the_backend() {
    let ready = setup();
    assert!(matches!(ready.speech.install("whisper-huge"), Err(ComputeError::ModelMissing(_))));
}

async fn health(router: &Router) -> Value {
    let request = Request::builder().uri("/health").header("authorization", bearer(TOKEN)).body(Body::empty()).unwrap();
    send(router, request).await.body
}

#[tokio::test]
async fn health_lists_the_speech_models() {
    let ready = setup();
    let body = health(&ready.router).await;
    let model = body["models"].as_array().unwrap().iter().find(|m| m["id"] == "whisper-base").unwrap().clone();
    assert_eq!((model["kind"].as_str(), model["state"].as_str(), model["percent"].as_u64()), (Some("speech"), Some("ready"), Some(100)));
    assert_eq!(body["limits"]["transcription_parallelism"], 2);
    let status = send(&ready.router, Request::builder().uri("/qamille/v1/models/whisper-base").header("authorization", bearer(TOKEN)).body(Body::empty()).unwrap()).await;
    assert_eq!(status.body["state"], "ready");
    let missing = setup_with(true, None, 1, 1);
    let body = health(&missing.router).await;
    let model = body["models"].as_array().unwrap().iter().find(|m| m["id"] == "whisper-base").unwrap().clone();
    assert_eq!(model["state"], "missing");
}

fn alive(pid: &str) -> bool {
    Path::new("/proc").join(pid.trim()).exists() && !std::fs::read_to_string(Path::new("/proc").join(pid.trim()).join("stat")).unwrap_or_default().contains(") Z ")
}

async fn wait_for(path: PathBuf) -> String {
    let began = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(&path)
            && !text.trim().is_empty()
        {
            return text;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{} never appeared", path.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_goes_away_stops_the_recognizer_and_frees_its_slot() {
    let setup = setup_with(true, Some(MODEL), 1, 1);
    setup.mark("slow");
    let router = setup.router.clone();
    let request = tokio::spawn(async move { transcribe(&router, VERBOSE, Some(&wav(1))).await });
    let pid = wait_for(setup.path().join("pid")).await;
    assert!(alive(&pid));
    request.abort();
    let began = Instant::now();
    while alive(&pid) {
        assert!(began.elapsed() < Duration::from_secs(5), "whisper-cli outlived its request");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!setup.path().join("finished").exists());
    // The only slot is free again: the next request runs at once.
    setup.unmark("slow");
    let _ = std::fs::remove_file(setup.path().join("pid"));
    let next = tokio::time::timeout(Duration::from_secs(10), transcribe(&setup.router, VERBOSE, Some(&wav(1)))).await.expect("the slot was released");
    assert_eq!(next.status.as_u16(), 200, "{}", next.text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn beyond_the_queue_the_answer_is_rate_limited_and_a_dropped_waiter_leaves_it() {
    let setup = setup_with(true, Some(MODEL), 1, 1);
    setup.mark("slow");
    let router = setup.router.clone();
    let running = tokio::spawn(async move { transcribe(&router, VERBOSE, Some(&wav(1))).await });
    wait_for(setup.path().join("pid")).await;
    // One waits (the queue holds one), the next is refused.
    let router = setup.router.clone();
    let waiting = tokio::spawn(async move { transcribe(&router, VERBOSE, Some(&wav(1))).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let refused = transcribe(&setup.router, VERBOSE, Some(&wav(1))).await;
    assert_error(&refused, 429, "rate_limited");
    assert_eq!(refused.body["error"]["retry_after_s"], 2);
    // The waiting request goes away: its place in the queue is given back.
    waiting.abort();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let again = tokio::time::timeout(Duration::from_millis(500), transcribe(&setup.router, VERBOSE, Some(&wav(1)))).await;
    assert!(again.is_err(), "the request waits instead of being refused: {:?}", again.map(|r| r.text));
    running.abort();
}

#[tokio::test]
async fn a_gate_counts_only_the_requests_still_waiting() {
    let gate = Arc::new(Gate::with_permits(1, 1));
    let held = gate.enter().await.unwrap();
    for _ in 0..5 {
        // A waiter dropped by a timeout (the TimeoutLayer, a closed connection) leaves the queue.
        assert!(tokio::time::timeout(Duration::from_millis(20), gate.enter()).await.is_err());
        assert_eq!(gate.waiting(), 0);
    }
    let waiter = {
        let gate = Arc::clone(&gate);
        tokio::spawn(async move { gate.enter().await.map(|_| ()) })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(gate.waiting(), 1);
    assert!(matches!(gate.enter().await, Err(ComputeError::RateLimited { .. })));
    drop(held);
    waiter.await.unwrap().unwrap();
    assert_eq!((gate.waiting(), gate.running()), (0, 0));
}
