//! The supervisor of the built-in LLM child (`llm_builtin.rs`) against the fake `llama-server`:
//! arguments, key, environment, readiness, impostors, logs (INV-34, INV-40; Review Focus 1 to 5, 10).
mod support;

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use qamille_server::llm::LlmError;
use qamille_server::llm_builtin::{ChildKey, LlmSupervisor, REQUIRED_FLAGS, SupervisorDeps, SupervisorState, child_args};
use support::fake_llama::*;

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

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// One subscriber for the whole file (the supervisor logs from its own threads).
fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing_subscriber::fmt().with_writer(capture.clone()).with_max_level(tracing::Level::TRACE).with_ansi(false).init();
        capture
    })
}

fn strings(report: &serde_json::Value, key: &str) -> Vec<String> {
    report[key].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
}

fn unavailable(error: &LlmError) -> bool {
    matches!(error, LlmError::Unavailable { .. })
}

// --- Arguments, key, environment ----------------------------------------------------

#[test]
fn the_arguments_are_exactly_the_expected_flags() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    let report = fixture.only_report();
    let port = report["port"].as_u64().unwrap().to_string();
    let model = fixture.model.to_string_lossy().into_owned();
    let argv = strings(&report, "argv");
    let expected = [
        "--model", &model, "--host", "127.0.0.1", "--port", &port, "--ctx-size", "4096", "--parallel", "1", "--cache-ram", "0", "--no-slots", "--jinja",
        "--no-webui", "--log-verbosity", "1",
    ];
    assert_eq!(argv[..expected.len()], expected, "{argv:?}");
    // The fake refuses unknown arguments (exit 2): the program started, so every flag is known.
    for forbidden in ["--api-key", "--api-key-file", "--verbose", "--log-file", "--metrics", "--props", "--threads", "-t"] {
        assert!(!argv.iter().any(|a| a == forbidden), "{forbidden}");
    }
    let built: Vec<String> = child_args(&fixture.model, port.parse().unwrap(), 4096, 1).iter().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(built, expected);
    for flag in REQUIRED_FLAGS {
        assert!(built.iter().any(|a| a == flag), "{flag} is checked in the Dockerfile but not passed");
    }
    drop(endpoint);
}

#[test]
fn ctx_size_is_the_context_times_the_slots() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.context = 4096;
    settings.slots = 3;
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let _endpoint = supervisor.endpoint().unwrap();
    let argv = strings(&fixture.only_report(), "argv");
    let at = |flag: &str| argv.iter().position(|a| a == flag).map(|i| argv[i + 1].clone());
    assert_eq!(at("--ctx-size").as_deref(), Some("12288"));
    assert_eq!(at("--parallel").as_deref(), Some("3"));
}

#[test]
fn the_key_is_not_in_the_child_arguments() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    let key = endpoint.key().expose().to_owned();
    let report_text = std::fs::read_to_string(fixture.dir.path().join(format!("report-{}.json", fixture.only_report()["pid"]))).unwrap();
    assert!(!report_text.contains(&key), "the key is not in argv, nor in the names of the environment");
    assert!(!fixture.report_pattern().contains(&key));
}

#[test]
fn the_child_gets_only_its_own_environment() {
    // SAFETY: names no other test reads; they stay set for the process.
    unsafe {
        std::env::set_var("QAMILLE_ADMIN_KEY", "chiave-admin-di-prova-0123456789abcdef");
        std::env::set_var("QAMILLE_LLM_API_KEY", "chiave-del-provider-di-prova");
        std::env::set_var("QAMILLE_HF_TOKEN", "hf_token_di_prova");
    }
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let _endpoint = supervisor.endpoint().unwrap();
    let mut names = strings(&fixture.only_report(), "env");
    names.sort_by_key(|n| n.to_ascii_lowercase());
    let expected: &[&str] = if cfg!(windows) { &["LLAMA_API_KEY", "SystemRoot"] } else { &["LLAMA_API_KEY"] };
    let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let want: Vec<String> = expected.iter().map(|n| n.to_ascii_lowercase()).collect();
    assert_eq!(lower, want, "{names:?}");
}

