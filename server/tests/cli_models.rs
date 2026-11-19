//! `model list`, `model install <id>` and `model remove <id> --yes` (Task 11): in process with a
//! fake of the installable models, and through the binary for usage errors and for the promise
//! that no listener is opened and the database is not touched.
mod support;

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qamille_server::cli::{ModelCommand, run_model_polling};
use qamille_server::compute::llm_models::{LlmDetails, LlmModels};
use qamille_server::compute::{ComputeError, ComputeState, ModelStatus};
use support::llm_env::{mount, ts_server};
use support::{TempDir, TestServer, test_config};

const POLL: Duration = Duration::from_millis(25);
const SECRET: &str = "hf_SECRET-TOKEN-4471";

fn status(id: &str, state: &str, percent: Option<u8>, error: Option<&str>) -> ModelStatus {
    ModelStatus {
        id: id.to_owned(),
        kind: "llm".to_owned(),
        state: state.to_owned(),
        percent,
        done_bytes: 0,
        total_bytes: 2_500_000_000,
        error: error.map(str::to_owned),
        loaded: false,
        dimensions: 0,
        quantization: "Q4_K_M".to_owned(),
    }
}

/// `llm-medio` installs in a few steps, `llm-rotto` fails, `llm-pronto` is there, `llm-occupato`
/// cannot be removed; `llm-nonfissato` is not available.
struct Models {
    states: Arc<Mutex<HashMap<String, ModelStatus>>>,
    removed: AtomicUsize,
}

impl Models {
    fn new() -> Arc<Models> {
        let mut states = HashMap::new();
        for (id, state, percent) in [
            ("llm-medio", "missing", None),
            ("llm-rotto", "missing", None),
            ("llm-pronto", "ready", Some(100)),
            ("llm-occupato", "ready", Some(100)),
            ("llm-nonfissato", "missing", None),
        ] {
            states.insert(id.to_owned(), status(id, state, percent, None));
        }
        Arc::new(Models { states: Arc::new(Mutex::new(states)), removed: AtomicUsize::new(0) })
    }

    fn state(&self, id: &str) -> String {
        self.states.lock().unwrap()[id].state.clone()
    }
}

impl LlmModels for Models {
    fn models(&self) -> Vec<ModelStatus> {
        let mut all: Vec<ModelStatus> = self.states.lock().unwrap().values().cloned().collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }

    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        let current = self.states.lock().unwrap().get(id).cloned().ok_or(ComputeError::NotFound)?;
        if id == "llm-nonfissato" {
            return Err(ComputeError::InvalidRequest("This model is not available yet: its catalog entry is not pinned.".to_owned()));
        }
        if current.state == "ready" {
            return Ok(current);
        }
        let states = Arc::clone(&self.states);
        let id = id.to_owned();
        let started = status(&id, "downloading", Some(0), None);
        states.lock().unwrap().insert(id.clone(), started.clone());
        std::thread::spawn(move || {
            for percent in [30, 60, 90] {
                std::thread::sleep(Duration::from_millis(60));
                states.lock().unwrap().insert(id.clone(), status(&id, "downloading", Some(percent), None));
            }
            std::thread::sleep(Duration::from_millis(60));
            let end = if id == "llm-rotto" {
                status(&id, "missing", None, Some("The download failed: the server answered 500."))
            } else {
                status(&id, "ready", Some(100), None)
            };
            states.lock().unwrap().insert(id, end);
        });
        Ok(started)
    }

    fn remove(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        if !self.states.lock().unwrap().contains_key(id) {
            return Err(ComputeError::NotFound);
        }
        if id == "llm-occupato" {
            return Err(ComputeError::InvalidRequest("The model is in use; try again when no request is running.".to_owned()));
        }
        self.removed.fetch_add(1, Ordering::SeqCst);
        let missing = status(id, "missing", None, None);
        self.states.lock().unwrap().insert(id.to_owned(), missing.clone());
        Ok(missing)
    }

    fn details(&self, _: &str) -> Option<LlmDetails> {
        Some(LlmDetails {
            ram_required_mb: 9000,
            disk_required_bytes: 2_700_000_000,
            context: 8192,
            license: "Apache-2.0".to_owned(),
            license_url: "https://example.test/terms".to_owned(),
            speed_note: String::new(),
            engine: "ready".to_owned(),
            in_use: false,
            pinned: true,
        })
    }
}

fn state_with(models: &Arc<Models>) -> (ComputeState, TestServer) {
    let ts = TestServer::new(test_config());
    let server = mount(ts_server(&ts), None, Duration::ZERO).with_llm_models(models.clone());
    (server.compute_state().expect("compute state"), ts)
}

