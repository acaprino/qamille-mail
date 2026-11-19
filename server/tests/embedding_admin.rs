//! `/admin/api/embedding`: il modello di embedding scelto dal pannello (quello locale o un servizio
//! compatibile con OpenAI su un indirizzo interno). Ciò che protegge gli indici delle app: i
//! vettori hanno sempre 256 valori, il modello in uso è quello che `/health` dichiara, le immagini
//! non si cercano con un modello remoto; e il testo della posta non esce verso un servizio esterno.
mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use qamille_server::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus};
use serde_json::{Value, json};
use support::llm_env::{Speech, rebuild, ts_server};
use support::{ADMIN_KEY, TestResponse, TestServer, bearer, loopback, request, request_raw, test_config, user_with_device};

const SERVICE_KEY: &str = "sk-embedding-del-servizio-9876";
const REMOTE_MODEL: &str = "test-embed";

/// Il calcolo locale: conta i vettori che calcola, e il suo modello può mancare.
struct Local {
    embedded: AtomicUsize,
    installed: AtomicBool,
}

impl Local {
    fn new(installed: bool) -> Arc<Local> {
        Arc::new(Local { embedded: AtomicUsize::new(0), installed: AtomicBool::new(installed) })
    }
}

impl ComputeBackend for Local {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        Ok(vec![text.to_owned()])
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        self.embedded.fetch_add(inputs.len(), Ordering::SeqCst);
        Ok(inputs.iter().map(|_| vec![0.0625; DIM]).collect())
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Ok(vec![0.0625; DIM])
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Ok(String::new())
    }
    fn models(&self) -> Vec<ModelStatus> {
        let installed = self.installed.load(Ordering::SeqCst);
        vec![ModelStatus {
            id: MODEL_ID.to_owned(),
            kind: "embedding".to_owned(),
            state: if installed { "ready" } else { "missing" }.to_owned(),
            percent: None,
            done_bytes: 0,
            total_bytes: 0,
            error: None,
            loaded: false,
            dimensions: DIM,
            quantization: "q4f16".to_owned(),
        }]
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
}

/// Un finto servizio di embedding: vettori di `length` valori, non normalizzati, e ciò che ha ricevuto.
struct Upstream {
    length: AtomicUsize,
    /// Un altro modello dietro lo stesso nome: vettori ortogonali a quelli di prima.
    other_vectors: AtomicBool,
    /// Senza `usage` nella risposta.
    silent_usage: AtomicBool,
    /// Rifiuta con `400` gli ingressi più lunghi di così (0: nessun limite).
    max_input: AtomicUsize,
    seen: Mutex<Vec<(Option<String>, Value)>>,
}

async fn upstream_embeddings(State(upstream): State<Arc<Upstream>>, headers: HeaderMap, body: Bytes) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let count = body["input"].as_array().map_or(1, Vec::len);
    let longest = body["input"].as_array().map_or(0, |inputs| inputs.iter().filter_map(Value::as_str).map(str::len).max().unwrap_or(0));
    upstream.seen.lock().unwrap().push((authorization, body));
    let max_input = upstream.max_input.load(Ordering::SeqCst);
    if max_input > 0 && longest > max_input {
        return (StatusCode::BAD_REQUEST, "input too long for this model").into_response();
    }
    let length = upstream.length.load(Ordering::SeqCst);
    let other = upstream.other_vectors.load(Ordering::SeqCst);
    let data: Vec<Value> = (0..count)
        .map(|index| {
            let mut vector: Vec<f32> = (0..length).map(|i| if other && i % 2 == 1 { -2.0 } else { 2.0 }).collect();
            if !other {
                vector[0] = 2.0 + index as f32;
            }
            json!({"object": "embedding", "index": index, "embedding": vector})
        })
        .collect();
    let mut answer = json!({"object": "list", "data": data});
    if !upstream.silent_usage.load(Ordering::SeqCst) {
        answer["usage"] = json!({"prompt_tokens": 11, "total_tokens": 11});
    }
    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], answer.to_string()).into_response()
}