#[test]
fn the_key_is_32_random_bytes_in_hex_and_fresh_for_every_start() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let first = supervisor.endpoint().unwrap().key().expose().to_owned();
    assert_eq!(first.len(), 64);
    assert!(first.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')), "{first}");
    supervisor.stop();
    let second = supervisor.endpoint().unwrap().key().expose().to_owned();
    assert_ne!(first, second);
    assert_eq!(format!("{:?}", ChildKey::generate()), "ChildKey(<nascosta>)");
}

#[test]
fn the_child_receives_the_same_key_the_supervisor_uses() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    let key = endpoint.key().expose().to_owned();
    let url = endpoint.chat_url();
    assert_eq!(post(&url, Some(&key), r#"{"messages":[]}"#), 200);
    let mut off = key.clone();
    off.pop();
    off.push(if key.ends_with('0') { '1' } else { '0' });
    assert_eq!(post(&url, Some(&off), "{}"), 401, "a key one character away");
    assert_eq!(post(&url, None, "{}"), 401);
}

#[test]
fn the_child_listens_on_loopback_only() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    assert_eq!(fixture.only_report()["host"], "127.0.0.1");
    assert!(endpoint.chat_url().starts_with("http://127.0.0.1:"));
    // A non-loopback address of this machine (no packet is sent by connecting a UDP socket).
    let local = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect("192.0.2.1:9").map(|_| s)).and_then(|s| s.local_addr());
    if let Some(addr) = local.ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) {
        let port = port_of(&endpoint.chat_url());
        assert!(std::net::TcpStream::connect_timeout(&(addr, port).into(), Duration::from_secs(1)).is_err(), "reachable on {addr}");
    }
}

// --- Readiness and impostors --------------------------------------------------------

#[test]
fn ready_needs_the_child_alive_and_the_key_to_work() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-never-ready"]);
    settings.ready_timeout = Duration::from_millis(1500);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let waiter = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || supervisor.endpoint().err())
    };
    wait_until(Duration::from_secs(5), "the child to be starting", || supervisor.state() == SupervisorState::Starting);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(supervisor.state(), SupervisorState::Starting, "alive but not listening is not ready");
    assert!(waiter.join().unwrap().is_some_and(|e| unavailable(&e)));
}

#[test]
fn a_listener_that_answers_without_a_key_is_never_ready() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&["--fake-public-all"]);
    let error = supervisor.endpoint().expect_err("never ready");
    assert!(unavailable(&error), "{error:?}");
    assert_ne!(supervisor.state(), SupervisorState::Ready);
    assert_eq!(supervisor.pid(), None, "the child was stopped");
    let reports = fixture.reports();
    assert!(!reports.is_empty());
    for report in &reports {
        assert!(!pid_alive(report["pid"].as_u64().unwrap() as u32), "left running");
        for request in report["requests"].as_array().unwrap() {
            assert_eq!(request["auth"], false, "the key went to a listener that does not ask for it: {request}");
        }
    }
}

