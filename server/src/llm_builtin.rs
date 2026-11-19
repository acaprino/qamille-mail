//! The supervisor of the built-in LLM child (`llm_provider = "builtin"`, spec L8, INV-40): one
//! `llama-server` process on `127.0.0.1`, started lazily by the first request, with a fresh key
//! for every start that lives only in the environment of the child (never in its arguments), a
//! minimal environment, and a readiness check that proves the key is asked for.
//!
//! The thread `llm-supervisor` owns the [`Child`]: it starts it, watches it and stops it. The
//! callers ([`LlmSupervisor::endpoint`]) only wait on a condition variable, so a child started
//! from a pool thread never depends on that thread living on (`PR_SET_PDEATHSIG` binds to the
//! thread that forked).

use std::ffi::OsString;
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use qamille_compute::llama::LlamaError;

use crate::llm::LlmError;
use crate::process::ChildGroup;

/// The flags the supervisor passes that the Dockerfile checks `llama-server --help` for (the names
/// of the pinned release, b12081: `--no-webui` is still accepted as an alias of `--no-ui`).
pub const REQUIRED_FLAGS: &[&str] = &[
    "--model",
    "--host",
    "--port",
    "--ctx-size",
    "--parallel",
    "--cache-ram",
    "--no-slots",
    "--jinja",
    "--no-webui",
    "--log-verbosity",
];

/// `with_stopped` found a request in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy;

/// The seconds left of a pause after a crash or a failed start (at least 1), if one is running.
fn remaining_pause(inner: &Inner) -> Option<u64> {
    let left = inner.backoff_until?.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
    Some(left.as_secs() + u64::from(left.subsec_nanos() > 0))
}

/// The seconds a caller is told to wait after a failed start.
const FAILED_START_RETRY_S: u64 = 30;
/// What a request is told to wait while the server downloads the model by itself.
const PREPARING_RETRY_S: u64 = 60;
/// How long a child that already answers `200` without a key may take to exit (a bind that
/// failed because another listener holds the port) before it is judged to be that server.
const IMPOSTOR_GRACE: Duration = Duration::from_secs(3);
/// After the first keyed `200` the child must stay alive this long and the check must hold again.
const READY_SETTLE: Duration = Duration::from_millis(500);
/// The startup error lines kept in the log, and their length.
const MAX_STDERR_LINES: usize = 32;
const MAX_STDERR_CHARS: usize = 200;

/// The arguments of the child: no key and no text (the key travels in the environment).
pub fn child_args(model: &Path, port: u16, context_total: u32, slots: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--model".into(), model.as_os_str().to_owned()];
    for (flag, value) in [
        ("--host", "127.0.0.1".to_owned()),
        ("--port", port.to_string()),
        ("--ctx-size", context_total.to_string()),
        ("--parallel", slots.to_string()),
        ("--cache-ram", "0".to_owned()),
    ] {
        args.push(flag.into());
        args.push(value.into());
    }
    for flag in ["--no-slots", "--jinja", "--no-webui"] {
        args.push(flag.into());
    }
    args.push("--log-verbosity".into());
    args.push("1".into());
    args
}

/// The key of one start: 32 random bytes in hex. Its `Debug` hides it.
#[derive(Clone)]
pub struct ChildKey(String);

impl ChildKey {
    pub fn generate() -> ChildKey {
        ChildKey(hex::encode(crate::password::random_bytes_32()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ChildKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChildKey(<nascosta>)")
    }
}

/// Where and how to reach the running child. Holds the child's use count: while an `Endpoint`
/// lives, the request counts as in flight (the idle stop waits for it).
pub struct Endpoint {
    port: u16,
    key: ChildKey,
    _use: UseGuard,
}

impl Endpoint {
    pub fn chat_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }

    pub fn key(&self) -> &ChildKey {
        &self.key
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Endpoint { .. }")
    }
}

struct UseGuard(Arc<Shared>);

impl Drop for UseGuard {
    fn drop(&mut self) {
        let mut inner = lock(&self.0.inner);
        inner.in_flight = inner.in_flight.saturating_sub(1);
        inner.last_activity = Instant::now();
        self.0.cv.notify_all();
    }
}

/// Free memory, in MiB, if the system can tell.
pub trait MemoryProbe: Send + Sync {
    fn available_mb(&self) -> Option<u64>;
}

pub struct SystemMemory;

impl MemoryProbe for SystemMemory {
    fn available_mb(&self) -> Option<u64> {
        qamille_compute::memory::available_memory_mb().ok().flatten()
    }
}

/// The port of each start.
pub trait PortPicker: Send + Sync {
    fn pick(&self) -> u16;
}

/// Binds `127.0.0.1:0`, reads the port and releases it.
pub struct LoopbackPorts;

impl PortPicker for LoopbackPorts {
    fn pick(&self) -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0)).ok().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port())
    }
}

