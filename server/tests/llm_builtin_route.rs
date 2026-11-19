//! `POST /v1/chat/completions` with `llm_provider = "builtin"` against the fake `llama-server`
//! (Task 9): the child starts with the first valid request and never before, the app never sees
//! its key or port, and the server's shutdown takes it down (INV-34, INV-40; Review Focus 5, 7, 18).
mod support;

use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use qamille_server::compute::{LLM_NOT_ENOUGH_MEMORY, LLM_PREPARING};
use qamille_server::llm::{BUILTIN_TIMEOUT_SECS, LlmConfig};
use qamille_server::llm_builtin::{BuiltinSettings, LlmSupervisor, SupervisorDeps, SupervisorState};
use serde_json::{Value, json};
use support::fake_llama::*;
use support::llm_env::*;
use support::{TestServer, user_with_device};

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

/// One subscriber for the whole file, at the level the real server logs at (INFO): the HTTP
/// client dumps whole requests at TRACE, which nobody turns on in production.
fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing_subscriber::fmt().with_writer(capture.clone()).with_max_level(tracing::Level::INFO).with_ansi(false).init();
        capture
    })
}

fn log_text() -> String {
    String::from_utf8_lossy(&capture().0.lock().unwrap()).into_owned()
}

const PROMPT: &str = "PROMPT-RICONOSCIBILE-7731";

struct Rig {
    fixture: Fixture,
    supervisor: Arc<LlmSupervisor>,
    ts: TestServer,
    key: String,
    user_id: String,
}

async fn rig_with(
    extra: &[&str],
    llm: LlmConfig,
    tweak: impl FnOnce(&mut BuiltinSettings),
    deps: SupervisorDeps,
    entitlement: Option<Arc<dyn qamille_server::ext::Entitlement>>,
) -> Rig {
    capture();
    let fixture = Fixture::new();
    let mut settings = fixture.settings(extra);
    tweak(&mut settings);
    let supervisor = LlmSupervisor::new(settings, deps);
    let ts = ts_with_builtin(llm, Arc::clone(&supervisor), entitlement);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    Rig { fixture, supervisor, ts, key: login.device_key, user_id: login.user_id }
}

async fn rig(extra: &[&str]) -> Rig {
    rig_with(extra, builtin_llm(), |_| {}, SupervisorDeps::default(), None).await
}

/// A start that takes 1.5 seconds before the program is even resolved.
fn slow_start(settings: &mut BuiltinSettings) {
    let inner = Arc::clone(&settings.program);
    settings.program = Arc::new(move || {
        std::thread::sleep(Duration::from_millis(1500));
        inner()
    });
}

impl Rig {
    async fn chat(&self, consent: Option<&str>, body: Value) -> support::TestResponse {
        chat(&self.ts.public, &self.key, consent, body).await
    }

    async fn ok(&self) -> support::TestResponse {
        self.chat(Some(BUILTIN_CONSENT), hello()).await
    }

    async fn get(&self, path: &str) -> support::TestResponse {
        support::request(&self.ts.public, "GET", path, None, support::loopback(), &[("authorization", &support::bearer(&self.key))]).await
    }

    fn chats(&self) -> usize {
        self.fixture
            .reports()
            .iter()
            .flat_map(|r| r["requests"].as_array().unwrap().clone())
            .filter(|r| r["path"] == "/v1/chat/completions")
            .count()
    }

    fn no_process(&self) {
        assert!(self.fixture.reports().is_empty(), "{:?}", self.fixture.reports());
        assert_eq!(self.supervisor.state(), SupervisorState::Stopped);
        assert!(self.supervisor.pid().is_none());
    }

    /// The key and the base address of the running child, the way only the gateway sees them.
    async fn secrets(&self) -> (String, String) {
        let supervisor = Arc::clone(&self.supervisor);
        tokio::task::spawn_blocking(move || {
            let endpoint = supervisor.endpoint().unwrap();
            (endpoint.key().expose().to_owned(), endpoint.chat_url())
        })
        .await
        .unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.supervisor.shutdown();
    }
}

