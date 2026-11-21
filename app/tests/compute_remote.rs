//! The remote mode of the compute gateway (wave 3 of the compute plan, access wave 2 Compito 2)
//! against a compute server of the test, in process: the factory and its consent, the device key
//! only towards the session's address, a `401` halfway through indexing (no attempt consumed, one
//! notice), the session changed after the `401`, another model than the index's (D4), the
//! lexical fallback of search (D8) and the MCP server (A11).
//!
//! Tests that index write under `QAMILLE_DATA`, which is process-wide: they run inside one test
//! function; the others never touch the data folder.
use serde_json::{Value, json};
use qamille::application::compute::{self, ComputeGateway, codes};
use qamille::application::events::AppEvent;
use qamille::application::indexing::MailIndexService;
use qamille::application::indexing_ports::IndexSource;
use qamille::application::ports::*;
use qamille::application::{AppResult, ServiceContext};
use qamille::domain::indexing::FolderProgress;
use qamille::domain::model::{Access, Account, AccountKind, ComputeMode, Email, Store};
use qamille::infrastructure::compute_factory::{ComputeFactory, Route};
use qamille::infrastructure::fake_compute::bag_of_words;
use qamille::infrastructure::gateways::LocalSearch;
use qamille::infrastructure::http_compute::HttpCompute;
use qamille::infrastructure::repository::{JsonStoreRepository, MemoryRepository};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

// --- A compute server of the test ------------------------------------------------------------

#[derive(Default)]
struct ServerState {
    keys: HashSet<String>,
    /// `(path, Authorization)` of every request, in order.
    log: Vec<(String, String)>,
    /// Embedding requests accepted before every key is refused (EX-15 halfway).
    embeddings_left: Option<usize>,
    model: String,
}

#[derive(Clone)]
struct ComputeServer {
    url: String,
    state: Arc<Mutex<ServerState>>,
}

impl ComputeServer {
    fn start(keys: &[&str]) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let state = Arc::new(Mutex::new(ServerState { keys: keys.iter().map(|k| k.to_string()).collect(), model: compute::EMBEDDING_MODEL_ID.into(), ..Default::default() }));
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let shared = shared.clone();
                std::thread::spawn(move || serve(stream, &shared));
            }
        });
        ComputeServer { url, state }
    }
    fn log(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().log.clone()
    }
    fn revoke_all(&self) {
        self.state.lock().unwrap().keys.clear();
    }
    fn accept(&self, key: &str) {
        self.state.lock().unwrap().keys.insert(key.into());
    }
}

