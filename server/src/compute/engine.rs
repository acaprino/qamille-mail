//! The real backend: EmbeddingGemma 2 in this process, documents and images in the parser
//! process. Carries over the protections of the app's former embedding worker:
//! - the model loads only with enough available memory (`paused_for_memory`);
//! - an inference failure drops the model and pauses for 60 s (`paused_after_failure`);
//!   a failure on an image or a document costs only that content;
//! - the model leaves memory after 60 s without requests (the process stays).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qamille_compute::embedder::{DIM, Embedder, MAX_INPUT_TOKENS, QUANTIZATION};
use qamille_compute::memory;
use qamille_compute::models::{self, MODEL_ID};
use qamille_compute::vision::Pixels;

use super::parser::{ParserCommand, ParserPool};
use super::{ComputeBackend, ComputeError, ModelStatus};

const FAILURE_PAUSE: Duration = Duration::from_secs(60);
const IDLE: Duration = Duration::from_secs(60);
/// Free memory needed to keep working with the model loaded, and on top of it to load it.
const WORKING_MB: u64 = 384;

struct Slot {
    embedder: Option<Embedder>,
    last_used: Option<Instant>,
    retry_after: Option<Instant>,
}

pub struct OnnxBackend {
    models_dir: PathBuf,
    parser: ParserPool,
    slot: Mutex<Slot>,
    loaded: AtomicBool,
    loading: AtomicBool,
    installing: Arc<AtomicBool>,
    /// `models_mirror_url`: where the model comes from, when not Hugging Face.
    mirror: Option<qamille_compute::download::Mirror>,
}

impl OnnxBackend {
    /// `models_dir` holds one folder per model (`<models>/embeddinggemma-2`).
    pub fn new(models_dir: impl Into<PathBuf>, parser: ParserCommand) -> Self {
        Self::with_parser(models_dir, ParserPool::new(parser))
    }

    /// With a parser pool of its own (tests: another program, a short budget).
    pub fn with_parser(models_dir: impl Into<PathBuf>, parser: ParserPool) -> Self {
        OnnxBackend {
            models_dir: models_dir.into(),
            parser,
            slot: Mutex::new(Slot { embedder: None, last_used: None, retry_after: None }),
            loaded: AtomicBool::new(false),
            loading: AtomicBool::new(false),
            installing: Arc::new(AtomicBool::new(false)),
            mirror: None,
        }
    }

    /// Downloads the model through `mirror` (`models_mirror_url`).
    pub fn with_mirror(mut self, mirror: Option<qamille_compute::download::Mirror>) -> Self {
        self.mirror = mirror;
        self
    }

    fn model_dir(&self) -> PathBuf {
        self.models_dir.join(MODEL_ID)
    }

    /// Runs `work` with the model loaded. `content` failures (an image) never pause the others.
    fn with_model<T>(&self, content: bool, work: impl FnOnce(&mut Embedder) -> Result<T, String>) -> Result<T, ComputeError> {
        let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = slot.retry_after {
            let now = Instant::now();
            if now < at {
                return Err(ComputeError::PausedAfterFailure { retry_after_s: (at - now).as_secs().max(1) });
            }
            slot.retry_after = None;
        }
        let dir = self.model_dir();
        if !models::installed(&dir) {
            let download = models::download_status();
            return Err(if download.running {
                ComputeError::ModelDownloading { percent: percent(download.done, download.total) }
            } else {
                ComputeError::ModelMissing(models::MODEL_MISSING_MESSAGE.to_owned())
            });
        }
        let minimum = if slot.embedder.is_some() { WORKING_MB } else { memory::MEMORY_LIMIT_MB as u64 + WORKING_MB };
        if memory::available_memory_mb().map_err(ComputeError::Internal)?.is_some_and(|free| free < minimum) {
            self.unload(&mut slot);
            return Err(ComputeError::PausedForMemory { retry_after_s: FAILURE_PAUSE.as_secs() });
        }
        if slot.embedder.is_none() {
            self.loading.store(true, Ordering::SeqCst);
            let loaded = Embedder::load(&dir);
            self.loading.store(false, Ordering::SeqCst);
            match loaded {
                Ok(embedder) => {
                    slot.embedder = Some(embedder);
                    self.loaded.store(true, Ordering::SeqCst);
                }
                Err(error) => {
                    slot.retry_after = Some(Instant::now() + FAILURE_PAUSE);
                    return Err(ComputeError::Internal(error));
                }
            }
        }
        let result = work(slot.embedder.as_mut().expect("model loaded"));
        slot.last_used = Some(Instant::now());
        result.map_err(|error| {
            // Inference/allocation errors may leave large arenas behind.
            self.unload(&mut slot);
            if content {
                ComputeError::DocumentUnreadable(error)
            } else {
                slot.retry_after = Some(Instant::now() + FAILURE_PAUSE);
                ComputeError::Internal(error)
            }
        })
    }

