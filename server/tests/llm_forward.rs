//! L'inoltro al provider, a livello di `LlmGateway`: intestazioni, redirect, limiti, tempo,
//! traduzione degli stati, posto in volo, conto dei token e tetto giornaliero.
mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qamille_server::accounts::ComputeUsage;
use qamille_server::compute::{Usage, UsageSink};
use qamille_server::llm::{
    ApiKey, LlmConfig, LlmError, LlmGateway, LlmProvider, MAX_RESPONSE_BYTES, UNAVAILABLE_RETRY_S,
};
use serde_json::{Value, json};
use support::fake_openai::{FakeOpenai, Scenario};
use support::{TestServer, test_config, user_with_device};

const PROVIDER_KEY: &str = "sk-prova-chiave-del-provider-1234";

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Usage)>>);

impl UsageSink for Recorder {
    fn record(&self, user_id: &str, usage: Usage) {
        self.0.lock().unwrap().push((user_id.to_owned(), usage));
    }
}

fn config_for(url: &str) -> LlmConfig {
    LlmConfig {
        provider: LlmProvider::OpenAiCompatible,
        base_url: Some(url.to_owned()),
        model: Some("fake-model".to_owned()),
        api_key: ApiKey::new(PROVIDER_KEY),
        ..LlmConfig::default()
    }
}

fn ok(content: &str, usage: Option<(u64, u64)>) -> Scenario {
    Scenario::Ok { content: content.to_owned(), usage }
}

fn chat_body() -> Vec<u8> {
    serde_json::to_vec(&json!({"messages": [{"role": "user", "content": "ciao"}]})).unwrap()
}

struct Rig {
    ts: TestServer,
    gateway: Arc<LlmGateway>,
    sink: Arc<Recorder>,
    user_id: String,
}

async fn rig(config: LlmConfig) -> Rig {
    let ts = TestServer::new(test_config());
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let sink = Arc::new(Recorder::default());
    let gateway = Arc::new(LlmGateway::new(&config, Arc::clone(ts.accounts()), sink.clone()).expect("gateway"));
    Rig { ts, gateway, sink, user_id: login.user_id }
}

impl Rig {
    async fn call(&self, body: Vec<u8>) -> Result<Value, LlmError> {
        let slot = Arc::new(self.gateway.enter(&self.user_id).expect("un posto libero"));
        let bytes = self.gateway.complete(&self.user_id, slot, body).await?;
        Ok(serde_json::from_slice(&bytes).unwrap())
    }
}

#[tokio::test]
async fn the_key_goes_downstream_in_the_authorization_header_only() {
    let fake = FakeOpenai::start(ok("ciao", Some((3, 2)))).await;
    let with_key = rig(config_for(&fake.url)).await;
    with_key.call(chat_body()).await.unwrap();
    let seen = fake.seen();
    assert_eq!(seen[0].header("authorization"), Some(format!("Bearer {PROVIDER_KEY}").as_str()));
    // La chiave non è in nessun altro posto: né in altre intestazioni né nel corpo.
    for (name, value) in &seen[0].headers {
        if !name.eq_ignore_ascii_case("authorization") {
            assert!(!value.contains(PROVIDER_KEY), "{name}");
        }
    }
    assert!(!seen[0].body.to_string().contains(PROVIDER_KEY));

    // Senza chiave (Ollama): nessuna intestazione Authorization.
    let mut config = config_for(&fake.url);
    config.api_key = None;
    let without = rig(config).await;
    without.call(chat_body()).await.unwrap();
    assert_eq!(fake.hits(), 2);
    assert!(fake.seen()[1].header("authorization").is_none());
}

#[tokio::test]
async fn the_downstream_request_has_the_whitelist_and_the_hashed_user() {
    let fake = FakeOpenai::start(ok("ciao", Some((3, 2)))).await;
    let rig = rig(config_for(&fake.url)).await;
    let body = serde_json::to_vec(&json!({
        "messages": [{"role": "user", "content": "ciao", "name": "mario"}],
        "user": "mario@example.com", "logit_bias": {"1": 5}, "store": true, "metadata": {"a": 1},
        "temperature": 0.5,
    }))
    .unwrap();
    rig.call(body).await.unwrap();
    let seen = &fake.seen()[0];
    assert_eq!(seen.path, "/v1/chat/completions");
    let mut keys: Vec<_> = seen.body.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["max_tokens", "messages", "model", "temperature", "user"]);
    assert_eq!(seen.body["user"], qamille_server::llm::user_hash(&rig.user_id));
    assert_eq!(seen.body["messages"], json!([{"role": "user", "content": "ciao"}]));
}

#[tokio::test]
async fn no_header_of_the_app_is_forwarded() {
    // Il gateway non vede le intestazioni dell'app (Authorization del dispositivo, consenso,
    // cookie): al provider arrivano solo quelle che costruisce da sé.
    let fake = FakeOpenai::start(ok("ciao", Some((1, 1)))).await;
    let rig = rig(config_for(&fake.url)).await;
    rig.call(chat_body()).await.unwrap();
    let allowed = [
        "authorization",
        "content-type",
        "user-agent",
        "host",
        "accept",
        "accept-encoding",
        "content-length",
    ];
    for (name, _) in &fake.seen()[0].headers {
        assert!(allowed.contains(&name.as_str()), "intestazione inattesa: {name}");
    }
    assert!(fake.seen()[0].header("user-agent").unwrap().starts_with("qamille-server/"));
}

