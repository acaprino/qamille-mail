//! `POST /v1/audio/transcriptions` with the real whisper.cpp: needs `whisper-cli` (in the
//! folder, next to the test binary or on PATH), the official `ggml-base.bin` in
//! `QAMILLE_TEST_SPEECH_DIR` and a 16 kHz mono WAV with speech in `QAMILLE_TEST_WAV`.
//!
//! QAMILLE_TEST_SPEECH_DIR=… QAMILLE_TEST_WAV=… cargo test --jobs 2 --test compute_real_whisper -- --ignored

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use qamille_server::compute::speech::WhisperBackend;
use qamille_server::compute::{ComputeAuth, ComputeBackend, ComputeError, ComputeState, ModelStatus, fixed_router};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

struct NoEmbedding;
impl ComputeBackend for NoEmbedding {
    fn chunk(&self, _: &str) -> Result<Vec<String>, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::NotFound)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs whisper-cli, ggml-base.bin and a recording"]
async fn a_real_recording_is_transcribed_by_whisper_cpp() {
    let dir = std::env::var("QAMILLE_TEST_SPEECH_DIR").expect("QAMILLE_TEST_SPEECH_DIR");
    let wav = std::fs::read(std::env::var("QAMILLE_TEST_WAV").expect("QAMILLE_TEST_WAV")).unwrap();
    let state = ComputeState::new(Arc::new(NoEmbedding), ComputeAuth::fixed(TOKEN), "fixed").with_speech(Arc::new(WhisperBackend::new(dir)), 2);
    let boundary = "real-whisper";
    let mut body = Vec::new();
    for (name, value) in [("model", "whisper-base"), ("response_format", "verbose_json")] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\n\r\n").as_bytes());
    body.extend_from_slice(&wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap();
    let response = fixed_router(state).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status.as_u16(), 200, "{value}");
    assert!(value["text"].as_str().is_some_and(|text| !text.trim().is_empty()), "{value}");
    assert!(!value["segments"].as_array().unwrap().is_empty());
    println!("{}", value["text"]);
}
