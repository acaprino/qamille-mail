//! The model of the built-in assistant is fetched by the server itself (spec L8): at the start, in
//! the background, from a fake Hugging Face used as the mirror, with waits after a failure; while it
//! lasts the assistant answers «in preparation»; a model that asks for an acknowledgement of its
//! licence is not downloaded without it, and the status says why.
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use qamille_compute::download::Mirror;
use qamille_compute::llama::Platform;
use qamille_compute::llm_catalog::{self, LlmModelSpec};
use qamille_server::compute::ModelStatus;
use qamille_server::compute::llm_models::BuiltinLlmModels;
use qamille_server::llm::LlmError;
use qamille_server::llm_builtin::LlmSupervisor;
use sha2::{Digest, Sha256};
use support::TempDir;
use support::fake_hf::FakeHf;
use support::fake_llama::*;

const ID: &str = "llm-piccolo";

fn leak<T>(value: T) -> &'static T {
    Box::leak(Box::new(value))
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

fn spec(body: &[u8], accept_license: &'static str) -> &'static LlmModelSpec {
    leak(LlmModelSpec {
        id: ID,
        repo: "acme/test-GGUF",
        revision: Box::leak("a".repeat(40).into_boxed_str()),
        file: "test-q4.gguf",
        bytes: body.len() as u64,
        sha256: Box::leak(hex::encode(Sha256::digest(body)).into_boxed_str()),
        license: "Apache-2.0",
        license_url: "https://example.test/terms",
        quantization: "Q4_0",
        accept_license,
        ram_mb: 4000,
        kv_mb_per_1k: 100,
        context: 8192,
        speed_note: "a test model",
        gated: false,
    })
}

struct Rig {
    hf: FakeHf,
    dir: TempDir,
    spec: &'static LlmModelSpec,
    preparing: Arc<AtomicBool>,
    models: BuiltinLlmModels,
}

impl Rig {
    fn status(&self) -> ModelStatus {
        use qamille_server::compute::llm_models::LlmModels;
        self.models.models().into_iter().find(|m| m.id == ID).unwrap()
    }

    fn wait_state(&self, state: &str) -> ModelStatus {
        let mut last = self.status();
        for _ in 0..400 {
            last = self.status();
            if last.state == state {
                return last;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("never {state}: {last:?}");
    }
}

fn manager(dir: &TempDir, hf: &FakeHf, spec: &'static LlmModelSpec, accepted: Option<&str>, configured: Option<&str>, supervisor: Option<Arc<LlmSupervisor>>) -> BuiltinLlmModels {
    let catalog: &'static [LlmModelSpec] = std::slice::from_ref(spec);
    BuiltinLlmModels::new(
        dir.path().to_path_buf(),
        Mirror::parse(&hf.url).ok(),
        None,
        4096,
        Platform::Linux,
        Some(fake_bin()),
        supervisor,
        configured.map(str::to_owned),
    )
    .with_catalog(catalog)
    .with_accepted_license(accepted.map(str::to_owned))
    .with_retry_waits(vec![Duration::from_millis(100), Duration::from_millis(200)])
}

fn rig_with(accept_license: &'static str, accepted: Option<&str>, configured: Option<&str>) -> Rig {
    let body = payload(20_000);
    let spec = spec(&body, accept_license);
    let hf = FakeHf::start(body);
    let dir = TempDir::new();
    let preparing = Arc::new(AtomicBool::new(false));
    let models = manager(&dir, &hf, spec, accepted, configured, None).with_preparing(Arc::clone(&preparing));
    Rig { hf, dir, spec, preparing, models }
}

fn rig() -> Rig {
    rig_with("", None, Some(ID))
}

#[test]
fn the_configured_model_is_downloaded_in_the_background() {
    let rig = rig();
    rig.hf.slow(2_000, Duration::from_millis(60));
    rig.models.start_auto_install();
    // It returns at once, and the model is on its way.
    assert!(rig.preparing.load(Ordering::SeqCst), "the assistant is told «in preparation»");
    assert_eq!(rig.status().state, "downloading");
    let done = rig.wait_state("ready");
    assert_eq!(done.percent, Some(100));
    assert!(llm_catalog::present(rig.dir.path(), rig.spec));
    llm_catalog::verify(&llm_catalog::model_file(rig.dir.path(), rig.spec), rig.spec, &AtomicBool::new(false)).unwrap();
    wait_until(Duration::from_secs(5), "the end of the preparation", || !rig.preparing.load(Ordering::SeqCst));
}

#[test]
fn a_failure_waits_and_tries_again_until_the_model_is_there() {
    let rig = rig();
    rig.hf.fail_with(500);
    rig.models.start_auto_install();
    // The failure is shown while the server waits for the next attempt; the model stays «in preparation».
    let mut shown = None;
    for _ in 0..400 {
        let status = rig.status();
        if status.state == "downloading" && status.error.is_some() {
            shown = status.error;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let shown = shown.expect("the last error is shown while waiting");
    assert!(!shown.contains("http") && !shown.contains("127.0.0.1"), "{shown}");
    assert!(rig.preparing.load(Ordering::SeqCst));
    let before = rig.hf.seen().len();
    rig.hf.fail_with(0);
    rig.wait_state("ready");
    assert!(rig.hf.seen().len() > before, "it asked again");
    assert!(llm_catalog::present(rig.dir.path(), rig.spec));
    wait_until(Duration::from_secs(5), "the end of the preparation", || !rig.preparing.load(Ordering::SeqCst));
}

#[test]
fn a_partial_file_is_resumed_with_a_range_request() {
    let rig = rig();
    // Where a first run was cut: a part file of the download, which the second continues.
    rig.hf.slow(2_000, Duration::from_millis(40));
    rig.models.start_auto_install();
    std::thread::sleep(Duration::from_millis(250));
    drop(rig.models);
    wait_until(Duration::from_secs(5), "the first run to stop", || !rig.preparing.load(Ordering::SeqCst));
    assert!(!llm_catalog::present(rig.dir.path(), rig.spec));
    let seen = rig.hf.seen().len();
    rig.hf.slow(usize::MAX, Duration::ZERO);
    let again = manager(&rig.dir, &rig.hf, rig.spec, None, Some(ID), None);
    again.start_auto_install();
    let started = std::time::Instant::now();
    while !llm_catalog::present(rig.dir.path(), rig.spec) && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(llm_catalog::present(rig.dir.path(), rig.spec));
    let ranges: Vec<_> = rig.hf.seen().into_iter().skip(seen).filter_map(|s| s.range).collect();
    assert!(ranges.iter().any(|r| r.starts_with("bytes=") && !r.starts_with("bytes=0-")), "{ranges:?}");
}

#[test]
fn without_the_acknowledgement_nothing_is_downloaded_and_the_status_says_why() {
    for accepted in [None, Some("other")] {
        let rig = rig_with("gemma", accepted, Some(ID));
        rig.models.start_auto_install();
        std::thread::sleep(Duration::from_millis(300));
        assert!(rig.hf.seen().is_empty(), "no request: {:?}", rig.hf.seen());
        assert!(!rig.preparing.load(Ordering::SeqCst));
        let status = rig.status();
        assert_eq!(status.state, "missing");
        let error = status.error.expect("the reason");
        assert!(error.contains("QAMILLE_LLM_ACCEPT_LICENSE=gemma"), "{error}");
    }
}

#[test]
fn with_the_acknowledgement_the_model_is_downloaded() {
    let rig = rig_with("gemma", Some("gemma"), Some(ID));
    rig.models.start_auto_install();
    rig.wait_state("ready");
    assert!(llm_catalog::present(rig.dir.path(), rig.spec));
}

#[test]
fn nothing_is_downloaded_without_a_configured_model_or_when_it_is_there() {
    let idle = rig_with("", None, None);
    idle.models.start_auto_install();
    std::thread::sleep(Duration::from_millis(200));
    assert!(idle.hf.seen().is_empty());
    assert!(!idle.preparing.load(Ordering::SeqCst));

    let rig = rig();
    rig.models.start_auto_install();
    rig.wait_state("ready");
    wait_until(Duration::from_secs(5), "the end of the preparation", || !rig.preparing.load(Ordering::SeqCst));
    let seen = rig.hf.seen().len();
    let second = manager(&rig.dir, &rig.hf, rig.spec, None, Some(ID), None);
    second.start_auto_install();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.hf.seen().len(), seen, "the model is there: no request");
}

#[test]
fn while_the_model_is_prepared_the_assistant_answers_in_preparation() {
    let fixture = Fixture::new();
    let supervisor = LlmSupervisor::new(fixture.settings(&[]), Default::default());
    let body = payload(20_000);
    let spec = spec(&body, "");
    let hf = FakeHf::start(body);
    let dir = TempDir::new();
    let models = manager(&dir, &hf, spec, None, Some(ID), Some(Arc::clone(&supervisor))).with_preparing(supervisor.preparing_flag());
    hf.slow(2_000, Duration::from_millis(60));
    models.start_auto_install();
    match supervisor.endpoint() {
        Err(LlmError::Preparing { retry_after_s }) => assert!(retry_after_s > 0),
        other => panic!("expected Preparing, got {:?}", other.map(|_| ())),
    }
    assert_eq!(fixture.reports().len(), 0, "no child was started");
    wait_until(Duration::from_secs(10), "the end of the preparation", || !supervisor.preparing_flag().load(Ordering::SeqCst));
    assert!(llm_catalog::present(dir.path(), spec));
    supervisor.shutdown();
}

#[test]
fn dropping_the_manager_stops_the_download() {
    let rig = rig();
    rig.hf.slow(500, Duration::from_millis(100));
    rig.models.start_auto_install();
    std::thread::sleep(Duration::from_millis(150));
    let Rig { preparing, models, dir, spec, .. } = rig;
    drop(models);
    wait_until(Duration::from_secs(10), "the download to stop", || !preparing.load(Ordering::SeqCst));
    assert!(!llm_catalog::present(dir.path(), spec));
}
