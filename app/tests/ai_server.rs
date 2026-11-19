//! The «Qamille server» provider of the assistant (gateway LLM, wave 5) against a fake gateway
//! on loopback: the request, the key only towards the session's address, no redirect, the
//! errors, the consent and the length of the text. `compute_factory::configure` is
//! process-wide, so the tests run one at a time.
use qamille::application::AppResult;
use qamille::application::ports::{SecretStore, StoreRepository};
use qamille::application::server_account::SERVER_SESSION;
use qamille::domain::model::{AiProvider, Email, LlmConsent, Settings, Store};
use qamille::infrastructure::ai;
use qamille::infrastructure::compute_factory;
use qamille::infrastructure::repository::MemoryRepository;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

static SERIAL: Mutex<()> = Mutex::new(());
const KEY: &str = "spk_dev_secret_value_9f3";

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
impl SecretStore for Vault {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Seen {
    line: String,
    headers: String,
    body: String,
}
type Handler = dyn Fn(&Seen) -> String + Send + Sync;

/// A gateway of the test: every request is recorded and answered by `handler` (a raw response).
struct Fake {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}
impl Fake {
    fn start(handler: Arc<Handler>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if raw.len() >= end + 4 + length || n == 0 {
                            break;
                        }
                    } else if n == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let (line, headers) = head.split_once("\r\n").unwrap_or((head, ""));
                let request = Seen { line: line.into(), headers: headers.into(), body: body.into() };
                log.lock().unwrap().push(request.clone());
                let _ = stream.write_all(handler(&request).as_bytes());
            }
        });
        Fake { url, seen }
    }
    fn chats(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().iter().filter(|s| s.line.contains("/v1/chat/completions")).cloned().collect()
    }
}

fn json(status: &str, body: &str) -> String {
    format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}
