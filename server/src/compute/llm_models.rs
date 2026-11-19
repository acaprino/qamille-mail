//! The `llm-*` models of the built-in assistant (spec L8, INV-39): the catalog of
//! `qamille_compute::llm_catalog` seen as [`ModelStatus`] entries, installed from Hugging Face (or
//! the mirror) on a thread of their own, removed with the idle child stopped first. Nothing here
//! shows a path, an address or a token: only ids, sizes, states and the figures `/admin` needs
//! before «Installa».

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use qamille_compute::download::{HfToken, Mirror, Options};
use qamille_compute::llama::{self, Platform};
use qamille_compute::llm_catalog::{self, LlmModelSpec};
use serde::Serialize;

use super::{ComputeError, ModelStatus};
use crate::llm_builtin::LlmSupervisor;

/// What `/admin` shows beside an `llm-*` model before «Installa» (never a path or an address).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LlmDetails {
    pub ram_required_mb: u64,
    pub disk_required_bytes: u64,
    pub context: u32,
    pub license: String,
    /// Where the terms of the licence are published (the operator sees it before installing).
    pub license_url: String,
    pub speed_note: String,
    /// `ready` or `missing`.
    pub engine: String,
    pub in_use: bool,
    pub pinned: bool,
}

/// Free memory and free disk of this server, where the system can tell.
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct LlmHost {
    pub available_memory_mb: Option<u64>,
    pub available_disk_bytes: Option<u64>,
}

/// The installable language models. Every method may block: handlers call them on
/// `spawn_blocking`.
pub trait LlmModels: Send + Sync + 'static {
    fn models(&self) -> Vec<ModelStatus>;
    /// Starts installing `id` and returns at once; progress is in [`Self::models`].
    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError>;
    fn remove(&self, id: &str) -> Result<ModelStatus, ComputeError>;
    fn details(&self, id: &str) -> Option<LlmDetails>;
    fn host(&self) -> LlmHost {
        LlmHost::default()
    }
}

/// No language models (the default).
pub struct NoLlmModels;

impl LlmModels for NoLlmModels {
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn remove(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::NotFound)
    }
    fn details(&self, _: &str) -> Option<LlmDetails> {
        None
    }
}

#[derive(Clone, Default)]
struct Install {
    running: bool,
    /// A removal is deleting the files: no installation starts, no second removal.
    removing: bool,
    done: u64,
    total: u64,
    error: Option<String>,
}

/// Downloads the engine bundle (Windows); a closure so that the tests give an archive of their own.
pub type RuntimeInstaller = Arc<dyn Fn(&Path, &Options, &AtomicBool, &dyn Fn(u64)) -> Result<(), String> + Send + Sync>;

pub struct BuiltinLlmModels {
    models_dir: PathBuf,
    catalog: &'static [LlmModelSpec],
    options: Options,
    context_total: u32,
    platform: Platform,
    program: Option<PathBuf>,
    supervisor: Option<Arc<LlmSupervisor>>,
    configured_model: Option<String>,
    runtime_installer: RuntimeInstaller,
    /// The engine bundle checked, with the size and time of its program then.
    engine_checked: Mutex<Option<(u64, Option<SystemTime>)>>,
    installs: Arc<Mutex<HashMap<&'static str, Install>>>,
    /// `QAMILLE_LLM_ACCEPT_LICENSE`: what the operator accepted, checked before an automatic download.
    accepted_license: Option<String>,
    /// How long the automatic download waits after a failure (the last wait repeats).
    retry_waits: Vec<Duration>,
    /// Raised while the automatic download runs: the assistant answers «in preparation».
    preparing: Arc<AtomicBool>,
    /// Stops the automatic download when the manager goes away.
    cancel: Arc<AtomicBool>,
}

/// The waits between the attempts of the automatic download: they grow, and the last one repeats.
pub const AUTO_RETRY_WAITS_S: [u64; 5] = [10, 30, 60, 300, 900];