    fn unload(&self, slot: &mut Slot) {
        slot.embedder = None;
        self.loaded.store(false, Ordering::SeqCst);
    }

    /// The parser process (for tests).
    pub fn parser(&self) -> &ParserPool {
        &self.parser
    }

    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }
}

fn percent(done: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (done.saturating_mul(100) / total).min(100) as u8)
}

impl ComputeBackend for OnnxBackend {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        self.with_model(false, |model| model.chunk_text(text))
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        self.embed_counted(inputs).map(|(vectors, _)| vectors)
    }

    /// Every input is measured first: one longer than [`MAX_INPUT_TOKENS`] refuses the request
    /// (`payload_too_large`, that content only) before any inference, without pausing the model.
    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        self.with_model(false, |model| {
            let mut tokens = 0u64;
            for input in inputs {
                let count = model.input_tokens(input)?;
                if count > MAX_INPUT_TOKENS {
                    return Ok(Err(ComputeError::PayloadTooLarge));
                }
                tokens += count as u64;
            }
            let vectors = inputs.iter().map(|input| model.embed(input)).collect::<Result<Vec<_>, _>>()?;
            Ok(Ok((vectors, tokens)))
        })?
    }

    fn embed_image(&self, bytes: &[u8]) -> Result<Vec<f32>, ComputeError> {
        // Decoding happens in the parser process; only patches reach this one (INV-11).
        let (response, payload) = self.parser.request("image", "", bytes)?;
        let pixels = Pixels::from_bytes(&payload, response.tokens).map_err(ComputeError::DocumentUnreadable)?;
        drop(payload);
        self.with_model(true, |model| model.embed_pixels(&pixels))
    }

    fn extract(&self, name: &str, bytes: &[u8]) -> Result<String, ComputeError> {
        // Text extraction never loads the model, so neither pause applies to it.
        let (response, _) = self.parser.request("extract", name, bytes)?;
        Ok(response.text)
    }

    fn models(&self) -> Vec<ModelStatus> {
        let installed = models::installed(&self.model_dir());
        let download = models::download_status();
        let loading = self.loading.load(Ordering::SeqCst);
        let state = if installed {
            if loading { "loading" } else { "ready" }
        } else if download.running || self.installing.load(Ordering::SeqCst) {
            "downloading"
        } else {
            "missing"
        };
        let total = models::total_bytes();
        let done = if installed { total } else { download.done };
        vec![ModelStatus {
            id: MODEL_ID.to_owned(),
            kind: "embedding".to_owned(),
            state: state.to_owned(),
            percent: if installed { Some(100) } else if state == "downloading" { percent(done, total) } else { None },
            done_bytes: done,
            total_bytes: total,
            error: if installed { None } else { download.error },
            loaded: self.loaded.load(Ordering::SeqCst),
            dimensions: DIM,
            quantization: QUANTIZATION.to_owned(),
        }]
    }

    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        if id != MODEL_ID {
            return Err(ComputeError::ModelMissing(format!("This server serves only {MODEL_ID}.")));
        }
        let dir = self.model_dir();
        if !models::installed(&dir) && !self.installing.swap(true, Ordering::SeqCst) {
            let installing = Arc::clone(&self.installing);
            let options = qamille_compute::download::Options::resumable().with_mirror(self.mirror.clone());
            let spawned = std::thread::Builder::new().name("model-install".to_owned()).spawn(move || {
                let result = models::download_model_with(&dir, &options, &AtomicBool::new(false), &|_, _| {});
                installing.store(false, Ordering::SeqCst);
                match result {
                    Ok(()) => tracing::info!(model = MODEL_ID, "model installed"),
                    // The message names a file or a network failure, never user content.
                    Err(error) => tracing::warn!(model = MODEL_ID, error = %error, "model installation failed"),
                }
            });
            if let Err(error) = spawned {
                self.installing.store(false, Ordering::SeqCst);
                return Err(ComputeError::Internal(format!("Cannot start the installation: {error}")));
            }
        }
        Ok(self.models().remove(0))
    }

    fn shutdown(&self) {
        self.parser.shutdown();
    }

    fn release_idle(&self) {
        self.parser.release_idle();
        if let Ok(mut slot) = self.slot.try_lock()
            && slot.embedder.is_some()
            && slot.last_used.is_some_and(|at| at.elapsed() >= IDLE)
        {
            self.unload(&mut slot);
        }
    }
}
