//! Un finto provider compatibile con OpenAI per le prove del gateway LLM: un router axum su
//! `127.0.0.1:0` che risponde secondo lo `Scenario` corrente e ricorda ciò che ha ricevuto.
//! Non scrive mai nulla nel log.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

#[derive(Clone, Debug)]
pub enum Scenario {
    /// Una risposta normale; `usage` assente: il provider non dichiara i token.
    Ok { content: String, usage: Option<(u64, u64)> },
    /// Una risposta con campi che l'app non deve vedere.
    Extra,
    /// Un errore HTTP con il suo corpo.
    Status { status: u16, retry_after: Option<u64>, body: String },
    /// Risponde bene, ma dopo `ms` millisecondi.
    Delay { ms: u64 },
    /// Un corpo di `bytes` byte.
    Huge { bytes: usize },
    /// Un redirect `302`.
    Redirect { to: String },
    /// `200` con un corpo che non è JSON.
    InvalidJson,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Inner {
    scenario: Mutex<Scenario>,
    seen: Mutex<Vec<Seen>>,
}

pub struct FakeOpenai {
    /// `http://127.0.0.1:PORT/v1`
    pub url: String,
    inner: Arc<Inner>,
}

pub fn ok_body(content: &str, usage: Option<(u64, u64)>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "id": "chatcmpl-fake",
        "object": "chat.completion",
        "created": 1_700_000_000u64,
        "model": "fake-model",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
    });
    if let Some((prompt, completion)) = usage {
        body["usage"] = serde_json::json!({
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "total_tokens": prompt + completion,
        });
    }
    body
}

fn json(body: serde_json::Value) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

async fn chat(State(inner): State<Arc<Inner>>, headers: HeaderMap, body: Bytes) -> Response {
    inner.seen.lock().unwrap().push(Seen {
        path: "/v1/chat/completions".to_owned(),
        headers: headers
            .iter()
            .map(|(n, v)| (n.as_str().to_owned(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .collect(),
        body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    });
    let scenario = inner.scenario.lock().unwrap().clone();
    match scenario {
        Scenario::Ok { content, usage } => json(ok_body(&content, usage)),
        Scenario::Extra => {
            let mut body = ok_body("ciao", Some((3, 2)));
            body["system_fingerprint"] = "fp_segreto".into();
            body["service_tier"] = "default".into();
            body["choices"][0]["logprobs"] = serde_json::json!({"content": []});
            body["choices"][0]["message"]["tool_calls"] = serde_json::json!([{"id": "call_1"}]);
            json(body)
        }
        Scenario::Status { status, retry_after, body } => {
            let mut response = (
                StatusCode::from_u16(status).unwrap(),
                [(header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response();
            if let Some(seconds) = retry_after {
                response.headers_mut().insert(header::RETRY_AFTER, seconds.to_string().parse().unwrap());
            }
            response
        }
        Scenario::Delay { ms } => {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            json(ok_body("lenta", Some((1, 1))))
        }
        Scenario::Huge { bytes } => {
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], vec![b'a'; bytes]).into_response()
        }
        Scenario::Redirect { to } => {
            (StatusCode::FOUND, [(header::LOCATION, to)], "").into_response()
        }
        Scenario::InvalidJson => {
            (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], "non json {").into_response()
        }
    }
}

impl FakeOpenai {
    pub async fn start(scenario: Scenario) -> FakeOpenai {
        let inner = Arc::new(Inner { scenario: Mutex::new(scenario), seen: Mutex::new(Vec::new()) });
        let router = Router::new().route("/v1/chat/completions", post(chat)).with_state(Arc::clone(&inner));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("porta del finto provider");
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        FakeOpenai { url: format!("http://127.0.0.1:{port}/v1"), inner }
    }

    pub fn set(&self, scenario: Scenario) {
        *self.inner.scenario.lock().unwrap() = scenario;
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.inner.seen.lock().unwrap().clone()
    }

    pub fn hits(&self) -> usize {
        self.inner.seen.lock().unwrap().len()
    }
}
