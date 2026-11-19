//! The `llm-*` models in `/health`, `/v1/models` and the admin routes (Task 10): installation from
//! a fake Hugging Face used as the mirror, removal with the idle child stopped first, and nothing
//! of paths, addresses or tokens in what is shown (INV-39; Review Focus 11, 12, 16, 17, 19).
mod support;

use std::sync::Arc;
use std::time::Duration;

use qamille_compute::download::{HfToken, Mirror};
use qamille_compute::llama::{self, Platform};
use qamille_compute::llm_catalog::{self, LlmModelSpec};
use qamille_server::compute::MODEL_ID;
use qamille_server::compute::llm_models::{BuiltinLlmModels, LlmModels};
use qamille_server::llm_builtin::{LlmSupervisor, SupervisorState};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::fake_hf::FakeHf;
use support::fake_llama::*;
use support::llm_env::*;
use support::{ADMIN_KEY, TempDir, TestServer, bearer, loopback, request, user_with_device};

const TOKEN: &str = "hf_SECRET-TOKEN-4471";

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

fn catalog(specs: Vec<LlmModelSpec>) -> &'static [LlmModelSpec] {
    Box::leak(specs.into_boxed_slice())
}

fn make(id: &'static str, bytes: &[u8], pinned: bool) -> LlmModelSpec {
    LlmModelSpec {
        id,
        repo: "acme/test-GGUF",
        revision: leak("a".repeat(40)),
        file: "test-Q4_K_M.gguf",
        bytes: bytes.len() as u64,
        sha256: if pinned { leak(hex::encode(Sha256::digest(bytes))) } else { "" },
        license: "Apache-2.0",
        license_url: "https://example.test/terms",
        quantization: "Q4_K_M",
        accept_license: "",
        ram_mb: 4000,
        kv_mb_per_1k: 100,
        context: 8192,
        speed_note: "a test model",
        gated: false,
    }
}

struct Rig {
    ts: TestServer,
    hf: FakeHf,
    dir: TempDir,
    spec: &'static LlmModelSpec,
    key: String,
    fixture: Fixture,
    supervisor: Arc<LlmSupervisor>,
    /// While set, the model check of the supervisor is slow: a start (and so a stop) takes time.
    slow: Arc<std::sync::atomic::AtomicBool>,
}

async fn rig_on(platform: Platform) -> Rig {
    let body = payload(20_000);
    let catalog = catalog(vec![make("llm-medio", &body, true), make("llm-grande", &body, false)]);
    let spec = &catalog[0];
    let hf = FakeHf::start(body);
    let dir = TempDir::new();
    let fixture = Fixture::new();
    // The child starts from the model of the catalog: a removed model is a failed start.
    let model_path = llm_catalog::model_file(dir.path(), spec);
    let mut settings = fixture.settings(&[]);
    let slow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let slow_model = Arc::clone(&slow);
    settings.model = Arc::new(move || {
        if slow_model.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(800));
        }
        if model_path.is_file() { Ok(model_path.clone()) } else { Err("The model is not installed.".to_owned()) }
    });
    let supervisor = LlmSupervisor::new(settings, Default::default());
    let models = Arc::new(
        BuiltinLlmModels::new(
            dir.path().to_path_buf(),
            Mirror::parse(&hf.url).ok(),
            HfToken::new(TOKEN),
            4096,
            platform,
            Some(fake_bin()),
            Some(Arc::clone(&supervisor)),
            Some("llm-medio".to_owned()),
        )
        .with_catalog(catalog),
    );
    let mut config = support::test_config();
    config.llm = builtin_llm();
    let ts = TestServer::new(config);
    let server = mount(ts_server(&ts), None, Duration::ZERO).with_llm_supervisor(Arc::clone(&supervisor)).with_llm_models(models.clone());
    let ts = rebuild(server, ts);
    let key = user_with_device(&ts, "mario", "una password lunga").await.device_key;
    Rig { ts, hf, dir, spec, key, fixture, supervisor, slow }
}