fn serve(mut stream: std::net::TcpStream, state: &Mutex<ServerState>) {
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
    let (mut length, mut auth) = (0usize, String::new());
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            auth = line["authorization:".len()..].trim().to_owned();
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (status, answer) = {
        let mut state = state.lock().unwrap();
        state.log.push((path.clone(), auth.clone()));
        let known = auth.strip_prefix("Bearer ").is_some_and(|key| state.keys.contains(key));
        let refused = (401, json!({"error": {"code": "unauthorized", "message": "The compute server refused the credentials of this request.", "retry_after_s": null}}));
        match path.as_str() {
            "/health" if known => (200, json!({
                "status": "ok", "version": "test", "mode": "remote",
                "models": [{"id": state.model, "state": "ready", "percent": 100, "done_bytes": 1, "total_bytes": 1, "dimensions": 256, "quantization": "q4f16"},
                           {"id": "whisper-base", "state": "downloading", "percent": 40, "done_bytes": 4, "total_bytes": 10, "dimensions": 0, "quantization": "ggml"}],
                "embedding": {"model": state.model, "dimensions": 256, "quantization": "q4f16"},
            })),
            "/health" => (200, json!({"status": "ok", "version": "test"})),
            _ if !known => refused,
            "/qamille/v1/chunks" => {
                let text = body["text"].as_str().unwrap_or("");
                let chunks: Vec<&str> = text.split_inclusive(['.', '\n']).map(str::trim).filter(|t| !t.is_empty()).collect();
                (200, json!({"chunks": chunks}))
            }
            "/v1/embeddings" => {
                if let Some(left) = state.embeddings_left.as_mut() {
                    if *left == 0 {
                        state.keys.clear();
                        return reply(&mut stream, refused.0, &refused.1);
                    }
                    *left -= 1;
                }
                let inputs: Vec<String> = body["input"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                let data: Vec<Value> = inputs.iter().enumerate().map(|(i, text)| json!({"object": "embedding", "index": i, "embedding": bag_of_words(text)})).collect();
                (200, json!({"object": "list", "data": data}))
            }
            _ => (404, json!({"error": {"code": "not_found", "message": "Not found."}})),
        }
    };
    reply(&mut stream, status, &answer);
}

fn reply(stream: &mut std::net::TcpStream, status: u16, body: &Value) {
    let body = body.to_string();
    let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

// --- The app's side ----------------------------------------------------------------------------

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

#[derive(Default)]
struct Recorder(Mutex<Vec<AppEvent>>);
impl EventSink for Recorder {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}
impl Recorder {
    fn notices(&self, text: &str) -> usize {
        self.0.lock().unwrap().iter().filter(|e| matches!(e, AppEvent::Notice { text: t } if t == text)).count()
    }
}

const KEY: &str = "spk_dev_1111111111111111111111111111111111111111111111111111111111111111";
const NEW_KEY: &str = "spk_dev_2222222222222222222222222222222222222222222222222222222222222222";

fn session(url: &str, key: &str) -> String {
    json!({"url": url, "user_id": "u1", "username": "mario", "device_id": format!("d-{}", &key[8..12]), "device_key": key}).to_string()
}

fn remote_store(url: &str, consented: bool) -> Store {
    let mut store = Store::default();
    store.settings.compute_mode = ComputeMode::Server;
    if consented {
        store.settings.compute_consented_urls.push(url.into());
    }
    store
}

/// A factory whose sessions talk plain HTTP to the test server.
fn factory(repository: Arc<dyn StoreRepository>, vault: Arc<Vault>) -> Arc<ComputeFactory> {
    Arc::new(ComputeFactory::with(repository, vault).with_remote_builder(|source, _ca| Ok(Arc::new(HttpCompute::new(source)) as Arc<dyn ComputeGateway>)))
}

#[test]
fn without_consent_or_session_nothing_is_sent() {
    let server = ComputeServer::start(&[KEY]);
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&server.url, KEY)).unwrap();
    // A session, server mode, no consent for its address (INV-10).
    let repository = Arc::new(MemoryRepository::new(remote_store(&server.url, false)));
    let unconsented = factory(repository.clone(), vault.clone());
    let error = unconsented.current().embed_query("fattura").unwrap_err();
    assert_eq!(error.code, codes::NOT_CONNECTED);
    assert!(compute::is_transient(error.code), "a pause: nothing to retry until the person signs in");
    assert!(unconsented.current().health().is_err());
    assert!(server.log().is_empty(), "no request without consent: {:?}", server.log());
    // Server mode without a session.
    let empty = factory(Arc::new(MemoryRepository::new(remote_store(&server.url, true))), Arc::new(Vault::default()));
    assert_eq!(empty.current().chunk_text("a. b").unwrap_err().code, codes::NOT_CONNECTED);
    assert!(server.log().is_empty());
}

#[test]
fn with_mode_off_nothing_is_sent() {
    let server = ComputeServer::start(&[KEY]);
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&server.url, KEY)).unwrap();
    let mut store = remote_store(&server.url, true);
    store.settings.compute_mode = ComputeMode::Off;
    let factory = factory(Arc::new(MemoryRepository::new(store)), vault);
    assert_eq!(factory.route(), Route::Off);
    assert_eq!(factory.current().embed_query("x").unwrap_err().code, codes::NOT_CONNECTED);
    assert!(server.log().is_empty(), "INV-10: nothing leaves the process: {:?}", server.log());
    assert_eq!(factory.status().route, "off");
}