/// Il servizio su `127.0.0.1` e il suo indirizzo di base (`http://127.0.0.1:PORTA/v1`).
async fn upstream(length: usize) -> (Arc<Upstream>, String) {
    let state = Arc::new(Upstream {
        length: AtomicUsize::new(length),
        other_vectors: AtomicBool::new(false),
        silent_usage: AtomicBool::new(false),
        max_input: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let router = Router::new().route("/v1/embeddings", post(upstream_embeddings)).with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("porta del finto servizio");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (state, format!("http://127.0.0.1:{port}/v1"))
}

/// Senza la pausa dopo un cambio di modello: queste prove chiedono vettori subito dopo un salvataggio.
fn server(local: Arc<Local>) -> TestServer {
    server_with_pause(local, 0)
}

fn server_with_pause(local: Arc<Local>, pause_secs: u64) -> TestServer {
    let mut config = test_config();
    config.embedding_switch_pause_secs = pause_secs;
    let ts = TestServer::new(config);
    let server = ts_server(&ts).with_compute(local, Arc::new(Speech), 1);
    rebuild(server, ts)
}

async fn admin(ts: &TestServer, method: &str, body: Option<Value>) -> TestResponse {
    let key = bearer(ADMIN_KEY);
    request(ts.admin(), method, "/admin/api/embedding", body, loopback(), &[("authorization", key.as_str())]).await
}

fn remote_settings(url: &str) -> Value {
    json!({"provider": "openai-compatible", "base_url": url, "model": REMOTE_MODEL, "api_key": SERVICE_KEY})
}

async fn health(ts: &TestServer, device_key: &str) -> TestResponse {
    request(&ts.public, "GET", "/health", None, loopback(), &[("authorization", &bearer(device_key))]).await
}

/// Come le app rilasciate: sempre con il nome del modello locale e 256 dimensioni.
async fn embed(ts: &TestServer, device_key: &str, inputs: &[&str]) -> TestResponse {
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "input": inputs, "dimensions": DIM})).unwrap();
    request_raw(&ts.public, "POST", "/v1/embeddings", Some(body), loopback(), &[("authorization", &bearer(device_key))]).await
}

async fn embed_image(ts: &TestServer, device_key: &str) -> TestResponse {
    request_raw(&ts.public, "POST", "/qamille/v1/embeddings/image", Some(b"un'immagine".to_vec()), loopback(), &[("authorization", &bearer(device_key))]).await
}

fn settings_file(ts: &TestServer) -> std::path::PathBuf {
    ts.dir.path().join("embedding-settings.json")
}

