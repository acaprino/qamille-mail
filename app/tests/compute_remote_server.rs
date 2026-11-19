//! End to end with the real `qamille-server serve` and the real model (ignored by default,
//! compute wave 3):
//!
//! ```text
//! QAMILLE_SERVER_BIN=../server/target/debug/qamille-server \
//! QAMILLE_TEST_MODELS=<data>/models \
//! cargo test --jobs 2 --no-default-features --test compute_remote_server -- --ignored --nocapture
//! ```
//!
//! The server runs on a temporary data folder with an administrator key; the administrator API
//! creates a user; the app signs in through `ServerAccountService` and `HttpServerAccount`
//! (temporary password, then its own), which turns the remote mode on; a few synthetic messages
//! are indexed by the remote server and found by meaning; the administrator revokes the device
//! and the next indexing step stops the account with the notice of EX-15, consuming no attempt.
use serde_json::{Value, json};
use qamille::application::compute;
use qamille::application::events::AppEvent;
use qamille::application::indexing::MailIndexService;
use qamille::application::indexing_ports::IndexSource;
use qamille::application::ports::*;
use qamille::application::server_account::{ConnectOutcome, Credentials, ServerAccountService};
use qamille::application::{AppResult, ServiceContext};
use qamille::domain::indexing::{FolderProgress, VERSION};
use qamille::domain::model::{Account, AccountKind, ComputeMode, Email, Store};
use qamille::edition::Edition;
use qamille::infrastructure::compute_factory::ComputeFactory;
use qamille::infrastructure::gateways::{KeyringSecrets, LocalSearch};
use qamille::infrastructure::repository::JsonStoreRepository;
use qamille::infrastructure::server_account::HttpServerAccount;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ADMIN_KEY: &str = "e2e-admin-key-0123456789abcdefghijklmnop";

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn http(agent: &ureq::Agent, method: &str, url: &str, body: Option<Value>) -> (u16, Value) {
    let auth = format!("Bearer {ADMIN_KEY}");
    let response = match (method, body) {
        ("GET", _) => agent.get(url).header("Authorization", &auth).call(),
        ("DELETE", _) => agent.delete(url).header("Authorization", &auth).call(),
        (_, Some(body)) => agent.post(url).header("Authorization", &auth).send_json(body),
        (_, None) => agent.post(url).header("Authorization", &auth).send_empty(),
    };
    let mut response = response.unwrap();
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
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

struct NoSource;
impl IndexSource for NoSource {
    fn folders(&self, _: &Account) -> AppResult<Vec<FolderProgress>> {
        Ok(vec![])
    }
}
struct TestClock(AtomicI64);
impl Clock for TestClock {
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
    Email { id, account_id: 1, folder: "Inbox".into(), subject: subject.into(), from_name: "Maria Rossi".into(), from_addr: "maria@example.com".into(), body: body.into(), remote_content_hash: format!("hash-{id}"), date: 1_700_000_000 + id as i64, ..Default::default() }
}

#[test]
#[ignore]
fn sign_in_index_remotely_search_and_revoke_the_device() {
    let binary = PathBuf::from(std::env::var("QAMILLE_SERVER_BIN").expect("QAMILLE_SERVER_BIN: the qamille-server executable"));
    let models = PathBuf::from(std::env::var("QAMILLE_TEST_MODELS").expect("QAMILLE_TEST_MODELS: the folder holding embeddinggemma-2/"));
    let root = std::env::temp_dir().join(format!("qamille-remote-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (server_dir, app_dir) = (root.join("server"), root.join("app"));
    std::fs::create_dir_all(&server_dir).unwrap();
    std::fs::create_dir_all(&app_dir).unwrap();
    unsafe { std::env::set_var("QAMILLE_DATA", &app_dir) };

    // 1. The server: `serve` with a temporary data folder, an administrator key and the models.
    let (port, admin_port) = (free_port(), free_port());
    let log_path = root.join("server.log");
    let started = Instant::now();
    let _server = Server(
        Command::new(&binary)
            .arg("serve")
            .env("QAMILLE_DATA_DIR", &server_dir)
            .env("QAMILLE_BIND", format!("127.0.0.1:{port}"))
            .env("QAMILLE_ADMIN_BIND", format!("127.0.0.1:{admin_port}"))
            .env("QAMILLE_ADMIN_KEY", ADMIN_KEY)
            .env("QAMILLE_MODELS_DIR", &models)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log_path).unwrap())
            .spawn()
            .unwrap(),
    );
    let url = format!("http://127.0.0.1:{port}");
    let admin = format!("http://127.0.0.1:{admin_port}");
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
    loop {
        if let Ok(mut response) = agent.get(&format!("{url}/health")).call()
            && response.status() == 200
        {
            let health: Value = serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap();
            assert_eq!(health.as_object().unwrap().keys().collect::<Vec<_>>(), ["status", "version"], "anonymous health says only this");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "the server did not start: {}", std::fs::read_to_string(&log_path).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("server up in {:?}", started.elapsed());

    // 2. The administrator creates the user.
    let (status, created) = http(&agent, "POST", &format!("{admin}/admin/api/users"), Some(json!({"username": "mario"})));
    assert_eq!(status, 201, "{created}");
    let user_id = created["id"].as_str().unwrap().to_owned();
    let temporary = created["temporary_password"].as_str().unwrap().to_owned();

    // 3. The app signs in (temporary password, then its own): one session, remote mode.
    let mut store = Store::default();
    let mut account = Account { id: 1, email: "sample@example.com".into(), kind: AccountKind::Sample, ..Default::default() };
    account.search_index.enabled = true;
    store.accounts.push(account);
    store.emails = vec![
        message(1, "Fattura ottobre", "Ti confermo che la fattura di ottobre è stata saldata ieri con un bonifico bancario."),
        message(2, "Treno", "Il treno regionale per Cefalù parte alle nove dal binario tre, porta l'ombrello."),
        message(3, "Riunione", "La riunione con il fornitore delle sedie è spostata a giovedì pomeriggio."),
        message(4, "Cena", "Sabato sera ceniamo da Lucia: porta tu il dolce e una bottiglia di vino."),
    ];
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();
    let repository: Arc<dyn StoreRepository> = Arc::new(JsonStoreRepository::new(Store::path()));
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecrets);
    let events = Arc::new(Recorder::default());
    let factory = Arc::new(ComputeFactory::with(repository.clone(), secrets.clone()));
    factory.set_notices(events.clone());
    let clock = Arc::new(TestClock(AtomicI64::new(1_800_000_000)));
    let context = Arc::new(ServiceContext {
        events: events.clone(),
        repository: repository.clone(),
        mail: Arc::new(NoMail),
        secrets: secrets.clone(),
        clock: clock.clone(),
        attachments: Arc::new(NoFiles),
        compute: factory.routed(),
    });
    let account_service = ServerAccountService::new(context.clone(), Arc::new(HttpServerAccount), Edition::SelfHosted);
    assert!(matches!(account_service.probe(&url).unwrap(), qamille::application::server_account::Probe::Found { .. }));
    assert_eq!(factory.current().health().unwrap_err().code, compute::codes::NOT_CONNECTED, "no server until the sign-in");
    account_service.record_consent(&url).unwrap();
    let credentials = Credentials { identifier: "mario", password: &temporary, device_name: "Qamille e2e" };
    assert!(matches!(account_service.connect(&url, &credentials).unwrap(), ConnectOutcome::PasswordChangeRequired));
    let signed = account_service.change_temporary_password(&url, &credentials, "una password lunga e mia", false).unwrap();
    assert!(matches!(signed, ConnectOutcome::Connected { .. }));
    assert_eq!(repository.read().unwrap().settings.compute_mode, ComputeMode::Server);
    let session = account_service.session().unwrap().expect("a session in the vault");
    assert_eq!(session.url, url);
    let gateway = factory.current();
    assert_eq!(gateway.identity(), url, "the server of the session");
    let health = gateway.health().unwrap();
    assert!(health.embedding_ready(), "{:?}", health.models);
    assert_eq!(health.embedding_identity(), compute::EmbeddingIdentity::local(), "D4: the same model as the local mode");

    // 4. Remote indexing with the real model.
    let search = Arc::new(LocalSearch::with_notices(context.compute.clone(), factory.notices()));
    let indexer = MailIndexService::new(context.clone(), Arc::new(NoSource), search.clone()).with_batch_size(4);
    let started = Instant::now();
    while indexer.step().unwrap() {}
    let after = repository.read().unwrap();
    println!("4 messages indexed remotely in {:?}", started.elapsed());
    assert!(after.emails.iter().all(|e| e.search_index.version == VERSION && !e.search_index.semantic_pending), "{:?}", after.emails.iter().map(|e| (e.id, e.search_index.version, e.search_index.notice.clone())).collect::<Vec<_>>());
    let identity = qamille::infrastructure::rag_segments::index_identity(&app_dir).unwrap();
    assert_eq!(identity, compute::EmbeddingIdentity::local());

    // 5. Search by meaning: no word of the query is in the invoice.
    let started = Instant::now();
    let hits = search.search(&after, "pagamento ricevuto", 10).unwrap();
    println!("semantic search in {:?}: {:?}", started.elapsed(), hits.iter().map(|h| (h.email_id, h.score)).collect::<Vec<_>>());
    assert_eq!(hits.first().map(|h| h.email_id), Some(1));
    let hits = search.search(&after, "viaggio in ferrovia", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(2));
    assert_eq!(events.notices(compute::SEMANTIC_UNAVAILABLE_MESSAGE), 0);

    // Per-user counters, numbers only.
    let (_, users) = http(&agent, "GET", &format!("{admin}/admin/api/users"), None);
    let usage = &users["users"][0]["usage"];
    println!("usage of mario: {usage}");
    assert!(usage["requests"].as_u64().unwrap() >= 8, "{usage}");
    assert!(usage["embedded_tokens"].as_u64().unwrap() > 0, "{usage}");

    // 6. The administrator revokes the device (EX-15); a new message arrives.
    let (status, _) = http(&agent, "DELETE", &format!("{admin}/admin/api/users/{user_id}/devices/{}", session.device_id), None);
    assert_eq!(status, 204);
    repository.transaction(&mut |store| {
        store.emails.push(message(5, "Preventivo", "Il preventivo del falegname per la libreria arriva domani."));
        Ok(())
    }).unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // the health cache (1 s)
    let started = Instant::now();
    assert!(indexer.step().unwrap());
    println!("the next step after the revocation took {:?}", started.elapsed());
    let after = repository.read().unwrap();
    assert_eq!(after.accounts[0].search_index.state, "unlinked");
    assert_eq!(after.accounts[0].search_index.notice.as_deref(), Some(compute::UNLINKED_MESSAGE));
    assert!(after.emails.iter().all(|e| e.search_index.attempts == 0), "no attempt consumed");
    assert_eq!(after.emails[4].search_index.version, 0, "the new message waits");
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1);
    clock.0.fetch_add(61, Ordering::SeqCst);
    assert!(indexer.step().unwrap());
    assert_eq!(events.notices(compute::UNLINKED_MESSAGE), 1, "said once per session");
    assert!(factory.status().unlinked, "Settings → Compute offers «Sign in again»");
    assert!(account_service.session().unwrap().is_some(), "the session stays");
    // Search stays lexical, with the notice of D8.
    let hits = search.search(&after, "Cefalù", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(2));
    assert_eq!(events.notices(compute::SEMANTIC_UNAVAILABLE_MESSAGE), 1);

    // The server's log has codes, routes and sizes, never the mail.
    let log = std::fs::read_to_string(&log_path).unwrap();
    for secret in ["Cefalù", "fattura", "falegname", "pagamento", session.device_key.as_str(), temporary.as_str(), "una password lunga"] {
        assert!(!log.contains(secret), "the server log holds «{secret}»");
    }
    assert!(log.contains("compute"), "the access log of the compute routes is there");
    drop(_server);
    let _ = std::fs::remove_dir_all(&root);
}
