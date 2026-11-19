//! `/admin/api/llm`: l'assistente scelto dal pannello (spento, API compatibile OpenAI, modello
//! locale), salvato nella cartella dei dati e applicato senza riavvio; la chiave del provider non
//! torna mai indietro.
mod support;

use std::time::Duration;

use serde_json::{Value, json};
use support::fake_openai::FakeOpenai;
use support::llm_env::*;
use support::{ADMIN_KEY, TestServer, bearer, loopback, request, user_with_device};

const PANEL_KEY: &str = "sk-dal-pannello-abcdefgh9876";

async fn admin(ts: &TestServer, method: &str, body: Option<Value>) -> support::TestResponse {
    let key = bearer(ADMIN_KEY);
    request(ts.admin(), method, "/admin/api/llm", body, loopback(), &[("authorization", key.as_str())]).await
}

fn openai_settings(url: &str, key: Option<&str>) -> Value {
    let mut body = json!({
        "provider": "openai-compatible",
        "base_url": url,
        "model": "fake-model",
        "allowed_models": ["altro-modello"],
        "max_tokens_per_day_per_user": 1000,
    });
    if let Some(key) = key {
        body["api_key"] = json!(key);
    }
    body
}

fn last_authorization(fake: &FakeOpenai) -> String {
    fake.seen().last().and_then(|seen| seen.header("authorization").map(str::to_owned)).unwrap_or_default()
}

#[tokio::test]
async fn the_panel_needs_the_admin_key() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(llm_for(&fake.url));
    let none = request(ts.admin(), "GET", "/admin/api/llm", None, loopback(), &[]).await;
    assert_eq!(none.status, 401, "{}", none.text);
    let wrong = request(ts.admin(), "PUT", "/admin/api/llm", Some(openai_settings(&fake.url, Some(PANEL_KEY))), loopback(), &[("authorization", &bearer("chiave-sbagliata-ma-lunga-abbastanza-0000"))]).await;
    assert_eq!(wrong.status, 401, "{}", wrong.text);
    assert!(!ts.dir.path().join("llm-settings.json").exists());
}

#[tokio::test]
async fn the_panel_shows_where_the_key_comes_from_never_the_key() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(llm_for(&fake.url));
    let view = admin(&ts, "GET", None).await;
    assert_eq!(view.status, 200, "{}", view.text);
    assert_eq!(view.body["source"], "config");
    assert_eq!(view.body["provider"], "openai-compatible");
    assert_eq!(view.body["model"], "fake-model");
    assert_eq!(view.body["api_key"]["from"], "config");
    assert_eq!(view.body["api_key"]["hint"], "…1234");
    assert_eq!(view.body["active"]["model"], "fake-model");
    assert_eq!(view.body["restart_required"], false);
    assert!(!view.text.contains(PROVIDER_KEY), "{}", view.text);
}