#[tokio::test]
async fn a_builtin_request_starts_the_child_and_is_answered() {
    let rig = rig(&[]).await;
    let response = rig.ok().await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(response.body["choices"][0]["message"]["content"], "ok");
    let report = rig.fixture.only_report();
    let requests = report["requests"].as_array().unwrap();
    let chats: Vec<&Value> = requests.iter().filter(|r| r["path"] == "/v1/chat/completions").collect();
    assert_eq!(chats.len(), 1, "{requests:?}");
    assert_eq!(chats[0]["method"], "POST");
    assert_eq!(chats[0]["auth"], true, "the key of the child travels as a Bearer");
}

#[tokio::test]
async fn the_app_never_sees_the_child_key_or_port() {
    let rig = rig(&[]).await;
    let mut texts = vec![rig.ok().await.text];
    texts.push(rig.chat(Some(BUILTIN_CONSENT), json!({"messages": [{"role": "user", "content": "x"}], "stream": true})).await.text);
    texts.push(rig.chat(Some("false;altro"), hello()).await.text);
    texts.push(rig.chat(None, hello()).await.text);
    for path in ["/v1/info", "/v1/models"] {
        let response = rig.get(path).await;
        assert_eq!(response.status, 200, "{}", response.text);
        texts.push(response.text);
    }
    let (key, url) = rig.secrets().await;
    let port = url.split(':').nth(2).unwrap().split('/').next().unwrap().to_owned();
    let model = rig.fixture.model.to_string_lossy().into_owned();
    for text in &texts {
        assert!(!text.contains(&key) && !text.contains("127.0.0.1") && !text.contains(&format!(":{port}")), "{text}");
        assert!(!text.contains(&model) && !text.contains("model.gguf"), "{text}");
    }
}

#[tokio::test]
async fn info_declares_the_builtin_model_as_internal() {
    let rig = rig(&[]).await;
    let info = rig.get("/v1/info").await;
    let llm = &info.body["llm"];
    assert_eq!(support::keys(llm), ["context_tokens", "external", "label", "model", "models"]);
    assert_eq!(llm["model"], "llm-medio");
    assert_eq!(llm["models"], json!(["llm-medio"]));
    assert_eq!(llm["external"], false);
    assert_eq!(llm["label"], "questo server");
    assert_eq!(llm["context_tokens"], 4096);
    rig.no_process();
}

#[tokio::test]
async fn a_wrong_consent_never_starts_the_child() {
    let rig = rig(&[]).await;
    for consent in [None, Some("false;127.0.0.1"), Some("true;questo server"), Some("false;Questo server")] {
        let response = rig.chat(consent, hello()).await;
        assert_eq!(response.status, 409, "{}", response.text);
        assert_eq!(response.code(), Some("llm_consent_required"));
    }
    rig.no_process();
}

#[tokio::test]
async fn the_matching_consent_passes() {
    let rig = rig(&[]).await;
    assert_eq!(rig.chat(Some("false;questo server"), hello()).await.status, 200);
}

#[tokio::test]
async fn a_user_without_entitlement_never_starts_the_child() {
    let rig = rig_with(&[], builtin_llm(), |_| {}, SupervisorDeps::default(), Some(Arc::new(NoLlm))).await;
    for consent in [None, Some(BUILTIN_CONSENT)] {
        let response = rig.chat(consent, hello()).await;
        assert_eq!(response.status, 402, "{}", response.text);
        assert_eq!(response.code(), Some("subscription_required"));
    }
    rig.no_process();
}

#[tokio::test]
async fn a_body_that_fails_validation_never_starts_the_child() {
    let rig = rig(&[]).await;
    for body in [json!({"messages": [{"role": "user", "content": "x"}], "stream": true}), json!({"messages": []}), json!({})] {
        let response = rig.chat(Some(BUILTIN_CONSENT), body).await;
        assert_eq!(response.status, 400, "{}", response.text);
        assert_eq!(response.code(), Some("invalid_request"));
    }
    rig.no_process();
}