#[test]
fn an_impostor_on_the_port_gets_no_prompt() {
    for status in [200_u16, 401] {
        let impostor = listener(status);
        let fixture = Fixture::new();
        let ports = PortSequence::new(&[impostor.port]);
        let supervisor = LlmSupervisor::new(fixture.settings(&[]), deps_with_ports(ports.clone()));
        let endpoint = supervisor.endpoint().unwrap_or_else(|e| panic!("status {status}: {e:?}"));
        let key = endpoint.key().expose().to_owned();
        assert_eq!(post(&endpoint.chat_url(), Some(&key), r#"{"messages":[]}"#), 200);
        assert_ne!(port_of(&endpoint.chat_url()), impostor.port, "the final child is on another port");
        let seen = impostor.seen.lock().unwrap().join("\n");
        assert!(!seen.contains("POST /v1/chat/completions"), "the impostor saw a prompt: {seen}");
        assert!(!seen.contains(&key), "the impostor saw the key of the real child");
    }
}

#[test]
fn an_impostor_that_checks_the_key_is_not_ready_while_the_child_dies() {
    let impostor = keyed_listener();
    let fixture = Fixture::new();
    let ports = PortSequence::new(&[impostor.port]);
    // The child stays alive for a moment (it never binds) and then exits: the port is the impostor's.
    let mut settings = fixture.settings(&["--fake-never-ready", "--fake-die-after-ms", "200"]);
    settings.backoff = vec![Duration::from_secs(5)];
    let supervisor = LlmSupervisor::new(settings, deps_with_ports(ports.clone()));
    let first = supervisor.endpoint();
    assert!(first.is_err(), "ready on a port held by an impostor while the child died");
    assert!(
        impostor.seen.lock().unwrap().iter().all(|line| !line.contains("/v1/chat/completions")),
        "the impostor saw a prompt"
    );
}

#[test]
fn stop_returns_when_it_is_called_during_a_failing_start() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.model = Arc::new(|| {
        std::thread::sleep(Duration::from_millis(400));
        Err("damaged".to_owned())
    });
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let waiter = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || supervisor.endpoint().err())
    };
    wait_until(Duration::from_secs(5), "the start to begin", || supervisor.state() == SupervisorState::Starting);
    let (done, finished) = std::sync::mpsc::channel();
    {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || {
            supervisor.stop();
            supervisor.shutdown();
            let _ = done.send(());
        });
    }
    assert!(finished.recv_timeout(Duration::from_secs(10)).is_ok(), "stop() hangs after a failed start");
    assert!(waiter.join().unwrap().is_some());
}

#[test]
fn a_port_conflict_retries_on_another_port() {
    let held: Vec<_> = (0..4).map(|_| listener(0)).collect();
    let fixture = Fixture::new();
    let ports = PortSequence::new(&[held[0].port, held[1].port, held[2].port]);
    let supervisor = LlmSupervisor::new(fixture.settings(&[]), deps_with_ports(ports.clone()));
    let _endpoint = supervisor.endpoint().expect("the fourth port is free");
    assert_eq!(ports.picked.lock().unwrap().len(), 4);
    // Four conflicts in a row: the start fails.
    let fixture = Fixture::new();
    let ports = PortSequence::new(&held.iter().map(|l| l.port).collect::<Vec<_>>());
    let supervisor = LlmSupervisor::new(fixture.settings(&[]), deps_with_ports(ports.clone()));
    let error = supervisor.endpoint().expect_err("four conflicts");
    assert!(unavailable(&error), "{error:?}");
    assert_eq!(ports.picked.lock().unwrap().len(), 4, "no fifth attempt");
}

// --- Output of the child ------------------------------------------------------------

#[test]
fn the_child_output_is_discarded() {
    let log = capture();
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&["--fake-log-prompt", "--fake-echo"]);
    let endpoint = supervisor.endpoint().unwrap();
    let key = endpoint.key().expose().to_owned();
    let body = r#"{"messages":[{"role":"user","content":"PROMPT-RICONOSCIBILE-7731"}]}"#;
    assert_eq!(post(&endpoint.chat_url(), Some(&key), body), 200);
    std::thread::sleep(Duration::from_millis(300));
    let text = log.text();
    assert!(!text.contains("PROMPT-RICONOSCIBILE-7731"), "{text}");
    assert!(!text.contains(&key));
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("finto")).collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("WARN"), "{}", lines[0]);
    assert!(lines[0].len() < 400, "short: {}", lines[0]);
}

#[test]
fn startup_error_lines_are_capped() {
    let log = capture();
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&["--fake-error-lines", "100"]);
    let _endpoint = supervisor.endpoint().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let count = log.text().lines().filter(|l| l.contains("riga di avvio numero")).count();
    assert!((1..=32).contains(&count), "{count} startup lines in the log");
}

