//! L'assistente non scrive segreti nei log: né la chiave del provider, né quella del dispositivo,
//! né il testo delle richieste e delle risposte (INV-32, INV-33), nemmeno nei percorsi d'errore.
mod support;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use qamille_server::compute::{Usage, UsageSink};
use qamille_server::llm::{ApiKey, LlmConfig, LlmGateway, LlmProvider};
use serde_json::json;
use support::fake_openai::{FakeOpenai, Scenario};
use support::llm_env::*;
use support::{TestServer, test_config, user_with_device};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Un solo sottoscrittore per tutto il file (anche i thread di `spawn_blocking` scrivono qui).
fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .init();
        capture
    })
}

const REQUEST_TEXT: &str = "CONTENUTO-DELLA-RICHIESTA";
const ANSWER_TEXT: &str = "CONTENUTO-DELLA-RISPOSTA";
const PROVIDER_TEXT: &str = "TESTO-DEL-PROVIDER";

struct NullSink;

impl UsageSink for NullSink {
    fn record(&self, _: &str, _: Usage) {}
}

fn request_with(text: &str) -> serde_json::Value {
    json!({"messages": [{"role": "user", "content": text}]})
}

#[tokio::test]
async fn secrets_in_logs() {
    let log = capture();
    let fake = FakeOpenai::start(Scenario::Ok { content: ANSWER_TEXT.to_owned(), usage: Some((60, 50)) }).await;
    let config = LlmConfig { timeout_secs: 1, ..llm_for(&fake.url) };
    let ts = ts_with_llm(config);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let key = login.device_key.clone();
    let provider_error = format!(r#"{{"error":{{"message":"{PROVIDER_TEXT} {PROVIDER_KEY} {REQUEST_TEXT}"}}}}"#);

    // 200.
    let ok = chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await;
    assert_eq!(ok.status, 200, "{}", ok.text);
    assert!(ok.text.contains(ANSWER_TEXT), "la risposta arriva all'app, non al log");
    // 400 nostro (il testo del messaggio è nel ruolo) e 400 del provider.
    let invalid = json!({"messages": [{"role": REQUEST_TEXT, "content": REQUEST_TEXT}]});
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), invalid).await.status, 400);
    fake.set(Scenario::Status { status: 400, retry_after: None, body: provider_error.clone() });
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 400);
    // 401 con la chiave nel corpo, 429, 500.
    fake.set(Scenario::Status { status: 401, retry_after: None, body: provider_error.clone() });
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 503);
    fake.set(Scenario::Status { status: 429, retry_after: Some(3), body: provider_error.clone() });
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 429);
    fake.set(Scenario::Status { status: 500, retry_after: None, body: provider_error.clone() });
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 503);
    // Risposta non valida (con il testo dentro) e tempo scaduto.
    fake.set(Scenario::InvalidJson);
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 503);
    fake.set(Scenario::Delay { ms: 2500 });
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 503);
    // Consenso diverso.
    fake.set(Scenario::Ok { content: ANSWER_TEXT.to_owned(), usage: Some((60, 50)) });
    assert_eq!(chat(&ts.public, &key, Some("true;altro"), request_with(REQUEST_TEXT)).await.status, 409);

    // Tetto superato (un altro server: il tetto vale per tutti i suoi utenti).
    let capped = ts_with_llm(LlmConfig { max_tokens_per_day_per_user: 100, ..llm_for(&fake.url) });
    let other = user_with_device(&capped, "anna", "una password lunga").await;
    assert_eq!(chat(&capped.public, &other.device_key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 200);
    assert_eq!(chat(&capped.public, &other.device_key, Some(CONSENT), request_with(REQUEST_TEXT)).await.status, 429);

    // Il log: nessuna delle stringhe segrete, e le righe che ci devono essere.
    let text = log.text();
    for secret in [REQUEST_TEXT, ANSWER_TEXT, PROVIDER_TEXT, PROVIDER_KEY, key.as_str(), other.device_key.as_str()] {
        assert!(!text.contains(secret), "il log contiene «{secret}»:\n{text}");
    }
    assert!(text.contains("llm"), "{text}");
    assert!(text.contains("model=fake-model"), "{text}");
    assert!(text.contains("prompt_tokens=60") && text.contains("completion_tokens=50"), "{text}");
    assert!(text.contains("chiave del provider LLM rifiutata"), "{text}");
}

#[tokio::test]
async fn the_startup_line_names_the_provider_without_the_key_or_the_full_address() {
    let log = capture();
    let ts = TestServer::new(test_config());
    let startup_key = "sk-avvio-segreta-9876";
    let external = LlmConfig {
        provider: LlmProvider::OpenAiCompatible,
        base_url: Some("https://api.openai.com/v1".to_owned()),
        model: Some("gpt-4.1-mini".to_owned()),
        api_key: ApiKey::new(startup_key),
        ..LlmConfig::default()
    };
    let gateway = LlmGateway::new(&external, Arc::clone(ts.accounts()), Arc::new(NullSink)).unwrap();
    gateway.log_startup();
    let keyless = LlmConfig { api_key: None, ..external.clone() };
    LlmGateway::new(&keyless, Arc::clone(ts.accounts()), Arc::new(NullSink)).unwrap().log_startup();

    let text = log.text();
    let line = text.lines().find(|l| l.contains("gateway LLM") && l.contains("gpt-4.1-mini")).expect("la riga di avvio");
    assert!(line.contains("openai-compatible"), "{line}");
    assert!(line.contains("external=true"), "{line}");
    assert!(line.contains("host=api.openai.com"), "{line}");
    assert!(!text.contains("https://api.openai.com/v1"), "l'indirizzo intero non va nel log:\n{text}");
    assert!(!text.contains(startup_key), "{text}");
    assert!(text.contains("senza QAMILLE_LLM_API_KEY"), "l'avviso per il provider esterno senza chiave:\n{text}");
}