#[tokio::test]
async fn a_memory_pause_is_503_llm_unavailable_with_the_memory_text() {
    let deps = SupervisorDeps { memory: Memory::new(Some(100)), ..SupervisorDeps::default() };
    let rig = rig_with(&[], builtin_llm(), |s| s.required_mb = 50_000, deps, None).await;
    let response = rig.ok().await;
    assert_eq!(response.status, 503, "{}", response.text);
    assert_eq!(response.code(), Some("llm_unavailable"));
    assert!(response.headers.contains_key("retry-after"), "{:?}", response.headers);
    assert_eq!(response.body["error"]["message"], LLM_NOT_ENOUGH_MEMORY);
    rig.no_process();
}

#[tokio::test]
async fn a_model_being_downloaded_is_503_llm_unavailable_in_preparation_and_starts_nothing() {
    let rig = rig_with(&[], builtin_llm(), |s| s.preparing.store(true, std::sync::atomic::Ordering::SeqCst), SupervisorDeps::default(), None).await;
    let response = rig.ok().await;
    assert_eq!(response.status, 503, "{}", response.text);
    assert_eq!(response.code(), Some("llm_unavailable"));
    assert!(response.headers.contains_key("retry-after"), "{:?}", response.headers);
    assert_eq!(response.body["error"]["message"], LLM_PREPARING);
    rig.no_process();
}

