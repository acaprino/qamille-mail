//! Bounded meeting processing. Downloads and cloud requests happen only on explicit jobs.
mod export;
mod import;
mod models;
mod speech;
mod summary;

pub use export::export_meeting;
pub use import::import_audio;

use crate::{
    application::{AppError, AppResult, compute::ComputeGateway},
    domain::meeting::*,
    model::Settings,
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// Transcription, model installation and summaries of the meetings. Speech recognition goes
/// through the compute gateway ([`ComputeGateway::transcribe_window`]); unless one is given, it
/// is the one of `compute_factory::current()` when each job starts.
#[derive(Clone, Default)]
pub struct ProcessingEngine {
    compute: Option<Arc<dyn ComputeGateway>>,
}
impl ProcessingEngine {
    /// The engine of this process: the gateway follows Settings > Compute (D3).
    pub fn new() -> Self {
        Self { compute: None }
    }
    /// With a gateway of its own (tests, tools).
    pub fn with_compute(compute: Arc<dyn ComputeGateway>) -> Self {
        Self { compute: Some(compute) }
    }
    fn compute(&self) -> Arc<dyn ComputeGateway> {
        self.compute.clone().unwrap_or_else(crate::infrastructure::compute_factory::current)
    }
    pub fn models(&self) -> AppResult<Vec<SpeechModel>> {
        models::list(&*self.compute())
    }
    pub fn transcribe(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        options: &TranscribeOptions,
        settings: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<TranscriptVersion> {
        speech::transcribe(
            &*self.compute(), folder, chunks, options, settings, cancel, progress, &|| Ok(()),
        )
    }
    pub fn transcribe_authorized(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        options: &TranscribeOptions,
        settings: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
        authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<TranscriptVersion> {
        speech::transcribe(&*self.compute(), folder, chunks, options, settings, cancel, progress, authorize)
    }
    pub fn summarize(
        &self,
        detail: &MeetingDetail,
        settings: &Settings,
        language: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<SummaryVersion> {
        summary::summarize(detail, settings, language, cancel, progress, &|| Ok(()))
    }
    pub fn summarize_authorized(
        &self,
        detail: &MeetingDetail,
        settings: &Settings,
        language: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
        authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<SummaryVersion> {
        summary::summarize(detail, settings, language, cancel, progress, authorize)
    }
}

/// Why the compute server cannot transcribe with `model`, or `None` when that model is
/// installed there. A model the catalog does not list is as unusable as one not downloaded yet.
pub(crate) fn local_blocker(models: &[SpeechModel], model: &str) -> Option<String> {
    if models.iter().any(|m| m.id == model && m.installed) {
        None
    } else {
        Some("Il server non ha questo modello di trascrizione: lo installa chi lo amministra.".into())
    }
}

pub(crate) fn cancelled(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(AppError::new("cancelled", "Operazione annullata."))
    } else {
        Ok(())
    }
}
pub(crate) fn unique_id() -> AppResult<String> {
    let mut bytes = [0_u8; 12];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::storage("Impossibile creare un identificatore di lavoro."))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn processing_error(message: impl Into<String>) -> AppError {
    AppError::new("processing", message)
}

pub(crate) fn reserve_disk(
    path: &Path,
    bytes: u64,
) -> AppResult<crate::infrastructure::storage_fs::DiskReservation> {
    // storage_fs already preserves 200 MB; bring meeting jobs to a 256 MiB margin.
    let bytes = bytes
        .checked_add(256 * 1024 * 1024 - 200_000_000)
        .ok_or_else(|| {
            AppError::new(
                "disk_full",
                "Spazio su disco insufficiente per completare l'operazione.",
            )
        })?;
    crate::infrastructure::storage_fs::reserve(path, bytes).map_err(|error| {
        if error.code == "disk_full" {
            AppError::new("disk_full", "Spazio su disco insufficiente. Libera spazio e riprova; l'audio esistente è conservato.")
        } else { error }
    })
}

pub(crate) fn write_wav(path: &Path, samples: &[f32], rate: u32) -> AppResult<()> {
    let _space = reserve_disk(
        path,
        (samples.len() as u64).saturating_mul(2).saturating_add(44),
    )?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| AppError::storage(e.to_string()))?;
    for sample in samples {
        if !sample.is_finite() {
            return Err(processing_error("Campione audio non valido."));
        }
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| AppError::storage(e.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|e| AppError::storage(e.to_string()))?;
    std::fs::OpenOptions::new().write(true).open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
pub(super) mod http_fixture {
    use std::{io::{Read, Write}, net::TcpListener, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}, time::{Duration, Instant}};
    pub struct Server {
        pub url: String,
        pub calls: Arc<AtomicUsize>,
        pub allowed: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    fn complete_request(bytes: &[u8]) -> bool {
        let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else { return false; };
        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
        if let Some(length) = headers.lines().find_map(|l| l.strip_prefix("content-length:")) {
            return bytes.len() >= end + 4 + length.trim().parse::<usize>().unwrap();
        }
        assert!(headers.lines().any(|line| line == "transfer-encoding: chunked"), "Request has no body framing");
        let mut position = end + 4;
        loop {
            let Some(line_end) = bytes[position..].windows(2).position(|v| v == b"\r\n") else { return false; };
            let line = std::str::from_utf8(&bytes[position..position + line_end]).unwrap();
            let size = usize::from_str_radix(line.split(';').next().unwrap(), 16).unwrap();
            position += line_end + 2;
            if size == 0 {
                return bytes[position..].starts_with(b"\r\n")
                    || bytes[position..].windows(4).any(|v| v == b"\r\n\r\n");
            }
            if bytes.len() < position + size + 2 { return false; }
            assert_eq!(&bytes[position + size..position + size + 2], b"\r\n");
            position += size + 2;
        }
    }
    impl Server {
        pub fn new(body: String, status: u16, revoke_after: usize) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/v1", listener.local_addr().unwrap());
            let calls = Arc::new(AtomicUsize::new(0));
            let allowed = Arc::new(AtomicBool::new(true));
            let stop = Arc::new(AtomicBool::new(false));
            let (count, grant, done) = (calls.clone(), allowed.clone(), stop.clone());
            let worker = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
                while !done.load(Ordering::SeqCst) && Instant::now() < deadline {
                    let mut stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => { std::thread::sleep(Duration::from_millis(5)); continue; }
                        Err(e) => panic!("{e}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 4096];
                    loop {
                        let n = stream.read(&mut buffer).unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&buffer[..n]);
                        assert!(bytes.len() < 2_000_000);
                        if complete_request(&bytes) { break; }
                    }
                    if count.fetch_add(1, Ordering::SeqCst) + 1 >= revoke_after { grant.store(false, Ordering::SeqCst); }
                    write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            Self { url, calls, allowed, stop, worker: Some(worker) }
        }
        pub fn authorize(&self) -> crate::application::AppResult<()> {
            if self.allowed.load(Ordering::SeqCst) { Ok(()) }
            else { Err(crate::application::AppError::new("permission_denied", "synthetic revoked grant")) }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let result = self.worker.take().unwrap().join();
            if !std::thread::panicking() { result.unwrap(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::meeting_ports::MeetingProcessor;
    use crate::model::AiProvider;

    fn options(provider: &str, model: &str) -> TranscribeOptions {
        TranscribeOptions { provider: provider.into(), model: model.into(), language: "it".into() }
    }
    fn engine_without_files() -> ProcessingEngine {
        ProcessingEngine::with_compute(Arc::new(crate::infrastructure::fake_compute::FakeCompute::default()))
    }

    #[test]
    fn engine_without_the_local_model_is_blocked() {
        let engine = engine_without_files();
        let blocker = engine.transcription_blocker(&options(ENGINE_COMPUTE, "base"), &Settings::default());
        assert_eq!(blocker.as_deref(), Some("Il server non ha questo modello di trascrizione: lo installa chi lo amministra."));
        // A model the catalog does not know can never be installed: also blocked.
        assert!(engine.transcription_blocker(&options(ENGINE_COMPUTE, "huge"), &Settings::default()).is_some());
        // The same engine through the trait object the service uses.
        let service_view: &dyn MeetingProcessor = &engine;
        assert!(service_view.transcription_blocker(&options(ENGINE_COMPUTE, "base"), &Settings::default()).is_some());
        // The names saved before the compute server mean the same engines.
        assert_eq!(engine.transcription_blocker(&options("local", "base"), &Settings::default()), blocker);
    }

    #[test]
    fn a_model_installed_on_the_compute_server_is_not_blocked() {
        let fake = Arc::new(crate::infrastructure::fake_compute::FakeCompute::default());
        fake.speech_ready.lock().unwrap().push("whisper-base".into());
        let engine = ProcessingEngine::with_compute(fake.clone());
        assert!(engine.transcription_blocker(&options(ENGINE_COMPUTE, "base"), &Settings::default()).is_none());
        assert!(engine.models().unwrap().iter().any(|m| m.id == "base" && m.installed));
    }

    #[test]
    fn the_meeting_blocker_without_a_server_says_to_connect_one() {
        let fake = Arc::new(crate::infrastructure::fake_compute::FakeCompute::default());
        *fake.fail.lock().unwrap() = Some((crate::application::compute::codes::NOT_CONNECTED, "not connected".into()));
        let engine = ProcessingEngine::with_compute(fake);
        let blocker = engine.transcription_blocker(&options(ENGINE_COMPUTE, "base"), &Settings::default());
        assert_eq!(blocker.as_deref(), Some(crate::application::meeting_ports::NO_SERVER_BLOCKER));
        assert_eq!(blocker.as_deref(), Some("Collega un server Qamille in Impostazioni per trascrivere con il motore del server."));
        assert!(!blocker.unwrap().contains("Installa il modello"));
    }

    #[test]
    fn an_unreachable_compute_server_says_so() {
        let fake = Arc::new(crate::infrastructure::fake_compute::FakeCompute::default());
        *fake.fail.lock().unwrap() = Some((crate::application::compute::codes::UNREACHABLE, "The compute server is not responding; the request will be retried.".into()));
        let engine = ProcessingEngine::with_compute(fake);
        assert_eq!(
            engine.transcription_blocker(&options(ENGINE_COMPUTE, "base"), &Settings::default()).as_deref(),
            Some("The compute server is not responding; the request will be retried.")
        );
    }

    #[test]
    fn an_installed_local_model_is_not_blocked() {
        let installed = |id: &str, installed: bool| SpeechModel { id: id.into(), installed, ..Default::default() };
        let models = [installed("tiny", true), installed("base", false)];
        assert!(local_blocker(&models, "tiny").is_none());
        assert!(local_blocker(&models, "base").is_some(), "listed but not installed");
        assert!(local_blocker(&models, "small").is_some(), "not listed");
    }

    #[test]
    fn remote_engine_needs_an_openai_compatible_provider() {
        let engine = engine_without_files();
        let remote = options(ENGINE_EXTERNAL, "whisper-1");
        let offline = Settings::default();
        assert_eq!(
            engine.transcription_blocker(&remote, &offline).as_deref(),
            Some("Configura un endpoint compatibile OpenAI in Impostazioni per trascrivere in remoto.")
        );
        let ollama = Settings { ai_provider: AiProvider::Ollama, ..Default::default() };
        assert!(engine.transcription_blocker(&remote, &ollama).is_some());
        let compatible = Settings { ai_provider: AiProvider::OpenAiCompatible, ..Default::default() };
        assert!(engine.transcription_blocker(&remote, &compatible).is_none());
        assert!(engine.transcription_blocker(&options("remote", "whisper-1"), &compatible).is_none(), "the old name");
    }

    #[test]
    fn an_unknown_engine_is_blocked() {
        let engine = engine_without_files();
        assert!(engine.transcription_blocker(&options("telepathy", "x"), &Settings::default()).is_some());
    }

    #[test]
    fn summary_ready_follows_ai_online() {
        let engine = engine_without_files();
        assert!(!engine.summary_ready(&Settings::default()), "offline");
        let ollama = Settings { ai_provider: AiProvider::Ollama, ..Default::default() };
        assert!(engine.summary_ready(&ollama));
        let no_key = Settings { ai_provider: AiProvider::OpenAiCompatible, ..Default::default() };
        assert!(!engine.summary_ready(&no_key), "a hosted provider needs its key");
        let keyed = Settings { ai_provider: AiProvider::OpenAiCompatible, ai_key: "synthetic".into(), ..Default::default() };
        assert!(engine.summary_ready(&keyed));
        for settings in [&Settings::default(), &ollama, &no_key, &keyed] {
            assert_eq!(engine.summary_ready(settings), crate::infrastructure::ai::online(settings));
        }
    }
}