#[tokio::test]
async fn a_redirect_is_not_followed() {
    let second = FakeOpenai::start(ok("altrove", Some((1, 1)))).await;
    let first = FakeOpenai::start(Scenario::Redirect { to: format!("{}/chat/completions", second.url) }).await;
    let rig = rig(config_for(&first.url)).await;
    let error = rig.call(chat_body()).await.unwrap_err();
    assert_eq!(error, LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
    assert_eq!(first.hits(), 1);
    assert_eq!(second.hits(), 0, "la chiave non deve arrivare a un altro indirizzo");
}

#[tokio::test]
async fn an_enormous_answer_is_cut_at_the_limit() {
    let fake = FakeOpenai::start(Scenario::Huge { bytes: MAX_RESPONSE_BYTES + 1 }).await;
    let rig = rig(config_for(&fake.url)).await;
    let error = rig.call(chat_body()).await.unwrap_err();
    assert_eq!(error, LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
}

#[tokio::test]
async fn a_slow_provider_times_out_as_unavailable() {
    let fake = FakeOpenai::start(Scenario::Delay { ms: 3000 }).await;
    let mut config = config_for(&fake.url);
    config.timeout_secs = 1;
    let rig = rig(config).await;
    let started = Instant::now();
    let error = rig.call(chat_body()).await.unwrap_err();
    assert_eq!(error, LlmError::Unavailable { retry_after_s: 30 });
    assert!(started.elapsed() < Duration::from_millis(2900), "{:?}", started.elapsed());
}

#[tokio::test]
async fn an_unreachable_provider_is_unavailable() {
    let rig = rig(config_for("http://127.0.0.1:1/v1")).await;
    let error = rig.call(chat_body()).await.unwrap_err();
    assert_eq!(error, LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
}

#[tokio::test]
async fn statuses_from_the_provider_are_translated() {
    let fake = FakeOpenai::start(ok("ciao", Some((3, 2)))).await;
    let rig = rig(config_for(&fake.url)).await;
    let answer = rig.call(chat_body()).await.unwrap();
    assert_eq!(answer["choices"][0]["message"]["content"], "ciao");
    assert_eq!(answer["usage"]["total_tokens"], 5);

    let status = |status: u16, retry_after: Option<u64>, body: &str| Scenario::Status {
        status,
        retry_after,
        body: body.to_owned(),
    };
    fake.set(status(429, Some(7), "{}"));
    assert_eq!(rig.call(chat_body()).await.unwrap_err(), LlmError::RateLimited { retry_after_s: 7 });
    fake.set(status(500, None, "{}"));
    assert_eq!(rig.call(chat_body()).await.unwrap_err(), LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
    fake.set(status(401, None, "{}"));
    assert_eq!(rig.call(chat_body()).await.unwrap_err(), LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
    fake.set(status(400, None, r#"{"error":{"code":"context_length_exceeded"}}"#));
    assert_eq!(rig.call(chat_body()).await.unwrap_err(), LlmError::ContextExceeded);
    fake.set(Scenario::InvalidJson);
    assert_eq!(rig.call(chat_body()).await.unwrap_err(), LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
}

#[tokio::test]
async fn a_provider_error_body_is_never_returned() {
    let body = format!(r#"{{"error":{{"message":"Incorrect API key {PROVIDER_KEY} SEGRETO"}}}}"#);
    let fake = FakeOpenai::start(Scenario::Status { status: 401, retry_after: None, body }).await;
    let rig = rig(config_for(&fake.url)).await;
    let error = rig.call(chat_body()).await.unwrap_err();
    for printed in [format!("{error}"), format!("{error:?}"), format!("{error:#?}")] {
        assert!(!printed.contains("SEGRETO"), "{printed}");
        assert!(!printed.contains(PROVIDER_KEY), "{printed}");
    }
}

#[tokio::test]
async fn the_slot_is_held_until_the_provider_answers_even_if_the_caller_goes_away() {
    let fake = FakeOpenai::start(Scenario::Delay { ms: 800 }).await;
    let rig = rig(config_for(&fake.url)).await;
    let slot = Arc::new(rig.gateway.enter(&rig.user_id).unwrap());
    assert_eq!(rig.gateway.in_flight(&rig.user_id), 1);
    let gateway = Arc::clone(&rig.gateway);
    let user_id = rig.user_id.clone();
    let abandoned = tokio::time::timeout(Duration::from_millis(100), gateway.complete(&user_id, slot, chat_body())).await;
    assert!(abandoned.is_err(), "l'app se n'è andata");
    assert_eq!(rig.gateway.in_flight(&rig.user_id), 1, "il posto resta fino alla risposta del provider");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(rig.gateway.in_flight(&rig.user_id), 0);
    // E i token della chiamata abbandonata sono stati contati lo stesso.
    assert_eq!(rig.sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tokens_are_recorded_on_the_sink_when_the_call_ends() {
    let fake = FakeOpenai::start(ok("ciao", Some((11, 7)))).await;
    let rig = rig(config_for(&fake.url)).await;
    assert!(rig.sink.0.lock().unwrap().is_empty());
    rig.call(chat_body()).await.unwrap();
    let seen = rig.sink.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, rig.user_id);
    assert_eq!((seen[0].1.requests, seen[0].1.llm_prompt_tokens, seen[0].1.llm_completion_tokens), (0, 11, 7));
    // Un errore non registra nulla.
    fake.set(Scenario::Status { status: 500, retry_after: None, body: String::new() });
    rig.call(chat_body()).await.unwrap_err();
    assert_eq!(rig.sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_daily_count_is_read_once_per_user_and_day() {
    let fake = FakeOpenai::start(ok("ciao", Some((60, 50)))).await;
    let mut config = config_for(&fake.url);
    config.max_tokens_per_day_per_user = 100;
    let rig = rig(config).await;

    // Primo controllo: legge dal database (vuoto) e poi tiene il numero in memoria.
    rig.gateway.within_daily_cap(&rig.user_id).await.unwrap();
    let usage = ComputeUsage { llm_prompt_tokens: 500, ..ComputeUsage::default() };
    rig.ts.accounts().record_usage(&rig.user_id, usage).await.unwrap();
    rig.gateway.within_daily_cap(&rig.user_id).await.expect("il conto non si rilegge nello stesso giorno");

    // Una chiamata da 110 token supera il tetto (soglia morbida: la richiesta passa).
    rig.call(chat_body()).await.unwrap();
    let error = rig.gateway.within_daily_cap(&rig.user_id).await.unwrap_err();
    // Le 09:00 del 13 novembre: 15 ore a mezzanotte.
    assert_eq!(error, LlmError::RateLimited { retry_after_s: 15 * 3600 });

    // Dopo mezzanotte UTC il giorno nuovo ricomincia da quello che il database ha per quel giorno.
    rig.ts.clock.advance(chrono::TimeDelta::hours(16));
    rig.gateway.within_daily_cap(&rig.user_id).await.unwrap();

    // Un tetto spento (0) non limita mai.
    let mut open = config_for(&fake.url);
    open.max_tokens_per_day_per_user = 0;
    let unlimited = LlmGateway::new(&open, Arc::clone(rig.ts.accounts()), rig.sink.clone()).unwrap();
    unlimited.within_daily_cap(&rig.user_id).await.unwrap();
}

#[tokio::test]
async fn a_failing_daily_count_fails_closed_and_is_not_remembered_as_zero() {
    let fake = FakeOpenai::start(ok("ciao", Some((60, 50)))).await;
    let mut config = config_for(&fake.url);
    config.max_tokens_per_day_per_user = 100;
    let rig = rig(config).await;
    let usage = ComputeUsage { llm_prompt_tokens: 500, ..ComputeUsage::default() };
    rig.ts.accounts().record_usage(&rig.user_id, usage).await.unwrap();

    // Il database non risponde (tabella dei conti assente): la richiesta e' rifiutata, non passa.
    let path = rig.ts.dir.path().join("server.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("ALTER TABLE llm_usage_day RENAME TO llm_usage_day_off").unwrap();
    let error = rig.gateway.within_daily_cap(&rig.user_id).await.unwrap_err();
    assert_eq!(error, LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
    assert_eq!(qamille_server::compute::ComputeError::from(error).status().as_u16(), 503);

    // Tornato il database, il conto vero (sopra il tetto) vale: lo zero non e' stato tenuto.
    conn.execute_batch("ALTER TABLE llm_usage_day_off RENAME TO llm_usage_day").unwrap();
    let error = rig.gateway.within_daily_cap(&rig.user_id).await.unwrap_err();
    assert!(matches!(error, LlmError::RateLimited { .. }), "{error:?}");
}

#[tokio::test]
async fn the_total_of_calls_in_flight_is_bounded_across_users_and_released_with_the_slot() {
    let fake = FakeOpenai::start(ok("ciao", Some((1, 1)))).await;
    let mut config = config_for(&fake.url);
    config.max_in_flight_per_user = 2;
    config.max_in_flight_total = 2;
    let rig = rig(config).await;
    let first = rig.gateway.enter("utente-a").expect("primo posto");
    let second = rig.gateway.enter("utente-b").expect("secondo posto, un altro utente");
    // Il tetto globale e' pieno anche se utente-a ha ancora un posto suo.
    assert!(rig.gateway.enter("utente-a").is_none());
    assert!(rig.gateway.enter("utente-c").is_none());
    // Un rifiuto per il tetto per utente non trattiene un posto globale.
    drop(second);
    let again = rig.gateway.enter("utente-a").expect("posto liberato");
    assert!(rig.gateway.enter("utente-a").is_none(), "il tetto globale e' di nuovo pieno");
    drop(first);
    drop(again);
    assert!(rig.gateway.enter("utente-c").is_some());
}