#[tokio::test]
async fn the_pause_after_a_crash_is_503_with_retry_after() {
    let rig = rig(&["--fake-crash-on-start"]).await;
    let first = rig.ok().await;
    assert_eq!(first.status, 503, "{}", first.text);
    assert_eq!(first.code(), Some("llm_unavailable"));
    let started = rig.fixture.reports().len();
    let second = rig.ok().await;
    assert_eq!(second.status, 503, "{}", second.text);
    assert_eq!(second.code(), Some("llm_unavailable"));
    let seconds: u64 = second.headers["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((1..=5).contains(&seconds), "{seconds}");
    assert_eq!(rig.fixture.reports().len(), started, "no new process during the pause");
}

#[tokio::test]
async fn the_first_request_may_take_the_start_time_without_hitting_the_llm_timeout() {
    let llm = LlmConfig { timeout_secs: 1, ..builtin_llm() };
    let rig = rig_with(&[], llm, slow_start, SupervisorDeps::default(), None).await;
    let started = std::time::Instant::now();
    let response = rig.ok().await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert!(started.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test]
async fn a_slow_answer_past_the_timeout_is_503() {
    let llm = LlmConfig { timeout_secs: 1, ..builtin_llm() };
    let rig = rig_with(&["--fake-delay-ms", "2500"], llm, |_| {}, SupervisorDeps::default(), None).await;
    let response = rig.ok().await;
    assert_eq!(response.status, 503, "{}", response.text);
    assert_eq!(response.code(), Some("llm_unavailable"));
    assert_eq!(response.body["error"]["retry_after_s"], 30);
    assert_eq!(rig.supervisor.state(), SupervisorState::Ready, "the child stays alive");
    assert!(pid_alive(rig.supervisor.pid().unwrap()));
}

#[tokio::test]
async fn the_counters_and_the_daily_cap_work_with_builtin() {
    let llm = LlmConfig { max_tokens_per_day_per_user: 1, ..builtin_llm() };
    let rig = rig_with(&[], llm, |_| {}, SupervisorDeps::default(), None).await;
    let other = user_with_device(&rig.ts, "anna", "una password lunga").await;
    // The soft cap lets the first request through; the second finds the day spent.
    assert_eq!(rig.ok().await.status, 200);
    let capped = rig.ok().await;
    assert_eq!(capped.status, 429, "{}", capped.text);
    assert_eq!(capped.code(), Some("rate_limited"));
    assert_eq!(rig.chats(), 1, "the capped request never reached the child");
    // The tokens of the answer (3 + 1) reach the counters of the user, not of another one.
    let day = rig.ts.accounts().now().format("%Y-%m-%d").to_string();
    let mut spent = 0;
    for _ in 0..100 {
        spent = rig.ts.accounts().llm_tokens_on(&rig.user_id, &day).await.unwrap();
        if spent >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(spent, 4);
    assert_eq!(rig.ts.accounts().llm_tokens_on(&other.user_id, &day).await.unwrap(), 0);
}

#[tokio::test]
async fn the_cap_stops_a_user_before_the_child_starts() {
    let llm = LlmConfig { max_tokens_per_day_per_user: 1, ..builtin_llm() };
    let rig = rig_with(&[], llm, |_| {}, SupervisorDeps::default(), None).await;
    assert_eq!(rig.ok().await.status, 200);
    rig.supervisor.stop();
    wait_until(Duration::from_secs(10), "the child stops", || rig.supervisor.state() == SupervisorState::Stopped);
    let before = rig.fixture.reports().len();
    assert_eq!(rig.ok().await.status, 429);
    assert_eq!(rig.supervisor.state(), SupervisorState::Stopped, "nothing was started to answer 429");
    assert_eq!(rig.fixture.reports().len(), before);
}

#[tokio::test]
async fn the_slot_is_held_while_the_child_starts() {
    let llm = LlmConfig { max_in_flight_per_user: 1, ..builtin_llm() };
    let rig = rig_with(&[], llm, slow_start, SupervisorDeps::default(), None).await;
    let (router, key) = (rig.ts.public.clone(), rig.key.clone());
    let first = tokio::spawn(async move { chat(&router, &key, Some(BUILTIN_CONSENT), hello()).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The app leaves during the start: the place stays taken until the start is over.
    first.abort();
    let busy = rig.ok().await;
    assert_eq!(busy.status, 429, "{}", busy.text);
    assert_eq!(busy.code(), Some("rate_limited"));
    wait_until(Duration::from_secs(15), "the child is ready", || rig.supervisor.state() == SupervisorState::Ready);
    for _ in 0..100 {
        if rig.ok().await.status == 200 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the place was never released");
}

#[tokio::test]
async fn secrets_in_logs_builtin() {
    let rig = rig(&["--fake-log-prompt", "--fake-echo"]).await;
    let response = rig.chat(Some(BUILTIN_CONSENT), json!({"messages": [{"role": "user", "content": PROMPT}]})).await;
    assert_eq!(response.status, 200, "{}", response.text);
    assert_eq!(rig.ok().await.status, 200);
    let (key, url) = rig.secrets().await;
    let base = url.trim_end_matches("/v1/chat/completions").to_owned();
    let log = log_text();
    assert!(log.contains("llm"), "the request line is there: {log}");
    assert!(!log.contains(PROMPT), "{log}");
    assert!(!log.contains(&key), "{log}");
    assert!(!log.contains(&base), "{log}");
    assert!(!log.contains(&rig.fixture.model.to_string_lossy().into_owned()), "{log}");
}

#[tokio::test]
async fn the_startup_line_for_builtin_names_the_model_without_paths() {
    let rig = rig(&[]).await;
    let gateway = rig.ts.server.compute_state().unwrap().llm_gateway().unwrap().clone();
    gateway.log_startup();
    let log = log_text();
    let line = log.lines().rev().find(|l| l.contains("gateway LLM")).expect("startup line");
    assert!(line.contains("builtin") && line.contains("llm-medio"), "{line}");
    assert!(!line.contains("model.gguf") && !line.contains("127.0.0.1") && !line.contains("llama-server"), "{line}");
    rig.no_process();
}

#[tokio::test]
async fn shutdown_of_the_server_kills_the_child() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let mut config = support::test_config();
    config.llm = builtin_llm();
    config.bind = "127.0.0.1:0".parse().unwrap();
    let ts = TestServer::new(config);
    let server = mount(ts_server(&ts), None, Duration::ZERO).with_llm_supervisor(Arc::clone(&supervisor));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve(async move {
        let _ = stop_rx.await;
    }));
    let worker = Arc::clone(&supervisor);
    let pid = tokio::task::spawn_blocking(move || {
        let _endpoint = worker.endpoint().unwrap();
        worker.pid().unwrap()
    })
    .await
    .unwrap();
    assert!(pid_alive(pid));
    stop_tx.send(()).unwrap();
    serving.await.unwrap().unwrap();
    assert!(!pid_alive(pid), "the child is gone when serve returns");
}

#[test]
fn the_default_timeout_for_builtin_is_600_seconds() {
    assert_eq!(BUILTIN_TIMEOUT_SECS, 600);
}