#[tokio::test]
async fn the_panel_needs_the_admin_key_and_starts_from_the_local_model() {
    let ts = server(Local::new(true));
    let none = request(ts.admin(), "GET", "/admin/api/embedding", None, loopback(), &[]).await;
    assert_eq!(none.status, 401, "{}", none.text);
    let view = admin(&ts, "GET", None).await;
    assert_eq!(view.status, 200, "{}", view.text);
    assert_eq!(view.body, json!({
        "provider": "builtin",
        "base_url": null,
        "model": null,
        "api_key": null,
        "dimensions": 256,
        "active": {"model": MODEL_ID, "dimensions": 256},
        "builtin": {"model": MODEL_ID, "state": "ready"},
        "last_error": null,
    }));
    // Non esiste sull'ascolto pubblico.
    let public = request(&ts.public, "GET", "/admin/api/embedding", None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
    assert_eq!(public.status, 404, "{}", public.text);
}

#[tokio::test]
async fn a_model_with_another_vector_length_is_refused_and_nothing_changes() {
    let ts = server(Local::new(true));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (service, url) = upstream(768).await;
    let refused = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    let message = refused.body["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("768") && message.contains("256"), "{message}");
    assert!(!refused.text.contains(SERVICE_KEY));
    // La prova ha chiesto 256 dimensioni con un testo nostro.
    let seen = service.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1["dimensions"], 256);
    assert_eq!(seen[0].1["model"], REMOTE_MODEL);
    // Nulla salvato, nulla cambiato.
    assert!(!settings_file(&ts).exists());
    assert_eq!(admin(&ts, "GET", None).await.body["provider"], "builtin");
    let health = health(&ts, &login.device_key).await;
    assert_eq!(health.body["embedding"]["model"], MODEL_ID, "{}", health.text);
}

#[tokio::test]
async fn an_external_address_is_refused() {
    let ts = server(Local::new(true));
    for address in ["https://api.openai.com/v1", "http://8.8.8.8:8080/v1"] {
        let mut body = remote_settings(address);
        body["model"] = json!("text-embedding-3-small");
        let refused = admin(&ts, "PUT", Some(body)).await;
        assert_eq!(refused.status, 400, "{address}: {}", refused.text);
        let message = refused.body["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("indirizzo interno") && message.contains("consenso"), "{message}");
        assert!(!settings_file(&ts).exists());
    }
}

#[tokio::test]
async fn the_local_model_must_be_installed_for_its_tokenizer() {
    let ts = server(Local::new(false));
    let (service, url) = upstream(DIM).await;
    let refused = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    assert!(refused.body["error"]["message"].as_str().unwrap_or_default().contains("«Modelli»"), "{}", refused.text);
    assert!(service.seen.lock().unwrap().is_empty(), "nessuna chiamata al servizio");
    assert!(!settings_file(&ts).exists());
}

#[tokio::test]
async fn a_good_model_is_saved_declared_used_and_removed() {
    let local = Local::new(true);
    let ts = server(Arc::clone(&local));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (service, url) = upstream(DIM).await;

    let saved = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    assert_eq!(saved.body["provider"], "openai-compatible");
    assert_eq!(saved.body["base_url"], url);
    assert_eq!(saved.body["model"], REMOTE_MODEL);
    assert_eq!(saved.body["api_key"], json!({"hint": "…9876"}));
    assert_eq!(saved.body["active"], json!({"model": REMOTE_MODEL, "dimensions": 256}));
    assert!(!saved.text.contains(SERVICE_KEY), "{}", saved.text);
    assert!(std::fs::read_to_string(settings_file(&ts)).unwrap().contains(SERVICE_KEY));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(settings_file(&ts)).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // `/health` dichiara il modello nuovo (l'app con un indice di un altro modello si ferma) e lo
    // elenca pronto accanto a quello locale.
    let declared = health(&ts, &login.device_key).await;
    assert_eq!(declared.body["embedding"], json!({"model": REMOTE_MODEL, "dimensions": 256, "quantization": ""}), "{}", declared.text);
    let models = declared.body["models"].as_array().unwrap();
    assert!(models.iter().any(|m| m["id"] == REMOTE_MODEL && m["state"] == "ready" && m["dimensions"] == 256), "{}", declared.text);
    assert!(models.iter().any(|m| m["id"] == MODEL_ID), "{}", declared.text);
    assert_eq!(declared.body["capabilities"]["image_embeddings"], false, "{}", declared.text);

    // I vettori vengono dal servizio, portati a norma 1, nell'ordine degli ingressi.
    let answer = embed(&ts, &login.device_key, &["uno", "due"]).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.body["model"], REMOTE_MODEL);
    assert_eq!(answer.body["usage"]["total_tokens"], 11);
    let data = answer.body["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    for item in data {
        let vector: Vec<f64> = item["embedding"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        assert_eq!(vector.len(), DIM);
        let norm = vector.iter().map(|v| v * v).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "{norm}");
    }
    assert!(data[1]["embedding"][0].as_f64().unwrap() > data[0]["embedding"][0].as_f64().unwrap());
    assert_eq!(local.embedded.load(Ordering::SeqCst), 0, "il modello locale non ha calcolato nulla");
    let last = service.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.0.as_deref(), Some(bearer(SERVICE_KEY).as_str()));
    assert_eq!(last.1["dimensions"], 256);
    assert_eq!(last.1["input"], json!(["uno", "due"]));

    // Un servizio che non dichiara i token non rende gratuita la ricerca: si stimano dai byte.
    service.silent_usage.store(true, Ordering::SeqCst);
    let estimated = embed(&ts, &login.device_key, &["uno", "due"]).await;
    assert_eq!(estimated.body["usage"]["total_tokens"], 2, "{}", estimated.text);
    service.silent_usage.store(false, Ordering::SeqCst);

    // Le immagini avrebbero vettori di un altro modello: rifiutate, solo quel contenuto.
    let image = embed_image(&ts, &login.device_key).await;
    assert_eq!((image.status.as_u16(), image.code()), (422, Some("document_unreadable")), "{}", image.text);

    // Un salvataggio senza chiave tiene quella salvata; la chiave non torna mai.
    let mut again = remote_settings(&url);
    again.as_object_mut().unwrap().remove("api_key");
    let kept = admin(&ts, "PUT", Some(again)).await;
    assert_eq!(kept.body["api_key"], json!({"hint": "…9876"}), "{}", kept.text);
    assert!(!admin(&ts, "GET", None).await.text.contains(SERVICE_KEY));

    // Un riavvio sulla stessa cartella riparte dal modello salvato.
    let restarted = ts_server(&ts).with_compute(Local::new(true), Arc::new(Speech), 1);
    let ts = rebuild(restarted, ts);
    let after = health(&ts, &login.device_key).await;
    assert_eq!(after.body["embedding"]["model"], REMOTE_MODEL, "{}", after.text);

    // Se il servizio cambia e dà vettori di un'altra lunghezza, nessuno raggiunge l'app: una pausa.
    service.length.store(768, Ordering::SeqCst);
    let paused = embed(&ts, &login.device_key, &["uno"]).await;
    assert_eq!((paused.status.as_u16(), paused.code()), (503, Some("paused_after_failure")), "{}", paused.text);
    assert!(paused.body["data"].is_null());
    // Il pannello lo vede: che cosa non ha funzionato e quando, mai la chiave.
    let failing = admin(&ts, "GET", None).await;
    let message = failing.body["last_error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("768") && message.contains("256"), "{}", failing.text);
    assert!(failing.body["last_error"]["at"].as_str().is_some_and(|at| at.ends_with('Z')), "{}", failing.text);
    assert!(!failing.text.contains(SERVICE_KEY));
    // E la prima risposta buona lo toglie.
    service.length.store(DIM, Ordering::SeqCst);
    assert_eq!(embed(&ts, &login.device_key, &["uno"]).await.status, 200);
    assert!(admin(&ts, "GET", None).await.body["last_error"].is_null());

    // Tolto dal pannello: torna il modello locale, con le immagini.
    let reset = admin(&ts, "DELETE", None).await;
    assert_eq!(reset.status, 200, "{}", reset.text);
    assert_eq!(reset.body["provider"], "builtin");
    assert_eq!(reset.body["active"]["model"], MODEL_ID);
    assert!(reset.body["api_key"].is_null());
    assert!(!settings_file(&ts).exists());
    let back = health(&ts, &login.device_key).await;
    assert_eq!(back.body["embedding"]["model"], MODEL_ID, "{}", back.text);
    assert_eq!(back.body["models"].as_array().unwrap().len(), 1, "{}", back.text);
    assert_eq!(back.body["capabilities"]["image_embeddings"], true, "{}", back.text);
    assert_eq!(embed(&ts, &login.device_key, &["uno"]).await.body["model"], MODEL_ID);
    assert_eq!(embed_image(&ts, &login.device_key).await.status, 200);
}

#[tokio::test]
async fn choosing_the_local_model_from_the_panel_is_the_reset() {
    let ts = server(Local::new(true));
    let (_service, url) = upstream(DIM).await;
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.status, 200);
    let builtin = admin(&ts, "PUT", Some(json!({"provider": "builtin"}))).await;
    assert_eq!(builtin.status, 200, "{}", builtin.text);
    assert_eq!(builtin.body["provider"], "builtin");
    assert!(!settings_file(&ts).exists());
    let unknown = admin(&ts, "PUT", Some(json!({"provider": "altro"}))).await;
    assert_eq!(unknown.status, 400, "{}", unknown.text);
}

