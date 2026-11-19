//! With the real EmbeddingGemma 2 (ignored by default):
//!
//! ```text
//! QAMILLE_MODELS=<data>/models cargo test --jobs 2 --test compute_real_model -- --ignored
//! ```
//!
//! The vectors, passages and image vector must match, within 1e-5, those the app's former
//! `--embedding-worker` produced for the same inputs (`fixtures/embeddinggemma2-reference.json`,
//! captured before the worker was removed).
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use qamille_compute::embedder::{document_prompt, query_prompt};
use qamille_server::compute::engine::OnnxBackend;
use qamille_server::compute::parser::ParserCommand;
use qamille_server::compute::{ComputeAuth, ComputeState, DIM, fixed_router};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

fn router() -> Router {
    let models = std::env::var("QAMILLE_MODELS").expect("QAMILLE_MODELS: the folder holding embeddinggemma-2/");
    let parser = ParserCommand { program: env!("CARGO_BIN_EXE_qamille-server").into(), args: vec!["--parser-worker".to_owned()] };
    fixed_router(ComputeState::new(Arc::new(OnnxBackend::new(models, parser)), ComputeAuth::fixed(TOKEN), "fixed"))
}

async fn post(router: &Router, path: &str, content_type: &str, body: Vec<u8>) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&bytes));
    serde_json::from_slice(&bytes).unwrap()
}

fn floats(value: &Value) -> Vec<f32> {
    value.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect()
}

/// Largest absolute difference, after checking length and unit norm.
fn compare(got: &[f32], want: &[f32]) -> f32 {
    assert_eq!(got.len(), DIM);
    let norm: f32 = got.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "norm {norm}");
    got.iter().zip(want).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_model_matches_the_former_worker_within_1e_5() {
    let reference: Value = serde_json::from_str(include_str!("fixtures/embeddinggemma2-reference.json")).unwrap();
    let router = router();
    let mut worst = 0f32;
    let mut inputs = Vec::new();
    let mut wanted = Vec::new();
    for query in reference["queries"].as_array().unwrap() {
        inputs.push(query_prompt(query["text"].as_str().unwrap()));
        wanted.push(floats(&query["vector"]));
    }
    for document in reference["documents"].as_array().unwrap() {
        inputs.push(document_prompt(document["title"].as_str().unwrap(), document["text"].as_str().unwrap()));
        wanted.push(floats(&document["vector"]));
    }
    // One batch, as the app sends a document's passages.
    let response = post(&router, "/v1/embeddings", "application/json", serde_json::to_vec(&json!({"model": "embeddinggemma-2", "input": inputs, "dimensions": 256})).unwrap()).await;
    let data = response["data"].as_array().unwrap();
    assert_eq!(data.len(), wanted.len());
    for (item, want) in data.iter().zip(&wanted) {
        let difference = compare(&floats(&item["embedding"]), want);
        println!("text vector: max |difference| = {difference:e}");
        worst = worst.max(difference);
    }
    // And one at a time.
    let single = post(&router, "/v1/embeddings", "application/json", serde_json::to_vec(&json!({"input": inputs[0]})).unwrap()).await;
    worst = worst.max(compare(&floats(&single["data"][0]["embedding"]), &wanted[0]));

    let chunks = post(&router, "/qamille/v1/chunks", "application/json", serde_json::to_vec(&json!({"text": reference["chunks"]["text"]})).unwrap()).await;
    assert_eq!(chunks["chunks"], reference["chunks"]["chunks"], "same passages");

    let hex = reference["image"]["png_hex"].as_str().unwrap();
    let png: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    let image = post(&router, "/qamille/v1/embeddings/image", "application/octet-stream", png).await;
    let difference = compare(&floats(&image["embedding"]), &floats(&reference["image"]["vector"]));
    println!("image vector: max |difference| = {difference:e}");
    worst = worst.max(difference);
    println!("worst max |difference| over {} vectors: {worst:e}", wanted.len() + 2);
    assert!(worst <= 1e-5, "vectors differ by {worst}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_model_refuses_inputs_beyond_its_token_limit_without_pausing() {
    let router = router();
    // About 3 000 tokens in well under the byte cap: refused under the permit, before inference.
    let long = "a b c d e f g h i l ".repeat(300);
    assert!(long.len() < qamille_server::compute::MAX_INPUT_BYTES);
    let request = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({"input": [long]})).unwrap()))
        .unwrap();
    let started = std::time::Instant::now();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 413);
    println!("refused in {:?}", started.elapsed());
    // The model is not paused for the others.
    let ok = post(&router, "/v1/embeddings", "application/json", serde_json::to_vec(&json!({"input": [document_prompt("Fattura", "pagata ieri")]})).unwrap()).await;
    assert_eq!(ok["data"][0]["embedding"].as_array().unwrap().len(), DIM);
}