// --- Deadline, concurrency, checks --------------------------------------------------

#[test]
fn a_child_that_never_gets_ready_is_stopped_after_the_deadline() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-never-ready"]);
    settings.ready_timeout = Duration::from_secs(1);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let waiter = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || supervisor.endpoint().err())
    };
    let mut pid = None;
    wait_until(Duration::from_secs(5), "a pid", || {
        pid = pid.or(supervisor.pid());
        pid.is_some()
    });
    let error = waiter.join().unwrap().expect("not ready");
    assert!(matches!(error, LlmError::Unavailable { retry_after_s: 30 }), "{error:?}");
    assert!(matches!(supervisor.state(), SupervisorState::Stopped | SupervisorState::Backoff { .. }), "{:?}", supervisor.state());
    wait_until(Duration::from_secs(5), "the process to be gone", || !pid_alive(pid.unwrap()));
}

#[test]
fn concurrent_first_requests_start_one_child() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let supervisor = supervisor.clone();
            std::thread::spawn(move || supervisor.endpoint().unwrap())
        })
        .collect();
    let endpoints: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(fixture.reports().len(), 1, "one child");
    assert!(endpoints.iter().all(|e| e.chat_url() == endpoints[0].chat_url()));
    assert_eq!(supervisor.in_flight(), 8);
    drop(endpoints);
    assert_eq!(supervisor.in_flight(), 0);
}

#[test]
fn the_program_is_resolved_and_checked_at_every_start() {
    let fixture = Fixture::new();
    let calls = Arc::new(AtomicU32::new(0));
    let mut settings = fixture.settings(&[]);
    let program = fake_bin();
    let counter = calls.clone();
    settings.program = Arc::new(move || {
        if counter.fetch_add(1, Ordering::SeqCst) < 2 {
            Ok(program.clone())
        } else {
            Err(qamille_compute::llama::LlamaError::Invalid("the program changed".to_owned()))
        }
    });
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    drop(supervisor.endpoint().unwrap());
    supervisor.stop();
    drop(supervisor.endpoint().unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 2, "two starts, two checks");
    supervisor.stop();
    let processes = fixture.reports().len();
    let error = supervisor.endpoint().expect_err("the third check fails");
    assert!(unavailable(&error), "{error:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.reports().len(), processes, "no process for a program that fails its check");
    assert!(!format!("{error:?} {error}").contains("changed"), "the text of the failure stays in the server");
}

#[test]
fn the_model_check_runs_at_every_start_and_its_failure_is_a_pause() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.model = Arc::new(|| Err("/percorso/segreto/del/modello.gguf is damaged".to_owned()));
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let error = supervisor.endpoint().expect_err("a damaged model");
    assert!(unavailable(&error), "{error:?}");
    assert!(fixture.reports().is_empty(), "no process");
    let text = format!("{error:?} {error}");
    assert!(!text.contains("segreto") && !text.contains("damaged"), "{text}");
}

#[test]
fn the_endpoint_and_the_supervisor_debug_hide_the_key_and_the_port() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    let key = endpoint.key().expose().to_owned();
    let port = port_of(&endpoint.chat_url()).to_string();
    for text in [format!("{endpoint:?}"), format!("{endpoint:#?}"), format!("{supervisor:?}"), format!("{:?}", endpoint.key())] {
        assert!(!text.contains(&key), "{text}");
        assert!(!text.contains(&port), "{text}");
    }
}

// --- Task 8: memory, idle stop, restarts, no orphans ---------------------------------

use qamille_server::llm_builtin::BuiltinSettings;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn counted_program(settings: &mut BuiltinSettings) -> Arc<AtomicU32> {
    let calls = Arc::new(AtomicU32::new(0));
    let counter = calls.clone();
    let program = fake_bin();
    settings.program = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(program.clone())
    });
    calls
}

