//! The real thing, on request (Task 14): a real `llama-server` and a real model of the catalog.
//!
//! ```text
//! cargo test --manifest-path server/Cargo.toml --jobs 2 --test llm_real_model -- --ignored --nocapture
//! ```
//!
//! Needs a PC with network (the model is downloaded if it is not there) and about 6 GB free for
//! `llm-piccolo` (Gemma 4 E4B, 5.2 GB). Environment: `QAMILLE_REAL_LLM_MODELS_DIR` (default: a folder in the system
//! temporary folder, kept between runs), `QAMILLE_REAL_LLM_ID` (default `llm-piccolo`) and, on Linux,
//! `QAMILLE_REAL_LLAMA_PROGRAM` (absolute path of `llama-server`); on Windows x64 the verified bundle
//! is downloaded into the models folder. It also confirms or corrects what the engine pins
//! assume: `/health` is public, `POST /tokenize` wants the key, and the answer has no reasoning text (Gemma 4 thinks only when the
//! system prompt asks for it with `<|think|>`; the engine's `--jinja` keeps its channel tags out of `content`).
mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use qamille_compute::download::Options;
use qamille_compute::{llama, llm_catalog};
use qamille_server::llm_builtin::{BuiltinSettings, LlmSupervisor, SupervisorDeps, SupervisorState};
use support::fake_llama::{pid_alive, wait_until};

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(600))).proxy(None).build().into()
}

#[test]
#[ignore = "needs network, a real engine and about 6 GB; run with --ignored"]
fn the_smallest_real_model_answers_and_unloads_when_idle() {
    let id = std::env::var("QAMILLE_REAL_LLM_ID").unwrap_or_else(|_| "llm-piccolo".to_owned());
    let spec = llm_catalog::spec(&id).unwrap_or_else(|| panic!("{id} is not in the catalog"));
    let models_dir: PathBuf = std::env::var_os("QAMILLE_REAL_LLM_MODELS_DIR").map_or_else(|| std::env::temp_dir().join("qamille-real-llm"), PathBuf::from);
    std::fs::create_dir_all(&models_dir).unwrap();
    let cancel = AtomicBool::new(false);
    let options = Options::resumable();

    // The engine: the verified bundle on Windows, the given program on Linux.
    let runtime_dir = llama::runtime_dir(&models_dir);
    if llama::bundled_runtime() {
        llama::install_runtime(&runtime_dir, &options, &cancel, &|_| {}).expect("the engine bundle");
    }
    let setting = std::env::var_os("QAMILLE_REAL_LLAMA_PROGRAM").map(PathBuf::from);
    let program_setting = setting.clone();
    let program_dir = runtime_dir.clone();
    llama::resolve_program(llama::current_platform(), setting.as_deref(), &runtime_dir).expect("the engine program");

    // The model: installed with the real catalog if it is missing, checked in full once.
    llm_catalog::install(&models_dir, spec, &options, &cancel, &|done, total| {
        eprintln!("download: {done}/{total}");
    })
    .expect("the model");
    let model_path = llm_catalog::model_file(&models_dir, spec);
    llm_catalog::verify(&model_path, spec, &cancel).expect("the SHA-256 of the model");

    let settings = BuiltinSettings::new(
        Arc::new(move || llama::resolve_program(llama::current_platform(), program_setting.as_deref(), &program_dir)),
        Arc::new(move || Ok(model_path.clone())),
        id.clone(),
        4096,
        1,
        Duration::from_secs(3),
        0,
    );
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());

    let endpoint = supervisor.endpoint().expect("the engine starts and is ready");
    let pid = supervisor.pid().expect("pid");
    let base = endpoint.chat_url().trim_end_matches("/v1/chat/completions").to_owned();
    let agent = agent();

    // What the pins assume: `/health` is public, `/tokenize` wants the key.
    let health = agent.get(&format!("{base}/health")).call().expect("/health");
    assert_eq!(health.status().as_u16(), 200, "/health is public");
    let no_key = agent.post(&format!("{base}/tokenize")).header("Content-Type", "application/json").send(&br#"{"content":"ok"}"#[..]).expect("/tokenize without key");
    assert_eq!(no_key.status().as_u16(), 401, "/tokenize without the key is refused");
    let with_key = agent
        .post(&format!("{base}/tokenize"))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", endpoint.key().expose()))
        .send(&br#"{"content":"ok"}"#[..])
        .expect("/tokenize with key");
    assert_eq!(with_key.status().as_u16(), 200);

    // A real answer.
    let body = serde_json::json!({
        "model": id,
        "messages": [{"role": "user", "content": "Rispondi solo con la parola ok"}],
        "max_tokens": 64,
    });
    let mut response = agent
        .post(&endpoint.chat_url())
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", endpoint.key().expose()))
        .send(&serde_json::to_vec(&body).unwrap()[..])
        .expect("chat");
    assert_eq!(response.status().as_u16(), 200);
    let answer: serde_json::Value = serde_json::from_slice(&response.body_mut().read_to_vec().unwrap()).unwrap();
    let content = answer["choices"][0]["message"]["content"].as_str().unwrap_or("").to_owned();
    eprintln!("answer: {content:?}");
    assert!(!content.trim().is_empty(), "an empty answer: {answer}");
    assert!(!content.contains("<think>") && !content.contains("<|channel>") && !content.contains("<channel|>") && answer["choices"][0]["message"].get("reasoning_content").is_none_or(|v| v.is_null() || v == ""), "reasoning text in the answer: {answer}");

    // Idle stop: with the request over, the child leaves within the idle time.
    drop(endpoint);
    wait_until(Duration::from_secs(30), "the idle stop", || supervisor.state() == SupervisorState::Stopped);
    assert!(!pid_alive(pid), "no process is left");
    supervisor.shutdown();
}