impl Rig {
    async fn admin(&self, method: &str, path: &str) -> support::TestResponse {
        request(self.ts.admin(), method, path, None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await
    }

    async fn health(&self) -> support::TestResponse {
        request(&self.ts.public, "GET", "/health", None, loopback(), &[("authorization", &bearer(&self.key))]).await
    }

    async fn list(&self) -> Value {
        let response = self.admin("GET", "/admin/api/models").await;
        assert_eq!(response.status, 200, "{}", response.text);
        response.body
    }

    async fn state_of(&self, id: &str) -> Value {
        self.list().await["models"].as_array().unwrap().iter().find(|m| m["id"] == id).cloned().unwrap()
    }

    async fn wait_state(&self, id: &str, state: &str) -> Value {
        for _ in 0..300 {
            let model = self.state_of(id).await;
            if model["state"] == state {
                return model;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("{id} never became {state}: {}", self.state_of(id).await);
    }

    async fn install_ready(&self) {
        let response = self.admin("POST", "/admin/api/models/llm-medio/install").await;
        assert!(response.status == 200 || response.status == 202, "{}", response.text);
        self.wait_state("llm-medio", "ready").await;
    }

    fn model_file(&self) -> std::path::PathBuf {
        llm_catalog::model_file(self.dir.path(), self.spec)
    }

}

impl Drop for Rig {
    fn drop(&mut self) {
        self.supervisor.shutdown();
    }
}

async fn rig() -> Rig {
    rig_on(Platform::Linux).await
}

#[tokio::test(flavor = "multi_thread")]
async fn models_list_the_builtin_model_once() {
    let rig = rig().await;
    rig.install_ready().await;
    let response = request(&rig.ts.public, "GET", "/v1/models", None, loopback(), &[("authorization", &bearer(&rig.key))]).await;
    assert_eq!(response.status, 200, "{}", response.text);
    let ids: Vec<&str> = response.body["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids.iter().filter(|id| **id == "llm-medio").count(), 1, "{ids:?}");
    assert_eq!(ids.iter().filter(|id| **id == MODEL_ID).count(), 1, "{ids:?}");
    assert!(!ids.contains(&"llm-grande"), "{ids:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn llm_models_appear_in_health_and_in_the_admin_list() {
    let rig = rig().await;
    let before = rig.health().await;
    let llm: Vec<&Value> = before.body["models"].as_array().unwrap().iter().filter(|m| m["kind"] == "llm").collect();
    assert_eq!(llm.len(), 2, "{}", before.text);
    assert_eq!(llm[0]["id"], "llm-medio");
    assert_eq!(llm[0]["state"], "missing");
    assert_eq!(llm[0]["quantization"], "Q4_K_M");
    let started = rig.admin("POST", "/admin/api/models/llm-medio/install").await;
    assert!(started.status == 200 || started.status == 202, "{}", started.text);
    let done = rig.wait_state("llm-medio", "ready").await;
    assert_eq!(done["loaded"], false);
    let after = rig.health().await;
    assert!(after.body["models"].as_array().unwrap().iter().any(|m| m["id"] == "llm-medio" && m["state"] == "ready"));
    for text in [&before.text, &after.text, &rig.admin("GET", "/admin/api/models").await.text] {
        // The address of the licence terms is public and shown on purpose; any other address is a leak.
        let text = &text.replace("https://example.test/terms", "");
        assert!(!text.contains("127.0.0.1") && !text.contains("http") && !text.contains(TOKEN), "{text}");
        assert!(!text.contains(&rig.dir.path().to_string_lossy().into_owned()) && !text.contains("test-Q4_K_M.gguf"), "{text}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_admin_list_shows_memory_and_disk_before_install() {
    let rig = rig().await;
    let list = rig.list().await;
    let model = list["models"].as_array().unwrap().iter().find(|m| m["id"] == "llm-medio").unwrap();
    let details = &model["llm"];
    assert_eq!(details["ram_required_mb"], llm_catalog::required_ram_mb(rig.spec, 4096));
    assert_eq!(details["disk_required_bytes"], llm_catalog::required_disk_bytes(rig.spec));
    assert_eq!(details["context"], 8192);
    assert_eq!(details["license"], "Apache-2.0");
    assert_eq!(details["license_url"], "https://example.test/terms");
    assert_eq!(details["engine"], "ready");
    assert_eq!(details["in_use"], false);
    assert_eq!(details["pinned"], true);
    assert!(list["host"]["available_memory_mb"].is_u64(), "{list}");
    // Free disk space is read only where the platform tells it (qamille_compute::memory): Windows.
    #[cfg(windows)]
    assert!(list["host"]["available_disk_bytes"].is_u64(), "{list}");
    #[cfg(not(windows))]
    assert!(list["host"]["available_disk_bytes"].is_null(), "{list}");
    // Only the language models carry the block.
    assert!(list["models"].as_array().unwrap().iter().filter(|m| m["kind"] != "llm").all(|m| m.get("llm").is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn install_downloads_from_the_mirror_and_reports_progress() {
    let rig = rig().await;
    rig.hf.slow(2_000, Duration::from_millis(150));
    let started = rig.admin("POST", "/admin/api/models/llm-medio/install").await;
    assert_eq!(started.status, 202, "{}", started.text);
    assert_eq!(started.body["state"], "downloading");
    let mut percents = Vec::new();
    loop {
        let model = rig.state_of("llm-medio").await;
        if model["state"] == "ready" {
            break;
        }
        assert_eq!(model["state"], "downloading", "{model}");
        if let Some(p) = model["percent"].as_u64() {
            percents.push(p);
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    assert!(percents.windows(2).all(|w| w[0] <= w[1]), "{percents:?}");
    assert!(percents.iter().any(|p| (1..100).contains(p)), "{percents:?}");
    assert_eq!(std::fs::read(rig.model_file()).unwrap(), payload(20_000));
    let seen = rig.hf.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(seen[0].path.contains("/acme/test-GGUF/resolve/") && seen[0].path.ends_with("/test-Q4_K_M.gguf"), "{seen:?}");
    assert!(!seen[0].authorization, "the token never goes to a mirror");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_install_of_the_same_model_returns_the_running_one() {
    let rig = rig().await;
    rig.hf.slow(2_000, Duration::from_millis(150));
    assert_eq!(rig.admin("POST", "/admin/api/models/llm-medio/install").await.status, 202);
    let second = rig.admin("POST", "/admin/api/models/llm-medio/install").await;
    assert_eq!(second.status, 202, "{}", second.text);
    assert_eq!(second.body["state"], "downloading");
    rig.wait_state("llm-medio", "ready").await;
    assert_eq!(rig.hf.seen().len(), 1);
    // And once ready, the same call is a plain 200.
    assert_eq!(rig.admin("POST", "/admin/api/models/llm-medio/install").await.status, 200);
    assert_eq!(rig.hf.seen().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn install_of_an_unpinned_entry_is_refused_with_a_clear_message() {
    let rig = rig().await;
    let response = rig.admin("POST", "/admin/api/models/llm-grande/install").await;
    assert_eq!(response.status, 400, "{}", response.text);
    assert_eq!(response.code(), Some("invalid_request"));
    assert!(response.body["error"]["message"].as_str().unwrap().contains("not available yet"), "{}", response.text);
    assert!(rig.hf.seen().is_empty());
    let model = rig.state_of("llm-grande").await;
    assert_eq!(model["state"], "missing");
    assert_eq!(model["llm"]["pinned"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn install_without_the_engine_is_refused_outside_windows_like_whisper() {
    let body = payload(1000);
    let hf = FakeHf::start(body.clone());
    let dir = TempDir::new();
    let models = BuiltinLlmModels::new(dir.path().to_path_buf(), Mirror::parse(&hf.url).ok(), None, 4096, Platform::Linux, None, None, None)
        .with_catalog(catalog(vec![make("llm-medio", &body, true)]));
    let error = models.install("llm-medio").unwrap_err();
    assert!(error.message().contains("engine"), "{}", error.message());
    assert!(hf.seen().is_empty());
    assert_eq!(models.models()[0].state, "missing");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_windows_install_adds_the_runtime_bytes_to_the_total() {
    let body = payload(1000);
    let hf = FakeHf::start(body.clone());
    let dir = TempDir::new();
    let catalog = catalog(vec![make("llm-medio", &body, true)]);
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&runs);
    let models = BuiltinLlmModels::new(dir.path().to_path_buf(), Mirror::parse(&hf.url).ok(), None, 4096, Platform::WindowsX64, None, None, None)
        .with_catalog(catalog)
        .with_runtime_installer(Arc::new(move |_, _, _, progress| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            progress(llama::RUNTIME_BYTES);
            Ok(())
        }));
    let before = models.models().remove(0);
    assert_eq!(before.total_bytes, 1000 + llama::RUNTIME_BYTES);
    assert_eq!(before.state, "missing");
    assert_eq!(before.error, None, "nothing went wrong yet: the engine is shown in the details");
    let details = models.details("llm-medio").unwrap();
    assert_eq!(details.engine, "missing");
    assert_eq!(details.disk_required_bytes, 1000 + qamille_compute::download::PHYSICAL_RESERVE + llama::RUNTIME_BYTES);
    models.install("llm-medio").unwrap();
    for _ in 0..200 {
        if hf.seen().len() == 1 && models.models()[0].state != "downloading" {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1, "the engine is downloaded first");
    assert_eq!(std::fs::read(llm_catalog::model_file(dir.path(), &catalog[0])).unwrap(), body);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_deletes_the_files_and_the_status_goes_missing() {
    let rig = rig().await;
    rig.install_ready().await;
    assert!(rig.model_file().is_file());
    let removed = rig.admin("DELETE", "/admin/api/models/llm-medio").await;
    assert_eq!(removed.status, 200, "{}", removed.text);
    assert_eq!(removed.body["id"], "llm-medio");
    assert_eq!(removed.body["state"], "missing");
    assert!(!rig.model_file().exists());
    assert!(!rig.model_file().parent().unwrap().exists());
    assert_eq!(rig.state_of("llm-medio").await["state"], "missing");
    // Removing what is not there is not an error.
    assert_eq!(rig.admin("DELETE", "/admin/api/models/llm-medio").await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_is_refused_while_a_request_runs() {
    let rig = rig().await;
    rig.install_ready().await;
    let supervisor = Arc::clone(&rig.supervisor);
    let held = tokio::task::spawn_blocking(move || supervisor.endpoint().unwrap()).await.unwrap();
    assert_eq!(rig.state_of("llm-medio").await["llm"]["in_use"], true);
    let refused = rig.admin("DELETE", "/admin/api/models/llm-medio").await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    assert_eq!(refused.code(), Some("invalid_request"));
    assert!(rig.model_file().is_file(), "the files stay");
    drop(held);
    assert_eq!(rig.admin("DELETE", "/admin/api/models/llm-medio").await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_stops_the_idle_child_first() {
    let rig = rig().await;
    rig.install_ready().await;
    let supervisor = Arc::clone(&rig.supervisor);
    let pid = tokio::task::spawn_blocking(move || {
        let _endpoint = supervisor.endpoint().unwrap();
        supervisor.pid().unwrap()
    })
    .await
    .unwrap();
    assert!(pid_alive(pid));
    assert_eq!(rig.state_of("llm-medio").await["loaded"], true);
    assert_eq!(rig.admin("DELETE", "/admin/api/models/llm-medio").await.status, 200);
    assert!(!pid_alive(pid), "the child is gone before the files");
    assert!(!rig.model_file().exists());
    assert_eq!(rig.supervisor.state(), SupervisorState::Stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_install_cannot_start_while_a_removal_is_running() {
    let rig = rig().await;
    rig.install_ready().await;
    rig.slow.store(true, std::sync::atomic::Ordering::SeqCst);
    // A slow start: the removal has to wait for it to stop, and holds the model meanwhile.
    let supervisor = Arc::clone(&rig.supervisor);
    let starter = std::thread::spawn(move || supervisor.endpoint().err());
    wait_until(Duration::from_secs(5), "the start to begin", || rig.supervisor.state() == SupervisorState::Starting);
    let removal = rig.admin("DELETE", "/admin/api/models/llm-medio");
    let racing = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let install = rig.admin("POST", "/admin/api/models/llm-medio/install").await;
        let second = rig.admin("DELETE", "/admin/api/models/llm-medio").await;
        (install, second)
    };
    let (removed, (install, second)) = tokio::join!(removal, racing);
    rig.slow.store(false, std::sync::atomic::Ordering::SeqCst);
    let _ = starter.join();
    assert_eq!(removed.status, 200, "{}", removed.text);
    assert_eq!(install.status, 400, "an installation started during the removal: {}", install.text);
    assert_eq!(second.status, 400, "{}", second.text);
    assert!(!rig.model_file().exists(), "the removal finished and nothing came back");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_is_refused_during_an_install() {
    let rig = rig().await;
    rig.hf.slow(2_000, Duration::from_millis(200));
    assert_eq!(rig.admin("POST", "/admin/api/models/llm-medio/install").await.status, 202);
    let refused = rig.admin("DELETE", "/admin/api/models/llm-medio").await;
    assert_eq!(refused.status, 400, "{}", refused.text);
    rig.wait_state("llm-medio", "ready").await;
    assert!(rig.model_file().is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_removed_configured_model_makes_the_next_request_a_pause_not_a_crash() {
    let rig = rig().await;
    rig.install_ready().await;
    assert_eq!(rig.admin("DELETE", "/admin/api/models/llm-medio").await.status, 200);
    for _ in 0..3 {
        let response = chat(&rig.ts.public, &rig.key, Some(BUILTIN_CONSENT), hello()).await;
        assert_eq!(response.status, 503, "{}", response.text);
        assert_eq!(response.code(), Some("llm_unavailable"));
    }
    assert!(rig.fixture.reports().is_empty(), "no process was ever started without the model");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_or_malicious_llm_id_is_404_or_400() {
    let rig = rig().await;
    rig.install_ready().await;
    for id in ["llm-..%2Fx", "llm-%2e%2e", "llm-..%5Cx", "llm-inesistente", "llm-", "x%00y", "llm-medio%20"] {
        for (method, path) in [("DELETE", format!("/admin/api/models/{id}")), ("POST", format!("/admin/api/models/{id}/install"))] {
            let response = rig.admin(method, &path).await;
            assert!(response.status == 404 || response.status == 400, "{method} {path}: {} {}", response.status, response.text);
        }
    }
    assert_eq!(rig.admin("DELETE", "/admin/api/models/").await.status, 404);
    assert!(rig.model_file().is_file(), "nothing was deleted");
    assert!(rig.dir.path().is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_model_routes_need_the_admin_key() {
    let rig = rig().await;
    for (method, path) in [("GET", "/admin/api/models"), ("POST", "/admin/api/models/llm-medio/install"), ("DELETE", "/admin/api/models/llm-medio")] {
        let none = request(rig.ts.admin(), method, path, None, loopback(), &[]).await;
        assert_eq!(none.status, 401, "{method} {path}");
        let device = request(rig.ts.admin(), method, path, None, loopback(), &[("authorization", &bearer(&rig.key))]).await;
        assert_eq!(device.status, 401, "{method} {path}: a device key is not the admin key");
    }
    assert!(rig.hf.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_public_listener_has_no_install_or_remove_route() {
    let rig = rig().await;
    for (method, path) in [("POST", "/admin/api/models/llm-medio/install"), ("DELETE", "/admin/api/models/llm-medio"), ("GET", "/admin/api/models")] {
        let response = request(&rig.ts.public, method, path, None, loopback(), &[("authorization", &bearer(ADMIN_KEY))]).await;
        assert_eq!(response.status, 404, "{method} {path}: {}", response.text);
        assert_eq!(response.code(), Some("not_found"));
    }
    assert!(rig.hf.seen().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_install_is_reported_without_urls_or_tokens() {
    let rig = rig().await;
    rig.hf.fail_with(500);
    assert_eq!(rig.admin("POST", "/admin/api/models/llm-medio/install").await.status, 202);
    let model = rig.wait_state("llm-medio", "missing").await;
    let error = model["error"].as_str().expect("the failure is reported");
    for secret in ["127.0.0.1", "http", TOKEN, "acme", "test-Q4_K_M"] {
        assert!(!error.contains(secret), "{error}");
    }
    assert!(!rig.model_file().exists());
    // Once the mirror answers again, the same call installs.
    rig.hf.fail_with(0);
    rig.install_ready().await;
}
