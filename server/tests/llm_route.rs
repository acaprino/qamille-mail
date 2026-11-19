//! `POST /v1/chat/completions`: chiave del dispositivo, diritto d'uso, consenso, tetto
//! giornaliero, cancello suo, `/v1/info` e `/v1/models`.
mod support;

use std::sync::Arc;
use std::time::Duration;

use qamille_server::compute::MODEL_ID;
use qamille_server::llm::LlmConfig;
use serde_json::json;
use support::fake_openai::{FakeOpenai, Scenario};
use support::llm_env::*;
use support::{TestServer, bearer, keys, loopback, request, test_config, user_with_device};

#[tokio::test]
async fn the_route_exists_only_with_a_provider() {
    let ts = ts_with(None, None, Duration::ZERO);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let off = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!(off.status, 404, "{}", off.text);
    assert_eq!(off.code(), Some("not_found"));
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert!(info.body["llm"].is_null());
    let models = request(&ts.public, "GET", "/v1/models", None, loopback(), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(models.body["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn the_route_needs_a_device_key() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(llm_for(&fake.url));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let none = request(&ts.public, "POST", "/v1/chat/completions", Some(hello()), loopback(), &[("x-qamille-llm-consent", CONSENT)]).await;
    assert_eq!((none.status.as_u16(), none.code()), (401, Some("unauthorized")));
    let wrong = chat(&ts.public, "chiave-sbagliata", Some(CONSENT), hello()).await;
    assert_eq!((wrong.status.as_u16(), wrong.code()), (401, Some("unauthorized")));
    let good = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!(good.status, 200, "{}", good.text);
    // Un dispositivo disattivato è 401 alla richiesta dopo.
    ts.accounts().revoke_device_admin(&login.user_id, &login.device_id).await.unwrap();
    let revoked = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!((revoked.status.as_u16(), revoked.code()), (401, Some("unauthorized")));
    assert_eq!(fake.hits(), 1);
}

#[tokio::test]
async fn a_valid_request_is_answered_with_the_filtered_response() {
    let (fake, ts, key, _) = setup(llm_for).await;
    fake.set(Scenario::Extra);
    let response = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(response.headers["content-type"], "application/json");
    assert_eq!(response.body["object"], "chat.completion");
    assert_eq!(response.body["choices"][0]["message"]["content"], "ciao");
    assert_eq!(response.body["usage"]["total_tokens"], 5);
    assert!(!response.text.contains("fp_segreto"));
    assert_eq!(fake.hits(), 1);
}

#[tokio::test]
async fn info_declares_the_provider_without_the_key() {
    let (_fake, ts, _, _) = setup(|url| {
        let mut config = llm_for(url);
        config.allowed_models = vec!["altro".to_owned()];
        config.context_tokens = Some(8192);
        config
    })
    .await;
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert_eq!(info.status, 200);
    assert_eq!(keys(&info.body["llm"]), ["context_tokens", "external", "label", "model", "models"]);
    assert_eq!(info.body["llm"]["model"], "fake-model");
    assert_eq!(info.body["llm"]["models"], json!(["fake-model", "altro"]));
    assert_eq!(info.body["llm"]["external"], false);
    assert_eq!(info.body["llm"]["label"], "127.0.0.1");
    assert_eq!(info.body["llm"]["context_tokens"], 8192);
    assert!(!info.text.contains(PROVIDER_KEY));
    assert!(!info.text.contains("/v1"), "nemmeno l'indirizzo: {}", info.text);
}

#[tokio::test]
async fn models_lists_the_llm_models_beside_the_embedding_one() {
    let (_fake, ts, key, _) = setup(|url| LlmConfig { allowed_models: vec!["altro".to_owned()], ..llm_for(url) }).await;
    let models = request(&ts.public, "GET", "/v1/models", None, loopback(), &[("authorization", &bearer(&key))]).await;
    let data = models.body["data"].as_array().unwrap();
    let ids: Vec<&str> = data.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [MODEL_ID, "fake-model", "altro"]);
    for model in &data[1..] {
        assert_eq!(model["object"], "model");
        assert_eq!(model["owned_by"], "qamille");
        assert_eq!(model["state"], "ready");
    }
}

#[tokio::test]
async fn a_missing_or_different_consent_is_409_before_any_forwarding() {
    let (fake, ts, key, _) = setup(llm_for).await;
    for consent in [None, Some("true;OpenAI"), Some("false;altro"), Some("false;127.0.0.1 "), Some("")] {
        let response = chat(&ts.public, &key, consent, hello()).await;
        assert_eq!(response.status, 409, "{consent:?}: {}", response.text);
        assert_eq!(response.code(), Some("llm_consent_required"));
    }
    assert_eq!(fake.hits(), 0);
}

#[tokio::test]
async fn the_matching_consent_passes() {
    let (fake, ts, key, _) = setup(llm_for).await;
    assert_eq!(chat(&ts.public, &key, Some("false;127.0.0.1"), hello()).await.status, 200);
    let (_fake2, ts2, key2, _) = setup(|url| LlmConfig { external: Some(true), label: Some("OpenAI".to_owned()), ..llm_for(url) }).await;
    assert_eq!(chat(&ts2.public, &key2, Some("true;OpenAI"), hello()).await.status, 200);
    assert_eq!(chat(&ts2.public, &key2, Some("false;OpenAI"), hello()).await.status, 409);
    assert_eq!(fake.hits(), 1);
}

#[tokio::test]
async fn a_user_without_entitlement_gets_402_before_the_consent_check() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with(Some(llm_for(&fake.url)), Some(Arc::new(NoLlm)), Duration::ZERO);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    assert_eq!(embed(&ts.public, &login.device_key).await.status, 200);
    // Senza consenso e con quello giusto: sempre 402, prima di ogni altro controllo.
    for consent in [None, Some(CONSENT)] {
        let response = chat(&ts.public, &login.device_key, consent, hello()).await;
        assert_eq!(response.status, 402, "{}", response.text);
        assert_eq!(response.code(), Some("subscription_required"));
    }
    assert_eq!(fake.hits(), 0);
}

#[tokio::test]
async fn at_most_n_requests_in_flight_per_user_then_429_with_retry_after_2() {
    let (fake, ts, key, user_id) = setup(|url| LlmConfig { max_in_flight_per_user: 1, ..llm_for(url) }).await;
    fake.set(Scenario::Delay { ms: 700 });
    let router = ts.public.clone();
    let first_key = key.clone();
    let first = tokio::spawn(async move { chat(&router, &first_key, Some(CONSENT), hello()).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let busy = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(busy.status, 429, "{}", busy.text);
    assert_eq!(busy.code(), Some("rate_limited"));
    assert_eq!(busy.body["error"]["retry_after_s"], 2);
    assert_eq!(busy.headers["retry-after"], "2");
    assert_eq!(first.await.unwrap().status, 200);
    assert_eq!(fake.hits(), 1);
    // Il posto è tornato libero.
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    let _ = user_id;
}

#[tokio::test]
async fn at_most_n_requests_in_flight_in_total_then_429_with_retry_after_2() {
    let (fake, ts, key, _) = setup(|url| LlmConfig { max_in_flight_per_user: 2, max_in_flight_total: 1, ..llm_for(url) }).await;
    fake.set(Scenario::Delay { ms: 700 });
    let router = ts.public.clone();
    let first_key = key.clone();
    let first = tokio::spawn(async move { chat(&router, &first_key, Some(CONSENT), hello()).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let busy = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(busy.status, 429, "{}", busy.text);
    assert_eq!(busy.code(), Some("rate_limited"));
    assert_eq!(busy.body["error"]["retry_after_s"], 2);
    assert_eq!(busy.headers["retry-after"], "2");
    assert_eq!(first.await.unwrap().status, 200);
    assert_eq!(fake.hits(), 1);
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
}

#[tokio::test]
async fn the_llm_gate_is_separate_from_the_compute_gate() {
    let fake = FakeOpenai::start(ok()).await;
    let mut llm = llm_for(&fake.url);
    llm.max_in_flight_per_user = 1;
    let mut config = test_config();
    config.llm = llm;
    config.compute_max_in_flight_per_user = 2;
    let ts = TestServer::new(config);
    let server = mount(ts_server(&ts), None, Duration::from_millis(700));
    let ts = rebuild(server, ts);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let key = login.device_key;

    // Due embedding lenti occupano il calcolo dell'utente: una chat passa lo stesso.
    let spawned: Vec<_> = (0..2)
        .map(|_| {
            let (router, key) = (ts.public.clone(), key.clone());
            tokio::spawn(async move { embed(&router, &key).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(embed(&ts.public, &key).await.status, 429, "il calcolo è pieno");
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    for task in spawned {
        assert_eq!(task.await.unwrap().status, 200);
    }

    // E viceversa: una chat lenta occupa l'assistente, un embedding passa.
    fake.set(Scenario::Delay { ms: 700 });
    let (router, slow_key) = (ts.public.clone(), key.clone());
    let slow = tokio::spawn(async move { chat(&router, &slow_key, Some(CONSENT), hello()).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 429);
    assert_eq!(embed(&ts.public, &key).await.status, 200);
    assert_eq!(slow.await.unwrap().status, 200);
}

#[tokio::test]
async fn the_daily_cap_answers_429_before_forwarding_until_midnight() {
    let (fake, ts, key, _) = setup(|url| LlmConfig { max_tokens_per_day_per_user: 100, ..llm_for(url) }).await;
    // 110 token: la soglia è morbida, la prima richiesta passa.
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    let capped = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(capped.status, 429, "{}", capped.text);
    assert_eq!(capped.code(), Some("rate_limited"));
    // Le 09:00 del 13 novembre: 15 ore a mezzanotte.
    assert_eq!(capped.body["error"]["retry_after_s"], 15 * 3600);
    assert_eq!(fake.hits(), 1);
    ts.clock.advance(chrono::TimeDelta::hours(16));
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    assert_eq!(fake.hits(), 2);
}

#[tokio::test]
async fn the_cap_survives_a_restart() {
    let (fake, ts, key, user_id) = setup(|url| LlmConfig { max_tokens_per_day_per_user: 100, ..llm_for(url) }).await;
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
    // I token arrivano al database fuori dalla richiesta.
    let day = ts.accounts().now().format("%Y-%m-%d").to_string();
    for _ in 0..100 {
        if ts.accounts().llm_tokens_on(&user_id, &day).await.unwrap() >= 110 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let restarted = mount(ts_server(&ts), None, Duration::ZERO);
    let router = restarted.public_router();
    let capped = chat(&router, &key, Some(CONSENT), hello()).await;
    assert_eq!(capped.status, 429, "{}", capped.text);
    assert_eq!(fake.hits(), 1, "nessun inoltro dopo il riavvio");
}

#[tokio::test]
async fn a_body_over_the_limit_is_payload_too_large() {
    let (fake, ts, key, _) = setup(|url| LlmConfig { max_request_bytes: 1024, ..llm_for(url) }).await;
    let big = json!({"messages": [{"role": "user", "content": "x".repeat(4000)}]});
    let response = chat(&ts.public, &key, Some(CONSENT), big).await;
    assert_eq!(response.status, 413, "{}", response.text);
    assert_eq!(response.code(), Some("payload_too_large"));
    assert_eq!(fake.hits(), 0);
    // Il posto è libero e una richiesta piccola passa.
    assert_eq!(chat(&ts.public, &key, Some(CONSENT), hello()).await.status, 200);
}

#[tokio::test]
async fn a_context_error_from_the_provider_is_llm_context_exceeded() {
    let (fake, ts, key, _) = setup(llm_for).await;
    fake.set(Scenario::Status { status: 400, retry_after: None, body: r#"{"error":{"code":"context_length_exceeded","message":"TESTO-DEL-PROVIDER"}}"#.to_owned() });
    let response = chat(&ts.public, &key, Some(CONSENT), hello()).await;
    assert_eq!(response.status, 400, "{}", response.text);
    assert_eq!(response.code(), Some("llm_context_exceeded"));
    assert!(!response.text.contains("TESTO-DEL-PROVIDER"));
}

/// Nega l'assistente con un errore proprio (una quota finita), non con il `402`.
struct QuotaOut;

impl qamille_server::ext::Entitlement for QuotaOut {
    fn require<'a>(
        &'a self,
        _: &'a str,
        resource: qamille_server::ext::Resource,
    ) -> qamille_server::ext::BoxFut<'a, Result<(), qamille_server::ServerError>> {
        Box::pin(async move {
            match resource {
                qamille_server::ext::Resource::Llm => Err(qamille_server::ServerError::Extension {
                    status: axum::http::StatusCode::TOO_MANY_REQUESTS,
                    code: "quota_exceeded",
                    message: "La quota del mese è finita.".to_owned(),
                    retry_after_s: Some(3600),
                    extra: Some(json!({"resource": "llm", "limit": 5, "used": 5, "renews_at": "2026-12-01T00:00:00.000Z"})),
                }),
                _ => Ok(()),
            }
        })
    }
}

#[tokio::test]
async fn the_assistant_passes_the_entitlement_error_too() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with(Some(llm_for(&fake.url)), Some(Arc::new(QuotaOut)), Duration::ZERO);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let response = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!(response.status, 429, "{}", response.text);
    assert_eq!(response.code(), Some("quota_exceeded"));
    assert_eq!(response.body["error"]["resource"], "llm");
    assert_eq!(response.body["error"]["used"], 5);
    assert_eq!(fake.hits(), 0);
    // The embeddings are not touched by the assistant's quota.
    assert_eq!(embed(&ts.public, &login.device_key).await.status, 200);
}