impl Drop for BuiltinLlmModels {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// What an installation needs, to run it on a thread of its own.
struct Job {
    spec: &'static LlmModelSpec,
    models_dir: PathBuf,
    options: Options,
    platform: Platform,
    runtime_dir: PathBuf,
    installer: RuntimeInstaller,
    runtime_bytes: u64,
}

impl Job {
    /// The engine bundle first (Windows), then the model; `report` gets the bytes done of both.
    fn run(&self, cancel: &AtomicBool, report: &(dyn Fn(u64) + Sync)) -> Result<(), String> {
        if self.platform == Platform::WindowsX64 {
            (*self.installer)(&self.runtime_dir, &self.options, cancel, report)?;
        }
        let runtime_bytes = self.runtime_bytes;
        llm_catalog::install(&self.models_dir, self.spec, &self.options, cancel, &|got, _| report(runtime_bytes + got))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn percent(done: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (done.saturating_mul(100) / total).min(100) as u8)
}

impl BuiltinLlmModels {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        models_dir: PathBuf,
        mirror: Option<Mirror>,
        token: Option<HfToken>,
        context_total: u32,
        platform: Platform,
        program: Option<PathBuf>,
        supervisor: Option<Arc<LlmSupervisor>>,
        configured_model: Option<String>,
    ) -> BuiltinLlmModels {
        BuiltinLlmModels {
            models_dir,
            catalog: llm_catalog::CATALOG,
            options: Options::resumable().with_mirror(mirror),
            context_total,
            platform,
            program,
            supervisor,
            configured_model,
            runtime_installer: Arc::new(|dir, options, cancel, progress| llama::install_runtime(dir, options, cancel, progress)),
            engine_checked: Mutex::new(None),
            installs: Arc::new(Mutex::new(HashMap::new())),
            accepted_license: None,
            retry_waits: AUTO_RETRY_WAITS_S.map(Duration::from_secs).to_vec(),
            preparing: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
        .with_token(token)
    }

    fn with_token(mut self, token: Option<HfToken>) -> Self {
        self.options.token = token;
        self
    }

    /// From the configuration of the server, sharing the supervisor of the gateway.
    pub fn from_config(config: &crate::config::Config, supervisor: Option<Arc<LlmSupervisor>>) -> BuiltinLlmModels {
        let llm = &config.llm;
        let preparing = supervisor.as_ref().map(|s| s.preparing_flag());
        let models = BuiltinLlmModels::new(
            config.models_dir(),
            config.models_mirror(),
            config.hf_token.clone(),
            llm.builtin_context.saturating_mul(llm.builtin_slots),
            llama::current_platform(),
            llm.builtin_program.clone(),
            supervisor,
            llm.builtin_model.clone(),
        )
        .with_accepted_license(llm.accept_license.clone());
        match preparing {
            Some(flag) => models.with_preparing(flag),
            None => models,
        }
    }

    /// What the operator accepted with `QAMILLE_LLM_ACCEPT_LICENSE`.
    pub fn with_accepted_license(mut self, accepted: Option<String>) -> Self {
        self.accepted_license = accepted;
        self
    }

    /// The flag that tells the gateway the model is being prepared (the supervisor's).
    pub fn with_preparing(mut self, flag: Arc<AtomicBool>) -> Self {
        self.preparing = flag;
        self
    }

    /// Other waits between the attempts of the automatic download (tests).
    pub fn with_retry_waits(mut self, waits: Vec<Duration>) -> Self {
        self.retry_waits = waits;
        self
    }

    fn job(&self, spec: &'static LlmModelSpec) -> Job {
        Job {
            spec,
            models_dir: self.models_dir.clone(),
            options: self.options.clone(),
            platform: self.platform,
            runtime_dir: self.runtime_dir(),
            installer: Arc::clone(&self.runtime_installer),
            runtime_bytes: self.runtime_bytes(),
        }
    }

    /// At the start of the server: if the configured model is not there, downloads it in the
    /// background, with waits that grow after a failure, and meanwhile the assistant answers
    /// «in preparation». A model that asks for an acknowledgement of its licence
    /// (`accept_license`) is not downloaded until the operator gives it: the status says why.
    /// Returns at once; does nothing when there is no configured model, or it is there already.
    pub fn start_auto_install(&self) {
        let Some(spec) = self.configured_model.as_deref().and_then(|id| llm_catalog::spec_in(self.catalog, id)) else {
            return;
        };
        if !llm_catalog::is_pinned(spec) || llm_catalog::present(&self.models_dir, spec) {
            return;
        }
        if !spec.accept_license.is_empty() && self.accepted_license.as_deref() != Some(spec.accept_license) {
            tracing::warn!(model = spec.id, "LLM model not downloaded: its licence is not accepted (QAMILLE_LLM_ACCEPT_LICENSE)");
            let error = format!("The download has not started: accept the licence terms by setting QAMILLE_LLM_ACCEPT_LICENSE={}.", spec.accept_license);
            lock(&self.installs).insert(spec.id, Install { error: Some(error), ..Install::default() });
            return;
        }
        {
            let mut installs = lock(&self.installs);
            if installs.values().any(|install| install.running || install.removing) {
                return;
            }
            installs.insert(spec.id, Install { running: true, total: spec.bytes + self.runtime_bytes(), ..Install::default() });
        }
        self.preparing.store(true, Ordering::SeqCst);
        let (job, installs, cancel, preparing) = (self.job(spec), Arc::clone(&self.installs), Arc::clone(&self.cancel), Arc::clone(&self.preparing));
        let waits = if self.retry_waits.is_empty() { vec![Duration::from_secs(60)] } else { self.retry_waits.clone() };
        let spawned = std::thread::Builder::new().name("llm-model-autoinstall".to_owned()).spawn(move || {
            let mut failures = 0usize;
            loop {
                let report = |done: u64| {
                    if let Some(state) = lock(&installs).get_mut(spec.id) {
                        state.done = done;
                    }
                };
                match job.run(&cancel, &report) {
                    Ok(()) => {
                        lock(&installs).insert(spec.id, Install::default());
                        tracing::info!(model = spec.id, "LLM model downloaded");
                        break;
                    }
                    Err(error) => {
                        if cancel.load(Ordering::SeqCst) {
                            lock(&installs).remove(spec.id);
                            break;
                        }
                        // The message names a failure of the network or of a file, never an address or a token.
                        tracing::warn!(model = spec.id, error = %error, "LLM model download failed; it will be tried again");
                        if let Some(state) = lock(&installs).get_mut(spec.id) {
                            state.error = Some(error);
                            state.done = 0;
                        }
                        let until = Instant::now() + waits[failures.min(waits.len() - 1)];
                        failures += 1;
                        while Instant::now() < until && !cancel.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        if cancel.load(Ordering::SeqCst) {
                            lock(&installs).remove(spec.id);
                            break;
                        }
                    }
                }
            }
            preparing.store(false, Ordering::SeqCst);
        });
        if let Err(error) = spawned {
            lock(&self.installs).remove(spec.id);
            self.preparing.store(false, Ordering::SeqCst);
            tracing::warn!(model = spec.id, kind = ?error.kind(), "LLM model download cannot start");
        }
    }

    /// Another catalog (tests).
    pub fn with_catalog(mut self, catalog: &'static [LlmModelSpec]) -> Self {
        self.catalog = catalog;
        self
    }

    /// Another downloader of the engine bundle (tests).
    pub fn with_runtime_installer(mut self, installer: RuntimeInstaller) -> Self {
        self.runtime_installer = installer;
        self
    }

    /// Another place for the token to go to (tests: a mirror on loopback is not `huggingface.co`).
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    fn spec(&self, id: &str) -> Result<&'static LlmModelSpec, ComputeError> {
        llm_catalog::spec_in(self.catalog, id).ok_or(ComputeError::NotFound)
    }