fn deps_with_memory(memory: Arc<Memory>) -> SupervisorDeps {
    SupervisorDeps { memory, ..SupervisorDeps::default() }
}

#[test]
fn not_enough_memory_is_refused_before_spawning() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.required_mb = 1000;
    let memory = Memory::new(Some(100));
    let supervisor = LlmSupervisor::new(settings, deps_with_memory(memory.clone()));
    let error = supervisor.endpoint().expect_err("not enough memory");
    assert_eq!(error, LlmError::NotEnoughMemory { retry_after_s: 30 });
    assert!(fixture.reports().is_empty(), "no process");
    assert_eq!(supervisor.pid(), None);
    assert!(matches!(supervisor.state(), SupervisorState::Stopped), "a memory pause is not a crash: {:?}", supervisor.state());
    memory.set(Some(5000));
    let _endpoint = supervisor.endpoint().expect("memory is free now");
    assert_eq!(fixture.reports().len(), 1);
}

#[test]
fn unknown_memory_does_not_block_the_start() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.required_mb = 1_000_000;
    let supervisor = LlmSupervisor::new(settings, deps_with_memory(Memory::new(None)));
    let _endpoint = supervisor.endpoint().expect("unknown memory: start and warn");
}

#[test]
fn the_required_memory_uses_the_context_times_the_slots_plus_15_percent() {
    use qamille_compute::llm_catalog::{CATALOG, required_ram_mb};
    for spec in CATALOG {
        for (context, slots) in [(8192_u32, 1_u32), (4096, 3)] {
            let total = u64::from(context * slots);
            let by_hand = (u64::from(spec.ram_mb) + u64::from(spec.kv_mb_per_1k) * total / 1024) * 115 / 100;
            assert_eq!(required_ram_mb(spec, context * slots), by_hand, "{} {context}x{slots}", spec.id);
        }
    }
}

#[test]
fn idle_stop_unloads_the_child_after_the_idle_time() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.idle = Duration::from_secs(1);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let endpoint = supervisor.endpoint().unwrap();
    let pid = supervisor.pid().unwrap();
    drop(endpoint);
    let released = Instant::now();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(supervisor.state(), SupervisorState::Ready, "not before the idle time");
    wait_until(Duration::from_secs(6), "the idle stop", || supervisor.state() == SupervisorState::Stopped);
    assert!(released.elapsed() >= Duration::from_millis(900), "{:?}", released.elapsed());
    wait_until(Duration::from_secs(5), "the process to be gone", || !pid_alive(pid));
    assert_eq!(supervisor.pid(), None);
}

#[test]
fn a_request_in_flight_keeps_the_child_alive_past_the_idle_time() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.idle = Duration::from_secs(1);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let endpoint = supervisor.endpoint().unwrap();
    let pid = supervisor.pid().unwrap();
    std::thread::sleep(Duration::from_millis(2300));
    assert_eq!(supervisor.state(), SupervisorState::Ready);
    assert!(pid_alive(pid), "a long answer is never cut by the idle stop");
    drop(endpoint);
    wait_until(Duration::from_secs(6), "the idle stop after the release", || supervisor.state() == SupervisorState::Stopped);
}

#[test]
fn the_next_request_after_an_idle_stop_starts_again() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&[]);
    settings.idle = Duration::from_secs(1);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let first = supervisor.endpoint().unwrap();
    let (pid, key) = (supervisor.pid().unwrap(), first.key().expose().to_owned());
    drop(first);
    wait_until(Duration::from_secs(6), "the idle stop", || supervisor.state() == SupervisorState::Stopped);
    let second = supervisor.endpoint().unwrap();
    assert_ne!(supervisor.pid().unwrap(), pid);
    assert_ne!(second.key().expose(), key);
}

