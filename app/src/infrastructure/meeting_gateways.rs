use super::{
    meeting_audio::{NativeAudioCapture, mix_audio},
    meeting_processing::{self, ProcessingEngine},
};
use crate::{
    application::{AppError, AppResult, meeting_ports::*},
    domain::{
        meeting::*,
        model::{AiProvider, Settings},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

impl AudioCapture for NativeAudioCapture {
    fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        self.devices()
    }
    fn start(&self, request: CaptureRequest, folder: PathBuf) -> AppResult<CaptureSnapshot> {
        self.start(request, folder)
    }
    fn pause(&self) -> AppResult<CaptureSnapshot> {
        self.pause()
    }
    fn resume(&self) -> AppResult<CaptureSnapshot> {
        self.resume()
    }
    fn stop(&self) -> AppResult<CaptureSnapshot> {
        self.stop()
    }
    fn snapshot(&self) -> AppResult<CaptureSnapshot> {
        self.snapshot()
    }
    fn audio_range(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        start_ms: u64,
        duration_ms: u64,
    ) -> AppResult<Vec<u8>> {
        let samples = mix_audio(
            folder,
            chunks,
            start_ms,
            start_ms.saturating_add(duration_ms),
            16_000,
        )?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav = hound::WavWriter::new(&mut bytes, spec)
            .map_err(|e| AppError::external(e.to_string()))?;
        for sample in samples {
            wav.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| AppError::external(e.to_string()))?;
        }
        wav.finalize()
            .map_err(|e| AppError::external(e.to_string()))?;
        Ok(bytes.into_inner())
    }
}
/// The meeting engine of this process (window, MCP server): its gateway follows
/// `compute_factory::current()` at each job.
pub fn shared_processor() -> std::sync::Arc<dyn MeetingProcessor> {
    std::sync::Arc::new(ProcessingEngine::new())
}

impl MeetingProcessor for ProcessingEngine {
    fn authorize_mcp_ai(&self, profile: Option<&str>, maximum: crate::model::Access) -> AppResult<()> {
        let Some(name) = profile else { return Ok(()); };
        let grant = super::mcp_config::profile(name, maximum)?;
        if !grant.ai_execute
            || super::mcp_config::rank(grant.access) < super::mcp_config::rank(crate::model::Access::Triage)
        {
            return Err(AppError::new("permission_denied", "MCP AI permission was revoked; remaining processing was stopped"));
        }
        Ok(())
    }
    fn models(&self) -> AppResult<Vec<SpeechModel>> {
        self.models()
    }
    fn transcription_blocker(&self, options: &TranscribeOptions, settings: &Settings) -> Option<String> {
        match canonical_engine(&options.provider) {
            // A server that does not answer says why; its models are then unknown.
            ENGINE_COMPUTE => match self.models() {
                Ok(models) => meeting_processing::local_blocker(&models, &options.model),
                Err(error) if error.code == crate::application::compute::codes::NOT_CONNECTED => {
                    Some(crate::application::meeting_ports::NO_SERVER_BLOCKER.into())
                }
                Err(error) => Some(error.message),
            },
            ENGINE_EXTERNAL => (settings.ai_provider != AiProvider::OpenAiCompatible).then(|| {
                "Configura un endpoint compatibile OpenAI in Impostazioni per trascrivere in remoto.".into()
            }),
            _ => Some("Trascrizione non configurata.".into()),
        }
    }
    fn summary_ready(&self, settings: &Settings) -> bool {
        super::ai::online(settings)
    }
    fn transcribe(
        &self,
        folder: &Path,
        chunks: &[AudioChunk],
        options: &TranscribeOptions,
        settings: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<TranscriptVersion> {
        self.transcribe(folder, chunks, options, settings, cancel, progress)
    }
    fn summarize(
        &self,
        detail: &MeetingDetail,
        settings: &Settings,
        language: &str,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<SummaryVersion> {
        self.summarize(detail, settings, language, cancel, progress)
    }
    fn import_audio(
        &self,
        source: &Path,
        folder: &Path,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<Vec<AudioChunk>> {
        meeting_processing::import_audio(source, folder, cancel, progress)
    }
    fn transcribe_authorized(
        &self, folder: &Path, chunks: &[AudioChunk], options: &TranscribeOptions,
        settings: &Settings, cancel: &AtomicBool, progress: &dyn Fn(u32),
        authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<TranscriptVersion> {
        self.transcribe_authorized(folder, chunks, options, settings, cancel, progress, authorize)
    }
    fn summarize_authorized(
        &self, detail: &MeetingDetail, settings: &Settings, language: &str,
        cancel: &AtomicBool, progress: &dyn Fn(u32), authorize: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<SummaryVersion> {
        self.summarize_authorized(detail, settings, language, cancel, progress, authorize)
    }
    fn export(&self, detail: &MeetingDetail, format: &str) -> AppResult<String> {
        meeting_processing::export_meeting(detail, format)
    }
}