    fn runtime_dir(&self) -> PathBuf {
        llama::runtime_dir(&self.models_dir)
    }

    /// The engine is there and is what the server accepts. On Windows the bundle is hashed once
    /// per program file (and again if it changes); every start checks it again.
    fn engine(&self) -> Result<(), String> {
        match self.platform {
            Platform::WindowsX64 => {
                let program = self.runtime_dir().join(llama::PROGRAM);
                let stamp = std::fs::metadata(&program).ok().map(|m| (m.len(), m.modified().ok()));
                let Some(stamp) = stamp else {
                    return Err(llama::RUNTIME_MISSING_MESSAGE.to_owned());
                };
                if *lock(&self.engine_checked) == Some(stamp) {
                    return Ok(());
                }
                llama::verified_bundle(&self.runtime_dir()).map_err(|e| e.to_string())?;
                *lock(&self.engine_checked) = Some(stamp);
                Ok(())
            }
            platform => llama::resolve_program(platform, self.program.as_deref(), &self.runtime_dir()).map(|_| ()).map_err(|e| e.to_string()),
        }
    }

    /// The bytes of the engine bundle that an installation downloads too.
    fn runtime_bytes(&self) -> u64 {
        if self.platform == Platform::WindowsX64 { llama::RUNTIME_BYTES } else { 0 }
    }