#[test]
fn the_default_waits_are_5_10_30_60() {
    let fixture = Fixture::new();
    let defaults = fixture.settings(&[]);
    let fresh = BuiltinSettings::new(defaults.program.clone(), defaults.model.clone(), "m".into(), 8192, 1, Duration::from_secs(600), 0);
    assert_eq!(fresh.backoff, [5, 10, 30, 60].map(Duration::from_secs));
    assert_eq!(fresh.ready_timeout, Duration::from_secs(120));
    assert_eq!(fresh.stable_after, Duration::from_secs(120));
}

fn retry_of(error: &LlmError) -> u64 {
    match error {
        LlmError::Unavailable { retry_after_s } => *retry_after_s,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_crash_pauses_the_requests_with_the_growing_waits() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-crash-on-start"]);
    settings.backoff = [200, 400, 600, 800].map(Duration::from_millis).to_vec();
    settings.port_retries = 0;
    // Slower than the exit of the fake, so no probe goes to a port nobody listens on.
    settings.poll = Duration::from_millis(120);
    // The instant of every start: the program is checked once per start.
    let starts: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let log = starts.clone();
    let program = fake_bin();
    settings.program = Arc::new(move || {
        log.lock().unwrap().push(Instant::now());
        Ok(program.clone())
    });
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let count = || starts.lock().unwrap().len();
    let waits = [200_u64, 400, 600, 800, 800];
    let error = supervisor.endpoint().expect_err("the child crashes at once");
    assert!(retry_of(&error) >= 1);
    let mut failed_at = Instant::now();
    for wait in waits {
        let pause = Duration::from_millis(wait);
        assert!(matches!(supervisor.state(), SupervisorState::Backoff { retry_after_s } if retry_after_s >= 1) || failed_at.elapsed() >= pause, "{:?}", supervisor.state());
        // Inside the pause every request is refused at once and nothing is started.
        let before = count();
        while failed_at.elapsed() + Duration::from_millis(80) < pause {
            assert!(supervisor.endpoint().is_err());
            assert_eq!(count(), before, "no start during the pause");
            std::thread::sleep(Duration::from_millis(15));
        }
        // After it a request tries again (and fails again, opening the next pause).
        let waited = Instant::now();
        while count() == before {
            let _ = supervisor.endpoint();
            assert!(waited.elapsed() < Duration::from_secs(30), "never retried");
        }
        failed_at = Instant::now();
    }
    let starts = starts.lock().unwrap();
    assert_eq!(starts.len(), 1 + waits.len());
    // Only the lower bound: starting a process takes seconds under load, so a late start proves nothing.
    for (i, wait) in waits.iter().enumerate() {
        let gap = starts[i + 1].duration_since(starts[i]);
        assert!(gap >= Duration::from_millis(*wait), "start {} came {gap:?} after the previous one, wait {wait} ms", i + 1);
    }
}

#[test]
fn requests_during_the_pause_get_the_remaining_seconds() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-crash-on-start"]);
    settings.backoff = vec![Duration::from_secs(4)];
    settings.port_retries = 0;
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let _ = supervisor.endpoint();
    let first = retry_of(&supervisor.endpoint().expect_err("paused"));
    assert!((3..=4).contains(&first), "{first}");
    std::thread::sleep(Duration::from_millis(1500));
    let later = retry_of(&supervisor.endpoint().expect_err("paused"));
    assert!((1..=3).contains(&later) && later < first, "{later} after {first}");
}