#[tokio::test]
async fn after_a_switch_no_vectors_for_a_while() {
    // Un'app che ha letto `/health` prima del cambio e manda ora il lotto successivo del suo
    // messaggio non riceve vettori del modello nuovo: una pausa, e al nuovo tentativo vede il cambio.
    let ts = server_with_pause(Local::new(true), 60);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (service, url) = upstream(DIM).await;
    assert_eq!(embed(&ts, &login.device_key, &["prima"]).await.status, 200, "senza cambio, nessuna pausa");
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.status, 200);
    let calls = service.seen.lock().unwrap().len();

    assert_eq!(health(&ts, &login.device_key).await.body["embedding"]["model"], REMOTE_MODEL);
    let paused = embed(&ts, &login.device_key, &["dopo"]).await;
    assert_eq!((paused.status.as_u16(), paused.code()), (429, Some("rate_limited")), "{}", paused.text);
    let wait = paused.body["error"]["retry_after_s"].as_u64().unwrap_or(0);
    assert!((1..=60).contains(&wait), "{}", paused.text);
    assert_eq!(service.seen.lock().unwrap().len(), calls, "nessuna chiamata al servizio durante la pausa");
    // La spezzatura dei passaggi non produce vettori: continua.
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "title": "", "text": "un testo"})).unwrap();
    let chunks = request_raw(&ts.public, "POST", "/qamille/v1/chunks", Some(body), loopback(), &[("authorization", &bearer(&login.device_key))]).await;
    assert_eq!(chunks.status, 200, "{}", chunks.text);

    // Anche tornando al modello locale: i vettori di un messaggio a metà non si mescolano.
    assert_eq!(admin(&ts, "DELETE", None).await.status, 200);
    assert_eq!(embed(&ts, &login.device_key, &["di nuovo"]).await.code(), Some("rate_limited"));
    assert_eq!(embed_image(&ts, &login.device_key).await.code(), Some("rate_limited"));
}

