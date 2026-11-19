use super::AppResult;
use crate::domain::{meeting::*, model::Settings};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

/// Why the server's engine cannot transcribe when no server is connected. A module constant
/// (not an associated one) so that `MeetingProcessor` stays usable as a trait object.
pub const NO_SERVER_BLOCKER: &str = "Collega un server Qamille in Impostazioni per trascrivere con il motore del server.";

/// Held until a worker exits. The operating system releases it after a crash.
pub trait MeetingJobLease: Send + Sync {
    fn cancellation_requested(&self, job_id: &str) -> AppResult<bool>;
}

pub trait MeetingRepository: Send + Sync {
    fn claim_job(&self, id: u64) -> AppResult<Arc<dyn MeetingJobLease>>;
    fn request_job_cancel(&self, id: u64, job_id: &str) -> AppResult<()>;
    fn unverified_audio(&self, id: u64, chunks: &[AudioChunk]) -> AppResult<Vec<String>>;
    fn list(&self) -> AppResult<Vec<MeetingDetail>>;
    fn read(&self, id: u64) -> AppResult<Option<MeetingDetail>>;
    fn create(&self, detail: &MeetingDetail) -> AppResult<()>;
    fn update(
        &self,
        id: u64,
        edit: &mut dyn FnMut(&mut MeetingDetail) -> AppResult<()>,
    ) -> AppResult<MeetingDetail>;
    fn folder(&self, id: u64) -> PathBuf;
    fn capture(&self, id: u64) -> AppResult<Option<CaptureSnapshot>>;
    fn recover_capture(&self, id: u64) -> AppResult<CaptureSnapshot>;
    fn remove_audio(&self, id: u64) -> AppResult<()>;
    /// Delete only the audio files of a live meeting and keep everything else; returns the
    /// bytes freed. It does NOT claim the job lease: the caller already holds it (a worker
    /// thread, or recovery). Fails with `recording_busy` while a recorder owns the folder.
    fn discard_audio(&self, id: u64) -> AppResult<u64>;
    /// What `discard_audio` would free, without touching anything.
    fn audio_bytes(&self, id: u64) -> AppResult<u64>;
    fn save_job_result(&self, checkpoint: &MeetingJobCheckpoint) -> AppResult<()>;
    fn job_results(&self, id: u64) -> AppResult<(Vec<MeetingJobCheckpoint>, Vec<String>)>;
    fn clear_job_result(&self, id: u64, job_id: &str) -> AppResult<()>;
}

pub trait AudioCapture: Send + Sync {
    fn devices(&self) -> AppResult<Vec<AudioDevice>>;
    fn start(&self, request: CaptureRequest, folder: PathBuf) -> AppResult<CaptureSnapshot>;
    fn pause(&self) -> AppResult<CaptureSnapshot>;
    fn resume(&self) -> AppResult<CaptureSnapshot>;
    fn stop(&self) -> AppResult<CaptureSnapshot>;
    fn snapshot(&self) -> AppResult<CaptureSnapshot>;
    fn audio_range(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        start_ms: u64,
        duration_ms: u64,
    ) -> AppResult<Vec<u8>>;
}

pub trait MeetingProcessor: Send + Sync {
    /// Re-read the current MCP grant before a remote effect. Native jobs have no
    /// profile. Implementations without grant support fail closed for MCP jobs.
    fn authorize_mcp_ai(&self, profile: Option<&str>, _maximum: crate::model::Access) -> AppResult<()> {
        if profile.is_some() {
            Err(super::AppError::new("permission_denied", "MCP AI authorization is unavailable"))
        } else {
            Ok(())
        }
    }
    fn models(&self) -> AppResult<Vec<SpeechModel>>;
    /// Why `options` cannot transcribe right now, in words for the person, or `None` when
    /// it can. The default fails closed: an engine that does not say stays blocked.
    fn transcription_blocker(&self, _options: &TranscribeOptions, _settings: &Settings) -> Option<String> {
        Some("Trascrizione non configurata.".into())
    }
    /// Whether a summary can be written with these settings. Fails closed by default.
    fn summary_ready(&self, _settings: &Settings) -> bool {
        false
    }
    fn transcribe(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        options: &TranscribeOptions,
        settings: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<TranscriptVersion>;
    fn summarize(
        &self,
        detail: &MeetingDetail,
        settings: &Settings,
        language: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<SummaryVersion>;
    /// Implementations that issue multiple requests must call authorize before
    /// each request, including retries. The default supports single-effect fakes.
    fn transcribe_authorized(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        options: &TranscribeOptions,
        settings: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
        authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<TranscriptVersion> {
        if options.provider == crate::domain::meeting::ENGINE_EXTERNAL { authorize()?; }
        self.transcribe(folder, chunks, options, settings, cancel, progress)
    }
    fn summarize_authorized(
        &self,
        detail: &MeetingDetail,
        settings: &Settings,
        language: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
        authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<SummaryVersion> {
        authorize()?;
        self.summarize(detail, settings, language, cancel, progress)
    }
    fn import_audio(
        &self,
        source: &Path,
        folder: &Path,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<Vec<AudioChunk>>;
    fn export(&self, detail: &MeetingDetail, format: &str) -> AppResult<String>;
}