    fn status(&self, spec: &'static LlmModelSpec, engine: &Result<(), String>) -> ModelStatus {
        let install = lock(&self.installs).get(spec.id).cloned().unwrap_or_default();
        let total = spec.bytes + self.runtime_bytes();
        let present = llm_catalog::present(&self.models_dir, spec);
        let (state, error) = if install.running {
            // After a failed attempt the automatic download waits and tries again: the last error shows.
            ("downloading", install.error)
        } else if !llm_catalog::is_pinned(spec) {
            ("missing", Some("This model is not available yet.".to_owned()))
        } else if present && engine.is_ok() {
            ("ready", None)
        } else {
            ("missing", install.error.or(if present { engine.as_ref().err().cloned() } else { None }))
        };
        let loaded = state == "ready"
            && self.configured_model.as_deref() == Some(spec.id)
            && self.supervisor.as_ref().is_some_and(|s| s.state() == crate::llm_builtin::SupervisorState::Ready);
        ModelStatus {
            id: spec.id.to_owned(),
            kind: "llm".to_owned(),
            state: state.to_owned(),
            percent: match state {
                "ready" => Some(100),
                "downloading" => percent(install.done, install.total.max(1)),
                _ => None,
            },
            done_bytes: match state {
                "ready" => total,
                "downloading" => install.done,
                _ => 0,
            },
            total_bytes: total,
            error,
            loaded,
            dimensions: 0,
            quantization: spec.quantization.to_owned(),
        }
    }

