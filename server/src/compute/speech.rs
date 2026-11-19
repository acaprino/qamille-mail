//! Speech recognition behind `POST /v1/audio/transcriptions` (D2, OpenAI-compatible): the
//! ggml models of whisper.cpp and `whisper-cli`, through [`qamille_compute::speech`].
//!
//! - Models have the protocol ids `whisper-tiny`, `whisper-base`, `whisper-small`; they are
//!   installed with `POST /qamille/v1/models/{id}/install` and their progress is in
//!   `GET /health`, like the embedding model.
//! - Each request runs one `whisper-cli` process on its own WAV file; up to `parallelism`
//!   requests run at once (D2: the cores of the machine, configurable), a bounded queue waits
//!   beyond them and a full queue answers `rate_limited`.
//! - A request whose client goes away (connection closed, timeout) stops its process.
//! - Audio files live in a private temporary folder for the time of the request; nothing of the
//!   audio or of the text reaches the log.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use qamille_compute::speech::{self as whisper, Segment, SpeechError, SpeechModelSpec};
use super::{ComputeError, ModelStatus};

pub use qamille_compute::speech::{CATALOG, ID_PREFIX};

/// Requests waiting for a recognizer beyond the running ones.
pub const SPEECH_QUEUE: usize = 32;
/// Threads of one `whisper-cli` process at most (as the app used).
pub const MAX_THREADS: usize = 4;
const MEMORY_RETRY_S: u64 = 60;

pub fn default_parallelism() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2)
}

/// One transcription to run.
pub struct TranscriptionInput {
    /// A WAV file.
    pub audio: Vec<u8>,
    /// `whisper-<size>` (or the bare size).
    pub model: String,
    /// A language code or `auto`.
    pub language: String,
    /// Threads for `whisper-cli`.
    pub threads: usize,
}

/// The speech side of the compute server. Every method may block.
pub trait SpeechBackend: Send + Sync + 'static {
    fn models(&self) -> Vec<ModelStatus>;
    /// Starts installing `id` and returns at once; progress is in [`Self::models`].
    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError>;
    /// Segments in milliseconds from the start of the audio. Stops early when `cancel` is set.
    fn transcribe(&self, input: &TranscriptionInput, cancel: &AtomicBool) -> Result<Vec<Segment>, ComputeError>;
}

/// A server without speech recognition (tests of the other routes).
pub struct NoSpeech;

impl SpeechBackend for NoSpeech {
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _id: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("This server does not transcribe audio.".to_owned()))
    }
    fn transcribe(&self, _input: &TranscriptionInput, _cancel: &AtomicBool) -> Result<Vec<Segment>, ComputeError> {
        Err(ComputeError::ModelMissing("This server does not transcribe audio.".to_owned()))
    }
}

#[derive(Clone, Default)]
struct Install {
    running: bool,
    done: u64,
    total: u64,
    error: Option<String>,
}

/// whisper.cpp in child processes, models in one folder.
pub struct WhisperBackend {
    dir: PathBuf,
    catalog: &'static [SpeechModelSpec],
    /// Models whose hash was checked, with the length and time of the file then.
    verified: Mutex<HashMap<&'static str, (u64, Option<SystemTime>)>>,
    runtime: Mutex<Option<PathBuf>>,
    installs: Arc<Mutex<HashMap<&'static str, Install>>>,
    /// Private folder (0700) of the requests' audio and results (INV-2): never the shared
    /// temporary folder; what a killed server left there is removed at start.
    work: PathBuf,
    /// `models_mirror_url`: where the runtime and the models come from, when not Hugging Face.
    mirror: Option<qamille_compute::download::Mirror>,
}

/// Prefix of the folder of one transcription inside the work folder.
const WORK_PREFIX: &str = "qamille-stt-";

/// Creates `dir` readable only by this user, and removes what a killed server left in it.
fn prepare_work_dir(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(WORK_PREFIX) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// A new private folder (0700) for one request.
fn private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Writes `bytes` to a new file readable only by this user (0600).
fn private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

fn unknown_model(id: &str) -> ComputeError {
    // The id is the caller's own text: only a safe form of it is repeated.
    let shown: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).take(64).collect();
    ComputeError::ModelMissing(format!("This server does not offer the transcription model {shown}."))
}

fn speech_error(error: SpeechError) -> ComputeError {
    match error {
        SpeechError::Cancelled => ComputeError::Internal("The transcription was cancelled.".to_owned()),
        SpeechError::Missing(message) => ComputeError::ModelMissing(message),
        SpeechError::Failed(message) => ComputeError::Internal(message),
    }
}

fn percent(done: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (done.saturating_mul(100) / total).min(100) as u8)
}

fn file_stamp(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    std::fs::metadata(path).ok().map(|meta| (meta.len(), meta.modified().ok()))
}