#[tokio::test]
async fn the_same_name_with_other_vectors_is_another_model() {
    let ts = server(Local::new(true));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (service, url) = upstream(DIM).await;
    let first = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(first.body["active"]["model"], REMOTE_MODEL, "{}", first.text);
    // Gli stessi vettori (un'altra chiave, un riavvio del servizio): lo stesso modello.
    let same = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(same.body["active"]["model"], REMOTE_MODEL, "{}", same.text);

    // Dietro lo stesso nome ora c'è un altro modello: per le app cambia nome, e reindicizzano.
    service.other_vectors.store(true, Ordering::SeqCst);
    let changed = admin(&ts, "PUT", Some(remote_settings(&url))).await;
    assert_eq!(changed.status, 200, "{}", changed.text);
    let declared = format!("{REMOTE_MODEL}#1");
    assert_eq!(changed.body["active"]["model"], declared.as_str(), "{}", changed.text);
    assert_eq!(changed.body["model"], REMOTE_MODEL, "il nome salvato resta quello scritto");
    let health_now = health(&ts, &login.device_key).await;
    assert_eq!(health_now.body["embedding"]["model"], declared.as_str(), "{}", health_now.text);
    assert!(health_now.body["models"].as_array().unwrap().iter().any(|m| m["id"] == declared.as_str() && m["state"] == "ready"), "{}", health_now.text);
    let answer = embed(&ts, &login.device_key, &["uno"]).await;
    assert_eq!(answer.body["model"], declared.as_str(), "{}", answer.text);
    // Al servizio va sempre il nome scritto dall'amministratore.
    assert_eq!(service.seen.lock().unwrap().last().unwrap().1["model"], REMOTE_MODEL);

    // Salvare ancora con gli stessi vettori non cambia più nulla, nemmeno dopo un riavvio.
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.body["active"]["model"], declared.as_str());
    let restarted = ts_server(&ts).with_compute(Local::new(true), Arc::new(Speech), 1);
    let ts = rebuild(restarted, ts);
    assert_eq!(health(&ts, &login.device_key).await.body["embedding"]["model"], declared.as_str());
    // Il vettore di prova sta nel file, non nelle risposte.
    assert!(admin(&ts, "GET", None).await.body.get("probe").is_none());
}