#[test]
fn mode_server_without_a_session_is_not_connected_and_sends_nothing() {
    let server = ComputeServer::start(&[KEY]);
    let factory = factory(Arc::new(MemoryRepository::new(remote_store(&server.url, true))), Arc::new(Vault::default()));
    assert_eq!(factory.route(), Route::NotConnected);
    assert_eq!(factory.current().embed_query("x").unwrap_err().code, codes::NOT_CONNECTED);
    assert!(server.log().is_empty());
    assert_eq!(factory.status().route, "not_connected");
}

#[test]
fn the_device_key_goes_only_to_the_session_address_and_consent_is_checked_at_every_request() {
    let server = ComputeServer::start(&[KEY]);
    let other = ComputeServer::start(&[KEY]);
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&server.url, KEY)).unwrap();
    let mut store = remote_store(&server.url, true);
    store.settings.compute_consented_urls.push(other.url.clone());
    let repository = Arc::new(MemoryRepository::new(store));
    let factory = factory(repository.clone(), vault);
    let gateway = factory.routed();
    assert!(gateway.is_remote());
    assert_eq!(gateway.identity(), server.url);
    assert_eq!(gateway.embed_query("fattura pagata").unwrap().len(), 256);
    assert!(gateway.health().unwrap().embedding_ready());
    for (path, auth) in server.log() {
        assert_eq!(auth, format!("Bearer {KEY}"), "{path}");
        assert!(!path.contains("/v1/account") && !path.contains("/v1/devices"), "compute routes only: {path}");
    }
    assert!(other.log().is_empty(), "the key never goes to another address");
    // Consent withdrawn while a job holds its gateway: the next request is not sent.
    let pinned = compute::pinned(&gateway);
    repository.transaction(&mut |store| {
        store.settings.compute_consented_urls.retain(|url| *url != server.url);
        Ok(())
    }).unwrap();
    let before = server.log().len();
    assert_eq!(pinned.embed_query("x").unwrap_err().code, codes::NOT_CONNECTED);
    assert_eq!(server.log().len(), before);
}

#[test]
fn a_401_with_a_session_changed_meanwhile_is_tried_once_with_the_new_one() {
    let server = ComputeServer::start(&[KEY]);
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&server.url, KEY)).unwrap();
    let repository = Arc::new(MemoryRepository::new(remote_store(&server.url, true)));
    let events = Arc::new(Recorder::default());
    let factory = factory(repository, vault.clone());
    factory.set_notices(events.clone());
    let gateway = factory.current();
    gateway.embed_query("prima").unwrap();
    // The window signed in again («Accedi di nuovo»): new key in the vault, old one revoked.
    server.revoke_all();
    server.accept(NEW_KEY);
    vault.set("server-session", &session(&server.url, NEW_KEY)).unwrap();
    gateway.embed_query("dopo").unwrap();
    let log = server.log();
    let last: Vec<_> = log.iter().rev().take(2).rev().map(|(_, auth)| auth.as_str()).collect();
    assert_eq!(last, [format!("Bearer {KEY}"), format!("Bearer {NEW_KEY}")]);
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 0, "a replaced session is no unlinked device");
    assert!(!factory.unlinked());
    // Revoked with no new session: refused once, said once, the session stays.
    server.revoke_all();
    for _ in 0..3 {
        let error = factory.current().embed_query("x").unwrap_err();
        assert_eq!(error.code, codes::UNAUTHORIZED);
        assert_eq!(error.message, compute::UNLINKED_MESSAGE);
        assert!(!compute::is_transient(error.code));
    }
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1);
    assert!(factory.unlinked());
    assert!(!vault.get("server-session").unwrap().is_empty(), "no process removes the session by itself");
    assert!(factory.status().unlinked);
}

// --- Indexing, search, D4 and MCP: one function, `QAMILLE_DATA` is process-wide ------------

struct NoSource;
impl IndexSource for NoSource {
    fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
        Ok(vec![])
    }
}

struct TestClock(AtomicI64);
impl qamille::application::ports::Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct NoMail;
impl MailGateway for NoMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}

struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &qamille::domain::model::Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

fn message(id: u64, subject: &str, body: &str) -> Email {
    Email { id, account_id: 1, folder: "Inbox".into(), subject: subject.into(), from_name: "Maria".into(), from_addr: "maria@example.com".into(), body: body.into(), remote_content_hash: format!("hash-{id}"), date: 1_700_000_000 + id as i64, ..Default::default() }
}

fn attempts(store: &Store) -> Vec<u32> {
    store.emails.iter().map(|e| e.search_index.attempts).collect()
}

#[test]
fn indexing_search_model_identity_and_mcp_with_a_remote_server() {
    let root = std::env::temp_dir().join(format!("qamille-compute-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    unsafe { std::env::set_var("QAMILLE_DATA", &root) };
    let server = ComputeServer::start(&[KEY]);
    let mut store = remote_store(&server.url, true);
    store.settings.mcp_access = Access::ReadOnly;
    let mut account = Account { id: 1, email: "sample@example.com".into(), kind: AccountKind::Sample, ..Default::default() };
    account.search_index.enabled = true;
    store.accounts.push(account);
    store.emails = vec![
        message(1, "Fattura", "La fattura di ottobre è stata pagata con bonifico."),
        message(2, "Treno", "Il treno regionale per Cefalù parte alle nove."),
        message(3, "Riunione", "La riunione con il fornitore è spostata a giovedì."),
        message(4, "Sedie", "Le sedie nuove arrivano lunedì mattina."),
    ];
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();
    let repository: Arc<dyn StoreRepository> = Arc::new(JsonStoreRepository::new(Store::path()));
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&server.url, KEY)).unwrap();
    let events = Arc::new(Recorder::default());
    let factory = factory(repository.clone(), vault.clone());
    factory.set_notices(events.clone());
    let clock = Arc::new(TestClock(AtomicI64::new(1_800_000_000)));
    let context = Arc::new(ServiceContext {
        events: events.clone(),
        repository: repository.clone(),
        mail: Arc::new(NoMail),
        secrets: vault.clone(),
        clock: clock.clone(),
        attachments: Arc::new(NoFiles),
        compute: factory.routed(),
    });
    let search = Arc::new(LocalSearch::with_notices(context.compute.clone(), factory.notices()));
    let indexer = MailIndexService::new(context.clone(), Arc::new(NoSource), search.clone()).with_batch_size(4);

    // EX-15 halfway: two messages are embedded, then the administrator revokes the device.
    server.state.lock().unwrap().embeddings_left = Some(2);
    assert!(indexer.step().unwrap());
    let after = repository.read().unwrap();
    let indexed: Vec<_> = after.emails.iter().filter(|e| e.search_index.version == qamille::domain::indexing::VERSION).map(|e| e.id).collect();
    assert_eq!(indexed, [3, 4], "the newest messages, embedded before the 401, keep their vectors");
    assert_eq!(attempts(&after), [0, 0, 0, 0], "no message loses an attempt");
    assert_eq!(after.accounts[0].search_index.state, "unlinked");
    assert_eq!(after.accounts[0].search_index.notice.as_deref(), Some(compute::UNLINKED_MESSAGE));
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1);
    // A minute later the next step asks again: refused, still said once, still no attempt.
    clock.0.fetch_add(61, Ordering::SeqCst);
    assert!(indexer.step().unwrap());
    let after = repository.read().unwrap();
    assert_eq!(attempts(&after), [0, 0, 0, 0]);
    assert_eq!(after.accounts[0].search_index.state, "unlinked");
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1);
    // Within the minute nothing is asked at all.
    let before = server.log().len();
    assert!(!indexer.step().unwrap());
    assert_eq!(server.log().len(), before);

    // Search: words only, with the notice of D8 once.
    let hits = search.search(&after, "Cefalù", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(2));
    search.search(&after, "fornitore", 10).unwrap();
    assert_eq!(events.notices(compute::SEMANTIC_UNAVAILABLE_MESSAGE), 1);

    // The window signs in again: the new session is read at the next step and indexing resumes.
    server.accept(NEW_KEY);
    vault.set("server-session", &session(&server.url, NEW_KEY)).unwrap();
    server.state.lock().unwrap().embeddings_left = None;
    clock.0.fetch_add(61, Ordering::SeqCst);
    while indexer.step().unwrap() {}
    let after = repository.read().unwrap();
    assert!(after.emails.iter().all(|e| e.search_index.version == qamille::domain::indexing::VERSION && !e.search_index.semantic_pending), "{:?}", after.emails.iter().map(|e| (e.id, e.search_index.version, e.search_index.semantic_pending)).collect::<Vec<_>>());
    assert_eq!(attempts(&after), [0, 0, 0, 0]);
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1);
    let hits = search.search(&after, "pagamento ottobre bonifico", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(1));
    // Old passage files (before D4) and new ones: the local model, recorded.
    let identity = qamille::infrastructure::rag_segments::index_identity(&root).unwrap();
    assert_eq!(identity, compute::EmbeddingIdentity::local());

    // D4: the server now serves another model. Indexing stops with one notice; search falls back.
    server.state.lock().unwrap().model = "other-model".into();
    let mut fifth = message(5, "Preventivo", "Il preventivo del falegname arriva domani.");
    fifth.search_index = Default::default();
    repository.transaction(&mut |store| {
        store.emails.push(fifth.clone());
        Ok(())
    }).unwrap();
    clock.0.fetch_add(61, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(1100)); // the health cache (1 s)
    assert!(indexer.step().unwrap());
    let after = repository.read().unwrap();
    assert_eq!(after.accounts[0].search_index.state, "model_mismatch");
    assert_eq!(after.emails[4].search_index.version, 0, "nothing is indexed with another model");
    assert_eq!(events.notices(compute::MODEL_MISMATCH_MESSAGE), 1);
    let status = factory.status();
    assert_eq!(status.model_mismatch.as_ref().map(|m| m.server.model.as_str()), Some("other-model"));
    assert_eq!(status.models.iter().find(|m| m.id == "whisper-base").map(|m| m.percent), Some(Some(40)));
    let before = server.log().iter().filter(|(path, _)| path == "/v1/embeddings").count();
    let hits = search.search(&after, "Cefalù", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(2), "words only");
    assert_eq!(server.log().iter().filter(|(path, _)| path == "/v1/embeddings").count(), before, "no query vector of another model");
    clock.0.fetch_add(61, Ordering::SeqCst);
    assert!(indexer.step().unwrap());
    assert_eq!(events.notices(compute::MODEL_MISMATCH_MESSAGE), 1, "said once");

    // «Reindex»: the vectors go, every message is embedded again with the server's model.
    indexer.reindex_vectors().unwrap();
    assert!(qamille::infrastructure::rag_segments::index_identity(&root).is_none());
    while indexer.step().unwrap() {}
    let after = repository.read().unwrap();
    assert!(after.emails.iter().all(|e| e.search_index.version == qamille::domain::indexing::VERSION && !e.search_index.semantic_pending));
    assert_eq!(attempts(&after), [0, 0, 0, 0, 0]);
    let identity = qamille::infrastructure::rag_segments::index_identity(&root).unwrap();
    assert_eq!(identity.model, "other-model");
    assert_eq!(identity.dimensions, 256);
    assert_eq!(identity.quantization, "q4f16");
    // MCP with the session revoked (last: the MCP process may move the store to its database):
    // lexical results and the words of A11; the vault is only read.
    server.revoke_all();
    // The vault key, wrapped by Windows for this user (phase 3, Task 7).
    let vault_file = root.join(if cfg!(windows) { "credentials.key.dpapi" } else { "credentials.key" });
    qamille::infrastructure::secret::set("server-session", &session(&server.url, KEY)).unwrap();
    let answer = mcp(&root, "Cefalù");
    let result = &answer["result"]["structuredContent"];
    let data = &result["data"];
    assert_eq!(result["meta"]["notice"], compute::MCP_NOT_CONNECTED_MESSAGE, "{answer}");
    assert!(data["items"].as_array().unwrap().iter().any(|item| item["email_id"] == "2" || item["email_id"] == 2), "{answer}");
    assert_eq!(qamille::infrastructure::secret::get("server-session"), session(&server.url, KEY), "MCP never changes the session");
    assert!(vault_file.exists());
    for (path, _) in server.log() {
        assert!(!path.starts_with("/v1/account") && !path.starts_with("/v1/devices") && !path.starts_with("/v1/login"), "never account routes: {path}");
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// One `semantic_search` through the `qamille-mcp` process of this data folder.
fn mcp(root: &std::path::Path, query: &str) -> Value {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_qamille-mcp"))
        .env("QAMILLE_DATA", root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"semantic_search","arguments":{"query":query}}}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let responses: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    responses.into_iter().find(|r| r["id"] == 2).expect("the tool answered")
}

