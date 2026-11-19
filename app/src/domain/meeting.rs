//! Meeting contracts shared by capture, processing, storage and desktop adapters.
use super::model::Meeting;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    #[default]
    Idle,
    Preparing,
    Recording,
    Paused,
    Saving,
    Saved,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CaptureRequest {
    pub meeting_id: u64,
    pub microphone: Option<String>,
    pub system: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AudioChunk {
    pub source: String,
    pub sequence: u64,
    pub path: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AudioGap {
    pub source: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CaptureSnapshot {
    pub meeting_id: Option<u64>,
    pub state: CaptureState,
    pub elapsed_ms: u64,
    pub saved_ms: u64,
    pub microphone_level: f32,
    pub system_level: f32,
    pub chunks: Vec<AudioChunk>,
    pub error: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub missing_chunks: Vec<AudioChunk>,
    #[serde(default)]
    pub incomplete: bool,
    #[serde(default)]
    pub gaps: Vec<AudioGap>,
    /// Legacy/imported captures predate independent integrity receipts.
    #[serde(default)]
    pub requires_receipts: bool,
    #[serde(default)]
    pub unverified_chunks: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TranscriptSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub original_text: String,
    pub source: String,
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TranscriptVersion {
    pub id: String,
    pub created_at: i64,
    pub model: String,
    pub language: String,
    pub segments: Vec<TranscriptSegment>,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MeetingNote {
    pub id: String,
    pub text: String,
    pub at_ms: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Evidence {
    pub kind: String,
    pub version: String,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Decision {
    pub text: String,
    pub sources: Vec<Evidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MeetingAction {
    pub id: String,
    pub text: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub done: bool,
    pub sources: Vec<Evidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SummaryVersion {
    pub id: String,
    pub created_at: i64,
    pub transcript_version: Option<String>,
    pub input_revision: u64,
    pub language: String,
    pub model: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub decisions: Vec<Decision>,
    pub actions: Vec<MeetingAction>,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MeetingJob {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub input_revision: u64,
    pub progress: u32,
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub options: Option<TranscribeOptions>,
    #[serde(default)]
    pub language: Option<String>,
    /// Persisted authorization origin; desktop retries must not turn MCP work
    /// into unrestricted native work. Contains no credentials.
    #[serde(default)]
    pub source_profile: Option<String>,
}

/// A completed worker's output, saved before changing the meeting manifest.
/// Contains results and input identity only, never provider credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeetingJobCheckpoint {
    pub schema_version: u32,
    pub meeting_id: u64,
    pub job_id: String,
    pub input_revision: u64,
    pub created_at: i64,
    pub transcript_version: Option<String>,
    pub cancelled: bool,
    pub output: Option<MeetingJobOutput>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MeetingJobOutput {
    Audio(Vec<AudioChunk>),
    Transcript(TranscriptVersion),
    Summary(SummaryVersion),
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StartMeeting {
    pub request_id: String,
    pub title: String,
    pub event_id: Option<u64>,
    pub microphone: Option<String>,
    pub system: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModelInstallStatus {
    pub state: String,
    pub model: String,
    pub progress: u32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeetingDetail {
    #[serde(default)]
    pub recovery_notice: Option<String>,
    #[serde(default)]
    pub recovery_copies: Vec<Meeting>,
    #[serde(default)]
    pub reconcile_projection: bool,
    pub schema_version: u32,
    pub revision: u64,
    pub meeting: Meeting,
    pub request_id: String,
    pub capture: CaptureSnapshot,
    pub transcripts: Vec<TranscriptVersion>,
    pub active_transcript: Option<String>,
    pub notes: Vec<MeetingNote>,
    #[serde(default)]
    pub note_history: Vec<MeetingNote>,
    #[serde(default)]
    pub legacy_transcript: Option<String>,
    #[serde(default)]
    pub legacy_notes: Option<String>,
    pub summaries: Vec<SummaryVersion>,
    pub active_summary: Option<String>,
    pub jobs: Vec<MeetingJob>,
    pub included_in_search: bool,
    #[serde(default)]
    pub deleted: bool,
    /// The recorded audio was deleted after a good transcript made it useless.
    #[serde(default)]
    pub audio_removed: bool,
    /// Why the audio was not deleted: `"empty_transcript"` or
    /// `"interrupted_capture"`. `None` while undecided or once removed.
    #[serde(default)]
    pub audio_kept: Option<String>,
    /// Computed by `refresh_flags`, never set by hand. The list page drops
    /// `capture.chunks`, `meeting.audio_path` and the transcripts, so it reads
    /// these two instead of looking inside.
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub has_text: bool,
    /// Stopped with no server to transcribe with: `MeetingService::resume_waiting` transcribes it
    /// when one is connected. Only flagged recordings resume, never the old ones left without text.
    #[serde(default)]
    pub transcription_waiting: bool,
}

impl MeetingDetail {
    pub fn legacy(meeting: Meeting) -> Self {
        let legacy_transcript =
            (!meeting.transcript.is_empty()).then(|| meeting.transcript.clone());
        let legacy_notes = (!meeting.notes.is_empty()).then(|| meeting.notes.clone());
        let notes = if meeting.notes.is_empty() {
            vec![]
        } else {
            vec![MeetingNote {
                id: "free".into(),
                text: meeting.notes.clone(),
                at_ms: None,
                revision: 1,
            }]
        };
        Self {
            recovery_notice: None,
            recovery_copies: vec![],
            reconcile_projection: false,
            schema_version: 2,
            revision: 0,
            meeting,
            request_id: String::new(),
            capture: CaptureSnapshot::default(),
            transcripts: vec![],
            active_transcript: None,
            notes,
            note_history: vec![],
            legacy_transcript,
            legacy_notes,
            summaries: vec![],
            active_summary: None,
            jobs: vec![],
            included_in_search: true,
            deleted: false,
            audio_removed: false,
            audio_kept: None,
            has_audio: false,
            has_text: false,
            transcription_waiting: false,
        }
    }
    /// Recomputes `has_audio` and `has_text` from the rest of the detail.
    /// Call it before a detail leaves the service, never from the UI.
    pub fn refresh_flags(&mut self) {
        self.has_audio = !self.audio_removed
            && (!self.capture.chunks.is_empty() || self.meeting.audio_path.is_some());
        self.has_text = self
            .transcript()
            .is_some_and(|t| t.segments.iter().any(|s| !s.text.trim().is_empty()))
            // A blank transcript joins its blank segments into blank text: still no text.
            || !self.meeting.transcript.trim().is_empty()
            || self.legacy_transcript.as_deref().is_some_and(|t| !t.trim().is_empty());
    }
    pub fn transcript(&self) -> Option<&TranscriptVersion> {
        self.active_transcript
            .as_ref()
            .and_then(|id| self.transcripts.iter().find(|t| &t.id == id))
    }
    pub fn summary(&self) -> Option<&SummaryVersion> {
        self.active_summary
            .as_ref()
            .and_then(|id| self.summaries.iter().find(|s| &s.id == id))
    }
}

/// What to do with a meeting's recorded audio once its transcript is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioVerdict {
    /// Not decidable yet: no complete transcript, capture not saved, nothing
    /// to delete, or the meeting is deleted / already cleaned.
    Pending,
    /// The transcript is complete and the capture is clean: delete the audio.
    Remove,
    /// Keep the audio, for this reason (stored in `MeetingDetail::audio_kept`).
    Keep(&'static str),
}

pub fn audio_verdict(detail: &MeetingDetail) -> AudioVerdict {
    if detail.deleted
        || detail.audio_removed
        || detail.capture.chunks.is_empty()
        || detail.capture.state != CaptureState::Saved
    {
        return AudioVerdict::Pending;
    }
    let Some(transcript) = detail.transcript().filter(|t| t.complete) else {
        return AudioVerdict::Pending;
    };
    if !transcript.segments.iter().any(|s| !s.text.trim().is_empty()) {
        return AudioVerdict::Keep("empty_transcript");
    }
    // A lost block or one that was never checked is not "clean", even when nothing marked the
    // capture incomplete: this audio may be the only copy of what the words missed.
    if detail.capture.incomplete
        || !detail.capture.gaps.is_empty()
        || !detail.capture.missing_chunks.is_empty()
        || !detail.capture.unverified_chunks.is_empty()
    {
        return AudioVerdict::Keep("interrupted_capture");
    }
    AudioVerdict::Remove
}

/// Whether the meeting engine can do its work now. A blocker is a sentence for the person,
/// so the page can say what to fix instead of showing a button that does nothing.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MeetingReadiness {
    /// Why transcribing with the engine chosen in Settings cannot start; `None` when it can.
    pub transcription_blocker: Option<String>,
    /// A summary can be written with the AI configured in Settings.
    pub summary_ready: bool,
}

/// Result of deleting the audio of old meetings in one sweep.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AudioCleanup {
    pub meetings: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct MeetingHit {
    pub meeting_id: u64,
    pub title: String,
    pub kind: String,
    pub text: String,
    pub at_ms: Option<u64>,
    pub version: Option<String>,
    pub segment_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeetingListPage {
    pub items: Vec<MeetingDetail>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub installed: bool,
    pub license: String,
}

/// «Server di calcolo»: whisper.cpp in `qamille-server`, the one of Settings > Compute (the
/// local child or the remote server), through the compute gateway (spec D7).
pub const ENGINE_COMPUTE: &str = "compute";
/// «Esterno compatibile OpenAI»: the `ai_base_url` endpoint with `ai_key`.
pub const ENGINE_EXTERNAL: &str = "external";
/// Values of `meeting_engine` and of `TranscribeOptions.provider` before the compute server:
/// `local` ran whisper.cpp in the app (now the compute server does), `remote` was the external
/// endpoint. Read as their new names; never written again.
pub const LEGACY_ENGINE_LOCAL: &str = "local";
pub const LEGACY_ENGINE_REMOTE: &str = "remote";
/// Ids of the Whisper models of the compute server (`whisper-<id>` on the server).
pub const LOCAL_SPEECH_MODELS: &[&str] = &["tiny", "base", "small"];

/// The current name of an engine value, old names included; other values stay as they are
/// (and are refused where they are checked).
pub fn canonical_engine(value: &str) -> &str {
    match value {
        LEGACY_ENGINE_LOCAL => ENGINE_COMPUTE,
        LEGACY_ENGINE_REMOTE => ENGINE_EXTERNAL,
        other => other,
    }
}

/// `serde(deserialize_with)` for engine fields: old files read as the new names.
pub fn deserialize_engine<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(canonical_engine(&value).to_owned())
}

/// A language code (`it`, `pt-BR`) or `auto`: ASCII letters and `-` only.
pub fn valid_transcription_language(language: &str) -> bool {
    !language.is_empty()
        && language.len() <= 16
        && language.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// The model name sent to the remote speech endpoint: empty (the engine then asks for
/// `whisper-1`), or up to 200 ASCII letters, digits and `-_./:`. Surrounding spaces are
/// ignored, as the engine ignores them.
pub fn valid_remote_model_name(name: &str) -> bool {
    let name = name.trim();
    name.len() <= 200
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:".contains(c))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscribeOptions {
    /// [`ENGINE_COMPUTE`] or [`ENGINE_EXTERNAL`] (jobs saved before read as these too).
    #[serde(deserialize_with = "deserialize_engine")]
    pub provider: String,
    pub model: String,
    pub language: String,
}
impl Default for TranscribeOptions {
    fn default() -> Self {
        Self {
            provider: ENGINE_COMPUTE.into(),
            model: "base".into(),
            language: "it".into(),
        }
    }
}