#[tokio::test]
async fn the_saved_key_does_not_follow_another_address() {
    let ts = server(Local::new(true));
    let (_first, url) = upstream(DIM).await;
    let (second, other_url) = upstream(DIM).await;
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.body["api_key"], json!({"hint": "…9876"}));
    // Un altro indirizzo, campo della chiave vuoto: la chiave di prima non ci va.
    let mut moved = remote_settings(&other_url);
    moved.as_object_mut().unwrap().remove("api_key");
    let saved = admin(&ts, "PUT", Some(moved)).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    assert!(saved.body["api_key"].is_null(), "{}", saved.text);
    let seen = second.seen.lock().unwrap().clone();
    assert!(!seen.is_empty() && seen.iter().all(|(authorization, _)| authorization.is_none()), "{seen:?}");
    assert!(!std::fs::read_to_string(settings_file(&ts)).unwrap().contains(SERVICE_KEY));
}

#[tokio::test]
async fn one_refused_input_is_set_aside_and_a_refusing_service_is_a_pause() {
    let ts = server(Local::new(true));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (service, url) = upstream(DIM).await;
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.status, 200);

    // Il servizio rifiuta un passaggio lungo ma risponde alla frase di prova: è quel contenuto.
    service.max_input.store(60, Ordering::SeqCst);
    let long = "x".repeat(200);
    let aside = embed(&ts, &login.device_key, &["breve", long.as_str()]).await;
    assert_eq!((aside.status.as_u16(), aside.code()), (413, Some("payload_too_large")), "{}", aside.text);
    assert!(admin(&ts, "GET", None).await.body["last_error"].is_null(), "il servizio funziona");
    assert_eq!(embed(&ts, &login.device_key, &["breve"]).await.status, 200);

    // Rifiuta anche la frase di prova: è il servizio, e i messaggi non perdono tentativi.
    service.max_input.store(3, Ordering::SeqCst);
    let paused = embed(&ts, &login.device_key, &["breve"]).await;
    assert_eq!((paused.status.as_u16(), paused.code()), (503, Some("paused_after_failure")), "{}", paused.text);
    assert!(admin(&ts, "GET", None).await.body["last_error"]["message"].as_str().is_some_and(|m| m.contains("400")));
}

#[tokio::test]
async fn a_remote_model_is_ready_only_while_the_local_tokenizer_is_there() {
    let local = Local::new(true);
    let ts = server(Arc::clone(&local));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let (_service, url) = upstream(DIM).await;
    assert_eq!(admin(&ts, "PUT", Some(remote_settings(&url))).await.status, 200);
    // Il modello locale sparisce (un volume dei modelli nuovo): i passaggi non si tagliano più,
    // e l'app deve leggere «manca un modello», non «pronto».
    local.installed.store(false, Ordering::SeqCst);
    let health = health(&ts, &login.device_key).await;
    let remote = health.body["models"].as_array().unwrap().iter().find(|m| m["id"] == REMOTE_MODEL).cloned().unwrap();
    assert_eq!(remote["state"], "missing", "{}", health.text);
    assert_eq!(admin(&ts, "GET", None).await.body["builtin"]["state"], "missing");
}

#[tokio::test]
async fn a_settings_file_with_an_external_address_is_ignored_at_startup() {
    let ts = server(Local::new(true));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    // Scritto a mano, o da una versione con altre regole: all'avvio vale lo stesso controllo del pannello.
    std::fs::write(settings_file(&ts), r#"{"provider":"openai-compatible","base_url":"https://api.openai.com/v1","model":"text-embedding-3-small","api_key":"sk-una-chiave-esterna-1234"}"#).unwrap();
    let restarted = ts_server(&ts).with_compute(Local::new(true), Arc::new(Speech), 1);
    let ts = rebuild(restarted, ts);
    let health = health(&ts, &login.device_key).await;
    assert_eq!(health.body["embedding"]["model"], MODEL_ID, "{}", health.text);
    assert_eq!(health.body["models"].as_array().unwrap().len(), 1, "{}", health.text);
    let view = admin(&ts, "GET", None).await;
    assert_eq!(view.body["provider"], "builtin", "{}", view.text);
    assert!(view.body["base_url"].is_null() && view.body["api_key"].is_null(), "{}", view.text);
    assert_eq!(embed(&ts, &login.device_key, &["uno"]).await.body["model"], MODEL_ID);
}