#[test]
fn the_certificate_authority_goes_only_with_its_own_address_and_names_the_gateway() {
    let old = ComputeServer::start(&[KEY]);
    let new = ComputeServer::start(&[NEW_KEY]);
    const CA: &str = "-----BEGIN CERTIFICATE-----\nOLD\n-----END CERTIFICATE-----";
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&new.url, NEW_KEY)).unwrap();
    let mut store = remote_store(&new.url, true);
    store.settings.compute_consented_urls.push(old.url.clone());
    // The settings still hold the old server's authority (the window wrote the vault first).
    store.settings.compute_ca_pem = Some(CA.into());
    store.settings.compute_ca_url = Some(old.url.clone());
    let repository = Arc::new(MemoryRepository::new(store));
    let built: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let seen = built.clone();
    let factory = Arc::new(ComputeFactory::with(repository.clone(), vault.clone()).with_remote_builder(move |source, ca| {
        seen.lock().unwrap().push(ca.map(str::to_owned));
        Ok(Arc::new(HttpCompute::new(source)) as Arc<dyn ComputeGateway>)
    }));
    factory.current().embed_query("x").unwrap();
    assert_eq!(built.lock().unwrap().as_slice(), [None], "the old authority never covers the new address");
    // The authority saved for this address: a gateway of its own, built with it.
    repository.transaction(&mut |store| {
        store.settings.compute_ca_url = Some(new.url.clone());
        Ok(())
    }).unwrap();
    let _ = factory.current();
    assert_eq!(built.lock().unwrap().as_slice(), [None, Some(CA.to_owned())]);
    let _ = factory.current();
    assert_eq!(built.lock().unwrap().len(), 2, "same link, same gateway");
}

