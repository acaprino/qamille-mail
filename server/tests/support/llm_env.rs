//! Aiuti comuni alle prove del gateway LLM nel core: un finto calcolo, un server con il
//! gateway acceso (e un riavvio sulla stessa cartella) e la chiamata a `/v1/chat/completions`.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use qamille_compute::speech::Segment;
use qamille_server::compute::speech::{SpeechBackend, TranscriptionInput};
use qamille_server::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus};
use qamille_server::db::CORE_MIGRATIONS;
use qamille_server::ext::{BoxFut, Entitlement, Resource};
use qamille_server::llm::{ApiKey, LlmConfig, LlmProvider};
use qamille_server::{Server, ServerError};
use serde_json::{Value, json};

use super::fake_openai::{FakeOpenai, Scenario};
use super::{TestResponse, TestServer, bearer, loopback, request, request_raw, test_config, user_with_device};

pub const PROVIDER_KEY: &str = "sk-prova-chiave-del-provider-1234";
pub const CONSENT: &str = "false;127.0.0.1";

pub struct Fake(pub Duration);

impl ComputeBackend for Fake {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        Ok(vec![text.to_owned()])
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        std::thread::sleep(self.0);
        Ok(inputs.iter().map(|_| vec![0.0; DIM]).collect())
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Ok(vec![0.0; DIM])
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Ok(String::new())
    }
    fn models(&self) -> Vec<ModelStatus> {
        vec![ModelStatus {
            id: MODEL_ID.to_owned(),
            kind: "embedding".to_owned(),
            state: "ready".to_owned(),
            percent: None,
            done_bytes: 0,
            total_bytes: 0,
            error: None,
            loaded: true,
            dimensions: DIM,
            quantization: "q8".to_owned(),
            }]
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
}

pub struct Speech;

impl SpeechBackend for Speech {
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
    fn transcribe(&self, _: &TranscriptionInput, _: &AtomicBool) -> Result<Vec<Segment>, ComputeError> {
        Ok(Vec::new())
    }
}

/// Nega solo l'assistente.
pub struct NoLlm;

impl Entitlement for NoLlm {
    fn require<'a>(&'a self, _: &'a str, resource: Resource) -> BoxFut<'a, Result<(), ServerError>> {
        Box::pin(async move {
            match resource {
                Resource::Llm => Err(ServerError::SubscriptionRequired),
                _ => Ok(()),
            }
        })
    }
}

pub fn llm_for(url: &str) -> LlmConfig {
    LlmConfig {
        provider: LlmProvider::OpenAiCompatible,
        base_url: Some(url.to_owned()),
        model: Some("fake-model".to_owned()),
        api_key: ApiKey::new(PROVIDER_KEY),
        ..LlmConfig::default()
    }
}

pub fn ok() -> Scenario {
    Scenario::Ok { content: "risposta".to_owned(), usage: Some((60, 50)) }
}

pub fn mount(server: Server, entitlement: Option<Arc<dyn Entitlement>>, embed_delay: Duration) -> Server {
    let server = server.with_compute(Arc::new(Fake(embed_delay)), Arc::new(Speech), 1);
    match entitlement {
        Some(entitlement) => server.with_entitlement(entitlement),
        None => server,
    }
}

pub fn rebuild(server: Server, ts: TestServer) -> TestServer {
    let TestServer { clock, dir, .. } = ts;
    TestServer { public: server.public_router(), admin: server.admin_router(), server, clock, dir }
}

/// Un server con il calcolo e (con `llm`) il gateway.
pub fn ts_with(llm: Option<LlmConfig>, entitlement: Option<Arc<dyn Entitlement>>, embed_delay: Duration) -> TestServer {
    let mut config = test_config();
    if let Some(llm) = llm {
        config.llm = llm;
    }
    let ts = TestServer::new(config);
    let server = mount(ts_server(&ts), entitlement, embed_delay);
    rebuild(server, ts)
}

/// Un secondo `Server` sulla stessa cartella e sullo stesso orologio (un riavvio).
pub fn ts_server(ts: &TestServer) -> Server {
    let mut config = ts.server.config().clone();
    config.data_dir = ts.dir.path().to_path_buf();
    Server::open_with_migrations(config, ts.clock.clone(), CORE_MIGRATIONS).expect("riapertura")
}

pub fn ts_with_llm(llm: LlmConfig) -> TestServer {
    ts_with(Some(llm), None, Duration::ZERO)
}

pub fn hello() -> Value {
    json!({"messages": [{"role": "user", "content": "ciao"}]})
}

pub async fn chat(router: &axum::Router, key: &str, consent: Option<&str>, body: Value) -> TestResponse {
    let auth = bearer(key);
    let mut headers = vec![("authorization", auth.as_str())];
    if let Some(consent) = consent {
        headers.push(("x-qamille-llm-consent", consent));
    }
    request(router, "POST", "/v1/chat/completions", Some(body), loopback(), &headers).await
}

pub async fn embed(router: &axum::Router, key: &str) -> TestResponse {
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "input": ["a"], "dimensions": DIM})).unwrap();
    request_raw(router, "POST", "/v1/embeddings", Some(body), loopback(), &[("authorization", &bearer(key))]).await
}

pub async fn setup(llm: impl FnOnce(&str) -> LlmConfig) -> (FakeOpenai, TestServer, String, String) {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(llm(&fake.url));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    (fake, ts, login.device_key, login.user_id)
}

/// The consent of the built-in provider.
pub const BUILTIN_CONSENT: &str = "false;questo server";

/// A configuration with the built-in provider (`llm-medio`, context 4096).
pub fn builtin_llm() -> LlmConfig {
    LlmConfig {
        provider: LlmProvider::Builtin,
        builtin_model: Some("llm-medio".to_owned()),
        builtin_context: 4096,
        ..LlmConfig::default()
    }
}

/// A server with the compute routes and the built-in gateway over `supervisor`.
pub fn ts_with_builtin(
    llm: LlmConfig,
    supervisor: Arc<qamille_server::llm_builtin::LlmSupervisor>,
    entitlement: Option<Arc<dyn Entitlement>>,
) -> TestServer {
    let mut config = test_config();
    config.llm = llm;
    let ts = TestServer::new(config);
    let server = mount(ts_server(&ts), entitlement, Duration::ZERO).with_llm_supervisor(supervisor);
    rebuild(server, ts)
}

