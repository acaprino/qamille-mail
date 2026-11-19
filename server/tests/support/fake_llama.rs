//! Support for the tests of the built-in LLM supervisor: the fake `llama-server`
//! (`src/bin/fake-llama-server.rs`), a model file, and the reports the fake writes.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qamille_server::llm_builtin::{BuiltinSettings, LlmSupervisor, LoopbackPorts, MemoryProbe, PortPicker, SupervisorDeps};
use serde_json::Value;

use super::TempDir;

pub fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-llama-server"))
}

/// A model file and a folder for the reports of the fake.
pub struct Fixture {
    pub dir: TempDir,
    pub model: PathBuf,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = TempDir::new();
        let model = dir.path().join("model.gguf");
        std::fs::write(&model, b"GGUF fake").unwrap();
        Fixture { dir, model }
    }

    /// The pattern the fake expands with its own pid: one report per process.
    pub fn report_pattern(&self) -> String {
        self.dir.path().join("report-{pid}.json").to_string_lossy().into_owned()
    }

    /// Settings for the fake program, with `extra` arguments for the fake (after the real ones).
    pub fn settings(&self, extra: &[&str]) -> BuiltinSettings {
        let program = fake_bin();
        let model = self.model.clone();
        let mut settings = BuiltinSettings::new(
            Arc::new(move || Ok(program.clone())),
            Arc::new(move || Ok(model.clone())),
            "llm-medio".to_owned(),
            4096,
            1,
            Duration::from_secs(600),
            0,
        );
        settings.ready_timeout = Duration::from_secs(30);
        settings.poll = Duration::from_millis(25);
        let mut args: Vec<OsString> = vec!["--fake-report".into(), self.report_pattern().into()];
        args.extend(extra.iter().map(OsString::from));
        settings.extra_args = args;
        settings
    }

    pub fn supervisor(&self, extra: &[&str]) -> Arc<LlmSupervisor> {
        LlmSupervisor::new(self.settings(extra), SupervisorDeps::default())
    }

    /// Every report written so far, one per process the fake ran, oldest first by pid.
    pub fn reports(&self) -> Vec<Value> {
        let mut out: Vec<Value> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("report-"))
            .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
            .collect();
        out.sort_by_key(|v| v["pid"].as_u64());
        out
    }

    /// The report of the only process the fake ran.
    pub fn only_report(&self) -> Value {
        let mut reports = self.reports();
        assert_eq!(reports.len(), 1, "{reports:?}");
        reports.remove(0)
    }
}

/// The processes the tests started must not outlive them.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        qamille_server::process::parent_alive(pid)
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
    }
}

/// Waits until `check` holds, or fails after `limit`.
pub fn wait_until(limit: Duration, what: &str, mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(start.elapsed() < limit, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The ports a test hands out first, then the free ones.
pub struct PortSequence {
    first: Mutex<VecDeque<u16>>,
    pub picked: Mutex<Vec<u16>>,
}

impl PortSequence {
    pub fn new(first: &[u16]) -> Arc<PortSequence> {
        Arc::new(PortSequence { first: Mutex::new(first.iter().copied().collect()), picked: Mutex::default() })
    }
}

impl PortPicker for PortSequence {
    fn pick(&self) -> u16 {
        let port = self.first.lock().unwrap().pop_front().unwrap_or_else(|| LoopbackPorts.pick());
        self.picked.lock().unwrap().push(port);
        port
    }
}

/// Free memory a test can change while the supervisor runs.
pub struct Memory(Mutex<Option<u64>>);

impl Memory {
    pub fn new(value: Option<u64>) -> Arc<Memory> {
        Arc::new(Memory(Mutex::new(value)))
    }

    pub fn set(&self, value: Option<u64>) {
        *self.0.lock().unwrap() = value;
    }
}

impl MemoryProbe for Memory {
    fn available_mb(&self) -> Option<u64> {
        *self.0.lock().unwrap()
    }
}

pub fn deps_with_ports(ports: Arc<dyn PortPicker>) -> SupervisorDeps {
    SupervisorDeps { ports, ..SupervisorDeps::default() }
}

/// A listener that answers every request with `status` and records what it saw
/// (`method path authorization`), or one that drops every connection when `status` is 0.
pub struct Listener {
    pub port: u16,
    pub seen: Arc<Mutex<Vec<String>>>,
}

pub fn listener(status: u16) -> Listener {
    listener_by(move |_| status)
}

/// A listener that answers `401` without a bearer key and `200` with any: what a
/// `llama-server` started with a key looks like, but not our child.
pub fn keyed_listener() -> Listener {
    listener_by(|authorization| if authorization.contains("Bearer ") { 200 } else { 401 })
}

fn listener_by(status_for: impl Fn(&str) -> u16 + Send + Sync + 'static) -> Listener {
    let status_for = Arc::new(status_for);
    use std::io::{BufRead, Write};
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in socket.incoming().flatten() {
            let log = log.clone();
            let status_for = status_for.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).unwrap_or(0) == 0 {
                    return;
                }
                let mut authorization = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim_end().is_empty() {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        authorization = line.trim_end().to_owned();
                    }
                }
                log.lock().unwrap().push(format!("{} {authorization}", first.trim_end()));
                let status = status_for(&authorization);
                if status == 0 {
                    return;
                }
                let reply = format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
                let _ = stream.write_all(reply.as_bytes());
            });
        }
    });
    Listener { port, seen }
}

/// A shared HTTP agent for the tests: no proxy, statuses are not errors.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().proxy(None).http_status_as_error(false).timeout_global(Some(Duration::from_secs(10))).build().into()
}

/// POSTs `body` to `url` with an optional bearer key; the status.
pub fn post(url: &str, key: Option<&str>, body: &str) -> u16 {
    let mut request = agent().post(url).header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Authorization", &format!("Bearer {key}"));
    }
    request.send(body).map(|r| r.status().as_u16()).unwrap_or(0)
}

pub fn port_of(url: &str) -> u16 {
    url.rsplit(':').next().unwrap().split('/').next().unwrap().parse().unwrap()
}

pub fn dir_of(path: &Path) -> &Path {
    path.parent().unwrap()
}