/// Removes the request's files and folder (only those it created).
struct WorkFolder(PathBuf);

impl Drop for WorkFolder {
    fn drop(&mut self) {
        for name in ["input.wav", "result.json"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

impl WhisperBackend {
    /// `dir` holds `ggml-<size>.bin` (and, on Windows, the downloaded runtime).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_catalog(dir, CATALOG)
    }

    /// With another catalog (tests: small synthetic model files).
    pub fn with_catalog(dir: impl Into<PathBuf>, catalog: &'static [SpeechModelSpec]) -> Self {
        let dir = dir.into();
        let work = dir.join(".work");
        prepare_work_dir(&work);
        WhisperBackend {
            dir,
            catalog,
            verified: Mutex::new(HashMap::new()),
            runtime: Mutex::new(None),
            installs: Arc::new(Mutex::new(HashMap::new())),
            work,
            mirror: None,
        }
    }

    /// Downloads the runtime and the models through `mirror` (`models_mirror_url`).
    pub fn with_mirror(mut self, mirror: Option<qamille_compute::download::Mirror>) -> Self {
        self.mirror = mirror;
        self
    }

    /// With the work folder `work` (`serve`: `<data_dir>/compute-work`), made private and
    /// emptied of what an earlier run left.
    pub fn with_work_dir(mut self, work: impl Into<PathBuf>) -> Self {
        self.work = work.into();
        prepare_work_dir(&self.work);
        self
    }

    pub fn work_dir(&self) -> &Path {
        &self.work
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn spec(&self, id: &str) -> Result<&'static SpeechModelSpec, ComputeError> {
        whisper::spec(self.catalog, id).ok_or_else(|| unknown_model(id))
    }

    fn install_state(&self, size: &str) -> Install {
        self.installs.lock().ok().and_then(|installs| installs.get(size).cloned()).unwrap_or_default()
    }

    fn runtime(&self) -> Result<PathBuf, SpeechError> {
        let mut cached = self.runtime.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = cached.as_ref()
            && path.is_file()
        {
            return Ok(path.clone());
        }
        let path = whisper::locate_runtime(&self.dir)?;
        *cached = Some(path.clone());
        Ok(path)
    }

    fn status(&self, spec: &'static SpeechModelSpec, runtime: Result<(), String>) -> ModelStatus {
        let install = self.install_state(spec.size);
        let total = whisper::install_bytes(spec);
        let present = whisper::model_present(&self.dir, spec);
        let (state, error) = if install.running {
            ("downloading", None)
        } else if present && runtime.is_ok() {
            ("ready", None)
        } else {
            ("missing", install.error.or(if present { runtime.err() } else { None }))
        };
        let done = match state {
            "ready" => total,
            "downloading" => install.done,
            _ => 0,
        };
        ModelStatus {
            id: whisper::model_id(spec),
            kind: "speech".to_owned(),
            state: state.to_owned(),
            percent: match state {
                "ready" => Some(100),
                "downloading" => percent(install.done, install.total.max(1)),
                _ => None,
            },
            done_bytes: done,
            total_bytes: total,
            error,
            loaded: false,
            dimensions: 0,
            quantization: "ggml".to_owned(),
        }
    }

    /// The model file, checked against its hash once per process (and again if it changes).
    fn checked_model(&self, spec: &'static SpeechModelSpec, cancel: &AtomicBool) -> Result<PathBuf, ComputeError> {
        let path = whisper::model_file(&self.dir, spec);
        if !whisper::model_present(&self.dir, spec) {
            return Err(ComputeError::ModelMissing(whisper::MODEL_MISSING_MESSAGE.to_owned()));
        }
        let stamp = file_stamp(&path);
        let known = self.verified.lock().ok().and_then(|verified| verified.get(spec.size).copied());
        if stamp.is_none() || known != stamp {
            whisper::verify(&path, spec.bytes, spec.sha256, cancel).map_err(speech_error)?;
            if let (Ok(mut verified), Some(stamp)) = (self.verified.lock(), stamp) {
                verified.insert(spec.size, stamp);
            }
        }
        Ok(path)
    }
}

impl SpeechBackend for WhisperBackend {
    fn models(&self) -> Vec<ModelStatus> {
        let runtime = self.runtime().map(|_| ()).map_err(|error| error.to_string());
        self.catalog.iter().map(|spec| self.status(spec, runtime.clone())).collect()
    }

    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        let spec = self.spec(id)?;
        if !whisper::bundled_runtime() {
            // Nothing downloads the recognizer here: a model alone would never run.
            self.runtime().map_err(speech_error)?;
        }
        let current = self.models().into_iter().find(|model| model.id == whisper::model_id(spec));
        if current.as_ref().is_some_and(|model| model.state == "ready" || model.state == "downloading") {
            return current.ok_or(ComputeError::NotFound);
        }
        {
            let mut installs = self.installs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            installs.insert(spec.size, Install { running: true, done: 0, total: whisper::install_bytes(spec), error: None });
        }
        let dir = self.dir.clone();
        let options = qamille_compute::download::Options::one_shot().with_mirror(self.mirror.clone());
        let installs = Arc::clone(&self.installs);
        let report = Arc::clone(&self.installs);
        let spawned = std::thread::Builder::new().name("speech-model-install".to_owned()).spawn(move || {
            let progress = move |done: u64, total: u64| {
                if let Ok(mut installs) = report.lock()
                    && let Some(state) = installs.get_mut(spec.size)
                {
                    state.done = done;
                    state.total = total;
                }
            };
            let result = whisper::install_with(&dir, spec, &options, &AtomicBool::new(false), &progress);
            if let Ok(mut installs) = installs.lock() {
                let error = result.as_ref().err().cloned();
                installs.insert(spec.size, Install { running: false, done: 0, total: 0, error });
            }
            match result {
                Ok(()) => tracing::info!(model = spec.size, "speech model installed"),
                // The message names a file or a network failure, never user content.
                Err(error) => tracing::warn!(model = spec.size, error = %error, "speech model installation failed"),
            }
        });
        if let Err(error) = spawned {
            if let Ok(mut installs) = self.installs.lock() {
                installs.remove(spec.size);
            }
            return Err(ComputeError::Internal(format!("Cannot start the installation: {error}")));
        }
        if let Ok(mut verified) = self.verified.lock() {
            verified.remove(spec.size);
        }
        self.models().into_iter().find(|model| model.id == whisper::model_id(spec)).ok_or(ComputeError::NotFound)
    }

    fn transcribe(&self, input: &TranscriptionInput, cancel: &AtomicBool) -> Result<Vec<Segment>, ComputeError> {
        let spec = self.spec(&input.model)?;
        let install = self.install_state(spec.size);
        if install.running {
            return Err(ComputeError::ModelDownloading { percent: percent(install.done, install.total.max(1)) });
        }
        let program = self.runtime().map_err(speech_error)?;
        let model = self.checked_model(spec, cancel)?;
        // whisper.cpp maps the model and allocates its buffers on top of it.
        let needed_mb = spec.bytes / (1024 * 1024) + 256;
        if qamille_compute::memory::available_memory_mb().map_err(ComputeError::Internal)?.is_some_and(|free| free < needed_mb) {
            return Err(ComputeError::PausedForMemory { retry_after_s: MEMORY_RETRY_S });
        }
        let unprepared = || ComputeError::Internal("Cannot prepare the speech recognizer.".to_owned());
        let _ = std::fs::create_dir_all(&self.work);
        let work = self.work.join(format!("{WORK_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        private_dir(&work).map_err(|_| unprepared())?;
        let work = WorkFolder(work);
        let wave = work.0.join("input.wav");
        private_file(&wave, &input.audio).map_err(|_| unprepared())?;
        whisper::run_window(&program, &model, &wave, &work.0, &input.language, input.threads, cancel).map_err(speech_error)
    }
}

/// Threads for a `whisper-cli` starting now: the cores shared among the running requests.
pub fn threads(gate: &super::Gate) -> usize {
    (default_parallelism() / gate.running().max(1)).clamp(1, MAX_THREADS)
}

/// Sets the flag when dropped: the request future went away before its work ended.
pub struct CancelOnDrop(pub Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_folder_is_private_and_emptied_of_what_a_killed_server_left() {
        let root = std::env::temp_dir().join(format!("qamille-stt-test-{}", uuid::Uuid::new_v4().simple()));
        let work = root.join("compute-work");
        let left = work.join(format!("{WORK_PREFIX}old"));
        std::fs::create_dir_all(&left).unwrap();
        std::fs::write(left.join("input.wav"), b"audio of an earlier run").unwrap();
        std::fs::write(work.join("keep.txt"), b"not ours").unwrap();
        let backend = WhisperBackend::new(root.join("speech")).with_work_dir(&work);
        assert_eq!(backend.work_dir(), work.as_path());
        assert!(!left.exists(), "the audio of a killed run is removed at start");
        assert!(work.join("keep.txt").exists(), "only our folders are removed");
        let request = work.join(format!("{WORK_PREFIX}new"));
        private_dir(&request).unwrap();
        private_file(&request.join("input.wav"), b"RIFF").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&work), 0o700);
            assert_eq!(mode(&request), 0o700);
            assert_eq!(mode(&request.join("input.wav")), 0o600);
        }
        assert!(!work.starts_with(std::env::temp_dir()) || work.starts_with(&root), "never the shared temporary folder itself");
        let _ = std::fs::remove_dir_all(&root);
    }
}