#[tokio::test]
async fn a_new_key_applies_without_a_restart() {
    let (fake, ts, device_key, _) = setup(llm_for).await;
    let before = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(before.status, 200, "{}", before.text);
    assert_eq!(last_authorization(&fake), bearer(PROVIDER_KEY));

    let saved = admin(&ts, "PUT", Some(openai_settings(&fake.url, Some(PANEL_KEY)))).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    assert_eq!(saved.body["source"], "admin");
    assert_eq!(saved.body["api_key"]["from"], "admin");
    assert_eq!(saved.body["api_key"]["hint"], "…9876");
    assert!(!saved.text.contains(PANEL_KEY), "{}", saved.text);

    // Lo stesso servizio: il consenso dato resta valido, la chiave nuova parte subito.
    let after = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(after.status, 200, "{}", after.text);
    assert_eq!(last_authorization(&fake), bearer(PANEL_KEY));
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert_eq!(info.body["llm"]["models"], json!(["fake-model", "altro-modello"]));

    // Un salvataggio senza chiave tiene quella del pannello.
    let again = admin(&ts, "PUT", Some(openai_settings(&fake.url, None))).await;
    assert_eq!(again.body["api_key"]["from"], "admin", "{}", again.text);
    let stored = std::fs::read_to_string(ts.dir.path().join("llm-settings.json")).unwrap();
    assert!(stored.contains(PANEL_KEY));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(ts.dir.path().join("llm-settings.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[tokio::test]
async fn removing_the_panel_key_brings_back_the_configured_one() {
    let (fake, ts, device_key, _) = setup(llm_for).await;
    admin(&ts, "PUT", Some(openai_settings(&fake.url, Some(PANEL_KEY)))).await;
    let mut body = openai_settings(&fake.url, None);
    body["remove_api_key"] = json!(true);
    let removed = admin(&ts, "PUT", Some(body)).await;
    assert_eq!(removed.status, 200, "{}", removed.text);
    assert_eq!(removed.body["api_key"]["from"], "config");
    let response = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(last_authorization(&fake), bearer(PROVIDER_KEY));
}

#[tokio::test]
async fn the_assistant_turns_off_and_back_on() {
    let (_fake, ts, device_key, _) = setup(llm_for).await;
    let off = admin(&ts, "PUT", Some(json!({"provider": "none"}))).await;
    assert_eq!(off.status, 200, "{}", off.text);
    assert!(off.body["active"].is_null());
    let chat_off = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!((chat_off.status.as_u16(), chat_off.code()), (404, Some("not_found")));
    let info = request(&ts.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert!(info.body["llm"].is_null(), "{}", info.text);

    // Tolte le impostazioni del pannello, vale di nuovo la configurazione.
    let reset = admin(&ts, "DELETE", None).await;
    assert_eq!(reset.status, 200, "{}", reset.text);
    assert_eq!(reset.body["source"], "config");
    assert!(!ts.dir.path().join("llm-settings.json").exists());
    let chat_on = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(chat_on.status, 200, "{}", chat_on.text);
}

#[tokio::test]
async fn a_server_without_a_provider_turns_one_on_from_the_panel() {
    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with(None, None, Duration::ZERO);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let before = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!(before.status, 404, "{}", before.text);
    let view = admin(&ts, "GET", None).await;
    assert_eq!(view.body["provider"], "none");
    assert!(view.body["api_key"].is_null());

    let saved = admin(&ts, "PUT", Some(openai_settings(&fake.url, Some(PANEL_KEY)))).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    let after = chat(&ts.public, &login.device_key, Some(CONSENT), hello()).await;
    assert_eq!(after.status, 200, "{}", after.text);
    assert_eq!(last_authorization(&fake), bearer(PANEL_KEY));
}

#[tokio::test]
async fn an_invalid_setting_changes_nothing() {
    let (fake, ts, device_key, _) = setup(llm_for).await;
    // Un servizio esterno in chiaro non è ammesso, come nella configurazione.
    let refused = admin(&ts, "PUT", Some(openai_settings("http://esempio.example/v1", Some(PANEL_KEY)))).await;
    assert_eq!((refused.status.as_u16(), refused.code()), (400, Some("invalid_request")), "{}", refused.text);
    assert!(!refused.text.contains(PANEL_KEY));
    let unknown = admin(&ts, "PUT", Some(json!({"provider": "altro"}))).await;
    assert_eq!(unknown.status, 400, "{}", unknown.text);
    let local = admin(&ts, "PUT", Some(json!({"provider": "builtin", "builtin_model": "non-esiste"}))).await;
    assert_eq!(local.status, 400, "{}", local.text);
    assert!(!ts.dir.path().join("llm-settings.json").exists());
    let still = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(still.status, 200, "{}", still.text);
    assert_eq!(last_authorization(&fake), bearer(PROVIDER_KEY));
}

#[tokio::test]
async fn a_local_model_the_server_did_not_start_with_waits_for_a_restart() {
    let (_fake, ts, device_key, _) = setup(llm_for).await;
    let saved = admin(&ts, "PUT", Some(json!({"provider": "builtin", "builtin_model": "llm-medio"}))).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    assert_eq!(saved.body["restart_required"], true);
    assert_eq!(saved.body["provider"], "builtin");
    // Fino al riavvio risponde ancora il provider di prima.
    assert_eq!(saved.body["active"]["provider"], "openai-compatible");
    let still = chat(&ts.public, &device_key, Some(CONSENT), hello()).await;
    assert_eq!(still.status, 200, "{}", still.text);
}

#[tokio::test]
async fn the_saved_settings_survive_a_restart() {
    let (fake, ts, _, _) = setup(llm_for).await;
    let mut body = openai_settings(&fake.url, Some(PANEL_KEY));
    body["model"] = json!("altro-modello");
    body["allowed_models"] = json!([]);
    admin(&ts, "PUT", Some(body)).await;
    // Un secondo server sulla stessa cartella (un riavvio).
    let restarted = rebuild(mount(ts_server(&ts), None, Duration::ZERO), ts);
    let info = request(&restarted.public, "GET", "/v1/info", None, loopback(), &[]).await;
    assert_eq!(info.body["llm"]["model"], "altro-modello", "{}", info.text);
    let view = admin(&restarted, "GET", None).await;
    assert_eq!(view.body["source"], "admin");
    assert_eq!(view.body["api_key"]["from"], "admin");
}