#[test]
fn a_401_does_not_move_a_gateway_to_another_server() {
    let first = ComputeServer::start(&[KEY]);
    let second = ComputeServer::start(&[NEW_KEY]);
    let vault = Arc::new(Vault::default());
    vault.set("server-session", &session(&first.url, KEY)).unwrap();
    let mut store = remote_store(&first.url, true);
    store.settings.compute_consented_urls.push(second.url.clone());
    let events = Arc::new(Recorder::default());
    let factory = factory(Arc::new(MemoryRepository::new(store)), vault.clone());
    factory.set_notices(events.clone());
    let gateway = factory.current();
    gateway.embed_query("prima").unwrap();
    // «Change server» meanwhile: the vault holds the second server; the first revokes the key.
    vault.set("server-session", &session(&second.url, NEW_KEY)).unwrap();
    first.revoke_all();
    // The job's gateway asks the vault before sending: the address changed, nothing is sent
    // (a 401 racing the change is final for it too: `renew` never switches address).
    let before = first.log().len();
    assert_eq!(gateway.embed_query("dopo").unwrap_err().code, codes::NOT_CONNECTED);
    assert_eq!(first.log().len(), before);
    assert!(second.log().is_empty(), "the job's gateway stays with its own server");
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 0, "a replaced session is no unlinked device");
    assert!(!factory.unlinked());
    factory.current().embed_query("nuovo").unwrap();
    assert_eq!(second.log().len(), 1);
}
