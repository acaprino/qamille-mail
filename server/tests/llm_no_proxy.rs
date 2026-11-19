//! Il gateway non usa mai un proxy preso dall'ambiente del processo: l'indirizzo del provider
//! lo sceglie l'operatore, non `HTTP_PROXY`. Un file solo, un test solo: cambia l'ambiente.
mod support;

use std::sync::Arc;

use qamille_server::compute::{Usage, UsageSink};
use qamille_server::llm::{LlmConfig, LlmGateway, LlmProvider};
use serde_json::json;
use support::fake_openai::{FakeOpenai, Scenario};
use support::{TestServer, test_config, user_with_device};

struct NullSink;
impl UsageSink for NullSink {
    fn record(&self, _: &str, _: Usage) {}
}

#[tokio::test]
async fn proxy_variables_of_the_environment_are_ignored() {
    let fake = FakeOpenai::start(Scenario::Ok { content: "ciao".to_owned(), usage: Some((1, 1)) }).await;
    // Un proxy che non esiste: se l'agente lo usasse, la chiamata fallirebbe.
    // SAFETY: l'unico test di questo binario, nessun altro thread legge l'ambiente.
    unsafe {
        for name in ["ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"] {
            std::env::set_var(name, "http://127.0.0.1:9");
        }
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }
    let ts = TestServer::new(test_config());
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let config = LlmConfig {
        provider: LlmProvider::OpenAiCompatible,
        base_url: Some(fake.url.clone()),
        model: Some("fake-model".to_owned()),
        ..LlmConfig::default()
    };
    let gateway = Arc::new(LlmGateway::new(&config, Arc::clone(ts.accounts()), Arc::new(NullSink)).unwrap());
    let slot = Arc::new(gateway.enter(&login.user_id).unwrap());
    let body = serde_json::to_vec(&json!({"messages": [{"role": "user", "content": "ciao"}]})).unwrap();
    gateway.complete(&login.user_id, slot, body).await.expect("la chiamata va dritta al provider");
    assert_eq!(fake.hits(), 1);
}