#[test]
fn a_request_cut_by_a_crash_is_a_pause_and_not_an_attempt() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-exit-after", "1"]);
    settings.backoff = vec![Duration::from_millis(300), Duration::from_secs(8)];
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let endpoint = supervisor.endpoint().unwrap();
    let key = endpoint.key().expose().to_owned();
    assert_eq!(post(&endpoint.chat_url(), Some(&key), r#"{"messages":[]}"#), 0, "the child dies without answering");
    wait_until(Duration::from_secs(5), "the pause", || matches!(supervisor.state(), SupervisorState::Backoff { .. }));
    match supervisor.state() {
        SupervisorState::Backoff { retry_after_s } => assert!(retry_after_s <= 1, "one exit counts once, so the first wait applies: {retry_after_s}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stable_child_resets_the_waits() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-exit-after", "1"]);
    settings.backoff = vec![Duration::from_millis(200), Duration::from_secs(8)];
    settings.stable_after = Duration::from_millis(300);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    for round in 0..2 {
        wait_until(Duration::from_secs(5), "the pause to end", || !matches!(supervisor.state(), SupervisorState::Backoff { retry_after_s } if retry_after_s > 0));
        let endpoint = supervisor.endpoint().unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        std::thread::sleep(Duration::from_millis(600));
        let key = endpoint.key().expose().to_owned();
        assert_eq!(post(&endpoint.chat_url(), Some(&key), "{}"), 0);
        drop(endpoint);
        wait_until(Duration::from_secs(5), "the pause", || matches!(supervisor.state(), SupervisorState::Backoff { .. }));
        match supervisor.state() {
            SupervisorState::Backoff { retry_after_s } => assert!(retry_after_s <= 1, "round {round}: a child that lived long enough restarts the waits: {retry_after_s}"),
            other => panic!("{other:?}"),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

#[test]
fn a_failed_start_counts_like_a_crash() {
    let fixture = Fixture::new();
    let mut settings = fixture.settings(&["--fake-never-ready"]);
    settings.ready_timeout = Duration::from_millis(300);
    settings.backoff = vec![Duration::from_secs(5)];
    let calls = counted_program(&mut settings);
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let error = supervisor.endpoint().expect_err("never ready");
    assert!(matches!(error, LlmError::Unavailable { .. }));
    assert!(matches!(supervisor.state(), SupervisorState::Backoff { retry_after_s } if retry_after_s >= 1), "{:?}", supervisor.state());
    assert!(retry_of(&supervisor.endpoint().expect_err("paused")) >= 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no start inside the pause");
}

// --- Stop and orphans ----------------------------------------------------------------

#[test]
fn stop_kills_the_child_and_waits_for_it() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    drop(supervisor.endpoint().unwrap());
    let pid = supervisor.pid().unwrap();
    supervisor.stop();
    assert!(!pid_alive(pid), "gone when stop returns");
    assert_eq!(supervisor.state(), SupervisorState::Stopped);
    assert!(matches!(supervisor.state(), SupervisorState::Stopped), "an intended stop is not a crash");
    drop(supervisor.endpoint().expect("it starts again at once: no pause after a stop"));
}

#[test]
fn shutdown_kills_the_child_and_refuses_new_starts() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    drop(supervisor.endpoint().unwrap());
    let pid = supervisor.pid().unwrap();
    supervisor.shutdown();
    assert!(!pid_alive(pid));
    assert!(matches!(supervisor.endpoint(), Err(LlmError::Unavailable { .. })));
    assert_eq!(fixture.reports().len(), 1, "no new process");
}

#[test]
fn dropping_the_supervisor_kills_the_child() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    drop(supervisor.endpoint().unwrap());
    let pid = supervisor.pid().unwrap();
    drop(supervisor);
    assert!(!pid_alive(pid));
}

#[test]
fn a_killed_server_leaves_no_child() {
    let dir = support::TempDir::new();
    let mut host = std::process::Command::new(fake_bin())
        .arg("--fake-host")
        .arg(dir.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let file = dir.path().join("host.json");
    wait_until(Duration::from_secs(30), "the host to write its pids", || file.exists() && std::fs::metadata(&file).map(|m| m.len() > 2).unwrap_or(false));
    let pids: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let (host_pid, child_pid) = (pids["host"].as_u64().unwrap() as u32, pids["child"].as_u64().unwrap() as u32);
    assert_eq!(host_pid, host.id());
    assert!(pid_alive(child_pid));
    host.kill().unwrap();
    let _ = host.wait();
    wait_until(Duration::from_secs(5), "the child to follow its killed host", || !pid_alive(child_pid));
}

#[cfg(windows)]
#[test]
fn the_child_is_in_a_job_object_on_windows() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        QueryInformationJobObject,
    };
    let mut command = std::process::Command::new(fake_bin());
    command.env("LLAMA_API_KEY", "k").arg("--fake-never-ready").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let (mut child, group) = qamille_server::process::ChildGroup::spawn(&mut command).unwrap();
    let job = group.job_handle() as HANDLE;
    let mut inside = 0;
    // SAFETY: the handles are valid for the whole call: the child and the job are alive.
    let (checked, info) = unsafe {
        let ok = IsProcessInJob(child.as_raw_handle() as HANDLE, job, &mut inside);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        let queried = QueryInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            std::ptr::null_mut(),
        );
        (ok != 0 && queried != 0, info)
    };
    assert!(checked && inside != 0, "the child is in the job of the supervisor");
    assert!(info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0);
    group.kill_all();
    let _ = child.wait();
}