/// Resolves and checks the program; called at **every** start.
pub type ProgramCheck = Arc<dyn Fn() -> Result<PathBuf, LlamaError> + Send + Sync>;
/// Checks the model file; called at every start.
pub type ModelCheck = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;

#[derive(Clone)]
pub struct BuiltinSettings {
    pub program: ProgramCheck,
    pub model: ModelCheck,
    pub model_id: String,
    pub context: u32,
    pub slots: u32,
    pub idle: Duration,
    pub ready_timeout: Duration,
    pub backoff: Vec<Duration>,
    pub stable_after: Duration,
    pub required_mb: u64,
    /// Set while the server downloads the model on its own: requests get `503 llm_unavailable`
    /// («in preparation») without a start being tried. Shared with the model manager.
    pub preparing: Arc<AtomicBool>,
    /// How often the child is watched and probed (200 ms; shorter in tests).
    pub poll: Duration,
    /// Starts retried on another port and key when the child exits before it is ready.
    pub port_retries: u32,
    /// Only for the tests: appended after the real arguments.
    pub extra_args: Vec<OsString>,
}

impl BuiltinSettings {
    pub fn new(program: ProgramCheck, model: ModelCheck, model_id: String, context: u32, slots: u32, idle: Duration, required_mb: u64) -> BuiltinSettings {
        BuiltinSettings {
            program,
            model,
            model_id,
            context,
            slots,
            idle,
            ready_timeout: Duration::from_secs(120),
            backoff: [5, 10, 30, 60].map(Duration::from_secs).to_vec(),
            stable_after: Duration::from_secs(120),
            required_mb,
            preparing: Arc::new(AtomicBool::new(false)),
            poll: Duration::from_millis(200),
            port_retries: 3,
            extra_args: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct SupervisorDeps {
    pub memory: Arc<dyn MemoryProbe>,
    pub ports: Arc<dyn PortPicker>,
}

impl Default for SupervisorDeps {
    fn default() -> Self {
        SupervisorDeps { memory: Arc::new(SystemMemory), ports: Arc::new(LoopbackPorts) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorState {
    Stopped,
    Starting,
    Ready,
    Backoff { retry_after_s: u64 },
    Stopping,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Stopped,
    Starting,
    Ready,
    Stopping,
}

struct Running {
    port: u16,
    key: ChildKey,
}

struct Inner {
    phase: Phase,
    running: Option<Running>,
    in_flight: usize,
    last_activity: Instant,
    start_requested: bool,
    stop_requested: bool,
    shutdown: bool,
    /// `with_stopped` is running: nothing starts.
    held: bool,
    /// Exits and failed starts in a row, for the length of the pause.
    failures: u32,
    /// Requests are refused until then.
    backoff_until: Option<Instant>,
    /// Starts begun so far.
    attempt: u64,
    /// The last failed start and the number of its attempt.
    failure: Option<(u64, LlmError)>,
    pid: Option<u32>,
}

struct Shared {
    settings: BuiltinSettings,
    deps: SupervisorDeps,
    inner: Mutex<Inner>,
    cv: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct LlmSupervisor {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl fmt::Debug for LlmSupervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmSupervisor").field("state", &self.state()).finish_non_exhaustive()
    }
}

impl LlmSupervisor {
    /// Starts the thread `llm-supervisor`; the child starts with the first request.
    pub fn new(settings: BuiltinSettings, deps: SupervisorDeps) -> Arc<LlmSupervisor> {
        let shared = Arc::new(Shared {
            settings,
            deps,
            inner: Mutex::new(Inner {
                phase: Phase::Stopped,
                running: None,
                in_flight: 0,
                last_activity: Instant::now(),
                start_requested: false,
                stop_requested: false,
                shutdown: false,
                held: false,
                failures: 0,
                backoff_until: None,
                attempt: 0,
                failure: None,
                pid: None,
            }),
            cv: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let handle = std::thread::Builder::new().name("llm-supervisor".to_owned()).spawn(move || run(&worker)).expect("thread llm-supervisor");
        Arc::new(LlmSupervisor { shared, thread: Mutex::new(Some(handle)) })
    }

    /// The flag the model manager raises while it downloads the model (see [`BuiltinSettings::preparing`]).
    pub fn preparing_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shared.settings.preparing)
    }

    /// The child, started if needed. **Blocking** (a start takes seconds): never on an async
    /// thread. Concurrent callers wait for the same start.
    pub fn endpoint(&self) -> Result<Endpoint, LlmError> {
        let shared = &self.shared;
        let mut inner = lock(&shared.inner);
        let mut needs: Option<u64> = None;
        loop {
            if inner.shutdown {
                return Err(LlmError::Unavailable { retry_after_s: FAILED_START_RETRY_S });
            }
            if shared.settings.preparing.load(Ordering::SeqCst) && inner.phase != Phase::Ready {
                return Err(LlmError::Preparing { retry_after_s: PREPARING_RETRY_S });
            }
            if inner.phase == Phase::Ready
                && !inner.held
                && let Some(running) = &inner.running
            {
                let (port, key) = (running.port, running.key.clone());
                inner.in_flight += 1;
                inner.last_activity = Instant::now();
                return Ok(Endpoint { port, key, _use: UseGuard(Arc::clone(shared)) });
            }
            if let (Some(needs), Some((attempt, error))) = (needs, &inner.failure)
                && *attempt >= needs
            {
                return Err(error.clone());
            }
            if inner.phase == Phase::Stopped
                && let Some(retry_after_s) = remaining_pause(&inner)
            {
                return Err(LlmError::Unavailable { retry_after_s });
            }
            match inner.phase {
                Phase::Stopped => {
                    needs.get_or_insert(inner.attempt + 1);
                    if !inner.start_requested && !inner.held {
                        inner.start_requested = true;
                        shared.cv.notify_all();
                    }
                }
                Phase::Starting => {
                    needs.get_or_insert(inner.attempt);
                }
                Phase::Ready | Phase::Stopping => {}
            }
            inner = shared.cv.wait_timeout(inner, Duration::from_secs(1)).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    pub fn state(&self) -> SupervisorState {
        let inner = lock(&self.shared.inner);
        match inner.phase {
            Phase::Stopped => remaining_pause(&inner).map_or(SupervisorState::Stopped, |retry_after_s| SupervisorState::Backoff { retry_after_s }),
            Phase::Starting => SupervisorState::Starting,
            Phase::Ready => SupervisorState::Ready,
            Phase::Stopping => SupervisorState::Stopping,
        }
    }

    /// Requests in progress (the endpoints alive).
    pub fn in_flight(&self) -> usize {
        lock(&self.shared.inner).in_flight
    }

    /// The pid of the child, from its creation to its end (tests and logs only).
    pub fn pid(&self) -> Option<u32> {
        lock(&self.shared.inner).pid
    }

    /// Stops the child and waits for it to be gone.
    pub fn stop(&self) {
        let shared = &self.shared;
        let mut inner = lock(&shared.inner);
        inner.start_requested = false;
        if inner.phase == Phase::Stopped {
            return;
        }
        inner.stop_requested = true;
        shared.cv.notify_all();
        while inner.phase != Phase::Stopped || inner.stop_requested {
            inner = shared.cv.wait_timeout(inner, Duration::from_millis(100)).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// Stops the child, keeps every start out while `f` runs, then lets them in again. `Busy` if a
    /// request is in flight (nothing is touched then).
    pub fn with_stopped<T>(&self, f: impl FnOnce() -> T) -> Result<T, Busy> {
        let shared = &self.shared;
        {
            let mut inner = lock(&shared.inner);
            if inner.in_flight > 0 {
                return Err(Busy);
            }
            inner.held = true;
            inner.start_requested = false;
        }
        struct Release<'a>(&'a Shared);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                lock(&self.0.inner).held = false;
                self.0.cv.notify_all();
            }
        }
        let _release = Release(shared);
        self.stop();
        Ok(f())
    }

    /// As [`stop`](Self::stop), then no more starts: `endpoint` answers `Unavailable`.
    pub fn shutdown(&self) {
        {
            let mut inner = lock(&self.shared.inner);
            inner.shutdown = true;
            self.shared.cv.notify_all();
        }
        self.stop();
        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl Drop for LlmSupervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// --- The thread -------------------------------------------------------------------

enum StartFailure {
    /// A stop or a shutdown arrived while starting.
    Interrupted,
    Failed,
    /// The model does not fit in the free memory.
    Memory,
}

/// One more exit or failed start: the pause grows (5, 10, 30, 60 s, then 60). A child that lived
/// for `stable_after` first brings the count back to zero.
fn register_failure(inner: &mut Inner, settings: &BuiltinSettings, lived: Duration) {
    if lived >= settings.stable_after {
        inner.failures = 0;
    }
    inner.failures = inner.failures.saturating_add(1);
    let index = (inner.failures as usize - 1).min(settings.backoff.len().saturating_sub(1));
    let wait = settings.backoff.get(index).copied().unwrap_or(Duration::from_secs(60));
    inner.backoff_until = Some(Instant::now() + wait);
}

fn run(shared: &Arc<Shared>) {
    loop {
        let mut inner = lock(&shared.inner);
        while !(inner.start_requested && inner.phase == Phase::Stopped && !inner.held && remaining_pause(&inner).is_none()) {
            if inner.shutdown {
                return;
            }
            inner = shared.cv.wait_timeout(inner, Duration::from_millis(200)).unwrap_or_else(|p| p.into_inner()).0;
        }
        inner.start_requested = false;
        inner.backoff_until = None;
        inner.phase = Phase::Starting;
        inner.attempt += 1;
        let attempt = inner.attempt;
        drop(inner);
        let outcome = start(shared);
        let mut inner = lock(&shared.inner);
        match outcome {
            Ok((child, group, port, key)) => {
                inner.running = Some(Running { port, key });
                inner.phase = Phase::Ready;
                inner.last_activity = Instant::now();
                shared.cv.notify_all();
                drop(inner);
                supervise(shared, child, group);
            }
            Err(StartFailure::Interrupted) => {
                inner.phase = Phase::Stopped;
                inner.stop_requested = false;
                inner.pid = None;
                shared.cv.notify_all();
            }
            Err(StartFailure::Failed) => {
                inner.failure = Some((attempt, LlmError::Unavailable { retry_after_s: FAILED_START_RETRY_S }));
                // A failed start counts like a crash: the pause grows.
                register_failure(&mut inner, &shared.settings, Duration::ZERO);
                inner.phase = Phase::Stopped;
                inner.stop_requested = false;
                inner.pid = None;
                shared.cv.notify_all();
            }
            Err(StartFailure::Memory) => {
                // Not a crash: nothing was started and no pause opens; the next request looks again.
                inner.failure = Some((attempt, LlmError::NotEnoughMemory { retry_after_s: FAILED_START_RETRY_S }));
                inner.phase = Phase::Stopped;
                inner.stop_requested = false;
                inner.pid = None;
                shared.cv.notify_all();
            }
        }
    }
}

/// Watches a ready child until it exits or has to stop (a request, the idle time, a shutdown).
fn supervise(shared: &Arc<Shared>, mut child: Child, group: ChildGroup) {
    let started = Instant::now();
    loop {
        let mut inner = lock(&shared.inner);
        inner = shared.cv.wait_timeout(inner, shared.settings.poll).unwrap_or_else(|p| p.into_inner()).0;
        let idle = inner.in_flight == 0 && inner.last_activity.elapsed() >= shared.settings.idle;
        let must_stop = inner.stop_requested || inner.shutdown || idle;
        let exited = !must_stop && child.try_wait().ok().flatten().is_some();
        if !must_stop && !exited {
            continue;
        }
        if must_stop {
            inner.phase = Phase::Stopping;
            drop(inner);
            kill(&mut child, &group);
            inner = lock(&shared.inner);
        } else {
            // It ended by itself: the pause opens, once, whatever the requests in flight do.
            tracing::warn!(model = %shared.settings.model_id, "llm-builtin: the engine exited");
            register_failure(&mut inner, &shared.settings, started.elapsed());
        }
        inner.phase = Phase::Stopped;
        inner.running = None;
        inner.pid = None;
        inner.stop_requested = false;
        shared.cv.notify_all();
        return;
    }
}

fn kill(child: &mut Child, group: &ChildGroup) {
    group.kill_all();
    let _ = child.kill();
    let _ = child.wait();
}

fn interrupted(shared: &Shared) -> bool {
    let inner = lock(&shared.inner);
    inner.stop_requested || inner.shutdown
}

/// Checks program and model, creates the child and waits for it to be ready.
fn start(shared: &Arc<Shared>) -> Result<(Child, ChildGroup, u16, ChildKey), StartFailure> {
    let settings = &shared.settings;
    let program = (settings.program)().map_err(|_| {
        tracing::warn!(model = %settings.model_id, "llm-builtin: the engine program failed its check");
        StartFailure::Failed
    })?;
    let model = (settings.model)().map_err(|_| {
        tracing::warn!(model = %settings.model_id, "llm-builtin: the model failed its check");
        StartFailure::Failed
    })?;
    match shared.deps.memory.available_mb() {
        Some(available) if available < settings.required_mb => {
            tracing::warn!(model = %settings.model_id, available, required = settings.required_mb, "llm-builtin: not enough free memory, nothing was started");
            return Err(StartFailure::Memory);
        }
        None => tracing::warn!(model = %settings.model_id, "llm-builtin: the free memory cannot be measured; starting anyway"),
        Some(_) => {}
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        // Windows takes about two seconds to refuse a connection to a port nobody listens on yet.
        .timeout_connect(Some(Duration::from_millis(300)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into();
    for attempt in 0..=settings.port_retries {
        if interrupted(shared) {
            return Err(StartFailure::Interrupted);
        }
        let port = shared.deps.ports.pick();
        let key = ChildKey::generate();
        let (mut child, group) = spawn(&program, &model, port, &key, settings).map_err(|error| {
            tracing::warn!(model = %settings.model_id, kind = ?error.kind(), "llm-builtin: cannot start the engine");
            StartFailure::Failed
        })?;
        lock(&shared.inner).pid = Some(child.id());
        let ready = Arc::new(AtomicBool::new(false));
        if let Some(stderr) = child.stderr.take() {
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || read_stderr(stderr, &ready));
        }
        match wait_ready(shared, &mut child, port, &key, &agent) {
            Waited::Ready => {
                ready.store(true, Ordering::SeqCst);
                tracing::info!(model = %settings.model_id, "llm-builtin: the model is ready");
                return Ok((child, group, port, key));
            }
            Waited::Exited => {
                // Most likely the port: another listener took it. Another port, another key.
                tracing::warn!(model = %settings.model_id, attempt, "llm-builtin: the engine exited before it was ready");
                lock(&shared.inner).pid = None;
            }
            Waited::Interrupted => {
                kill(&mut child, &group);
                return Err(StartFailure::Interrupted);
            }
            Waited::Failed(reason) => {
                tracing::warn!(model = %settings.model_id, reason, "llm-builtin: the engine did not become ready");
                kill(&mut child, &group);
                return Err(StartFailure::Failed);
            }
        }
    }
    Err(StartFailure::Failed)
}

fn spawn(program: &Path, model: &Path, port: u16, key: &ChildKey, settings: &BuiltinSettings) -> std::io::Result<(Child, ChildGroup)> {
    let mut command = Command::new(program);
    command
        .args(child_args(model, port, settings.context.saturating_mul(settings.slots), settings.slots))
        .args(&settings.extra_args)
        .env_clear()
        .env("LLAMA_API_KEY", key.expose())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Winsock does not work without it.
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    if let Some(folder) = program.parent().filter(|p| !p.as_os_str().is_empty()) {
        command.current_dir(folder);
    }
    ChildGroup::spawn(&mut command)
}

enum Waited {
    Ready,
    /// The child ended before it was ready.
    Exited,
    Interrupted,
    Failed(&'static str),
}

/// Ready = child alive **and** `POST /tokenize` answers `401` without the key and `200` with it. A
/// listener that answers `200` without the key is not a `llama-server` started with a key: it is
/// never ready and never gets the key.
fn wait_ready(shared: &Shared, child: &mut Child, port: u16, key: &ChildKey, agent: &ureq::Agent) -> Waited {
    let deadline = Instant::now() + shared.settings.ready_timeout;
    let poll = shared.settings.poll;
    let exited = |child: &mut Child| child.try_wait().ok().flatten().is_some();
    loop {
        if interrupted(shared) {
            return Waited::Interrupted;
        }
        std::thread::sleep(poll);
        if exited(child) {
            return Waited::Exited;
        }
        if Instant::now() >= deadline {
            return Waited::Failed("deadline");
        }
        match probe(agent, port, None) {
            Some(401) => {
                if exited(child) {
                    return Waited::Exited;
                }
                if probe(agent, port, Some(key)) == Some(200) {
                    // A listener that is not our child can answer like it while the child dies on
                    // the busy port: the child must outlive a short settle and the check must
                    // hold again before the key is trusted to this port.
                    let until = Instant::now() + READY_SETTLE;
                    while Instant::now() < until {
                        if interrupted(shared) {
                            return Waited::Interrupted;
                        }
                        std::thread::sleep(poll);
                        if exited(child) {
                            return Waited::Exited;
                        }
                    }
                    if probe(agent, port, None) == Some(401) && probe(agent, port, Some(key)) == Some(200) && !exited(child) {
                        return Waited::Ready;
                    }
                }
            }
            Some(200) => {
                // Either another listener holds the port (and the child is about to exit) or this
                // is a server that does not ask for the key: look again before deciding.
                let until = Instant::now() + IMPOSTOR_GRACE;
                while Instant::now() < until {
                    std::thread::sleep(poll);
                    if exited(child) {
                        return Waited::Exited;
                    }
                }
                return Waited::Failed("a listener that does not ask for the key");
            }
            _ => {}
        }
    }
}

/// The status of `POST /tokenize` (fixed content, no user text), or `None` without an answer.
fn probe(agent: &ureq::Agent, port: u16, key: Option<&ChildKey>) -> Option<u16> {
    let mut request = agent.post(format!("http://127.0.0.1:{port}/tokenize")).header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Authorization", format!("Bearer {}", key.expose()));
    }
    request.send(r#"{"content":"."}"#).ok().map(|response| response.status().as_u16())
}

/// Until the first "ready": at most 32 lines of 200 characters that look like an error, as `warn`.
/// After: everything is read and thrown away (with a high verbosity the child prints prompts).
fn read_stderr(stderr: ChildStderr, ready: &AtomicBool) {
    let mut reader = BufReader::new(stderr);
    let mut kept = 0;
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        // A line longer than this is cut: the rest is read as other lines and discarded below.
        match reader.by_ref().take(8192).read_until(b'\n', &mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if ready.load(Ordering::SeqCst) || kept >= MAX_STDERR_LINES {
            continue;
        }
        let line = String::from_utf8_lossy(&buffer);
        let lower = line.to_ascii_lowercase();
        if lower.contains("error") || lower.contains("failed") || lower.contains("out of memory") {
            let line: String = line.trim_end().chars().take(MAX_STDERR_CHARS).collect();
            tracing::warn!("llm-builtin child: {line}");
            kept += 1;
        }
    }
}

/// The settings of the real server: the program resolved and checked at every start
/// ([`qamille_compute::llama::resolve_program`]), the model of the catalog verified with its full
/// SHA-256 the first time and again only when its size or modification time change, and the
/// memory the catalog says is needed for the configured context.
pub fn settings_from_config(config: &crate::config::Config) -> Option<BuiltinSettings> {
    use qamille_compute::{llama, llm_catalog};
    let llm = &config.llm;
    let spec = llm_catalog::spec(llm.builtin_model.as_deref()?)?;
    let models_dir = config.models_dir();
    let (setting, runtime) = (llm.builtin_program.clone(), llama::runtime_dir(&models_dir));
    let program: ProgramCheck = Arc::new(move || llama::resolve_program(llama::current_platform(), setting.as_deref(), &runtime));
    let verified: Mutex<Option<(u64, Option<std::time::SystemTime>)>> = Mutex::new(None);
    let dir = models_dir.clone();
    let model: ModelCheck = Arc::new(move || {
        if !llm_catalog::is_pinned(spec) {
            return Err("The model is not available yet.".to_owned());
        }
        let path = llm_catalog::model_file(&dir, spec);
        let meta = std::fs::metadata(&path).map_err(|_| "The model is not installed.".to_owned())?;
        let stamp = (meta.len(), meta.modified().ok());
        let mut seen = lock(&verified);
        if *seen != Some(stamp) {
            llm_catalog::verify(&path, spec, &AtomicBool::new(false))?;
            *seen = Some(stamp);
        }
        Ok(path)
    });
    let context_total = llm.builtin_context.saturating_mul(llm.builtin_slots);
    Some(BuiltinSettings::new(
        program,
        model,
        spec.id.to_owned(),
        llm.builtin_context,
        llm.builtin_slots,
        Duration::from_secs(llm.builtin_idle_secs),
        llm_catalog::required_ram_mb(spec, context_total),
    ))
}