fn run(models: &Arc<Models>, command: ModelCommand) -> (u8, String, String) {
    let (state, _ts) = state_with(models);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_model_polling(&state, command, &mut out, &mut err, POLL);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

fn install(id: &str) -> ModelCommand {
    ModelCommand::Install(id.to_owned())
}

fn remove(id: &str, yes: bool) -> ModelCommand {
    ModelCommand::Remove { id: id.to_owned(), yes }
}

fn binary(dir: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_qamille-server"))
        .args(args)
        .env_clear()
        .env("QAMILLE_DATA_DIR", dir.path())
        .current_dir(dir.path())
        .output()
        .expect("binary")
}

#[test]
fn the_help_lists_the_model_commands() {
    let dir = TempDir::new();
    let out = binary(&dir, &["--help"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    for line in ["model list", "model install <id>", "model remove <id> --yes"] {
        assert!(text.contains(line), "{text}");
    }
}

#[test]
fn model_list_prints_one_line_per_model_with_state_and_memory() {
    let models = Models::new();
    let (code, out, err) = run(&models, ModelCommand::List);
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "id	stato	avanzamento	dimensione	memoria richiesta");
    assert_eq!(lines.iter().filter(|l| l.starts_with("llm-")).count(), 5, "{out}");
    let ready = lines.iter().find(|l| l.starts_with("llm-pronto\t")).unwrap();
    assert_eq!(*ready, "llm-pronto\tready\t100%\t2500 MB\t9000 MB");
    let missing = lines.iter().find(|l| l.starts_with("llm-medio\t")).unwrap();
    assert_eq!(*missing, "llm-medio\tmissing\t-\t2500 MB\t9000 MB");
    assert!(err.is_empty(), "{err}");
}

#[test]
fn model_install_waits_for_the_end_and_exits_0() {
    let models = Models::new();
    let (code, out, err) = run(&models, install("llm-medio"));
    assert_eq!(code, 0, "{err}");
    assert_eq!(models.state("llm-medio"), "ready");
    assert!(out.contains("llm-medio: installato"), "{out}");
    let progress: Vec<&str> = err.lines().collect();
    assert!(progress.contains(&"llm-medio: 0%") && progress.contains(&"llm-medio: 60%"), "{err}");
    assert!(progress.iter().all(|l| l.starts_with("llm-medio: ") && l.ends_with('%')), "{err}");
}

#[test]
fn model_install_tells_the_license_and_where_its_terms_are() {
    let models = Models::new();
    let (code, out, err) = run(&models, install("llm-medio"));
    assert_eq!(code, 0, "{err}");
    let first = out.lines().next().unwrap_or_default();
    assert_eq!(first, "llm-medio: licenza Apache-2.0, condizioni: https://example.test/terms", "{out}");
}

#[test]
fn model_install_of_an_already_ready_model_exits_0_at_once() {
    let models = Models::new();
    let (state, _ts) = state_with(&models);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let started = Instant::now();
    let code = run_model_polling(&state, install("llm-pronto"), &mut out, &mut err, Duration::from_secs(30));
    assert_eq!(code, 0);
    assert!(started.elapsed() < Duration::from_secs(5), "it did not wait");
    assert!(err.is_empty());
}

#[test]
fn model_install_failure_exits_1_with_the_message_and_no_url_or_token() {
    let models = Models::new();
    let (code, out, err) = run(&models, install("llm-rotto"));
    assert_eq!(code, 1);
    assert!(err.contains("The download failed"), "{err}");
    // The terms of the licence are public and shown on purpose; nothing else is an address.
    let out: String = out.lines().filter(|line| !line.contains(": licenza ")).collect();
    for text in [&out, &err] {
        assert!(!text.contains("http") && !text.contains("127.0.0.1") && !text.contains(SECRET), "{text}");
    }
    assert_eq!(models.state("llm-rotto"), "missing");
}

#[test]
fn model_install_of_an_unknown_or_unpinned_id_exits_1() {
    let models = Models::new();
    for id in ["llm-inesistente", "llm-nonfissato", "embeddinggemma-2"] {
        let (code, out, err) = run(&models, install(id));
        assert_eq!(code, 1, "{id}: {err}");
        assert!(out.is_empty(), "{out}");
        assert!(err.starts_with("errore: "), "{err}");
    }
}

#[test]
fn model_remove_needs_yes() {
    let models = Models::new();
    let (code, out, err) = run(&models, remove("llm-pronto", false));
    assert_eq!(code, 2, "{err}");
    assert!(out.is_empty());
    assert!(err.contains("--yes"), "{err}");
    assert_eq!(models.removed.load(Ordering::SeqCst), 0);
    assert_eq!(models.state("llm-pronto"), "ready");
}

#[test]
fn model_remove_deletes_and_prints_the_status() {
    let models = Models::new();
    let (code, out, err) = run(&models, remove("llm-pronto", true));
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "llm-pronto\tmissing\n");
    assert_eq!(models.state("llm-pronto"), "missing");
    let (code, _, err) = run(&models, remove("llm-inesistente", true));
    assert_eq!(code, 1, "{err}");
}

#[test]
fn model_remove_of_a_model_in_use_exits_1() {
    let models = Models::new();
    let (code, out, err) = run(&models, remove("llm-occupato", true));
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(err.contains("in use"), "{err}");
    assert_eq!(models.state("llm-occupato"), "ready");
}

#[test]
fn model_commands_open_no_listener() {
    let dir = TempDir::new();
    let first = binary(&dir, &["model", "list"]);
    assert_eq!(first.status.code(), Some(0), "{}", String::from_utf8_lossy(&first.stderr));
    let text = String::from_utf8_lossy(&first.stdout).into_owned();
    for id in ["llm-piccolo", "llm-medio", "llm-grande"] {
        assert!(text.lines().any(|l| l.starts_with(&format!("{id}\t"))), "{text}");
    }
    // It returned, so nothing was left listening. The database keeps its content (opening a
    // WAL file may rewrite its header, so the rows are compared, not the bytes) and no file appears.
    let names = || {
        let mut names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    };
    let before = names();
    let second = binary(&dir, &["model", "list"]);
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(names(), before);
    assert!(second.stdout == first.stdout, "the second listing differs");
}

#[test]
fn model_with_a_missing_argument_is_a_usage_error() {
    let dir = TempDir::new();
    for args in [&["model"][..], &["model", "install"], &["model", "remove"], &["model", "frobnicate"], &["model", "install", "a", "b"], &["model", "list", "x"]] {
        let out = binary(&dir, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        assert!(out.stdout.is_empty());
    }
    // Without --yes nothing is removed and the exit is a usage error too.
    let out = binary(&dir, &["model", "remove", "llm-medio"]);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
}