#[cfg(unix)]
#[test]
fn the_child_has_its_own_process_group_on_unix() {
    let mut command = std::process::Command::new(fake_bin());
    command.env("LLAMA_API_KEY", "k").arg("--fake-never-ready").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let (mut child, group) = qamille_server::process::ChildGroup::spawn(&mut command).unwrap();
    // SAFETY: `getpgid` only reads the process table.
    let (child_group, own_group) = unsafe { (libc::getpgid(child.id() as i32), libc::getpgid(0)) };
    assert_eq!(child_group, child.id() as i32);
    assert_ne!(child_group, own_group);
    group.kill_all();
    let _ = child.wait();
}

#[test]
fn with_stopped_runs_only_without_requests_in_flight() {
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let endpoint = supervisor.endpoint().unwrap();
    assert!(supervisor.with_stopped(|| ()).is_err(), "busy while a request runs");
    assert_eq!(supervisor.state(), SupervisorState::Ready, "and the child was not touched");
    drop(endpoint);
    let pid = supervisor.pid().unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let waiter = {
        let (supervisor, finished) = (supervisor.clone(), finished.clone());
        std::thread::spawn(move || {
            // Give `with_stopped` time to take the child down before this start request.
            std::thread::sleep(Duration::from_millis(300));
            let _endpoint = supervisor.endpoint().unwrap();
            finished.load(Ordering::SeqCst)
        })
    };
    let stopped_inside = supervisor
        .with_stopped(|| {
            let stopped = supervisor.state() == SupervisorState::Stopped && !pid_alive(pid);
            std::thread::sleep(Duration::from_millis(900));
            finished.store(true, Ordering::SeqCst);
            stopped
        })
        .unwrap();
    assert!(stopped_inside, "stopped while the closure runs");
    assert!(waiter.join().unwrap(), "a concurrent start waits for the closure to finish");
}

#[test]
fn the_child_is_started_from_the_dedicated_thread() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .thread_keep_alive(Duration::from_millis(50))
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::new();
    let supervisor = fixture.supervisor(&[]);
    let first = {
        let supervisor = supervisor.clone();
        runtime.block_on(async move { tokio::task::spawn_blocking(move || supervisor.endpoint().map(|e| e.chat_url())).await.unwrap() }).unwrap()
    };
    let pid = supervisor.pid().unwrap();
    // The blocking thread that made the first request ends after its keep-alive.
    std::thread::sleep(Duration::from_millis(500));
    let second = {
        let supervisor = supervisor.clone();
        runtime.block_on(async move { tokio::task::spawn_blocking(move || supervisor.endpoint().map(|e| e.chat_url())).await.unwrap() }).unwrap()
    };
    assert_eq!(first, second, "the same child serves both");
    assert!(pid_alive(pid), "it did not go away with the thread that asked");
    assert_eq!(fixture.reports().len(), 1);
}
