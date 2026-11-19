//! L'assistente da un capo all'altro nel core: la chiave del provider, i campi, i contatori, i
//! sink aggiuntivi, gli errori del provider e le chiamate abbandonate.
mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use qamille_server::compute::{Usage, UsageSink};
use qamille_server::llm::{LlmConfig, user_hash};
use serde_json::json;
use support::fake_openai::{FakeOpenai, Scenario};
use support::llm_env::*;
use support::{ADMIN_KEY, TestServer, bearer, keys, loopback, remote, request, test_config, user_with_device};

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Usage)>>);

impl UsageSink for Recorder {
    fn record(&self, user_id: &str, usage: Usage) {
        self.0.lock().unwrap().push((user_id.to_owned(), usage));
    }
}

fn status(status: u16, retry_after: Option<u64>, body: &str) -> Scenario {
    Scenario::Status { status, retry_after, body: body.to_owned() }
}

async fn wait_for_tokens(ts: &TestServer, user_id: &str, at_least: u64) -> qamille_server::accounts::ComputeUsage {
    let mut usage = ts.accounts().usage(user_id).await.unwrap();
    for _ in 0..100 {
        if usage.llm_prompt_tokens + usage.llm_completion_tokens >= at_least {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        usage = ts.accounts().usage(user_id).await.unwrap();
    }
    usage
}

fn every_header(response: &support::TestResponse) -> String {
    response.headers.iter().map(|(n, v)| format!("{n}: {}", v.to_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn the_provider_key_goes_downstream_and_is_never_in_any_response() {
    let (fake, ts, key, _) = setup(llm_for).await;
    let mut seen_texts = Vec::new();
    let info = request(&ts.public, "GET", "/v1/info", None, remote([10, 0, 0, 9]), &[]).await;
    seen_texts.push((info.text.clone(), every_header(&info)));
    let models = request(&ts.public, "GET", "/v1/models", None, remote([10, 0, 0, 9]), &[("authorization", &bearer(&key))]).await;
    seen_texts.push((models.text.clone(), every_header(&models)));
    let admin = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    seen_texts.push((admin.text.clone(), every_header(&admin)));

    let good = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(good.status, 200);
    seen_texts.push((good.text.clone(), every_header(&good)));
    let no_consent = chat(&ts.public, &key, None, hello()).await;
    seen_texts.push((no_consent.text.clone(), every_header(&no_consent)));
    let bad = chat(&ts.public, &key, Some(CONSENT), json!({"messages": []})).await;
    seen_texts.push((bad.text.clone(), every_header(&bad)));
    // Un provider che ripete la chiave in un errore.
    fake.set(status(401, None, &format!(r#"{{"error":{{"message":"bad key {PROVIDER_KEY}"}}}}"#)));
    let refused = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    seen_texts.push((refused.text.clone(), every_header(&refused)));

    for (body, headers) in &seen_texts {
        assert!(!body.contains(PROVIDER_KEY), "{body}");
        assert!(!headers.contains(PROVIDER_KEY), "{headers}");
    }
    // E al provider è andata solo nell'intestazione Authorization.
    assert_eq!(fake.seen()[0].header("authorization"), Some(format!("Bearer {PROVIDER_KEY}").as_str()));
}

#[tokio::test]
async fn the_answer_has_no_foreign_fields() {
    let (fake, ts, key, _) = setup(llm_for).await;
    fake.set(Scenario::Extra);
    let response = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(keys(&response.body), ["choices", "created", "id", "model", "object", "usage"]);
    assert_eq!(keys(&response.body["choices"][0]), ["finish_reason", "index", "message"]);
    assert_eq!(keys(&response.body["choices"][0]["message"]), ["content", "role"]);
    assert_eq!(keys(&response.body["usage"]), ["completion_tokens", "prompt_tokens", "total_tokens"]);
    for foreign in ["fp_segreto", "system_fingerprint", "service_tier", "logprobs", "tool_calls"] {
        assert!(!response.text.contains(foreign), "{foreign}");
    }
}

#[tokio::test]
async fn a_model_outside_the_list_is_400_and_nothing_is_forwarded() {
    let (fake, ts, key, _) = setup(|url| LlmConfig { allowed_models: vec!["altro".to_owned()], ..llm_for(url) }).await;
    let refused = chat(&ts.public, &key, Some(CONSENT), json!({"model": "gpt-lontano", "messages": [{"role": "user", "content": "x"}]})).await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    assert_eq!(refused.code(), Some("invalid_request"));
    assert_eq!(fake.hits(), 0);
    let allowed = chat(&ts.public, &key, Some(CONSENT), json!({"model": "altro", "messages": [{"role": "user", "content": "x"}]})).await;
    assert_eq!(allowed.status, 200, "{}", allowed.text);
    assert_eq!(fake.seen()[0].body["model"], "altro");
}

#[tokio::test]
async fn stream_is_refused_before_forwarding() {
    let (fake, ts, key, _) = setup(llm_for).await;
    let refused = chat(&ts.public, &key, Some(CONSENT), json!({"stream": true, "messages": [{"role": "user", "content": "x"}]})).await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    assert_eq!(refused.code(), Some("invalid_request"));
    assert_eq!(fake.hits(), 0);
}

#[tokio::test]
async fn the_ceiling_and_the_hashed_user_reach_the_provider() {
    let (fake, ts, key, user_id) = setup(|url| LlmConfig { max_output_tokens: 50, ..llm_for(url) }).await;
    let body = json!({"max_tokens": 9999, "user": "mario@example.com", "messages": [{"role": "user", "content": "x"}]});
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), body).await.status, 200);
    let seen = &fake.seen()[0];
    assert_eq!(seen.body["max_tokens"], 50);
    assert_eq!(seen.body["user"], user_hash(&user_id));
    assert!(!seen.body.to_string().contains(&user_id));
    assert!(!seen.body.to_string().contains("mario@example.com"));
}

#[tokio::test]
async fn each_provider_failure_is_translated() {
    let (fake, ts, key, _) = setup(|url| LlmConfig { timeout_secs: 1, ..llm_for(url) }).await;

    fake.set(status(500, None, "{}"));
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (503, Some("llm_unavailable")));
    assert_eq!(r.headers["retry-after"], "30");

    fake.set(status(429, Some(7), "{}"));
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (429, Some("rate_limited")));
    assert_eq!(r.body["error"]["retry_after_s"], 7);

    fake.set(status(401, None, "{}"));
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (503, Some("llm_unavailable")));

    fake.set(status(400, None, r#"{"error":{"message":"altro"}}"#));
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (400, Some("invalid_request")));

    fake.set(Scenario::InvalidJson);
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (503, Some("llm_unavailable")));

    fake.set(Scenario::Delay { ms: 2500 });
    let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!((r.status.as_u16(), r.code()), (503, Some("llm_unavailable")));
}

#[tokio::test]
async fn a_provider_error_text_never_reaches_the_app() {
    let (fake, ts, key, _) = setup(llm_for).await;
    let body = format!(r#"{{"error":{{"message":"TESTO-DEL-PROVIDER {PROVIDER_KEY}","type":"invalid_api_key"}}}}"#);
    for code in [400, 401, 403, 404, 422, 429, 500, 502] {
        fake.set(status(code, None, &body));
        let r = chat(&ts.public, &key, Some(CONSENT), hello()).await;
        assert!(!r.text.contains("TESTO-DEL-PROVIDER"), "{code}: {}", r.text);
        assert!(!r.text.contains(PROVIDER_KEY), "{code}: {}", r.text);
        let headers = every_header(&r);
        assert!(!headers.contains("TESTO-DEL-PROVIDER") && !headers.contains(PROVIDER_KEY), "{code}");
    }
}

#[tokio::test]
async fn the_counters_get_the_requests_and_the_tokens() {
    let (_fake, ts, key, user_id) = setup(llm_for).await;
    for _ in 0..2 {
        assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    }
    let usage = wait_for_tokens(&ts, &user_id, 220).await;
    assert_eq!((usage.llm_prompt_tokens, usage.llm_completion_tokens), (120, 100));
    // Le richieste le registra la rotta, i token il gateway: l'ordine fra i due non conta.
    for _ in 0..100 {
        if ts.accounts().usage(&user_id).await.unwrap().requests >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ts.accounts().usage(&user_id).await.unwrap().requests, 2);

    let users = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(users.status, 200, "{}", users.text);
    assert_eq!(users.body["llm"], true);
    let mine = &users.body["users"][0]["usage"];
    assert_eq!((mine["requests"].as_u64(), mine["llm_prompt_tokens"].as_u64(), mine["llm_completion_tokens"].as_u64()), (Some(2), Some(120), Some(100)));
}

#[tokio::test]
async fn an_extra_sink_receives_the_llm_tokens() {
    let fake = FakeOpenai::start(ok()).await;
    let recorder = Arc::new(Recorder::default());
    let mut config = test_config();
    config.llm = llm_for(&fake.url);
    let ts = TestServer::new(config);
    let server = ts.server;
    let server = server.with_compute(Arc::new(Fake(Duration::ZERO)), Arc::new(Speech), 1).with_usage_sink(recorder.clone());
    let ts = TestServer { public: server.public_router(), admin: server.admin_router(), server, clock: ts.clock, dir: ts.dir };
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    assert_eq!(chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await.status, 200);

    let usage = wait_for_tokens(&ts, &login.user_id, 110).await;
    for _ in 0..100 {
        if ts.accounts().usage(&login.user_id).await.unwrap().requests >= 1 && recorder.0.lock().unwrap().len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let seen = recorder.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "un record per i token e uno per la richiesta");
    assert!(seen.iter().all(|(user, _)| *user == login.user_id));
    let requests: u64 = seen.iter().map(|(_, u)| u.requests).sum();
    let prompt: u64 = seen.iter().map(|(_, u)| u.llm_prompt_tokens).sum();
    let completion: u64 = seen.iter().map(|(_, u)| u.llm_completion_tokens).sum();
    let core = ts.accounts().usage(&login.user_id).await.unwrap();
    assert_eq!((requests, prompt, completion), (core.requests, core.llm_prompt_tokens, core.llm_completion_tokens));
    assert_eq!((requests, prompt, completion), (1, 60, 50));
    let _ = usage;
}

#[tokio::test]
async fn tokens_of_a_dropped_call_are_counted() {
    let (fake, ts, key, user_id) = setup(llm_for).await;
    fake.set(Scenario::Delay { ms: 600 });
    let router = ts.public.clone();
    let task = tokio::spawn(async move { chat(&router, &key, Some(CONSENT), hello()).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    task.abort();
    assert!(task.await.is_err_and(|e| e.is_cancelled()));
    // L'app se n'è andata: i token compaiono lo stesso, quando il provider ha risposto.
    let usage = wait_for_tokens(&ts, &user_id, 2).await;
    assert_eq!((usage.llm_prompt_tokens, usage.llm_completion_tokens), (1, 1));
}

#[tokio::test]
async fn a_disabled_device_is_401_and_an_active_one_of_another_user_has_its_own_cap() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(LlmConfig { max_tokens_per_day_per_user: 100, ..llm_for(&fake.url) });
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let anna = user_with_device(&ts, "anna", "un'altra password lunga").await;
    assert_eq!(chat(&ts.public, &mario.device_key, Some(CONSENT), hello()).await.status, 200);
    assert_eq!(chat(&ts.public, &mario.device_key, Some(CONSENT), hello()).await.status, 429);
    // Il tetto è di ogni utente: anna passa.
    assert_eq!(chat(&ts.public, &anna.device_key, Some(CONSENT), hello()).await.status, 200);
    assert_eq!(chat(&ts.public, &anna.device_key, Some(CONSENT), hello()).await.status, 429);
    // Un dispositivo disattivato non arriva nemmeno al tetto.
    ts.accounts().revoke_device_admin(&mario.user_id, &mario.device_id).await.unwrap();
    let revoked = chat(&ts.public, &mario.device_key, Some(CONSENT), hello()).await;
    assert_eq!((revoked.status.as_u16(), revoked.code()), (401, Some("unauthorized")));
    assert_eq!(fake.hits(), 2);
}