    fn status_of(&self, spec: &'static LlmModelSpec) -> ModelStatus {
        self.status(spec, &self.engine())
    }
}

impl LlmModels for BuiltinLlmModels {
    fn models(&self) -> Vec<ModelStatus> {
        let engine = self.engine();
        self.catalog.iter().map(|spec| self.status(spec, &engine)).collect()
    }

    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        let spec = self.spec(id)?;
        if lock(&self.installs).values().any(|install| install.removing) {
            return Err(ComputeError::InvalidRequest("A model is being removed; wait for it to finish.".to_owned()));
        }
        if !llm_catalog::is_pinned(spec) {
            return Err(ComputeError::InvalidRequest("This model is not available yet: its catalog entry is not pinned.".to_owned()));
        }
        if self.platform != Platform::WindowsX64 {
            // Nothing downloads the engine here: a model alone would never run.
            self.engine().map_err(|_| ComputeError::InvalidRequest("The engine of the assistant is not installed on this server.".to_owned()))?;
        }
        let current = self.status_of(spec);
        if current.state == "ready" || current.state == "downloading" {
            return Ok(current);
        }
        {
            let mut installs = lock(&self.installs);
            if installs.values().any(|install| install.running) {
                return Err(ComputeError::InvalidRequest("Another model is being installed; wait for it to finish.".to_owned()));
            }
            if installs.values().any(|install| install.removing) {
                return Err(ComputeError::InvalidRequest("A model is being removed; wait for it to finish.".to_owned()));
            }
            installs.insert(spec.id, Install { running: true, removing: false, done: 0, total: spec.bytes + self.runtime_bytes(), error: None });
        }
        let job = self.job(spec);
        let installs = Arc::clone(&self.installs);
        let spawned = std::thread::Builder::new().name("llm-model-install".to_owned()).spawn(move || {
            let report = |done: u64| {
                if let Some(state) = lock(&installs).get_mut(spec.id) {
                    state.done = done;
                }
            };
            let cancel = AtomicBool::new(false);
            let result = job.run(&cancel, &report);
            let error = result.as_ref().err().cloned();
            lock(&installs).insert(spec.id, Install { running: false, removing: false, done: 0, total: 0, error });
            match result {
                Ok(()) => tracing::info!(model = spec.id, "LLM model installed"),
                // The message names a failure of the network or of a file, never an address or a token.
                Err(error) => tracing::warn!(model = spec.id, error = %error, "LLM model installation failed"),
            }
        });
        if let Err(error) = spawned {
            lock(&self.installs).remove(spec.id);
            return Err(ComputeError::Internal(format!("Cannot start the installation: {error}")));
        }
        Ok(self.status_of(spec))
    }

    fn remove(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        let spec = self.spec(id)?;
        let busy = || ComputeError::InvalidRequest("The model is in use; try again when no request is running.".to_owned());
        {
            // Checked and marked under one lock: an installation cannot slip in between.
            let mut installs = lock(&self.installs);
            if installs.values().any(|install| install.running || install.removing) {
                return Err(ComputeError::InvalidRequest("The model is being installed or removed; wait for it to finish.".to_owned()));
            }
            installs.insert(spec.id, Install { removing: true, ..Install::default() });
        }
        struct Done<'a>(&'a Mutex<HashMap<&'static str, Install>>, &'static str);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                lock(self.0).remove(self.1);
            }
        }
        let _done = Done(&self.installs, spec.id);
        let internal = |error: String| ComputeError::Internal(error);
        match (&self.supervisor, self.configured_model.as_deref() == Some(spec.id)) {
            (Some(supervisor), true) => {
                supervisor.with_stopped(|| llm_catalog::remove(&self.models_dir, spec)).map_err(|_| busy())?.map_err(internal)?;
            }
            _ => llm_catalog::remove(&self.models_dir, spec).map_err(internal)?,
        }
        drop(_done);
        Ok(self.status_of(spec))
    }

    fn details(&self, id: &str) -> Option<LlmDetails> {
        let spec = llm_catalog::spec_in(self.catalog, id)?;
        let engine = self.engine();
        let missing_runtime = if engine.is_err() { self.runtime_bytes() } else { 0 };
        let in_use = self.configured_model.as_deref() == Some(spec.id) && self.supervisor.as_ref().is_some_and(|s| s.in_flight() > 0);
        Some(LlmDetails {
            ram_required_mb: llm_catalog::required_ram_mb(spec, self.context_total),
            disk_required_bytes: llm_catalog::required_disk_bytes(spec).saturating_add(missing_runtime),
            context: spec.context,
            license: spec.license.to_owned(),
            license_url: spec.license_url.to_owned(),
            speed_note: spec.speed_note.to_owned(),
            engine: if engine.is_ok() { "ready" } else { "missing" }.to_owned(),
            in_use,
            pinned: llm_catalog::is_pinned(spec),
        })
    }

    fn host(&self) -> LlmHost {
        LlmHost {
            available_memory_mb: qamille_compute::memory::available_memory_mb().ok().flatten(),
            available_disk_bytes: {
                let probe = if self.models_dir.exists() { self.models_dir.clone() } else { self.models_dir.ancestors().find(|p| p.exists()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")) };
                qamille_compute::memory::available_disk_bytes(&probe)
            },
        }
    }
}