fn answer(text: &str) -> String {
    json("200 OK", &serde_json::json!({"choices":[{"message":{"role":"assistant","content":text}}]}).to_string())
}
fn error(status: &str, code: &str, message: &str) -> String {
    json(status, &serde_json::json!({"error":{"code":code,"message":message,"retry_after_s":30}}).to_string())
}
fn info(llm: &str) -> String {
    json("200 OK", &format!(r#"{{"version":"1","api":1,"llm":{llm}}}"#))
}
const INFO_PLAIN: &str = r#"{"model":"llm-medio","models":["llm-medio","llm-grande"],"external":false,"label":"Server","context_tokens":null}"#;

/// Connects this process to `url`: session in the vault, consent of the address and of the
/// provider (`false;Server` unless `label` says otherwise).
fn connect_as(url: &str, label: &str) {
    let mut store = Store::default();
    store.settings.compute_consented_urls.push(url.into());
    store.settings.llm_consent = Some(LlmConsent { url: url.into(), label: label.into(), external: false });
    let vault = Arc::new(Vault::default());
    vault.set(SERVER_SESSION, &format!(r#"{{"url":"{url}","user_id":"u","username":"m","device_id":"d","device_key":"{KEY}"}}"#)).unwrap();
    let repository: Arc<dyn StoreRepository> = Arc::new(MemoryRepository::new(store));
    compute_factory::configure(repository, vault);
}
fn connect(url: &str) {
    connect_as(url, "Server");
}
fn settings() -> Settings {
    Settings { ai_provider: AiProvider::QamilleServer, ..Default::default() }
}
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
fn gateway(chat: impl Fn(&Seen) -> String + Send + Sync + 'static) -> Fake {
    let fake = Fake::start(Arc::new(move |seen: &Seen| {
        if seen.line.starts_with("GET /v1/info") { info(INFO_PLAIN) } else { chat(seen) }
    }));
    connect(&fake.url);
    fake
}

#[test]
fn the_request_goes_to_the_session_address_with_the_key_and_the_consent() {
    let _g = serial();
    let fake = gateway(|_| answer("  Ciao  "));
    assert_eq!(ai::complete(&settings(), "sys", "hello").unwrap(), "Ciao");
    let chats = fake.chats();
    assert_eq!(chats.len(), 1);
    assert!(chats[0].line.starts_with("POST /v1/chat/completions"));
    let headers = chats[0].headers.to_ascii_lowercase();
    assert!(headers.contains(&format!("authorization: bearer {}", KEY.to_ascii_lowercase())));
    assert!(headers.contains("x-qamille-llm-consent: false;server"));
    let body: serde_json::Value = serde_json::from_str(&chats[0].body).unwrap();
    assert!(body.get("model").is_none(), "the server decides the model");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "hello");
}

#[test]
fn the_key_is_sent_only_to_the_session_address() {
    let _g = serial();
    let other = Fake::start(Arc::new(|_| answer("elsewhere")));
    let fake = gateway(|_| answer("here"));
    let mut s = settings();
    s.ai_base_url = other.url.clone();
    s.ai_key = "user-key".into();
    assert_eq!(ai::complete(&s, "sys", "hi").unwrap(), "here");
    assert!(other.seen.lock().unwrap().is_empty(), "nothing goes to the address in the settings");
    assert_eq!(fake.chats().len(), 1);
    // No link: nothing is sent at all.
    compute_factory::configure(Arc::new(MemoryRepository::new(Store::default())), Arc::new(Vault::default()));
    assert_eq!(ai::complete(&s, "sys", "hi").err().unwrap(), "offline");
    assert!(!ai::online(&s));
    assert_eq!(fake.chats().len(), 1);
}

#[test]
fn a_redirect_is_not_followed() {
    let _g = serial();
    let target = Fake::start(Arc::new(|_| answer("stolen")));
    let location = format!("{}/v1/chat/completions", target.url);
    let _fake = gateway(move |_| format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
    let error = ai::complete(&settings(), "sys", "hi").err().unwrap();
    assert!(target.seen.lock().unwrap().is_empty(), "the key never follows a redirect");
    assert!(!error.contains(KEY));
}

#[test]
fn errors_are_the_fixed_messages_of_the_server() {
    let _g = serial();
    let busy = "The compute server is busy; the request will be retried shortly.";
    let memory = "The assistant model needs more free memory than this server has right now.";
    let preparing = "The assistant model is being prepared on the server; try again in a few minutes.";
    let paid = "An active Qamille subscription is required for compute on the server.";
    for (status, code, message, expected) in [
        ("402 Payment Required", "subscription_required", paid, paid),
        ("401 Unauthorized", "unauthorized", "x", "This device is no longer connected to the server."),
        ("503 Service Unavailable", "llm_unavailable", memory, memory),
        ("503 Service Unavailable", "llm_unavailable", preparing, preparing),
        ("503 Service Unavailable", "llm_unavailable", "The assistant is temporarily unavailable.", "The assistant is temporarily unavailable."),
        ("429 Too Many Requests", "rate_limited", busy, busy),
    ] {
        let _fake = gateway(move |_| error(status, code, message));
        let got = ai::complete(&settings(), "sys", "hi").err().unwrap();
        assert_eq!(got, expected);
        assert!(ai::online(&settings()), "a pause does not turn the assistant off");
    }
    // A body that is not an error of the gateway never reaches the person.
    let _fake = gateway(|_| json("500 Internal Server Error", "SECRET internal detail"));
    let got = ai::complete(&settings(), "sys", "hi").err().unwrap();
    assert!(!got.contains("SECRET"));
}

#[test]
fn quota_exceeded_is_a_pause_with_its_own_message_and_online_stays_true() {
    let _g = serial();
    let _fake = gateway(|_| {
        json(
            "429 Too Many Requests",
            &serde_json::json!({"error":{"code":"quota_exceeded","message":"Hai finito i token dell'assistente di questo mese: tornano il 1° dicembre.","retry_after_s":3600,
                "resource":"llm","limit":100,"used":120,"renews_at":"2026-12-01T00:00:00.000Z"}})
            .to_string(),
        )
    });
    let got = ai::complete(&settings(), "sys", "hi").err().unwrap();
    assert_eq!(got, ai::MSG_QUOTA);
    assert_ne!(got, ai::MSG_BUSY, "a used-up quota is not a busy server");
    assert!(ai::online(&settings()), "a pause does not turn the assistant off");
}

#[test]
fn a_409_stops_the_assistant_until_the_consent_changes() {
    let _g = serial();
    let message = "The assistant provider has changed: the consent must be given again.";
    let fake = gateway(move |_| error("409 Conflict", "llm_consent_required", message));
    assert_eq!(ai::complete(&settings(), "sys", "hi").err().unwrap(), message);
    assert_eq!(ai::complete(&settings(), "sys", "hi").err().unwrap(), message);
    assert_eq!(fake.chats().len(), 1, "the second call sends nothing");
    // A new consent (another provider label) lifts the stop.
    connect_as(&fake.url, "Other");
    let _ = ai::complete(&settings(), "sys", "hi");
    assert_eq!(fake.chats().len(), 2);
}

#[test]
fn context_exceeded_is_reported_by_complete_and_retried_by_complete_fitted() {
    let _g = serial();
    let fake = gateway(|_| error("400 Bad Request", "llm_context_exceeded", "The conversation is too long for the assistant."));
    let long = "x".repeat(4000);
    let got = ai::complete(&settings(), "sys", &long).err().unwrap();
    assert!(got.contains("context length"), "{got}");
    assert_eq!(fake.chats().len(), 1);
    assert!(ai::complete_fitted(&settings(), "sys", &long).is_err());
    let chats = fake.chats();
    assert_eq!(chats.len(), 3, "one more try, with half the text");
    let sent: serde_json::Value = serde_json::from_str(&chats[2].body).unwrap();
    assert!(sent["messages"][1]["content"].as_str().unwrap().chars().count() <= 2100);
}

#[test]
fn a_long_prompt_is_cut_to_context_tokens_before_sending() {
    let _g = serial();
    let fake = Fake::start(Arc::new(|seen: &Seen| {
        if seen.line.starts_with("GET /v1/info") {
            info(r#"{"model":"m","models":["m"],"external":false,"label":"Server","context_tokens":100}"#)
        } else {
            answer("ok")
        }
    }));
    connect(&fake.url);
    let prompt = format!("Context:\n{}\n\nQuestion: the end", "y".repeat(10_000));
    assert_eq!(ai::complete_fitted(&settings(), "sys", &prompt).unwrap(), "ok");
    let sent: serde_json::Value = serde_json::from_str(&fake.chats()[0].body).unwrap();
    let content = sent["messages"][1]["content"].as_str().unwrap();
    assert!(content.chars().count() <= 400, "{}", content.len());
    assert!(content.ends_with("Question: the end"), "the end of the text is kept");
}

#[test]
fn the_chosen_model_is_sent_only_if_the_server_lists_it() {
    let _g = serial();
    let fake = gateway(|_| answer("ok"));
    let mut s = settings();
    s.ai_model = "gpt-4o-mini".into();
    ai::complete(&s, "sys", "hi").unwrap();
    s.ai_model = "llm-grande".into();
    ai::complete(&s, "sys", "hi").unwrap();
    let chats = fake.chats();
    assert!(serde_json::from_str::<serde_json::Value>(&chats[0].body).unwrap().get("model").is_none());
    assert_eq!(serde_json::from_str::<serde_json::Value>(&chats[1].body).unwrap()["model"], "llm-grande");
}

#[test]
fn the_functions_of_ai_use_the_server_without_a_key_of_their_own() {
    let _g = serial();
    let fake = gateway(|_| answer("{\"category\":\"newsletter\",\"labels\":[\"x\"]}"));
    let (category, labels) = ai::classify(&settings(), &Email::default(), &[]);
    assert_eq!(category.key(), "newsletter");
    assert_eq!(labels, ["x"]);
    assert_eq!(fake.chats().len(), 1);
}

#[test]
fn the_label_of_the_provider_travels_as_the_server_wrote_it_and_cannot_break_the_header() {
    let _g = serial();
    let fake = Fake::start(Arc::new(|seen: &Seen| if seen.line.starts_with("GET /v1/info") { info(INFO_PLAIN) } else { answer("ok") }));
    connect_as(&fake.url, "Società Ufficio; Roma");
    assert_eq!(ai::complete(&settings(), "sys", "hi").unwrap(), "ok");
    assert!(fake.chats()[0].headers.contains("x-qamille-llm-consent: false;Società Ufficio; Roma") || fake.chats()[0].headers.to_ascii_lowercase().contains("false;societ"));
    // A label with a line break is never sent: the request fails before leaving.
    connect_as(&fake.url, "Evil\r\nX-Injected: 1");
    assert!(ai::complete(&settings(), "sys", "hi").is_err());
    assert_eq!(fake.chats().len(), 1);
}
