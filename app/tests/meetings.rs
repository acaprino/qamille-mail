use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        meeting_ports::*,
        meeting_reminders::{MeetingReminderService, ReminderCalendar},
        meetings::{MeetingService, search_details},
        ports::*,
    },
    domain::{meeting::*, reminders::ReminderCandidate},
    infrastructure::{meeting_repository::FileMeetingRepository, repository::MemoryRepository},
    model::*,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

struct ClockStub;
impl Clock for ClockStub {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct Attachments;
impl AttachmentGateway for Attachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
struct Mail;
impl MailGateway for Mail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        unreachable!()
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        unreachable!()
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        unreachable!()
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        unreachable!()
    }
}
#[derive(Default)]
struct Audio {
    snapshot: Mutex<CaptureSnapshot>,
    starts: AtomicU64,
    devices: Mutex<Vec<AudioDevice>>,
    /// `stop` ends the capture as `Interrupted` (a recorder fault) instead of `Saved`.
    interrupt_on_stop: AtomicBool,
    /// What every `start` was asked to capture, in order.
    requests: Mutex<Vec<CaptureRequest>>,
    /// How many times `stop` reached the recorder.
    stops: AtomicU64,
    /// The next `snapshot` fails (the recorder cannot report), then they work again.
    fail_next_snapshot: AtomicBool,
}
impl AudioCapture for Audio {
    fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        Ok(self.devices.lock().unwrap().clone())
    }
    fn start(&self, r: CaptureRequest, _: PathBuf) -> AppResult<CaptureSnapshot> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(r.clone());
        *self.snapshot.lock().unwrap() = CaptureSnapshot {
            meeting_id: Some(r.meeting_id),
            state: CaptureState::Recording,
            elapsed_ms: 1000,
            saved_ms: 1000,
            chunks: vec![AudioChunk {
                source: "microphone".into(),
                path: "test.wav".into(),
                end_ms: 1000,
                sample_rate: 16000,
                channels: 1,
                ..Default::default()
            }],
            ..Default::default()
        };
        self.snapshot()
    }
    fn pause(&self) -> AppResult<CaptureSnapshot> {
        self.snapshot.lock().unwrap().state = CaptureState::Paused;
        self.snapshot()
    }
    fn resume(&self) -> AppResult<CaptureSnapshot> {
        self.snapshot.lock().unwrap().state = CaptureState::Recording;
        self.snapshot()
    }
    fn stop(&self) -> AppResult<CaptureSnapshot> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.snapshot.lock().unwrap().state = if self.interrupt_on_stop.load(Ordering::SeqCst) {
            CaptureState::Interrupted
        } else {
            CaptureState::Saved
        };
        self.snapshot()
    }
    fn snapshot(&self) -> AppResult<CaptureSnapshot> {
        if self.fail_next_snapshot.swap(false, Ordering::SeqCst) {
            return Err(AppError::external("Recorder non disponibile"));
        }
        Ok(self.snapshot.lock().unwrap().clone())
    }
    fn audio_range(&self, _: &Path, _: &[AudioChunk], _: u64, _: u64) -> AppResult<Vec<u8>> {
        Ok(vec![])
    }
}
struct Processor {
    calls: AtomicU64,
    panic_transcribe: AtomicBool,
    release: AtomicBool,
    entered: AtomicBool,
    fail: AtomicBool,
    ai_allowed: AtomicBool,
    /// Transcribe without recognising any words: a complete transcript with no text.
    empty_text: AtomicBool,
    /// The engine can transcribe (`transcription_blocker` is `None`). Off by default, so
    /// a test must opt in before anything starts by itself after a stop.
    ready: AtomicBool,
    /// A summary can be written (`summary_ready`). Off by default as well.
    summary: AtomicBool,
    /// Every summary request fails.
    fail_summary: AtomicBool,
    /// The remote engine cannot work, whatever `ready` says about the local one.
    remote_blocked: AtomicBool,
}
impl Default for Processor {
    fn default() -> Self {
        Self {
            calls: AtomicU64::new(0),
            panic_transcribe: AtomicBool::new(false),
            release: AtomicBool::new(true),
            entered: AtomicBool::new(false),
            fail: AtomicBool::new(false),
            ai_allowed: AtomicBool::new(true),
            empty_text: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            summary: AtomicBool::new(false),
            fail_summary: AtomicBool::new(false),
            remote_blocked: AtomicBool::new(false),
        }
    }
}
impl MeetingProcessor for Processor {
    fn authorize_mcp_ai(&self, profile: Option<&str>, _: Access) -> AppResult<()> {
        if profile.is_some() && !self.ai_allowed.load(Ordering::SeqCst) {
            Err(AppError::new("permission_denied", "synthetic revoked grant"))
        } else { Ok(()) }
    }
    fn models(&self) -> AppResult<Vec<SpeechModel>> {
        Ok(vec![SpeechModel {
            id: "fixture".into(),
            ..Default::default()
        }])
    }
    fn transcription_blocker(&self, options: &TranscribeOptions, _: &Settings) -> Option<String> {
        if !self.ready.load(Ordering::SeqCst) {
            Some("Trascrizione non configurata.".into())
        } else if options.provider == "external" && self.remote_blocked.load(Ordering::SeqCst) {
            Some("Configura un endpoint compatibile OpenAI in Impostazioni per trascrivere in remoto.".into())
        } else {
            None
        }
    }
    fn summary_ready(&self, _: &Settings) -> bool {
        self.summary.load(Ordering::SeqCst)
    }
    fn transcribe(
        &self,
        _: &Path,
        _: &[AudioChunk],
        _: &TranscribeOptions,
        _: &Settings,
        cancel: &AtomicBool,
        progress: &dyn Fn(u32),
    ) -> AppResult<TranscriptVersion> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_transcribe.load(Ordering::SeqCst),
            "synthetic processor panic"
        );
        self.entered.store(true, Ordering::SeqCst);
        progress(30);
        while !self.release.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppError::external("Model failed"));
        }
        if self.empty_text.load(Ordering::SeqCst) {
            return Ok(TranscriptVersion {
                segments: vec![TranscriptSegment {
                    id: "seg1".into(),
                    text: "  ".into(),
                    end_ms: 1000,
                    ..Default::default()
                }],
                complete: true,
                ..Default::default()
            });
        }
        Ok(TranscriptVersion {
            segments: vec![TranscriptSegment {
                id: "seg1".into(),
                text: "Decisione: rilascio venerdì".into(),
                original_text: "Decisione: rilascio venerdì".into(),
                end_ms: 1000,
                ..Default::default()
            }],
            complete: true,
            ..Default::default()
        })
    }
    fn summarize(
        &self,
        _: &MeetingDetail,
        _: &Settings,
        _: &str,
        _: &AtomicBool,
        _: &dyn Fn(u32),
    ) -> AppResult<SummaryVersion> {
        if self.fail_summary.load(Ordering::SeqCst) {
            return Err(AppError::external("Summary failed"));
        }
        Ok(SummaryVersion {
            summary: "Verbale".into(),
            complete: true,
            ..Default::default()
        })
    }
    fn import_audio(
        &self,
        _: &Path,
        _: &Path,
        _: &AtomicBool,
        _: &dyn Fn(u32),
    ) -> AppResult<Vec<AudioChunk>> {
        Ok(vec![])
    }
    fn export(&self, d: &MeetingDetail, format: &str) -> AppResult<String> {
        qamille::infrastructure::meeting_processing::export_meeting(d, format)
    }
}
/// Implements only what the trait requires: every default is what an engine that knows
/// nothing about readiness gets.
struct Bare;
impl MeetingProcessor for Bare {
    fn models(&self) -> AppResult<Vec<SpeechModel>> {
        Ok(vec![])
    }
    fn transcribe(
        &self,
        _: &Path,
        _: &[AudioChunk],
        _: &TranscribeOptions,
        _: &Settings,
        _: &AtomicBool,
        _: &dyn Fn(u32),
    ) -> AppResult<TranscriptVersion> {
        unreachable!()
    }
    fn summarize(
        &self,
        _: &MeetingDetail,
        _: &Settings,
        _: &str,
        _: &AtomicBool,
        _: &dyn Fn(u32),
    ) -> AppResult<SummaryVersion> {
        unreachable!()
    }
    fn import_audio(
        &self,
        _: &Path,
        _: &Path,
        _: &AtomicBool,
        _: &dyn Fn(u32),
    ) -> AppResult<Vec<AudioChunk>> {
        unreachable!()
    }
    fn export(&self, _: &MeetingDetail, _: &str) -> AppResult<String> {
        unreachable!()
    }
}
struct Fixture {
    service: MeetingService,
    context: Arc<ServiceContext>,
    repo: Arc<FileMeetingRepository>,
    audio: Arc<Audio>,
    processor: Arc<Processor>,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "qamille-meeting-service-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        let mut store = sample_store();
        store.meetings.clear();
        let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
            events: Arc::new(NullSink),
            repository: Arc::new(MemoryRepository::new(store)),
            mail: Arc::new(Mail),
            secrets: Arc::new(Secrets),
            clock: Arc::new(ClockStub),
            attachments: Arc::new(Attachments),
        });
        let repo = Arc::new(FileMeetingRepository::new(root.clone()));
        let audio = Arc::new(Audio::default());
        let processor = Arc::new(Processor::default());
        let service = MeetingService::new(
            context.clone(),
            repo.clone(),
            audio.clone(),
            processor.clone(),
        );
        Self {
            service,
            context,
            repo,
            audio,
            processor,
            root,
        }
    }
    fn start(&self) -> MeetingDetail {
        self.service
            .start(StartMeeting {
                request_id: "same-click".into(),
                microphone: Some("mic".into()),
                title: "Test".into(),
                ..Default::default()
            })
            .unwrap()
    }
    fn saved(&self) -> MeetingDetail {
        let d = self.start();
        self.service.stop().unwrap();
        self.service.read(d.meeting.id).unwrap()
    }
    /// Like `saved`, but the capture is marked incomplete: a transcript with text keeps
    /// the audio (`audio_kept`), so the meeting can be transcribed again. `saved()` is a
    /// clean capture, whose audio goes away after the first transcript with text.
    fn saved_keeping_audio(&self) -> MeetingDetail {
        let id = self.saved().meeting.id;
        self.repo
            .update(id, &mut |d| {
                d.capture.incomplete = true;
                Ok(())
            })
            .unwrap();
        self.service.read(id).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.processor.release.store(true, Ordering::SeqCst);
        self.service.cancel_all_jobs();
    }
}
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "worker did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn finished(f: &Fixture, id: u64) -> MeetingDetail {
    wait(|| {
        f.service
            .read(id)
            .unwrap()
            .jobs
            .last()
            .is_some_and(|j| !matches!(j.state.as_str(), "queued" | "running"))
    });
    f.service.read(id).unwrap()
}

#[test]
fn native_retry_after_restart_preserves_the_mcp_job_authorization_origin() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.processor.fail.store(true, Ordering::SeqCst);
    f.service.clone().with_source_profile("review-profile".into())
        .transcribe(id, TranscribeOptions::default()).unwrap();
    let failed = finished(&f, id);
    let job = failed.jobs.last().unwrap();
    assert_eq!(job.source_profile.as_deref(), Some("review-profile"));
    assert_eq!(job.state, "failed");
    let reopened = MeetingService::new(f.context.clone(), f.repo.clone(), f.audio.clone(), f.processor.clone());
    reopened.recover_interrupted().unwrap();
    f.processor.ai_allowed.store(false, Ordering::SeqCst);
    f.processor.fail.store(false, Ordering::SeqCst);
    let calls = f.processor.calls.load(Ordering::SeqCst);
    assert_eq!(reopened.retry_job(id, &job.id).unwrap_err().code, "permission_denied");
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), calls);
    // Starting a new, explicit native job has no MCP grant requirement.
    reopened.transcribe(id, TranscribeOptions::default()).unwrap();
    let completed = finished(&f, id);
    assert_eq!(completed.jobs.last().unwrap().state, "completed");
    assert!(completed.jobs.last().unwrap().source_profile.is_none());
}

fn completed_checkpoint(id: u64, job_id: &str, revision: u64) -> MeetingJobCheckpoint {
    MeetingJobCheckpoint {
        schema_version: 1,
        meeting_id: id,
        job_id: job_id.into(),
        input_revision: revision,
        created_at: 2_000_000_000,
        transcript_version: None,
        cancelled: false,
        error: None,
        output: Some(MeetingJobOutput::Transcript(TranscriptVersion {
            complete: true,
            segments: vec![TranscriptSegment {
                id: "recovered".into(),
                text: "Risultato già pronto".into(),
                ..Default::default()
            }],
            ..Default::default()
        })),
    }
}

#[test]
fn restart_applies_a_saved_worker_result_exactly_once_without_calling_the_provider() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "ready".into(),
                state: "running".into(),
                kind: "transcribe".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.repo
        .save_job_result(&completed_checkpoint(id, "ready", d.revision))
        .unwrap();
    let reopened = MeetingService::new(
        f.context.clone(),
        f.repo.clone(),
        f.audio.clone(),
        f.processor.clone(),
    );
    reopened.recover_interrupted().unwrap();
    let saved = reopened.read(id).unwrap();
    assert_eq!(saved.jobs[0].state, "completed");
    assert_eq!(saved.meeting.transcript, "Risultato già pronto");
    assert_eq!(saved.transcripts.len(), 1);
    reopened.recover_interrupted().unwrap();
    let again = reopened.read(id).unwrap();
    assert_eq!(again.revision, saved.revision);
    assert_eq!(again.transcripts.len(), 1);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
    assert!(f.repo.job_results(id).unwrap().0.is_empty());
    // Removing the journal must not leave the result in only one session copy.
    std::fs::write(
        f.repo.folder(id).join("session.json"),
        b"damaged after commit",
    )
    .unwrap();
    let from_backup = reopened.read(id).unwrap();
    assert_eq!(from_backup.transcripts.len(), 1);
    assert_eq!(from_backup.jobs[0].state, "completed");
}

#[cfg(windows)]
#[test]
fn a_failed_final_commit_exposes_recovery_and_retains_the_completed_result() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.processor.release.store(false, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    wait(|| {
        f.repo
            .read(id)
            .unwrap()
            .unwrap()
            .jobs
            .last()
            .unwrap()
            .progress
            == 30
    });
    // Permit readers but deny replacement of only the meeting manifest.
    let denied = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.repo.folder(id).join("session.json"))
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    wait(|| !f.repo.job_results(id).unwrap().0.is_empty());
    let pending = f.service.read(id).unwrap();
    assert_eq!(pending.jobs.last().unwrap().state, "interrupted");
    assert!(
        pending
            .jobs
            .last()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("salvataggio")
    );
    assert_eq!(pending.capture.chunks.len(), 1);
    drop(denied);
    let done = f.service.read(id).unwrap();
    assert_eq!(done.jobs.last().unwrap().state, "completed");
    assert_eq!(done.transcripts.len(), 1);
    assert_eq!(f.service.read(id).unwrap().transcripts.len(), 1);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn recovered_results_respect_cancellation_and_newer_user_edits() {
    for cancelled in [false, true] {
        let f = Fixture::new();
        let d = f.saved();
        let id = d.meeting.id;
        f.repo
            .update(id, &mut |d| {
                d.jobs.push(MeetingJob {
                    id: "old".into(),
                    kind: "transcribe".into(),
                    state: if cancelled { "cancelled" } else { "running" }.into(),
                    ..Default::default()
                });
                d.meeting.transcript = "Testo conservato".into();
                if !cancelled {
                    d.revision += 1;
                }
                Ok(())
            })
            .unwrap();
        f.repo
            .save_job_result(&completed_checkpoint(id, "old", d.revision))
            .unwrap();
        let saved = f.service.read(id).unwrap();
        assert_eq!(saved.meeting.transcript, "Testo conservato");
        assert!(saved.transcripts.is_empty());
        assert_eq!(
            saved.jobs[0].state,
            if cancelled { "cancelled" } else { "failed" }
        );
    }
}

#[cfg(windows)]
#[test]
fn simultaneous_journal_and_manifest_failure_retains_an_explicit_memory_fallback() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.processor.release.store(false, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| {
        f.repo
            .read(id)
            .unwrap()
            .unwrap()
            .jobs
            .last()
            .unwrap()
            .progress
            == 30
    });
    let blocked = f.repo.folder(id).join("job-results");
    std::fs::write(&blocked, b"synthetic full/unavailable journal").unwrap();
    let denied = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.repo.folder(id).join("session.json"))
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    wait(|| f.service.read(id).unwrap().jobs.last().unwrap().state == "interrupted");
    let pending = f.service.read(id).unwrap();
    assert!(
        pending
            .jobs
            .last()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("Mantieni l'app aperta")
    );
    assert_eq!(pending.capture.chunks.len(), 1);
    drop(denied);
    std::fs::remove_file(blocked).unwrap();
    let saved = f.service.read(id).unwrap();
    assert_eq!(saved.jobs.last().unwrap().state, "completed");
    assert_eq!(saved.transcripts.len(), 1);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn one_damaged_worker_result_does_not_hide_another_completed_result() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.repo
        .update(id, &mut |d| {
            for job_id in ["bad", "good"] {
                d.jobs.push(MeetingJob {
                    id: job_id.into(),
                    kind: "transcribe".into(),
                    state: "running".into(),
                    ..Default::default()
                });
            }
            Ok(())
        })
        .unwrap();
    f.repo
        .save_job_result(&completed_checkpoint(id, "bad", d.revision))
        .unwrap();
    let folder = f.repo.folder(id).join("job-results");
    let bad = std::fs::read_dir(&folder)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bad).unwrap()).unwrap();
    envelope["sha256"] = "wrong".into();
    std::fs::write(&bad, serde_json::to_vec(&envelope).unwrap()).unwrap();
    f.repo
        .save_job_result(&completed_checkpoint(id, "good", d.revision))
        .unwrap();
    f.service.recover_interrupted().unwrap();
    let recovered = f.service.read(id).unwrap();
    assert_eq!(recovered.jobs[0].state, "interrupted");
    assert_eq!(recovered.jobs[1].state, "completed");
    assert_eq!(recovered.transcripts.len(), 1);
    assert!(recovered.recovery_notice.unwrap().contains("danneggiato"));
    assert!(bad.is_file());
}

#[test]
fn panicked_processing_can_be_retried() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.processor.panic_transcribe.store(true, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    let failed = finished(&f, id);
    assert_eq!(failed.jobs[0].state, "failed");
    assert_eq!(failed.capture.chunks.len(), 1);
    f.processor.panic_transcribe.store(false, Ordering::SeqCst);
    wait(|| f.service.retry_job(id, &failed.jobs[0].id).is_ok());
    assert_eq!(finished(&f, id).jobs[1].state, "completed");
}

#[test]
fn a_recording_stopped_without_a_server_waits_flagged() {
    let f = Fixture::new(); // `ready` is off: transcription_blocker is Some
    let d = f.saved();
    assert!(d.transcription_waiting);
    assert!(d.jobs.is_empty());
}

#[test]
fn only_flagged_meetings_resume() {
    let f = Fixture::new();
    let flagged = f.saved().meeting.id;
    // An old recording left without a transcript on purpose: no marker.
    let old = f
        .service
        .start(StartMeeting { request_id: "old-click".into(), microphone: Some("mic".into()), title: "Old".into(), ..Default::default() })
        .unwrap()
        .meeting
        .id;
    f.service.stop().unwrap();
    f.repo.update(old, &mut |d| { d.transcription_waiting = false; Ok(()) }).unwrap();
    assert!(f.service.read(flagged).unwrap().transcription_waiting);
    // Still no server: nothing starts.
    assert_eq!(f.service.resume_waiting().unwrap(), 0);
    f.processor.ready.store(true, Ordering::SeqCst);
    assert_eq!(f.service.resume_waiting().unwrap(), 1);
    assert!(f.service.read(flagged).unwrap().jobs.iter().any(|j| j.kind == "transcribe"));
    assert!(!f.service.read(flagged).unwrap().transcription_waiting, "the marker is cleared when the job starts");
    assert!(f.service.read(old).unwrap().jobs.is_empty());
}

#[test]
fn start_is_idempotent_and_audio_progress_does_not_conflict_with_notes() {
    let f = Fixture::new();
    let d = f.start();
    assert_eq!(f.start().meeting.id, d.meeting.id);
    assert_eq!(f.audio.starts.load(Ordering::Relaxed), 1);
    f.service.pause().unwrap();
    f.service.resume().unwrap();
    let saved = f
        .service
        .save_notes(d.meeting.id, "Appunti".into(), d.revision)
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        f.service
            .save_notes(d.meeting.id, "stale".into(), 0)
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    assert_eq!(
        f.service.read(d.meeting.id).unwrap().meeting.notes,
        "Appunti"
    );
    f.service.stop().unwrap();
    assert_eq!(f.service.stop().unwrap().state, CaptureState::Saved);
}

#[test]
fn stale_processing_cannot_replace_an_existing_transcript() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.meeting.transcript = "Versione conservata".into();
            Ok(())
        })
        .unwrap();
    f.processor.release.store(false, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    f.service
        .save_notes(id, "Nuova revisione".into(), d.revision)
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    let result = finished(&f, id);
    assert_eq!(result.jobs.last().unwrap().state, "failed");
    assert_eq!(result.meeting.transcript, "Versione conservata");
    assert!(result.active_transcript.is_none());
}

#[test]
fn cancellation_and_model_failure_keep_the_previous_result() {
    let f = Fixture::new();
    let d = f.saved_keeping_audio();
    let id = d.meeting.id;
    f.processor.release.store(false, Ordering::SeqCst);
    let queued = f
        .service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    f.service
        .cancel_job(id, &queued.jobs.last().unwrap().id)
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    wait(|| {
        f.service
            .transcribe(id, TranscribeOptions::default())
            .is_ok()
    });
    let result = finished(&f, id);
    assert_eq!(result.jobs[0].state, "cancelled");
    assert_eq!(result.jobs[1].state, "completed");
    let version = result.active_transcript.clone();
    f.processor.fail.store(true, Ordering::SeqCst);
    wait(|| {
        f.service
            .transcribe(id, TranscribeOptions::default())
            .is_ok()
    });
    let failed = finished(&f, id);
    assert_eq!(failed.active_transcript, version);
    assert_eq!(failed.jobs.last().unwrap().state, "failed");
}

#[test]
fn successful_transcription_projects_and_correction_preserves_old_version() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    let done = finished(&f, id);
    let original = done.active_transcript.clone().unwrap();
    let edited = f
        .service
        .correct_segment(
            id,
            &original,
            "seg1",
            "Decisione: rilascio lunedì".into(),
            done.revision,
        )
        .unwrap();
    assert_eq!(edited.transcripts.len(), 2);
    assert_eq!(
        edited.transcripts[0].segments[0].text,
        "Decisione: rilascio venerdì"
    );
    assert_eq!(
        edited.transcripts[1].segments[0].original_text,
        "Decisione: rilascio venerdì"
    );
    assert_eq!(
        f.context
            .read()
            .unwrap()
            .meetings
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .transcript,
        "Decisione: rilascio lunedì"
    );
}

#[test]
fn restart_marks_work_interrupted_without_opening_devices() {
    let f = Fixture::new();
    let d = f.start();
    let id = d.meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "interrupted".into(),
                kind: "transcribe".into(),
                state: "running".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.service.recover_interrupted().unwrap();
    let recovered = f.service.read(id).unwrap();
    assert_eq!(recovered.capture.state, CaptureState::Interrupted);
    assert_eq!(recovered.jobs[0].state, "interrupted");
    assert_eq!(f.audio.starts.load(Ordering::Relaxed), 1);
}

#[test]
fn search_handles_unicode_and_respects_exclusions_and_versions() {
    let mut a = MeetingDetail::legacy(Meeting {
        id: 1,
        title: "Progetto".into(),
        notes: format!("{}決定 rilascio lunedì", "è".repeat(200)),
        ..Default::default()
    });
    let mut b = a.clone();
    b.meeting.id = 2;
    b.included_in_search = false;
    let hits = search_details(&[a.clone(), b], "rilascio", 100).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meeting_id, 1);
    assert!(hits[0].text.contains("lunedì"));
    a.deleted = true;
    assert!(search_details(&[a], "rilascio", 10).unwrap().is_empty());
}

#[test]
fn deleting_an_active_capture_is_rejected_and_tombstone_survives_legacy_projection() {
    let f = Fixture::new();
    let d = f.start();
    let id = d.meeting.id;
    assert_eq!(f.service.delete(id).unwrap_err().code, "recording_busy");
    f.service.stop().unwrap();
    f.service.delete(id).unwrap();
    assert!(f.service.read(id).is_err());
    assert!(f.service.list().unwrap().is_empty());
    f.context
        .update(|s| {
            s.meetings.push(d.meeting.clone());
            Ok(())
        })
        .unwrap();
    assert!(f.service.list().unwrap().is_empty());
    assert!(f.repo.read(id).unwrap().unwrap().deleted);
}

#[test]
fn audio_range_bounds_are_checked_before_any_io() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    assert!(f.service.audio_range(id, 0, 30_001).is_err());
    assert!(f.service.audio_range(id, 0, 0).is_err());
    assert!(f.service.audio_range(id, 0, 30_000).is_ok());
    assert!(
        f.root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("qamille-meeting-service-")
    );
}

#[test]
fn note_sources_retain_the_original_revision_after_edits_and_delete() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    let first = f
        .service
        .save_notes(id, "Decisione originale".into(), d.revision)
        .unwrap();
    let second = f
        .service
        .save_notes(id, "Decisione corretta".into(), first.revision)
        .unwrap();
    assert_eq!(second.notes[0].revision, 2);
    assert_eq!(second.note_history[0].revision, 1);
    assert_eq!(second.note_history[0].text, "Decisione originale");
    f.service.delete(id).unwrap();
    let removed = f.repo.read(id).unwrap().unwrap();
    assert!(removed.note_history.is_empty());
    assert!(removed.legacy_notes.is_none());
}

#[test]
fn archive_pages_are_bounded_and_do_not_transfer_transcripts() {
    let f = Fixture::new();
    f.context
        .update(|s| {
            s.meetings = (1..=120)
                .map(|id| Meeting {
                    id,
                    date: id as i64,
                    transcript: "private text".repeat(100),
                    notes: "private note".into(),
                    ..Default::default()
                })
                .collect();
            Ok(())
        })
        .unwrap();
    let page = f.service.list_page(0, 50, None, None, None).unwrap();
    assert_eq!(page.total, 120);
    assert_eq!(page.items.len(), 50);
    assert_eq!(page.next_offset, Some(50));
    assert_eq!(page.items[0].meeting.id, 120);
    assert!(page.items.iter().all(|d| d.meeting.transcript.is_empty()
        && d.meeting.notes.is_empty()
        && d.notes.is_empty()
        && d.legacy_transcript.is_none()));
    let second = f.service.list_page(100, 50, None, None, None).unwrap();
    assert_eq!(second.items.len(), 20);
    assert_eq!(second.next_offset, None);
    assert_eq!(
        f.service
            .list_page(0, usize::MAX, None, None, None)
            .unwrap()
            .items
            .len(),
        100
    );
    assert_eq!(
        f.service
            .list_page(0, 50, None, None, Some("saved"))
            .unwrap()
            .total,
        0
    );
    assert!(
        f.service
            .list_page(0, 50, Some("invalid"), None, None)
            .is_err()
    );
}

#[test]
fn metadata_write_failure_keeps_live_recording_and_stop_available() {
    let f = Fixture::new();
    let detail = f.start();
    let folder = f.repo.folder(detail.meeting.id);
    std::fs::create_dir(folder.join("session.json.prev.writing")).unwrap();
    {
        let mut live = f.audio.snapshot.lock().unwrap();
        live.saved_ms = 12_000;
        live.elapsed_ms = 12_000;
    }
    let status = f.service.capture_status().unwrap();
    assert_eq!(status.state, CaptureState::Recording);
    assert_eq!(status.saved_ms, 12_000);
    assert!(status.error.as_deref().unwrap().contains("indice"));
    let stopped = f.service.stop().unwrap();
    assert_eq!(stopped.state, CaptureState::Saved);
    assert!(stopped.error.is_some());
}

#[test]
fn restart_keeps_healthy_meetings_available_beside_an_unsupported_record() {
    let f = Fixture::new();
    let detail = f.saved();
    let id = detail.meeting.id;
    let other = MeetingDetail::legacy(Meeting {
        id: id + 10,
        title: "Future record".into(),
        ..Default::default()
    });
    f.repo.create(&other).unwrap();
    std::fs::write(
        f.repo.folder(other.meeting.id).join("session.json"),
        br#"{"schema_version":99}"#,
    )
    .unwrap();
    f.service.recover_interrupted().unwrap();
    assert_eq!(f.service.read(id).unwrap().meeting.id, id);
    assert_eq!(f.service.list().unwrap().len(), 2);
}

#[test]
fn preparing_capture_cannot_be_deleted_or_recovered() {
    let f = Fixture::new();
    let id = f.start().meeting.id;
    f.audio.snapshot.lock().unwrap().state = CaptureState::Preparing;
    assert_eq!(f.service.delete(id).unwrap_err().code, "recording_busy");
    assert_eq!(f.service.recover(id).unwrap_err().code, "recording_busy");
    assert!(!f.repo.read(id).unwrap().unwrap().deleted);
}

#[test]
fn recovering_old_manifest_preserves_the_newer_compatibility_copy() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    let first = f
        .service
        .save_notes(id, "Nota A".into(), d.revision)
        .unwrap();
    f.service
        .save_notes(id, "Nota B più recente".into(), first.revision)
        .unwrap();
    assert_eq!(
        f.context
            .read()
            .unwrap()
            .meetings
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .notes,
        "Nota B più recente"
    );
    std::fs::write(f.repo.folder(id).join("session.json"), b"damaged primary").unwrap();
    f.service.recover_interrupted().unwrap();
    let recovered = f.service.read(id).unwrap();
    assert_eq!(recovered.meeting.notes, "Nota A");
    assert!(
        recovered
            .recovery_copies
            .iter()
            .any(|copy| copy.notes == "Nota B più recente")
    );
    assert!(!recovered.reconcile_projection);
    f.service.recover_interrupted().unwrap();
    let again = f.service.read(id).unwrap();
    assert_eq!(again.recovery_copies.len(), recovered.recovery_copies.len());
    assert!(
        again
            .recovery_copies
            .iter()
            .any(|copy| copy.notes == "Nota B più recente")
    );
    assert!(
        f.service
            .list_page(0, 50, None, None, None)
            .unwrap()
            .items
            .iter()
            .all(|d| d.recovery_copies.is_empty())
    );
}

#[test]
fn selected_export_keeps_historical_sources_without_changing_active_versions() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.transcripts = ["v1", "v2"]
                .into_iter()
                .map(|v| TranscriptVersion {
                    id: v.into(),
                    segments: vec![TranscriptSegment {
                        id: "segment1".into(),
                        text: format!("Testo {v}"),
                        ..Default::default()
                    }],
                    complete: true,
                    ..Default::default()
                })
                .collect();
            d.active_transcript = Some("v2".into());
            d.summaries = ["s1", "s2"]
                .into_iter()
                .map(|v| SummaryVersion {
                    id: v.into(),
                    summary: format!("Sintesi {v}"),
                    decisions: vec![Decision {
                        text: "Decisione".into(),
                        sources: vec![Evidence {
                            kind: "segment".into(),
                            version: "v1".into(),
                            id: "segment1".into(),
                        }],
                    }],
                    complete: true,
                    ..Default::default()
                })
                .collect();
            d.active_summary = Some("s2".into());
            d.meeting.notes = "Appunti da escludere".into();
            Ok(())
        })
        .unwrap();
    let sections = vec!["summary".into()];
    let text = f
        .service
        .export_selected(id, "json", Some("v1"), Some("s1"), Some(&sections))
        .unwrap();
    let exported: MeetingDetail = serde_json::from_str(&text).unwrap();
    assert_eq!(exported.active_summary.as_deref(), Some("s1"));
    assert!(exported.active_transcript.is_none());
    assert!(exported.meeting.notes.is_empty());
    assert_eq!(exported.transcripts.len(), 1);
    assert_eq!(exported.transcripts[0].id, "v1");
    assert_eq!(exported.summaries.len(), 1);
    assert_eq!(
        f.service.read(id).unwrap().active_summary.as_deref(),
        Some("s2")
    );
    assert!(
        f.service
            .export_selected(id, "json", Some("missing"), None, None)
            .is_err()
    );
}

// Regressions for the recorder hardening review.
fn saved_audio_fixture(folder: &Path, starts: &[u64]) -> Vec<AudioChunk> {
    use sha2::{Digest, Sha256};
    std::fs::create_dir_all(folder).unwrap();
    let mut chunks = Vec::new();
    for (sequence, start_ms) in starts.iter().copied().enumerate() {
        let name = format!("microphone-{sequence:06}-{start_ms}.wav");
        let path = folder.join(&name);
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..8000 {
            wav.write_sample(8191_i16).unwrap();
        }
        wav.finalize().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let chunk = AudioChunk {
            source: "microphone".into(),
            sequence: sequence as u64,
            path: name,
            start_ms,
            end_ms: start_ms + 1000,
            sample_rate: 8000,
            channels: 1,
        };
        std::fs::write(folder.join(format!("{}.sha256.json", chunk.path)), serde_json::to_vec(&serde_json::json!({
            "schema": 1, "chunk": chunk, "bytes": bytes.len(), "sha256": format!("{:x}",Sha256::digest(&bytes)),
        })).unwrap()).unwrap();
        chunks.push(chunk);
    }
    let snapshot = CaptureSnapshot {
        meeting_id: folder
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse().ok()),
        state: CaptureState::Saved,
        requires_receipts: true,
        elapsed_ms: chunks.last().unwrap().end_ms,
        saved_ms: chunks.last().unwrap().end_ms,
        chunks: chunks.clone(),
        ..Default::default()
    };
    std::fs::write(
        folder.join("capture.json"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    chunks
}

#[test]
fn recovered_gap_is_reported_as_incomplete() {
    let f = Fixture::new();
    let folder = f.repo.folder(7);
    saved_audio_fixture(&folder, &[0, 2000]);
    let loaded = qamille::infrastructure::meeting_audio::load_capture(&folder)
        .unwrap()
        .unwrap();
    let samples =
        qamille::infrastructure::meeting_audio::mix_audio(&folder, &loaded.chunks, 0, 3000, 8000)
            .unwrap();
    assert!(samples[8000..16000].iter().all(|v| *v == 0.0));
    assert_eq!(loaded.state, CaptureState::Saved);
    assert!(loaded.incomplete);
    assert!(!loaded.warnings.is_empty());
    assert!(loaded.missing_chunks.is_empty());
    assert_eq!(loaded.gaps.len(), 1);
    assert_eq!(
        (loaded.gaps[0].start_ms, loaded.gaps[0].end_ms),
        (1000, 2000)
    );
    let mut detail = MeetingDetail::legacy(Meeting {
        id: 7,
        ..Default::default()
    });
    detail.capture = loaded.clone();
    let exported = f.processor.export(&detail, "text").unwrap();
    assert!(exported.contains("Intervallo audio mancante"));
    assert!(exported.contains("00:00:01–00:00:02"));
    let again = qamille::infrastructure::meeting_audio::load_capture(&folder)
        .unwrap()
        .unwrap();
    assert!(again.incomplete);
    assert_eq!(again.gaps.len(), 1);
}

#[test]
fn missing_receipt_keeps_audio_but_marks_integrity_unverified() {
    use std::io::{Seek, SeekFrom, Write};
    let f = Fixture::new();
    let folder = f.repo.folder(7);
    let chunks = saved_audio_fixture(&folder, &[0]);
    let first = qamille::infrastructure::meeting_audio::load_capture(&folder)
        .unwrap()
        .unwrap();
    assert!(!first.incomplete);
    std::fs::remove_file(folder.join(format!("{}.sha256.json", chunks[0].path))).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(folder.join(&chunks[0].path))
        .unwrap();
    file.seek(SeekFrom::Start(44)).unwrap();
    file.write_all(&16383_i16.to_le_bytes()).unwrap();
    drop(file);
    let loaded = qamille::infrastructure::meeting_audio::load_capture(&folder)
        .unwrap()
        .unwrap();
    let samples =
        qamille::infrastructure::meeting_audio::mix_audio(&folder, &loaded.chunks, 0, 1000, 8000)
            .unwrap();
    let mut wav = hound::WavReader::open(folder.join(&chunks[0].path)).unwrap();
    assert_eq!(wav.samples::<i16>().next().unwrap().unwrap(), 16383);
    assert!(samples[0] > 0.0);
    assert_eq!(loaded.state, CaptureState::Saved);
    assert!(loaded.incomplete);
    assert!(!loaded.warnings.is_empty());
    assert_eq!(loaded.unverified_chunks, vec![chunks[0].path.clone()]);
    assert_eq!(loaded.chunks.len(), 1);
}

#[test]
fn independent_runtimes_cannot_start_the_same_job() {
    let f = Fixture::new();
    let d = f.saved_keeping_audio();
    let id = d.meeting.id;
    f.processor.release.store(false, Ordering::SeqCst);
    let first = f
        .service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.calls.load(Ordering::SeqCst) == 1);
    let second_runtime = MeetingService::new(
        f.context.clone(),
        f.repo.clone(),
        f.audio.clone(),
        f.processor.clone(),
    );
    let error = second_runtime
        .transcribe(id, TranscribeOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "job_running");
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.repo.read(id).unwrap().unwrap().jobs.len(), 1);
    assert_eq!(first.jobs.len(), 1);
    f.processor.release.store(true, Ordering::SeqCst);
    wait(|| {
        f.service
            .read(id)
            .unwrap()
            .jobs
            .iter()
            .all(|j| !matches!(j.state.as_str(), "queued" | "running"))
    });
    wait(|| f.repo.claim_job(id).is_ok());
    second_runtime
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| second_runtime.read(id).unwrap().jobs.last().unwrap().state == "completed");
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn foreign_runtime_cancellation_reaches_the_worker_and_releases_the_lease() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.processor.release.store(false, Ordering::SeqCst);
    let queued = f
        .service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.calls.load(Ordering::SeqCst) == 1);
    let other = MeetingService::new(
        f.context.clone(),
        f.repo.clone(),
        f.audio.clone(),
        f.processor.clone(),
    );
    assert_eq!(other.delete(id).unwrap_err().code, "job_running");
    other.recover_interrupted().unwrap();
    assert!(matches!(
        f.repo.read(id).unwrap().unwrap().jobs[0].state.as_str(),
        "queued" | "running"
    ));
    let cancelled = other.cancel_job(id, &queued.jobs[0].id).unwrap();
    assert_eq!(cancelled.jobs[0].state, "cancelled");
    // The fake processor remains blocked unless it receives cancellation.
    wait(|| f.repo.claim_job(id).is_ok());
    assert!(!f.processor.release.load(Ordering::SeqCst));
    assert!(!f.repo.read(id).unwrap().unwrap().deleted);
    f.processor.release.store(true, Ordering::SeqCst);
    other.transcribe(id, TranscribeOptions::default()).unwrap();
    wait(|| other.read(id).unwrap().jobs.last().unwrap().state == "completed");
}

#[test]
fn a_dead_worker_state_does_not_block_a_new_claim_or_cancel_a_new_job() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "dead".into(),
                state: "running".into(),
                kind: "transcribe".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.service.cancel_job(id, "dead").unwrap();
    // Simulate an older crash leaving a persisted nonterminal state.
    f.repo
        .update(id, &mut |d| {
            d.jobs[0].state = "running".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(f.service.read(id).unwrap().jobs[0].state, "interrupted");
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.service.read(id).unwrap().jobs.last().unwrap().state == "completed");
    assert_eq!(f.service.read(id).unwrap().jobs[0].state, "interrupted");
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn player_detects_a_receipt_lost_after_startup_and_live_polling_keeps_the_warning() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    let folder = f.repo.folder(id);
    let chunks = saved_audio_fixture(&folder, &[0]);
    let capture: CaptureSnapshot =
        serde_json::from_slice(&std::fs::read(folder.join("capture.json")).unwrap()).unwrap();
    f.repo
        .update(id, &mut |d| {
            d.capture = capture.clone();
            Ok(())
        })
        .unwrap();
    std::fs::remove_file(folder.join(format!("{}.sha256.json", chunks[0].path))).unwrap();
    f.service.audio_range(id, 0, 1000).unwrap();
    assert_eq!(
        f.service.read(id).unwrap().capture.unverified_chunks.len(),
        1
    );
    for _ in 0..3 {
        assert!(f.service.capture_status().unwrap().incomplete);
    }
    let kept = f.service.read(id).unwrap();
    assert!(kept.capture.incomplete);
    assert_eq!(kept.capture.unverified_chunks.len(), 1);
    assert!(
        f.service
            .export(id, "text")
            .unwrap()
            .contains("Integrità non verificata")
    );
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    let transcribed = finished(&f, id);
    assert_eq!(transcribed.transcripts.len(), 1);
    assert_eq!(transcribed.capture.unverified_chunks.len(), 1);
}

#[test]
fn legacy_audio_without_receipts_is_readable_and_independent_source_offsets_are_not_gaps() {
    let f = Fixture::new();
    let folder = f.repo.folder(7);
    let chunks = saved_audio_fixture(&folder, &[100]);
    std::fs::remove_file(folder.join(format!("{}.sha256.json", chunks[0].path))).unwrap();
    let mut capture: CaptureSnapshot =
        serde_json::from_slice(&std::fs::read(folder.join("capture.json")).unwrap()).unwrap();
    capture.requires_receipts = false;
    std::fs::write(
        folder.join("capture.json"),
        serde_json::to_vec(&capture).unwrap(),
    )
    .unwrap();
    let loaded = qamille::infrastructure::meeting_audio::load_capture(&folder)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.chunks.len(), 1);
    assert!(!loaded.incomplete);
    assert!(loaded.gaps.is_empty());
    assert!(loaded.unverified_chunks.is_empty());
}

#[derive(Default)]
struct Calendars {
    local: Mutex<Vec<ReminderCandidate>>,
    google: Mutex<Vec<ReminderCandidate>>,
    google_error: Mutex<Option<AppError>>,
    accounts_error: Mutex<Option<AppError>>,
    /// Account ids with calendar access; one account when unset.
    accounts: Mutex<Option<Vec<u32>>>,
    google_reads: AtomicU64,
}
impl ReminderCalendar for Calendars {
    fn local(&self, _: &Store, _: i64, _: i64) -> AppResult<Vec<ReminderCandidate>> {
        Ok(self.local.lock().unwrap().clone())
    }
    fn google_accounts(&self, _: &Store) -> AppResult<Vec<u32>> {
        match self.accounts_error.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(self.accounts.lock().unwrap().clone().unwrap_or_else(|| vec![7])),
        }
    }
    fn google(&self, _: &Store, _: u32, _: i64, _: i64) -> AppResult<Vec<ReminderCandidate>> {
        self.google_reads.fetch_add(1, Ordering::SeqCst);
        match self.google_error.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(self.google.lock().unwrap().clone()),
        }
    }
}
fn upcoming(key: &str, event_id: Option<u64>) -> ReminderCandidate {
    ReminderCandidate {
        key: key.into(),
        title: format!("Riunione {key}"),
        start: 2_000_000_030,
        end: 2_000_001_800,
        link: Some(format!("https://meet.google.com/{key}")),
        local_event_id: event_id,
        ..Default::default()
    }
}
fn reminders(f: &Fixture, calendars: &Arc<Calendars>) -> MeetingReminderService {
    MeetingReminderService::new(f.context.clone(), calendars.clone(), f.service.clone())
}

#[test]
fn reminder_chips_follow_settings_and_stay_hidden_after_dismiss() {
    let f = Fixture::new();
    let calendars = Arc::new(Calendars::default());
    *calendars.local.lock().unwrap() = vec![upcoming("standup", None)];
    let service = reminders(&f, &calendars);
    let first = service.tick().unwrap();
    assert!(first.changed);
    assert_eq!(first.view.chips.len(), 1);
    assert_eq!(first.view.chips[0].title, "Riunione standup");
    assert!(!service.tick().unwrap().changed, "an unchanged check sends nothing");
    assert_eq!(service.chip(&first.view.chips[0].key).unwrap().link, "https://meet.google.com/standup");

    assert!(service.dismiss("standup").unwrap().chips.is_empty());
    assert!(service.tick().unwrap().view.chips.is_empty(), "a closed chip stays closed");
    assert_eq!(service.chip("standup").unwrap_err().code, "reminder_missing");

    *calendars.local.lock().unwrap() = vec![upcoming("review", None)];
    f.context.update(|s| { s.settings.meeting_reminders = false; Ok(()) }).unwrap();
    assert!(service.tick().unwrap().view.chips.is_empty(), "reminders can be turned off");
}

#[test]
fn google_is_read_once_per_minute_and_each_error_is_reported_once() {
    let f = Fixture::new();
    let calendars = Arc::new(Calendars::default());
    *calendars.google.lock().unwrap() = vec![upcoming("google", None)];
    let service = reminders(&f, &calendars);
    assert!(service.tick().unwrap().view.chips.is_empty(), "the local check never reads Google");
    assert_eq!(calendars.google_reads.load(Ordering::SeqCst), 0);
    assert!(service.refresh_google().unwrap().updated, "a new read asks for a check at once");
    assert_eq!(service.tick().unwrap().view.chips.len(), 1);
    assert!(!service.refresh_google().unwrap().updated);
    assert_eq!(service.tick().unwrap().view.chips.len(), 1);
    assert_eq!(calendars.google_reads.load(Ordering::SeqCst), 1, "the cached read is reused");

    // A failing account list keeps the cached events and reports once.
    *calendars.accounts_error.lock().unwrap() = Some(AppError::external("Keyring is locked"));
    assert_eq!(service.refresh_google().unwrap().notices, ["Keyring is locked"]);
    assert!(service.refresh_google().unwrap().notices.is_empty());
    assert_eq!(service.tick().unwrap().view.chips.len(), 1);

    let calendars = Arc::new(Calendars::default());
    *calendars.google_error.lock().unwrap() = Some(AppError::external("Google is unreachable"));
    let service = reminders(&f, &calendars);
    assert_eq!(service.refresh_google().unwrap().notices, ["Google is unreachable"]);
    assert!(service.refresh_google().unwrap().notices.is_empty());

    // The same failure in three accounts is one notice.
    let calendars = Arc::new(Calendars::default());
    *calendars.accounts.lock().unwrap() = Some(vec![1, 2, 3]);
    *calendars.google_error.lock().unwrap() = Some(AppError::external("Google is unreachable"));
    let service = reminders(&f, &calendars);
    assert_eq!(service.refresh_google().unwrap().notices, ["Google is unreachable"]);
    assert_eq!(calendars.google_reads.load(Ordering::SeqCst), 3);

    let calendars = Arc::new(Calendars::default());
    *calendars.google_error.lock().unwrap() = Some(AppError::new("calendar_consent_required", "consent"));
    let service = reminders(&f, &calendars);
    assert!(service.refresh_google().unwrap().notices.is_empty(), "accounts without consent stay quiet");
}

#[test]
fn join_and_record_starts_one_linked_recording_with_default_devices() {
    let f = Fixture::new();
    let event = f.context.read().unwrap().events[0].id;
    *f.audio.devices.lock().unwrap() = vec![
        AudioDevice { id: "mic-1".into(), name: "Mic".into(), kind: "microphone".into(), is_default: false },
        AudioDevice { id: "mic-2".into(), name: "Headset".into(), kind: "microphone".into(), is_default: true },
        AudioDevice { id: "out".into(), name: "Speakers".into(), kind: "system".into(), is_default: true },
    ];
    let calendars = Arc::new(Calendars::default());
    *calendars.local.lock().unwrap() = vec![upcoming("design", Some(event))];
    let service = reminders(&f, &calendars);
    service.tick().unwrap();
    // The chip is taken before the link opens; closing it meanwhile keeps the recording.
    let chip = service.chip("design").unwrap();
    service.dismiss("design").unwrap();
    service.record_chip(&chip).unwrap();
    assert!(service.view().unwrap().recording, "Join and record is disabled at once");
    service.record_chip(&chip).unwrap();
    assert_eq!(f.audio.starts.load(Ordering::SeqCst), 1, "a double click starts one recording");
    let meetings = f.service.list().unwrap();
    assert_eq!(meetings.len(), 1);
    assert_eq!(meetings[0].meeting.title, "Riunione design");
    assert_eq!(meetings[0].meeting.event_id, Some(event));
    assert!(service.tick().unwrap().view.recording, "Join and record is unavailable while recording");
}

fn two_microphones_and_two_outputs(f: &Fixture) {
    let device = |id: &str, kind: &str, is_default: bool| AudioDevice {
        id: id.into(),
        name: id.into(),
        kind: kind.into(),
        is_default,
    };
    *f.audio.devices.lock().unwrap() = vec![
        device("mic-1", "microphone", false),
        device("mic-2", "microphone", true),
        device("out-1", "system", true),
        device("out-2", "system", false),
    ];
}
fn recorded_with(f: &Fixture, key: &str) -> (Option<String>, Option<String>) {
    let calendars = Arc::new(Calendars::default());
    reminders(f, &calendars).record_meeting(key, "Riunione".into(), None).unwrap();
    let requests = f.audio.requests.lock().unwrap();
    let last = requests.last().expect("a recording started");
    (last.microphone.clone(), last.system.clone())
}

#[test]
fn join_and_record_uses_the_devices_chosen_in_settings() {
    let f = Fixture::new();
    two_microphones_and_two_outputs(&f);
    f.context
        .update(|s| {
            s.settings.meeting_microphone = "mic-1".into();
            s.settings.meeting_system_audio = "out-2".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(recorded_with(&f, "chosen"), (Some("mic-1".into()), Some("out-2".into())));
}

#[test]
fn join_and_record_falls_back_to_the_defaults_when_a_chosen_device_is_gone() {
    let f = Fixture::new();
    two_microphones_and_two_outputs(&f);
    f.context
        .update(|s| {
            s.settings.meeting_microphone = "unplugged-headset".into();
            // An output is not a microphone, whatever the id says.
            s.settings.meeting_system_audio = "mic-1".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(recorded_with(&f, "gone"), (Some("mic-2".into()), Some("out-1".into())));
}

#[test]
fn join_and_record_without_a_choice_uses_the_defaults() {
    let f = Fixture::new();
    two_microphones_and_two_outputs(&f);
    assert_eq!(recorded_with(&f, "default"), (Some("mic-2".into()), Some("out-1".into())));
}

#[test]
fn a_meeting_whose_event_is_gone_can_still_be_renamed() {
    let f = Fixture::new();
    let event = f.context.read().unwrap().events[0].id;
    let id = f.saved().meeting.id;
    let d = f.service.update(id, "Con evento".into(), Some(event), true, f.service.read(id).unwrap().revision).unwrap();
    assert_eq!(d.meeting.event_id, Some(event));
    f.context.update(|s| { s.events.retain(|e| e.id != event); Ok(()) }).unwrap();
    // The page resends the current link on every rename.
    let d = f.service.update(id, "Rinominata".into(), Some(event), true, d.revision).unwrap();
    assert_eq!((d.meeting.title.as_str(), d.meeting.event_id), ("Rinominata", Some(event)));
    // A new link must still point at an event that exists, or be cleared.
    let unknown = f.service.update(id, "Rinominata".into(), Some(event + 9999), true, d.revision).unwrap_err();
    assert_eq!(unknown.code, "not_found");
    assert_eq!(f.service.read(id).unwrap().meeting.event_id, Some(event));
    let d = f.service.update(id, "Senza".into(), None, true, d.revision).unwrap();
    assert_eq!(d.meeting.event_id, None);
}

#[test]
fn record_without_audio_devices_fails_and_keeps_the_chip() {
    let f = Fixture::new();
    let calendars = Arc::new(Calendars::default());
    *calendars.local.lock().unwrap() = vec![upcoming("solo", None)];
    let service = reminders(&f, &calendars);
    service.tick().unwrap();
    let chip = service.chip("solo").unwrap();
    assert_eq!(service.record_chip(&chip).unwrap_err().message, "No audio device is available");
    assert_eq!(service.view().unwrap().chips.len(), 1);
    assert_eq!(f.audio.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn closing_a_chip_makes_room_for_the_next_meeting() {
    let f = Fixture::new();
    let calendars = Arc::new(Calendars::default());
    *calendars.local.lock().unwrap() =
        vec![upcoming("a", None), upcoming("b", None), upcoming("c", None), upcoming("d", None)];
    let service = reminders(&f, &calendars);
    let keys = |view: &qamille::application::meeting_reminders::ReminderView| {
        view.chips.iter().map(|c| c.key.clone()).collect::<Vec<_>>()
    };
    assert_eq!(keys(&service.tick().unwrap().view), ["a", "b", "c"]);
    assert_eq!(keys(&service.dismiss("a").unwrap()), ["b", "c", "d"], "the waiting meeting appears at once");
    assert!(!service.tick().unwrap().changed, "the next check agrees");
}

mod audio_verdict {
    use super::*;

    fn segment(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: "s1".into(),
            text: text.into(),
            ..Default::default()
        }
    }

    /// A clean saved capture with a complete transcript that has text: the
    /// one case where the verdict is `Remove`. Each test breaks one thing.
    fn removable() -> MeetingDetail {
        let mut detail = MeetingDetail::legacy(Meeting {
            id: 1,
            ..Default::default()
        });
        detail.capture.state = CaptureState::Saved;
        detail.capture.chunks = vec![AudioChunk::default()];
        detail.transcripts = vec![TranscriptVersion {
            id: "t1".into(),
            complete: true,
            segments: vec![segment("buongiorno a tutti")],
            ..Default::default()
        }];
        detail.active_transcript = Some("t1".into());
        detail
    }

    #[test]
    fn verdict_is_pending_without_a_complete_transcript() {
        assert_eq!(audio_verdict(&removable()), AudioVerdict::Remove, "control");

        let mut no_transcript = removable();
        no_transcript.transcripts.clear();
        no_transcript.active_transcript = None;
        assert_eq!(audio_verdict(&no_transcript), AudioVerdict::Pending);

        let mut not_active = removable();
        not_active.active_transcript = None;
        assert_eq!(audio_verdict(&not_active), AudioVerdict::Pending);

        let mut partial = removable();
        partial.transcripts[0].complete = false;
        assert_eq!(audio_verdict(&partial), AudioVerdict::Pending);

        for state in [
            CaptureState::Idle,
            CaptureState::Preparing,
            CaptureState::Recording,
            CaptureState::Paused,
            CaptureState::Saving,
            CaptureState::Interrupted,
        ] {
            let mut detail = removable();
            detail.capture.state = state.clone();
            assert_eq!(audio_verdict(&detail), AudioVerdict::Pending, "{state:?}");
        }

        let mut no_chunks = removable();
        no_chunks.capture.chunks.clear();
        assert_eq!(audio_verdict(&no_chunks), AudioVerdict::Pending);
    }

    #[test]
    fn verdict_removes_a_clean_saved_capture_with_text() {
        assert_eq!(audio_verdict(&removable()), AudioVerdict::Remove);

        let mut blank_then_text = removable();
        blank_then_text.transcripts[0].segments = vec![segment("   "), segment("ok")];
        assert_eq!(audio_verdict(&blank_then_text), AudioVerdict::Remove);
    }

    #[test]
    fn verdict_keeps_an_empty_transcript() {
        let mut no_segments = removable();
        no_segments.transcripts[0].segments.clear();
        assert_eq!(audio_verdict(&no_segments), AudioVerdict::Keep("empty_transcript"));

        let mut blank = removable();
        blank.transcripts[0].segments = vec![segment(""), segment(" \n\t ")];
        assert_eq!(audio_verdict(&blank), AudioVerdict::Keep("empty_transcript"));

        // An empty transcript wins over an interrupted capture.
        blank.capture.incomplete = true;
        assert_eq!(audio_verdict(&blank), AudioVerdict::Keep("empty_transcript"));
    }

    #[test]
    fn verdict_keeps_a_capture_with_gaps_or_incomplete() {
        let mut incomplete = removable();
        incomplete.capture.incomplete = true;
        assert_eq!(audio_verdict(&incomplete), AudioVerdict::Keep("interrupted_capture"));

        let mut gaps = removable();
        gaps.capture.gaps = vec![AudioGap {
            source: "microphone".into(),
            start_ms: 1_000,
            end_ms: 2_000,
        }];
        assert_eq!(audio_verdict(&gaps), AudioVerdict::Keep("interrupted_capture"));
    }

    #[test]
    fn verdict_keeps_a_capture_with_a_lost_or_unverified_block() {
        // Not `incomplete` and without gaps, yet part of the audio is lost or unchecked.
        let mut missing = removable();
        missing.capture.missing_chunks = vec![AudioChunk::default()];
        assert_eq!(audio_verdict(&missing), AudioVerdict::Keep("interrupted_capture"));

        let mut unverified = removable();
        unverified.capture.unverified_chunks = vec!["microphone-000001-2000.wav".into()];
        assert_eq!(audio_verdict(&unverified), AudioVerdict::Keep("interrupted_capture"));

        // Still nothing to judge before a complete transcript, and an empty one wins.
        missing.transcripts[0].complete = false;
        assert_eq!(audio_verdict(&missing), AudioVerdict::Pending);
        unverified.transcripts[0].segments.clear();
        assert_eq!(audio_verdict(&unverified), AudioVerdict::Keep("empty_transcript"));
    }

    #[test]
    fn verdict_ignores_deleted_and_already_removed() {
        let mut deleted = removable();
        deleted.deleted = true;
        assert_eq!(audio_verdict(&deleted), AudioVerdict::Pending);

        let mut removed = removable();
        removed.audio_removed = true;
        assert_eq!(audio_verdict(&removed), AudioVerdict::Pending);
    }

    #[test]
    fn old_session_json_without_the_new_fields_still_loads() {
        let mut json = serde_json::to_value(removable()).unwrap();
        let object = json.as_object_mut().unwrap();
        for field in ["audio_removed", "audio_kept", "has_audio", "has_text"] {
            assert!(object.remove(field).is_some(), "{field} is serialized");
        }
        let loaded: MeetingDetail = serde_json::from_value(json).unwrap();
        assert!(!loaded.audio_removed);
        assert_eq!(loaded.audio_kept, None);
        assert!(!loaded.has_audio);
        assert!(!loaded.has_text);
        assert_eq!(loaded.capture.chunks.len(), 1, "the rest is untouched");
    }

    #[test]
    fn refresh_flags_reports_audio_and_text() {
        let flags = |detail: &mut MeetingDetail| {
            detail.refresh_flags();
            (detail.has_audio, detail.has_text)
        };

        // Audio: chunks, removed, only a legacy audio_path, nothing.
        let mut with_chunks = removable();
        assert_eq!(flags(&mut with_chunks), (true, true));

        let mut removed = removable();
        removed.audio_removed = true;
        assert_eq!(flags(&mut removed), (false, true), "removed audio is gone");

        let mut legacy_audio = MeetingDetail::legacy(Meeting {
            audio_path: Some("old.wav".into()),
            ..Default::default()
        });
        assert_eq!(flags(&mut legacy_audio), (true, false));
        legacy_audio.audio_removed = true;
        assert_eq!(flags(&mut legacy_audio), (false, false));

        // Text: segments, meeting.transcript, legacy_transcript, none.
        let mut empty = MeetingDetail::legacy(Meeting::default());
        assert_eq!(flags(&mut empty), (false, false));

        let mut from_segments = removable();
        from_segments.capture.chunks.clear();
        assert_eq!(flags(&mut from_segments), (false, true));
        from_segments.transcripts[0].segments = vec![segment("  ")];
        assert_eq!(flags(&mut from_segments), (false, false), "blank segments are no text");

        let mut from_meeting = MeetingDetail::legacy(Meeting::default());
        from_meeting.meeting.transcript = "testo vecchio".into();
        assert_eq!(flags(&mut from_meeting), (false, true));

        let mut from_legacy = MeetingDetail::legacy(Meeting::default());
        from_legacy.legacy_transcript = Some("testo importato".into());
        assert_eq!(flags(&mut from_legacy), (false, true));

        // A stale flag is overwritten, not kept.
        let mut stale = MeetingDetail::legacy(Meeting::default());
        stale.has_audio = true;
        stale.has_text = true;
        assert_eq!(flags(&mut stale), (false, false));
    }

    #[test]
    fn audio_cleanup_starts_at_zero() {
        let cleanup = AudioCleanup::default();
        assert_eq!((cleanup.meetings, cleanup.bytes), (0, 0));
        assert_eq!(
            serde_json::to_value(&cleanup).unwrap(),
            serde_json::json!({ "meetings": 0, "bytes": 0 })
        );
    }
}

// Dropping the audio keeps the meeting: the repository removes only audio files.
fn meeting_with_audio(f: &Fixture, id: u64) -> PathBuf {
    f.repo
        .create(&MeetingDetail::legacy(Meeting {
            id,
            title: "Con audio".into(),
            ..Default::default()
        }))
        .unwrap();
    let folder = f.repo.folder(id);
    saved_audio_fixture(&folder, &[0, 2000]);
    folder
}

/// Sizes of everything audio-shaped, counted independently of the code under test:
/// names containing `.wav`, plus the files inside `.stt-*` directories.
fn audio_size_on_disk(folder: &Path) -> u64 {
    fn tree(path: &Path) -> u64 {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    tree(&entry.path())
                } else {
                    entry.metadata().unwrap().len()
                }
            })
            .sum()
    }
    std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().unwrap().is_dir() {
                if name.starts_with(".stt-") {
                    tree(&entry.path())
                } else {
                    0
                }
            } else if name.contains(".wav") {
                entry.metadata().unwrap().len()
            } else {
                0
            }
        })
        .sum()
}

#[test]
fn discard_audio_removes_only_audio_and_returns_the_bytes() {
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 41);
    // Every other shape audio takes in a meeting folder.
    std::fs::write(folder.join("system-00000001-0000000000002000.wav.tmp"), [1u8; 10]).unwrap();
    std::fs::write(folder.join("microphone-000000-0.wav.quarantine-7-1-2"), [2u8; 7]).unwrap();
    std::fs::write(folder.join("microphone-000005-5000.recovered-9.wav"), [3u8; 11]).unwrap();
    std::fs::write(folder.join("import.wav"), [4u8; 13]).unwrap();
    std::fs::create_dir_all(folder.join(".stt-cache").join("nested")).unwrap();
    std::fs::write(folder.join(".stt-cache").join("nested").join("window.bin"), [5u8; 100])
        .unwrap();
    std::fs::create_dir(folder.join(".stt-run-3")).unwrap();
    std::fs::write(folder.join(".stt-run-3").join("part.raw"), [6u8; 50]).unwrap();
    // Everything that is not audio.
    std::fs::write(folder.join("legacy.json"), b"{}").unwrap();
    std::fs::write(folder.join("notes.txt"), b"keep me").unwrap();
    std::fs::create_dir(folder.join("attachments")).unwrap();
    std::fs::write(folder.join("attachments").join("scheda.pdf"), b"pdf").unwrap();
    let expected = audio_size_on_disk(&folder);
    assert!(expected > 191, "the fixture really holds audio");
    assert_eq!(f.repo.audio_bytes(41).unwrap(), expected, "the estimate matches");

    let freed = f.repo.discard_audio(41).unwrap();

    assert_eq!(freed, expected);
    let left: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.iter().all(|name| !name.contains(".wav")), "{left:?}");
    assert!(left.iter().all(|name| !name.starts_with(".stt-")), "{left:?}");
    for kept in ["session.json", ".capture.lock", "legacy.json", "notes.txt", "attachments"] {
        assert!(left.iter().any(|name| name == kept), "{kept} survives: {left:?}");
    }
    assert!(folder.join("attachments").join("scheda.pdf").exists());
    let meeting = f.repo.read(41).unwrap().expect("the meeting survives");
    assert_eq!(meeting.meeting.title, "Con audio");
    assert_eq!(f.repo.audio_bytes(41).unwrap(), 0);
}

#[test]
fn capture_reads_clean_after_discard() {
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 42);
    f.repo.discard_audio(42).unwrap();
    assert!(
        !folder.join("capture.json.prev").exists(),
        "no backup of the old chunk list is left behind"
    );
    for _ in 0..2 {
        // Reading rebuilds from what is on disk; it must not invent damage.
        if let Some(capture) = f.repo.capture(42).unwrap() {
            assert!(capture.chunks.is_empty());
            assert!(!capture.incomplete, "{:?}", capture.warnings);
            assert!(capture.missing_chunks.is_empty());
            assert!(capture.gaps.is_empty());
            assert!(capture.warnings.is_empty());
            assert!(capture.unverified_chunks.is_empty());
        }
    }
}

#[test]
fn discard_audio_clears_the_damage_notes_of_a_broken_capture() {
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 43);
    // One block is lost, so the recording reads as incomplete with a missing chunk and a gap.
    std::fs::remove_file(folder.join("microphone-000001-2000.wav")).unwrap();
    let before = f.repo.capture(43).unwrap().expect("a manifest");
    assert!(before.incomplete && !before.missing_chunks.is_empty() && !before.warnings.is_empty());

    f.repo.discard_audio(43).unwrap();

    if let Some(capture) = f.repo.capture(43).unwrap() {
        assert!(capture.chunks.is_empty());
        assert!(!capture.incomplete, "{:?}", capture.warnings);
        assert!(capture.missing_chunks.is_empty());
        assert!(capture.gaps.is_empty());
        assert!(capture.warnings.is_empty());
        assert!(capture.error.is_none());
    }
}

#[test]
fn discard_audio_of_a_capture_that_never_finished_leaves_no_alarming_manifest() {
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 44);
    let mut stale: CaptureSnapshot =
        serde_json::from_slice(&std::fs::read(folder.join("capture.json")).unwrap()).unwrap();
    stale.state = CaptureState::Recording;
    std::fs::write(folder.join("capture.json"), serde_json::to_vec(&stale).unwrap()).unwrap();

    f.repo.discard_audio(44).unwrap();

    match f.repo.capture(44).unwrap() {
        None => {}
        Some(capture) => {
            assert!(capture.chunks.is_empty() && !capture.incomplete, "{:?}", capture.warnings)
        }
    }
}

#[cfg(windows)]
#[test]
fn a_discard_that_cannot_finish_leaves_a_manifest_of_what_is_still_there() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 47);
    let before: CaptureSnapshot =
        serde_json::from_slice(&std::fs::read(folder.join("capture.json")).unwrap()).unwrap();
    // One block is in use elsewhere (readers only): it cannot be removed, the others can.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(folder.join("microphone-000001-2000.wav"))
        .unwrap();
    assert!(f.repo.discard_audio(47).is_err());
    drop(held);
    assert!(!folder.join("microphone-000000-0.wav").exists(), "the others went away");

    // The manifest lists the block that is left and no other, and says no more than before.
    let after: CaptureSnapshot =
        serde_json::from_slice(&std::fs::read(folder.join("capture.json")).unwrap()).unwrap();
    let listed: Vec<_> = after.chunks.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(listed, ["microphone-000001-2000.wav"]);
    assert_eq!(after.incomplete, before.incomplete);
    assert!(!folder.join("capture.json.prev").exists(), "no copy of the old list is kept");
    // A restart does not read the blocks that went away as lost.
    let capture = f.repo.capture(47).unwrap().unwrap();
    assert!(capture.missing_chunks.is_empty(), "{:?}", capture.warnings);
    assert_eq!(capture.chunks.len(), 1);
    // The next attempt finishes the job.
    assert!(f.repo.discard_audio(47).unwrap() > 0);
    assert_eq!(f.repo.audio_bytes(47).unwrap(), 0);
    if let Some(capture) = f.repo.capture(47).unwrap() {
        assert!(capture.chunks.is_empty() && !capture.incomplete, "{:?}", capture.warnings);
    }
}

#[test]
fn discard_audio_is_idempotent() {
    let f = Fixture::new();
    meeting_with_audio(&f, 45);
    assert!(f.repo.discard_audio(45).unwrap() > 0);
    assert_eq!(f.repo.discard_audio(45).unwrap(), 0);
    assert_eq!(f.repo.discard_audio(45).unwrap(), 0);
}

#[test]
fn discard_audio_on_a_meeting_without_audio_creates_no_manifest() {
    let f = Fixture::new();
    f.repo
        .create(&MeetingDetail::legacy(Meeting {
            id: 46,
            title: "Solo testo".into(),
            ..Default::default()
        }))
        .unwrap();
    assert_eq!(f.repo.discard_audio(46).unwrap(), 0);
    assert!(!f.repo.folder(46).join("capture.json").exists());
    assert!(f.repo.folder(46).join("session.json").exists());
    assert!(f.repo.discard_audio(999).is_err(), "an unknown meeting is not silently accepted");
}

#[test]
fn discard_audio_does_not_touch_an_external_audio_path() {
    let f = Fixture::new();
    meeting_with_audio(&f, 47);
    let external = f.root.with_extension("external");
    std::fs::create_dir_all(&external).unwrap();
    let outside = external.join("riunione-originale.wav");
    std::fs::write(&outside, [9u8; 64]).unwrap();
    let neighbour = external.join("window.bin");
    std::fs::write(&neighbour, [8u8; 32]).unwrap();
    let outside_text = outside.to_string_lossy().into_owned();
    f.repo
        .update(47, &mut |detail| {
            detail.meeting.audio_path = Some(outside_text.clone());
            Ok(())
        })
        .unwrap();
    // Links that point out of the meeting folder: one inside a scratch directory, one on
    // the folder itself. Neither may be followed.
    let scratch = f.repo.folder(47).join(".stt-cache");
    std::fs::create_dir_all(&scratch).unwrap();
    let top_level_link = f.repo.folder(47).join(".stt-link");
    let link = |from: &Path| -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(from)
                .arg(&external)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false)
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(&external, from).is_ok()
        }
    };
    let linked = link(&scratch.join("link")) && link(&top_level_link);

    f.repo.discard_audio(47).unwrap();

    assert_eq!(std::fs::read(&outside).unwrap(), vec![9u8; 64], "the original is untouched");
    assert_eq!(std::fs::read(&neighbour).unwrap(), vec![8u8; 32], "a link is not followed");
    let kept = f.repo.read(47).unwrap().unwrap();
    assert_eq!(
        kept.meeting.audio_path.as_deref().map(Path::new),
        Some(outside.as_path()),
        "the stored path is not edited"
    );
    assert!(linked, "the test could not create its links");
    assert!(!scratch.exists(), "the scratch directory is gone");
    assert!(
        std::fs::symlink_metadata(&top_level_link).is_err(),
        "the link itself is gone"
    );
    std::fs::remove_dir_all(&external).unwrap();
}

#[test]
fn discard_audio_refuses_while_the_recording_lock_is_held() {
    let f = Fixture::new();
    let folder = meeting_with_audio(&f, 48);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(folder.join(".capture.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let error = f.repo.discard_audio(48).unwrap_err();
    assert_eq!(error.code, "recording_busy");
    assert!(folder.join("microphone-000000-0.wav").exists(), "nothing was removed");
    drop(lock);
    assert!(f.repo.discard_audio(48).unwrap() > 0);
}

#[test]
fn discard_audio_of_a_removed_meeting_is_missing() {
    let f = Fixture::new();
    meeting_with_audio(&f, 49);
    f.repo
        .update(49, &mut |detail| {
            detail.deleted = true;
            Ok(())
        })
        .unwrap();
    assert!(f.repo.discard_audio(49).is_err());
}

// The audio goes away once a complete transcript with text is saved, on both paths:
// the worker that finishes live, and the restart that replays a saved result.

/// A meeting recorded by the service whose folder holds real wav chunks starting at
/// `starts`. A hole between them reads as an incomplete capture, as `recover` reports it.
fn meeting_with_real_audio(f: &Fixture, starts: &[u64]) -> u64 {
    let id = f.saved().meeting.id;
    saved_audio_fixture(&f.repo.folder(id), starts);
    let capture = f.repo.capture(id).unwrap().expect("a capture manifest");
    f.repo
        .update(id, &mut |d| {
            d.capture = capture.clone();
            d.meeting.duration_secs = capture.elapsed_ms / 1000;
            Ok(())
        })
        .unwrap();
    id
}

/// Like `meeting_with_real_audio`, for as many meetings as a test needs: the repository
/// owns them, no recording is involved.
fn recorded_meeting(f: &Fixture, id: u64, starts: &[u64]) -> PathBuf {
    let folder = f.repo.folder(id);
    let mut detail = MeetingDetail::legacy(Meeting {
        id,
        title: format!("Riunione {id}"),
        ..Default::default()
    });
    detail.capture.meeting_id = Some(id);
    f.repo.create(&detail).unwrap();
    saved_audio_fixture(&folder, starts);
    let capture = f.repo.capture(id).unwrap().expect("a capture manifest");
    f.repo
        .update(id, &mut |d| {
            d.capture = capture.clone();
            d.meeting.duration_secs = capture.elapsed_ms / 1000;
            Ok(())
        })
        .unwrap();
    folder
}

/// Saves a complete transcript straight into the repository, without going through a job:
/// the audio is left exactly where it was.
fn with_transcript(f: &Fixture, id: u64, text: &str) {
    f.repo
        .update(id, &mut |d| {
            d.transcripts.push(TranscriptVersion {
                id: "t1".into(),
                complete: true,
                segments: vec![TranscriptSegment {
                    id: "s1".into(),
                    text: text.into(),
                    ..Default::default()
                }],
                ..Default::default()
            });
            d.active_transcript = Some("t1".into());
            d.meeting.transcript = text.into();
            Ok(())
        })
        .unwrap();
}

fn audio_files(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".wav"))
        .collect()
}

/// Waits until no worker holds the meeting: its result and the audio decision are final.
fn worker_done(f: &Fixture, id: u64) {
    wait(|| f.repo.claim_job(id).is_ok());
}

#[test]
fn a_text_transcript_removes_the_audio_and_keeps_the_duration() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    let before = f.service.read(id).unwrap();
    assert!(before.has_audio && !before.audio_removed && !before.has_text);
    assert!(!audio_files(&folder).is_empty());
    assert!(before.meeting.duration_secs > 0);

    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);

    let after = f.service.read(id).unwrap();
    assert_eq!(after.jobs.last().unwrap().state, "completed");
    assert_eq!(audio_files(&folder), Vec::<String>::new(), "no wav is left");
    assert!(after.audio_removed);
    assert!(after.audio_kept.is_none());
    assert!(after.capture.chunks.is_empty());
    assert!(after.capture.gaps.is_empty() && after.capture.missing_chunks.is_empty());
    assert!(after.capture.unverified_chunks.is_empty());
    assert!(!after.has_audio && after.has_text);
    assert_eq!(after.meeting.duration_secs, before.meeting.duration_secs);
    assert_eq!(
        after.revision,
        before.revision + 1,
        "the transcript bumped it once; deleting the audio did not"
    );
    // session.json is what a reload falls back to when capture.json is gone.
    let stored = f.repo.read(id).unwrap().unwrap();
    assert!(stored.audio_removed && stored.capture.chunks.is_empty());
    if let Some(capture) = f.repo.capture(id).unwrap() {
        assert!(capture.chunks.is_empty());
    }
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn removing_the_audio_never_touches_an_external_audio_path() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let external = f.root.with_extension("source-audio");
    std::fs::create_dir_all(&external).unwrap();
    let original = external.join("registrazione-originale.wav");
    std::fs::write(&original, [7u8; 48]).unwrap();
    let original_text = original.to_string_lossy().into_owned();
    f.repo
        .update(id, &mut |d| {
            d.meeting.audio_path = Some(original_text.clone());
            Ok(())
        })
        .unwrap();

    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);

    let after = f.service.read(id).unwrap();
    assert!(after.audio_removed);
    assert_eq!(std::fs::read(&original).unwrap(), vec![7u8; 48]);
    assert_eq!(
        after.meeting.audio_path.as_deref().map(Path::new),
        Some(original.as_path())
    );
    assert!(!after.has_audio, "the removed flag wins over the external path");
    std::fs::remove_dir_all(&external).unwrap();
}

#[test]
fn empty_transcript_keeps_the_audio_and_says_why() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    f.processor.empty_text.store(true, Ordering::SeqCst);

    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);

    let kept = f.service.read(id).unwrap();
    assert_eq!(kept.jobs.last().unwrap().state, "completed");
    assert_eq!(kept.audio_kept.as_deref(), Some("empty_transcript"));
    assert!(!kept.audio_removed && kept.has_audio && !kept.has_text);
    assert_eq!(kept.capture.chunks.len(), 2);
    assert_eq!(audio_files(&folder).len(), 4, "two blocks and their receipts");

    // A later transcript with words finishes the job: the audio goes, the reason with it.
    f.processor.empty_text.store(false, Ordering::SeqCst);
    wait(|| {
        f.service
            .transcribe(id, TranscribeOptions::default())
            .is_ok()
    });
    worker_done(&f, id);
    let removed = f.service.read(id).unwrap();
    assert!(removed.audio_removed && removed.audio_kept.is_none());
    assert!(audio_files(&folder).is_empty());
}

#[test]
fn gaps_keep_the_audio() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 2000]);
    let folder = f.repo.folder(id);
    let before = f.service.read(id).unwrap();
    assert!(before.capture.incomplete && before.capture.gaps.len() == 1);

    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);

    let kept = f.service.read(id).unwrap();
    assert_eq!(kept.audio_kept.as_deref(), Some("interrupted_capture"));
    assert!(!kept.audio_removed && kept.has_audio && kept.has_text);
    assert_eq!(kept.capture.chunks.len(), 2);
    assert_eq!(kept.capture.gaps.len(), 1);
    assert!(!audio_files(&folder).is_empty());
}

#[test]
fn failed_or_cancelled_transcription_keeps_the_audio() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    let untouched = |detail: &MeetingDetail| {
        assert!(!detail.audio_removed && detail.audio_kept.is_none(), "{detail:?}");
        assert_eq!(detail.capture.chunks.len(), 2);
        assert_eq!(audio_files(&folder).len(), 4);
    };

    f.processor.fail.store(true, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);
    let failed = f.service.read(id).unwrap();
    assert_eq!(failed.jobs.last().unwrap().state, "failed");
    untouched(&failed);

    f.processor.fail.store(false, Ordering::SeqCst);
    f.processor.entered.store(false, Ordering::SeqCst);
    f.processor.release.store(false, Ordering::SeqCst);
    wait(|| {
        f.service
            .transcribe(id, TranscribeOptions::default())
            .is_ok()
    });
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    let running = f.service.read(id).unwrap();
    f.service
        .cancel_job(id, &running.jobs.last().unwrap().id)
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    worker_done(&f, id);
    let cancelled = f.service.read(id).unwrap();
    assert_eq!(cancelled.jobs.last().unwrap().state, "cancelled");
    untouched(&cancelled);
}

#[test]
fn retry_of_a_transcribe_after_removal_says_the_audio_is_gone() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);
    assert!(f.service.read(id).unwrap().audio_removed);
    // An earlier attempt that failed, as a retry would find it.
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "older".into(),
                kind: "transcribe".into(),
                state: "failed".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    let calls = f.processor.calls.load(Ordering::SeqCst);

    let retry = f.service.retry_job(id, "older").unwrap_err();
    assert_eq!(retry.code, "audio_removed");
    assert_eq!(
        retry.message,
        "L'audio di questa riunione è stato eliminato dopo la trascrizione."
    );
    let again = f
        .service
        .transcribe(id, TranscribeOptions::default())
        .unwrap_err();
    assert_eq!(again.code, "audio_removed");
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), calls);

    // Only transcribing needs the audio: a summary of the saved text still works.
    f.service.summarize(id, "it".into()).unwrap();
    worker_done(&f, id);
    assert_eq!(
        f.service.read(id).unwrap().jobs.last().unwrap().state,
        "completed"
    );
}

/// A saved worker result for `id`, waiting to be replayed by whoever reads the meeting.
fn saved_result_waiting(f: &Fixture, id: u64) -> MeetingDetail {
    let d = f.service.read(id).unwrap();
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "ready".into(),
                state: "running".into(),
                kind: "transcribe".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.repo
        .save_job_result(&completed_checkpoint(id, "ready", d.revision))
        .unwrap();
    d
}

#[test]
fn restart_reconcile_removes_the_audio_too() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    let d = saved_result_waiting(&f, id);
    let reopened = MeetingService::new(
        f.context.clone(),
        f.repo.clone(),
        f.audio.clone(),
        f.processor.clone(),
    );

    reopened.recover_interrupted().unwrap();

    let saved = reopened.read(id).unwrap();
    assert_eq!(saved.jobs[0].state, "completed");
    assert_eq!(saved.transcripts.len(), 1);
    assert_eq!(audio_files(&folder), Vec::<String>::new());
    assert!(saved.audio_removed && saved.audio_kept.is_none());
    assert!(saved.capture.chunks.is_empty());
    assert!(!saved.has_audio && saved.has_text);
    assert_eq!(saved.meeting.duration_secs, d.meeting.duration_secs);
    assert_eq!(saved.revision, d.revision + 1);
    reopened.recover_interrupted().unwrap();
    let again = reopened.read(id).unwrap();
    assert_eq!(again.revision, saved.revision, "a second pass changes nothing");
    assert!(again.audio_removed && again.capture.chunks.is_empty());
    assert_eq!(again.meeting.duration_secs, d.meeting.duration_secs);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
    assert!(f.repo.job_results(id).unwrap().0.is_empty());
}

#[test]
fn opening_a_meeting_with_a_saved_result_removes_the_audio_too() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    saved_result_waiting(&f, id);

    let opened = f.service.read(id).unwrap();

    assert_eq!(opened.transcripts.len(), 1);
    assert!(opened.audio_removed && !opened.has_audio && opened.has_text);
    assert_eq!(audio_files(&folder), Vec::<String>::new());
}

#[test]
fn a_job_holding_the_meeting_leaves_the_audio_for_the_cleanup() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    saved_result_waiting(&f, id);
    let lease = f.repo.claim_job(id).unwrap();

    let opened = f.service.read(id).unwrap();

    assert_eq!(opened.transcripts.len(), 1, "the result is committed anyway");
    assert!(!opened.audio_removed && opened.has_audio);
    assert!(!audio_files(&folder).is_empty());
    drop(lease);
    let swept = f.service.audio_cleanup().unwrap();
    assert_eq!(swept.meetings, 1);
    assert!(swept.bytes > 0);
    assert!(f.service.read(id).unwrap().audio_removed);
    assert!(audio_files(&folder).is_empty());
}

#[test]
fn a_busy_recorder_leaves_the_audio_and_the_commit_stands() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 1000]);
    let folder = f.repo.folder(id);
    saved_result_waiting(&f, id);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(folder.join(".capture.lock"))
        .unwrap();
    lock.try_lock().unwrap();

    let opened = f.service.read(id).unwrap();

    assert_eq!(opened.transcripts.len(), 1, "the transcript is saved");
    assert_eq!(opened.jobs.last().unwrap().state, "completed");
    assert!(!opened.audio_removed && opened.has_audio, "nothing was recorded as removed");
    assert_eq!(opened.capture.chunks.len(), 2);
    assert_eq!(audio_files(&folder).len(), 4);
    drop(lock);
    assert_eq!(f.service.audio_cleanup().unwrap().meetings, 1);
    assert!(f.service.read(id).unwrap().audio_removed);
}

#[test]
fn manual_discard_requires_a_kept_reason() {
    let f = Fixture::new();
    let id = meeting_with_real_audio(&f, &[0, 2000]);
    let folder = f.repo.folder(id);

    // Nothing decided yet: there is no reason to keep the audio, so no manual delete either.
    let early = f.service.discard_audio(id).unwrap_err();
    assert_eq!(early.code, "validation");
    assert_eq!(early.message, "Non c'è audio da eliminare.");
    assert!(!audio_files(&folder).is_empty());

    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    worker_done(&f, id);
    let kept = f.service.read(id).unwrap();
    assert_eq!(kept.audio_kept.as_deref(), Some("interrupted_capture"));

    // A job holding the meeting blocks the manual delete and leaves everything alone.
    let lease = f.repo.claim_job(id).unwrap();
    assert_eq!(f.service.discard_audio(id).unwrap_err().code, "job_running");
    assert!(!audio_files(&folder).is_empty());
    drop(lease);

    let gone = f.service.discard_audio(id).unwrap();
    assert!(gone.audio_removed && gone.audio_kept.is_none());
    assert!(gone.capture.chunks.is_empty() && gone.capture.gaps.is_empty());
    assert!(!gone.capture.incomplete);
    assert!(!gone.has_audio && gone.has_text, "the returned detail has fresh flags");
    assert_eq!(audio_files(&folder), Vec::<String>::new());
    assert_eq!(gone.meeting.duration_secs, kept.meeting.duration_secs);
    assert_eq!(gone.revision, kept.revision);
    assert!(f.service.read(id).unwrap().audio_removed);
    assert_eq!(f.service.discard_audio(id).unwrap_err().code, "validation");
}

#[test]
fn cleanup_counts_then_removes_only_clean_meetings() {
    let f = Fixture::new();
    let clean = recorded_meeting(&f, 81, &[0, 1000]);
    with_transcript(&f, 81, "Decisione: rilascio venerdì");
    let gap = recorded_meeting(&f, 82, &[0, 2000]);
    with_transcript(&f, 82, "Decisione: rilascio venerdì");
    let empty = recorded_meeting(&f, 83, &[0, 1000]);
    with_transcript(&f, 83, "   ");
    let pending = recorded_meeting(&f, 84, &[0, 1000]);
    let busy = recorded_meeting(&f, 85, &[0, 1000]);
    with_transcript(&f, 85, "Decisione: rilascio venerdì");
    let clean_bytes = audio_size_on_disk(&clean);
    let busy_bytes = audio_size_on_disk(&busy);
    assert!(clean_bytes > 0 && busy_bytes > 0);
    let lease = f.repo.claim_job(85).unwrap();

    let preview = f.service.audio_cleanup_preview().unwrap();
    assert_eq!(
        preview,
        AudioCleanup {
            meetings: 2,
            bytes: clean_bytes + busy_bytes
        },
        "the clean meeting and the one a job holds"
    );
    for folder in [&clean, &gap, &empty, &pending, &busy] {
        assert!(!audio_files(folder).is_empty(), "a preview deletes nothing");
    }

    let swept = f.service.audio_cleanup().unwrap();
    assert_eq!(
        swept,
        AudioCleanup {
            meetings: 1,
            bytes: clean_bytes
        },
        "the busy meeting is skipped"
    );
    assert!(audio_files(&clean).is_empty());
    for folder in [&gap, &empty, &pending, &busy] {
        assert!(!audio_files(folder).is_empty());
    }
    let removed = f.service.read(81).unwrap();
    assert!(removed.audio_removed && !removed.has_audio && removed.has_text);
    assert_eq!(
        f.service.read(82).unwrap().capture.chunks.len(),
        2,
        "a gap keeps its blocks"
    );
    assert!(!f.service.read(83).unwrap().audio_removed);
    assert!(!f.service.read(84).unwrap().audio_removed);

    drop(lease);
    assert_eq!(
        f.service.audio_cleanup().unwrap(),
        AudioCleanup {
            meetings: 1,
            bytes: busy_bytes
        }
    );
    assert!(audio_files(&busy).is_empty());
    assert_eq!(
        f.service.audio_cleanup_preview().unwrap(),
        AudioCleanup::default()
    );
    assert_eq!(f.service.audio_cleanup().unwrap(), AudioCleanup::default());
}

#[test]
fn list_page_keeps_has_audio_and_has_text_after_stripping() {
    let f = Fixture::new();
    recorded_meeting(&f, 71, &[0, 1000]);
    with_transcript(&f, 71, "Testo");
    recorded_meeting(&f, 72, &[0, 1000]);
    f.repo
        .create(&MeetingDetail::legacy(Meeting {
            id: 73,
            transcript: "Solo testo".into(),
            ..Default::default()
        }))
        .unwrap();
    // Known only to the compatibility store, with an audio file the old app imported.
    f.context
        .update(|s| {
            s.meetings.push(Meeting {
                id: 74,
                transcript: "Solo nello store".into(),
                audio_path: Some("C:/audio/vecchia.wav".into()),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    recorded_meeting(&f, 75, &[0, 1000]);
    with_transcript(&f, 75, "Testo");
    f.repo
        .update(75, &mut |d| {
            d.audio_removed = true;
            d.capture.chunks.clear();
            Ok(())
        })
        .unwrap();
    f.repo
        .create(&MeetingDetail::legacy(Meeting {
            id: 76,
            ..Default::default()
        }))
        .unwrap();

    let page = f.service.list_page(0, 100, None, None, None).unwrap();

    let flags = |id: u64| {
        let item = page.items.iter().find(|d| d.meeting.id == id).unwrap();
        // The heavy fields really are gone: only the flags can tell.
        assert!(item.capture.chunks.is_empty(), "{id}");
        assert!(item.meeting.transcript.is_empty() && item.transcripts.is_empty(), "{id}");
        assert!(item.meeting.audio_path.is_none(), "{id}");
        (item.has_audio, item.has_text)
    };
    assert_eq!(flags(71), (true, true), "audio and text");
    assert_eq!(flags(72), (true, false), "audio only");
    assert_eq!(flags(73), (false, true), "text only");
    assert_eq!(flags(74), (true, true), "a meeting only the compatibility store knows");
    assert_eq!(flags(75), (false, true), "the audio was removed");
    assert_eq!(flags(76), (false, false), "nothing recorded, nothing said");
}

#[test]
fn mutating_calls_return_correct_flags() {
    let f = Fixture::new();
    let folder = recorded_meeting(&f, 91, &[0, 1000]);
    with_transcript(&f, 91, "Decisione: rilascio venerdì");
    f.repo
        .update(91, &mut |d| {
            d.summaries.push(SummaryVersion {
                id: "sum".into(),
                actions: vec![MeetingAction {
                    id: "act".into(),
                    text: "Mandare il verbale".into(),
                    ..Default::default()
                }],
                complete: true,
                ..Default::default()
            });
            d.active_summary = Some("sum".into());
            Ok(())
        })
        .unwrap();
    let before = f.service.read(91).unwrap();
    assert!(before.has_audio && before.has_text);

    let toggled = f
        .service
        .toggle_action(91, "act", true, before.revision)
        .unwrap();
    assert!(toggled.has_text, "a transcript with text is reported after a toggle");
    assert!(toggled.has_audio);

    let renamed = f
        .service
        .update(91, "Nuovo titolo".into(), None, true, toggled.revision)
        .unwrap();
    assert!(renamed.has_audio && renamed.has_text);

    assert_eq!(
        f.service.discard_audio(91).unwrap_err().code,
        "validation",
        "nothing decided yet"
    );
    f.repo
        .update(91, &mut |d| {
            d.audio_kept = Some("interrupted_capture".into());
            Ok(())
        })
        .unwrap();
    let discarded = f.service.discard_audio(91).unwrap();
    assert!(!discarded.has_audio, "the manual delete reports the audio as gone");
    assert!(discarded.has_text);
    assert!(audio_files(&folder).is_empty());
}

// A transcribe job that did not save a transcript must not let the audio go, even when the
// meeting already holds an older complete transcript with text (every meeting recorded
// before this feature): `commit_job_result` returns Ok for such a checkpoint.

/// A meeting with a complete transcript with text already stored AND its audio on disk.
fn meeting_with_text_and_audio(f: &Fixture) -> (u64, PathBuf) {
    let id = meeting_with_real_audio(f, &[0, 1000]);
    with_transcript(f, id, "Testo già salvato");
    (id, f.repo.folder(id))
}

fn assert_audio_kept(f: &Fixture, id: u64, folder: &Path) {
    let detail = f.service.read(id).unwrap();
    assert!(!detail.audio_removed, "{detail:?}");
    assert!(detail.has_audio && detail.capture.chunks.len() == 2);
    assert_eq!(audio_files(folder).len(), 4, "the blocks and their receipts are still there");
}

#[test]
fn a_stale_transcribe_keeps_the_audio_of_a_meeting_that_already_has_text() {
    let f = Fixture::new();
    let (id, folder) = meeting_with_text_and_audio(&f);
    let revision = f.service.read(id).unwrap().revision;
    f.processor.release.store(false, Ordering::SeqCst);
    f.service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    f.service
        .save_notes(id, "Una nota durante l'elaborazione".into(), revision)
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    worker_done(&f, id);

    let result = f.service.read(id).unwrap();
    assert_eq!(result.jobs.last().unwrap().state, "failed", "the content changed");
    assert_eq!(result.transcripts.len(), 1, "no new transcript was saved");
    assert!(!result.audio_removed);
    assert_eq!(audio_files(&folder).len(), 4);
    assert_audio_kept(&f, id, &folder);
}

#[test]
fn a_cancelled_transcribe_keeps_the_audio_of_a_meeting_that_already_has_text() {
    let f = Fixture::new();
    let (id, folder) = meeting_with_text_and_audio(&f);
    f.processor.release.store(false, Ordering::SeqCst);
    let queued = f
        .service
        .transcribe(id, TranscribeOptions::default())
        .unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    f.service
        .cancel_job(id, &queued.jobs.last().unwrap().id)
        .unwrap();
    f.processor.release.store(true, Ordering::SeqCst);
    worker_done(&f, id);

    let result = f.service.read(id).unwrap();
    assert_eq!(result.jobs.last().unwrap().state, "cancelled");
    assert_eq!(result.transcripts.len(), 1, "no new transcript was saved");
    assert_audio_kept(&f, id, &folder);
}

#[test]
fn replaying_a_stale_or_cancelled_result_keeps_the_audio_too() {
    for state in ["stale", "cancelled"] {
        let f = Fixture::new();
        let (id, folder) = meeting_with_text_and_audio(&f);
        let d = f.service.read(id).unwrap();
        f.repo
            .update(id, &mut |d| {
                d.jobs.push(MeetingJob {
                    id: "ready".into(),
                    state: if state == "cancelled" { "cancelled" } else { "running" }.into(),
                    kind: "transcribe".into(),
                    input_revision: d.revision,
                    ..Default::default()
                });
                if state == "stale" {
                    // The user edited the meeting after the job started.
                    d.revision += 1;
                }
                Ok(())
            })
            .unwrap();
        f.repo
            .save_job_result(&completed_checkpoint(id, "ready", d.revision))
            .unwrap();
        let reopened = MeetingService::new(
            f.context.clone(),
            f.repo.clone(),
            f.audio.clone(),
            f.processor.clone(),
        );

        reopened.recover_interrupted().unwrap();

        let replayed = reopened.read(id).unwrap();
        let job = replayed.jobs.iter().find(|j| j.id == "ready").unwrap();
        assert_eq!(
            job.state,
            if state == "cancelled" { "cancelled" } else { "failed" },
            "{state}"
        );
        assert_eq!(replayed.transcripts.len(), 1, "{state}: nothing new was saved");
        assert!(!replayed.audio_removed, "{state}");
        assert_eq!(audio_files(&folder).len(), 4, "{state}");
        assert!(f.repo.job_results(id).unwrap().0.is_empty(), "{state}: the journal is consumed");
        // Opening it the plain way does not delete anything either.
        assert!(!f.service.read(id).unwrap().audio_removed, "{state}");
    }
}

#[test]
fn replaying_a_completed_job_whose_audio_was_never_settled_heals() {
    // The crash window: the job was committed as completed, then the process died before
    // the audio was dropped. The journal is still there, the replay finds the job completed.
    let f = Fixture::new();
    let (id, folder) = meeting_with_text_and_audio(&f);
    let d = f.service.read(id).unwrap();
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: "ready".into(),
                state: "completed".into(),
                kind: "transcribe".into(),
                input_revision: d.revision,
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.repo
        .save_job_result(&completed_checkpoint(id, "ready", d.revision))
        .unwrap();

    let opened = f.service.read(id).unwrap();

    assert!(opened.audio_removed, "{opened:?}");
    assert_eq!(audio_files(&folder), Vec::<String>::new());
    assert!(f.repo.job_results(id).unwrap().0.is_empty());
}

#[test]
fn default_processor_is_blocked_and_not_ready() {
    let options = TranscribeOptions::default();
    let settings = Settings::default();
    // The trait defaults fail closed: an engine that says nothing cannot transcribe.
    assert_eq!(
        Bare.transcription_blocker(&options, &settings).as_deref(),
        Some("Trascrizione non configurata.")
    );
    assert!(!Bare.summary_ready(&settings));
    // So does the test processor until a test says otherwise.
    let fake = Processor::default();
    assert!(fake.transcription_blocker(&options, &settings).is_some());
    assert!(!fake.summary_ready(&settings));
}

// --- The automatic chain: stop -> transcribe -> summarize ---------------------------------

/// Waits until the meeting has exactly `count` jobs and none of them is still going.
fn settled(service: &MeetingService, id: u64, count: usize) -> MeetingDetail {
    wait(|| {
        let d = service.read(id).unwrap();
        d.jobs.len() == count
            && d.jobs.iter().all(|j| !matches!(j.state.as_str(), "queued" | "running"))
    });
    service.read(id).unwrap()
}
/// Waits until no worker holds the meeting's job lease any more.
fn released(f: &Fixture, id: u64) {
    wait(|| f.repo.claim_job(id).is_ok());
}
/// For the assertions that nothing more happens: a worker would start within this time.
fn quiet() {
    std::thread::sleep(Duration::from_millis(250));
}
fn kinds(d: &MeetingDetail) -> Vec<&str> {
    d.jobs.iter().map(|j| j.kind.as_str()).collect()
}
fn states(d: &MeetingDetail) -> Vec<&str> {
    d.jobs.iter().map(|j| j.state.as_str()).collect()
}
fn running_transcribe_job(f: &Fixture, id: u64, job_id: &str) {
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(MeetingJob {
                id: job_id.into(),
                state: "running".into(),
                kind: "transcribe".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
}
fn reopened(f: &Fixture) -> MeetingService {
    MeetingService::new(f.context.clone(), f.repo.clone(), f.audio.clone(), f.processor.clone())
}

#[test]
fn default_engine_is_the_compute_server() {
    assert_eq!(Settings::default().meeting_options().provider, "compute");
}

#[test]
fn stop_queues_transcribe_then_summarize_without_more_clicks() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.summary.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    let stopped = f.service.stop().unwrap();
    assert_eq!(stopped.state, CaptureState::Saved);
    let d = settled(&f.service, id, 2);
    assert_eq!(kinds(&d), ["transcribe", "summarize"]);
    assert_eq!(states(&d), ["completed", "completed"]);
    assert_eq!(d.meeting.transcript, "Decisione: rilascio venerdì");
    assert_eq!(d.meeting.summary, "Verbale");
    // The summary says which transcript it was written from.
    assert_eq!(d.summary().unwrap().transcript_version, d.active_transcript);
    assert_eq!(d.jobs[0].options.as_ref().unwrap().provider, "compute");
    assert_eq!(d.jobs[1].language.as_deref(), Some("it"));
}

#[test]
fn stop_with_a_blocked_engine_queues_nothing_and_reports_why() {
    let f = Fixture::new();
    f.processor.summary.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    let stopped = f.service.stop().unwrap();
    assert_eq!(stopped.state, CaptureState::Saved);
    assert!(f.service.read(id).unwrap().jobs.is_empty());
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
    let readiness = f.service.readiness().unwrap();
    assert_eq!(readiness.transcription_blocker.as_deref(), Some("Trascrizione non configurata."));
    assert!(readiness.summary_ready);
    f.processor.ready.store(true, Ordering::SeqCst);
    assert_eq!(f.service.readiness().unwrap().transcription_blocker, None);
}

#[test]
fn stop_still_ends_the_recording_when_the_recorder_cannot_report_first() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.audio.fail_next_snapshot.store(true, Ordering::SeqCst);
    // The look at the recorder before the stop fails: "Termina" must reach the recorder anyway.
    let stopped = f.service.stop().unwrap();
    assert_eq!(f.audio.stops.load(Ordering::SeqCst), 1);
    assert_eq!(stopped.state, CaptureState::Saved);
    // It cannot tell this stop ended a live recording, so nothing starts by itself;
    // the meeting still reads as saved and can be transcribed from the page.
    quiet();
    assert!(f.service.read(id).unwrap().jobs.is_empty());
    assert_eq!(f.service.read(id).unwrap().capture.state, CaptureState::Saved);
}

#[test]
fn stop_does_not_transcribe_a_capture_that_did_not_end_saved() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    f.audio.interrupt_on_stop.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    assert_eq!(f.service.stop().unwrap().state, CaptureState::Interrupted);
    assert!(f.service.read(id).unwrap().jobs.is_empty());
}

#[test]
fn summary_is_skipped_when_ai_is_not_ready() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.service.stop().unwrap();
    let d = settled(&f.service, id, 1);
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe"], vec!["completed"]));
    quiet();
    assert_eq!(f.service.read(id).unwrap().jobs.len(), 1);
    assert!(f.service.read(id).unwrap().summary().is_none());
}

#[test]
fn a_transcript_without_words_is_not_summarized() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.summary.store(true, Ordering::SeqCst);
    f.processor.empty_text.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.service.stop().unwrap();
    settled(&f.service, id, 1);
    quiet();
    assert_eq!(f.service.read(id).unwrap().jobs.len(), 1);
}

#[test]
fn a_failed_summary_is_not_retried_by_itself() {
    let f = Fixture::new();
    // Not ready while stopping: nothing starts, and the audio is kept for later runs.
    let id = f.saved_keeping_audio().meeting.id;
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.summary.store(true, Ordering::SeqCst);
    f.processor.fail_summary.store(true, Ordering::SeqCst);
    f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    let d = settled(&f.service, id, 2);
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe", "summarize"], vec!["completed", "failed"]));
    // Another run that ends without a new transcript leads back to the same old one, still
    // without a summary: the pipeline must not try the summary again on its own.
    f.processor.fail.store(true, Ordering::SeqCst);
    released(&f, id);
    f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    let d = settled(&f.service, id, 3);
    assert_eq!(states(&d), ["completed", "failed", "failed"]);
    quiet();
    let d = f.service.read(id).unwrap();
    assert_eq!(kinds(&d), ["transcribe", "summarize", "transcribe"]);
    assert!(d.summary().is_none());
}

#[test]
fn a_new_transcript_gets_its_own_summary_even_after_a_failed_one() {
    let f = Fixture::new();
    let id = f.saved_keeping_audio().meeting.id;
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.summary.store(true, Ordering::SeqCst);
    f.processor.fail_summary.store(true, Ordering::SeqCst);
    f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    settled(&f.service, id, 2);
    f.processor.fail_summary.store(false, Ordering::SeqCst);
    released(&f, id);
    f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    let d = settled(&f.service, id, 4);
    assert_eq!(kinds(&d), ["transcribe", "summarize", "transcribe", "summarize"]);
    assert_eq!(states(&d), ["completed", "failed", "completed", "completed"]);
    assert_eq!(d.summary().unwrap().transcript_version.as_deref(), Some(d.jobs[2].id.as_str()));
}

#[test]
fn recover_queues_the_transcription() {
    let f = Fixture::new();
    let id = f.start().meeting.id;
    // The recorder died: one confirmed block is on disk and the manifest says Interrupted.
    let folder = f.repo.folder(id);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create(folder.join("confirmed.wav"), spec).unwrap();
    for _ in 0..800 {
        wav.write_sample(123_i16).unwrap();
    }
    wav.finalize().unwrap();
    let snapshot = CaptureSnapshot {
        meeting_id: Some(id),
        state: CaptureState::Interrupted,
        elapsed_ms: 130,
        saved_ms: 100,
        chunks: vec![AudioChunk {
            source: "microphone".into(),
            path: "confirmed.wav".into(),
            sample_rate: 8000,
            channels: 1,
            end_ms: 100,
            ..Default::default()
        }],
        ..Default::default()
    };
    std::fs::write(folder.join("capture.json"), serde_json::to_vec(&snapshot).unwrap()).unwrap();
    f.audio.snapshot.lock().unwrap().state = CaptureState::Interrupted;
    f.processor.ready.store(true, Ordering::SeqCst);
    let recovered = f.service.recover(id).unwrap();
    assert_eq!(recovered.capture.state, CaptureState::Saved);
    let d = settled(&f.service, id, 1);
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe"], vec!["completed"]));
    assert!(d.has_text);
}

#[test]
fn recover_that_fails_queues_nothing() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    // The recorder is still running: recovery is refused and nothing must start.
    assert_eq!(f.service.recover(id).unwrap_err().code, "recording_busy");
    assert!(f.service.read(id).unwrap().jobs.is_empty());
}

#[test]
fn restart_requeues_a_job_that_was_running() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    running_transcribe_job(&f, id, "was-running");
    f.processor.ready.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = settled(&service, id, 2);
    assert_eq!(d.jobs[0].id, "was-running");
    assert_eq!(d.jobs[0].state, "interrupted");
    assert_eq!((d.jobs[1].kind.as_str(), d.jobs[1].state.as_str()), ("transcribe", "completed"));
    assert!(d.has_text);
}

#[test]
fn restart_leaves_an_interrupted_transcription_alone_while_the_engine_is_blocked() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    running_transcribe_job(&f, id, "was-running");
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = service.read(id).unwrap();
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe"], vec!["interrupted"]));
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn second_restart_does_not_requeue() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    running_transcribe_job(&f, id, "was-running");
    f.processor.ready.store(true, Ordering::SeqCst);
    let first = reopened(&f);
    first.recover_interrupted().unwrap();
    settled(&first, id, 2);
    released(&f, id);
    let before = f.processor.calls.load(Ordering::SeqCst);
    let second = reopened(&f);
    second.recover_interrupted().unwrap();
    let d = second.read(id).unwrap();
    assert_eq!(states(&d), ["interrupted", "completed"]);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), before);
}

/// Starts a recording, stops it (which queues the transcription by itself) and waits until
/// the engine is working, held there until the test lets it go.
fn transcribing(f: &Fixture) -> (u64, String) {
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.release.store(false, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.service.stop().unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    let job = f.repo.read(id).unwrap().unwrap().jobs[0].id.clone();
    (id, job)
}

#[test]
fn closing_the_app_mid_transcription_leaves_the_job_to_be_requeued_at_the_next_launch() {
    let f = Fixture::new();
    let (id, job) = transcribing(&f);
    // What the close guard does: the recording is stopped, then every job is cancelled.
    f.service.stop().unwrap();
    f.service.cancel_all_jobs();
    released(&f, id);
    quiet();
    // The shutdown did not turn into a cancelled job and no other read marked it interrupted:
    // the next launch finds the job as the process left it.
    let left = f.repo.read(id).unwrap().unwrap();
    assert_eq!((kinds(&left), states(&left)), (vec!["transcribe"], vec!["running"]));
    assert_eq!(left.jobs[0].id, job);

    f.processor.release.store(true, Ordering::SeqCst);
    let next = reopened(&f);
    next.recover_interrupted().unwrap();
    let d = settled(&next, id, 2);
    assert_eq!(kinds(&d), ["transcribe", "transcribe"]);
    assert_eq!(states(&d), ["interrupted", "completed"]);
    assert!(d.has_text);
}

#[test]
fn a_job_the_person_cancelled_is_not_requeued_at_the_next_launch() {
    let f = Fixture::new();
    let (id, job) = transcribing(&f);
    f.service.cancel_job(id, &job).unwrap();
    released(&f, id);
    quiet();
    assert_eq!(states(&f.repo.read(id).unwrap().unwrap()), ["cancelled"]);

    f.processor.release.store(true, Ordering::SeqCst);
    let next = reopened(&f);
    next.recover_interrupted().unwrap();
    quiet();
    let d = next.read(id).unwrap();
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe"], vec!["cancelled"]));
    assert!(!d.has_text);
}

#[test]
fn a_cancel_by_the_person_then_the_close_stays_cancelled() {
    let f = Fixture::new();
    let (id, job) = transcribing(&f);
    f.service.cancel_job(id, &job).unwrap();
    f.service.cancel_all_jobs();
    released(&f, id);
    quiet();
    f.processor.release.store(true, Ordering::SeqCst);
    let next = reopened(&f);
    next.recover_interrupted().unwrap();
    quiet();
    assert_eq!(states(&next.read(id).unwrap()), ["cancelled"]);
}

#[test]
fn work_that_finished_before_the_close_is_still_saved() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.service.stop().unwrap();
    let d = settled(&f.service, id, 1);
    assert_eq!(states(&d), ["completed"]);
    f.service.cancel_all_jobs();
    quiet();
    assert_eq!(states(&f.service.read(id).unwrap()), ["completed"]);
    assert!(f.service.read(id).unwrap().has_text);
}

#[test]
fn startup_recovery_requeues_what_the_screen_asked_about_first() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    running_transcribe_job(&f, id, "was-running");
    f.processor.ready.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    // The window is already asking while the recovery has not reached this meeting.
    service.set_recovering(true);
    for _ in 0..2 {
        assert_eq!(states(&service.read(id).unwrap()), ["running"]);
        let listed = service.list().unwrap();
        assert_eq!(states(listed.iter().find(|d| d.meeting.id == id).unwrap()), ["running"]);
    }
    let page = service.list_page(0, 20, None, None, None).unwrap();
    assert_eq!(states(page.items.iter().find(|d| d.meeting.id == id).unwrap()), ["running"]);
    assert_eq!(states(&f.repo.read(id).unwrap().unwrap()), ["running"], "nothing was written");

    service.recover_interrupted().unwrap();
    service.set_recovering(false);
    let d = settled(&service, id, 2);
    assert_eq!(states(&d), ["interrupted", "completed"]);
    assert!(d.has_text);
}

#[test]
fn without_a_recovery_in_progress_a_free_lease_still_marks_the_job_interrupted() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    running_transcribe_job(&f, id, "crashed-elsewhere");
    let service = reopened(&f);
    assert_eq!(states(&service.read(id).unwrap()), ["interrupted"]);
}

#[test]
fn restart_does_not_transcribe_old_meetings() {
    let f = Fixture::new();
    let d = f.saved();
    assert!(d.has_audio);
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.summary.store(true, Ordering::SeqCst);
    reopened(&f).recover_interrupted().unwrap();
    assert!(f.service.read(d.meeting.id).unwrap().jobs.is_empty());
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn restart_chains_the_summary_after_a_recovered_transcript() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    running_transcribe_job(&f, id, "ready");
    f.repo.save_job_result(&completed_checkpoint(id, "ready", d.revision)).unwrap();
    f.processor.summary.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = settled(&service, id, 2);
    assert_eq!(kinds(&d), ["transcribe", "summarize"]);
    assert_eq!(states(&d), ["completed", "completed"]);
    assert_eq!(d.summary().unwrap().transcript_version.as_deref(), Some("ready"));
    // The transcript came from the saved result, not from the engine.
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn restart_requeues_a_summary_that_was_running() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.meeting.notes = "Appunti della riunione".into();
            d.jobs.push(MeetingJob {
                id: "summary-was-running".into(),
                state: "running".into(),
                kind: "summarize".into(),
                language: Some("en".into()),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    f.processor.summary.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = settled(&service, id, 2);
    assert_eq!(kinds(&d), ["summarize", "summarize"]);
    assert_eq!(states(&d), ["interrupted", "completed"]);
    assert_eq!(d.jobs[1].language.as_deref(), Some("en"));
}

#[test]
fn transcribe_default_follows_the_settings_and_refuses_while_blocked() {
    let f = Fixture::new();
    let id = f.saved_keeping_audio().meeting.id;
    let error = f.service.transcribe_default(id).unwrap_err();
    assert_eq!((error.code, error.message.as_str()), ("validation", "Trascrizione non configurata."));
    assert!(f.service.read(id).unwrap().jobs.is_empty());
    f.processor.ready.store(true, Ordering::SeqCst);
    f.context
        .update(|s| {
            s.settings.meeting_model = "small".into();
            s.settings.meeting_language = "en".into();
            Ok(())
        })
        .unwrap();
    f.service.transcribe_default(id).unwrap();
    let d = settled(&f.service, id, 1);
    let options = d.jobs[0].options.clone().unwrap();
    assert_eq!((options.provider.as_str(), options.model.as_str(), options.language.as_str()), ("compute", "small", "en"));
}

#[test]
fn summarize_default_uses_the_meeting_language_or_italian() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.service.save_notes(id, "Appunti".into(), f.service.read(id).unwrap().revision).unwrap();
    for (setting, expected) in [("en", "en"), ("fr", "fr"), ("pt", "it"), ("auto", "it")] {
        f.context
            .update(|s| {
                s.settings.meeting_language = setting.into();
                Ok(())
            })
            .unwrap();
        released(&f, id);
        f.service.summarize_default(id).unwrap();
        let d = f.service.read(id).unwrap();
        assert_eq!(d.jobs.last().unwrap().language.as_deref(), Some(expected), "setting {setting}");
        wait(|| f.service.read(id).unwrap().jobs.iter().all(|j| j.state == "completed" || j.state == "interrupted"));
    }
}

// --- The chain only follows desktop work, and only what just finished ---------------------

fn push_job(f: &Fixture, id: u64, job: MeetingJob) {
    f.repo
        .update(id, &mut |d| {
            d.jobs.push(job.clone());
            Ok(())
        })
        .unwrap();
}
fn set_transcript(f: &Fixture, id: u64, transcript_id: &str, text: &str) {
    f.repo
        .update(id, &mut |d| {
            d.transcripts.push(TranscriptVersion {
                id: transcript_id.into(),
                complete: true,
                segments: vec![TranscriptSegment {
                    id: "seg1".into(),
                    text: text.into(),
                    end_ms: 1000,
                    ..Default::default()
                }],
                ..Default::default()
            });
            d.active_transcript = Some(transcript_id.into());
            d.meeting.transcript = text.into();
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_transcription_started_through_mcp_is_not_followed_by_a_summary() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.processor.summary.store(true, Ordering::SeqCst);
    let mcp = f.service.clone().with_source_profile("review-profile".into());
    mcp.transcribe(id, TranscribeOptions::default()).unwrap();
    let d = settled(&f.service, id, 1);
    assert_eq!(states(&d), ["completed"]);
    assert_eq!(d.jobs[0].source_profile.as_deref(), Some("review-profile"));
    quiet();
    let d = f.service.read(id).unwrap();
    assert_eq!(kinds(&d), ["transcribe"]);
    assert!(d.summary().is_none());
}

#[test]
fn a_desktop_restart_does_not_summarize_a_transcript_an_mcp_job_made() {
    let f = Fixture::new();
    let d = f.saved();
    let id = d.meeting.id;
    push_job(
        &f,
        id,
        MeetingJob {
            id: "ready".into(),
            state: "running".into(),
            kind: "transcribe".into(),
            source_profile: Some("review-profile".into()),
            ..Default::default()
        },
    );
    f.repo.save_job_result(&completed_checkpoint(id, "ready", d.revision)).unwrap();
    f.processor.summary.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = service.read(id).unwrap();
    assert_eq!(d.active_transcript.as_deref(), Some("ready"), "the saved result was applied");
    quiet();
    let d = service.read(id).unwrap();
    assert_eq!(kinds(&d), ["transcribe"]);
    assert!(d.summary().is_none());
}

#[test]
fn a_cancelled_retranscription_does_not_summarize_the_older_transcript() {
    let f = Fixture::new();
    let id = f.saved_keeping_audio().meeting.id;
    f.processor.ready.store(true, Ordering::SeqCst);
    // The first transcript is saved while there is no AI: it stays without a summary.
    f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    let d = settled(&f.service, id, 1);
    assert_eq!(states(&d), ["completed"]);
    released(&f, id);
    // The AI is configured later, then a second transcription is started and cancelled.
    f.processor.summary.store(true, Ordering::SeqCst);
    f.processor.release.store(false, Ordering::SeqCst);
    f.processor.entered.store(false, Ordering::SeqCst);
    let started = f.service.transcribe(id, TranscribeOptions::default()).unwrap();
    wait(|| f.processor.entered.load(Ordering::SeqCst));
    f.service.cancel_job(id, &started.jobs.last().unwrap().id).unwrap();
    released(&f, id);
    quiet();
    let d = f.service.read(id).unwrap();
    assert_eq!(kinds(&d), ["transcribe", "transcribe"]);
    assert_eq!(states(&d), ["completed", "cancelled"]);
    assert!(d.summary().is_none(), "the older transcript was not summarized behind the person's back");
}

#[test]
fn a_second_stop_on_an_idle_recorder_queues_nothing_more() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    // The first transcription fails, so its audio is still there and a new job could start.
    f.processor.fail.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    f.service.stop().unwrap();
    let d = settled(&f.service, id, 1);
    assert_eq!(states(&d), ["failed"]);
    released(&f, id);
    // The recorder answers a stop it has nothing to stop with its last snapshot: Saved.
    assert_eq!(f.service.stop().unwrap().state, CaptureState::Saved);
    quiet();
    assert_eq!(kinds(&f.service.read(id).unwrap()), ["transcribe"]);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn stop_does_not_queue_a_meeting_that_already_has_a_transcribe_job() {
    let f = Fixture::new();
    // Stopped while the engine was blocked, so nothing was queued; the person tried by hand.
    let id = f.saved().meeting.id;
    f.processor.ready.store(true, Ordering::SeqCst);
    push_job(
        &f,
        id,
        MeetingJob {
            id: "tried".into(),
            state: "failed".into(),
            kind: "transcribe".into(),
            ..Default::default()
        },
    );
    // A recorder that is live for this meeting once more, as after a stale click.
    f.audio.snapshot.lock().unwrap().state = CaptureState::Recording;
    assert_eq!(f.service.stop().unwrap().state, CaptureState::Saved);
    quiet();
    assert_eq!(kinds(&f.service.read(id).unwrap()), ["transcribe"]);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_repeated_recover_queues_nothing() {
    let f = Fixture::new();
    f.processor.ready.store(true, Ordering::SeqCst);
    let id = f.start().meeting.id;
    // Same recovered block as `recover_queues_the_transcription`.
    let folder = f.repo.folder(id);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create(folder.join("confirmed.wav"), spec).unwrap();
    for _ in 0..800 {
        wav.write_sample(123_i16).unwrap();
    }
    wav.finalize().unwrap();
    let snapshot = CaptureSnapshot {
        meeting_id: Some(id),
        state: CaptureState::Interrupted,
        elapsed_ms: 130,
        saved_ms: 100,
        chunks: vec![AudioChunk {
            source: "microphone".into(),
            path: "confirmed.wav".into(),
            sample_rate: 8000,
            channels: 1,
            end_ms: 100,
            ..Default::default()
        }],
        ..Default::default()
    };
    std::fs::write(folder.join("capture.json"), serde_json::to_vec(&snapshot).unwrap()).unwrap();
    f.audio.snapshot.lock().unwrap().state = CaptureState::Interrupted;
    f.service.recover(id).unwrap();
    let d = settled(&f.service, id, 1);
    assert!(d.has_text);
    released(&f, id);
    // The recovered capture is flagged incomplete, so its audio is still there to recover.
    assert!(f.service.read(id).unwrap().has_audio, "the second recover needs the audio");
    let calls = f.processor.calls.load(Ordering::SeqCst);
    f.service.recover(id).unwrap();
    quiet();
    let d = f.service.read(id).unwrap();
    assert_eq!(kinds(&d), ["transcribe"]);
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), calls);
}

#[test]
fn a_restarted_transcription_is_judged_by_its_own_options() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    // Started on the remote engine, which cannot work now; the settings point at the local one.
    push_job(
        &f,
        id,
        MeetingJob {
            id: "remote-run".into(),
            state: "running".into(),
            kind: "transcribe".into(),
            options: Some(TranscribeOptions {
                provider: "external".into(),
                model: "whisper-1".into(),
                language: "it".into(),
            }),
            ..Default::default()
        },
    );
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.remote_blocked.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    quiet();
    let d = service.read(id).unwrap();
    assert_eq!((kinds(&d), states(&d)), (vec!["transcribe"], vec!["interrupted"]));
    assert_eq!(f.processor.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_restarted_transcription_does_not_wait_for_an_engine_it_does_not_use() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    // Started on the local engine; the settings now point at a remote one that cannot work.
    running_transcribe_job(&f, id, "local-run");
    f.context
        .update(|s| {
            s.settings.meeting_engine = "external".into();
            Ok(())
        })
        .unwrap();
    f.processor.ready.store(true, Ordering::SeqCst);
    f.processor.remote_blocked.store(true, Ordering::SeqCst);
    assert!(f.service.readiness().unwrap().transcription_blocker.is_some());
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    let d = settled(&service, id, 2);
    assert_eq!(states(&d), ["interrupted", "completed"]);
    assert_eq!(kinds(&d), ["transcribe", "transcribe"]);
}

#[test]
fn restart_does_not_summarize_old_transcripts() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    set_transcript(&f, id, "old", "Decisione presa mesi fa");
    f.processor.summary.store(true, Ordering::SeqCst);
    f.processor.ready.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    quiet();
    let d = service.read(id).unwrap();
    assert!(d.jobs.is_empty());
    assert!(d.summary().is_none());
}

#[test]
fn restart_does_not_requeue_a_summary_while_ai_is_not_ready() {
    let f = Fixture::new();
    let id = f.saved().meeting.id;
    f.repo
        .update(id, &mut |d| {
            d.meeting.notes = "Appunti della riunione".into();
            d.jobs.push(MeetingJob {
                id: "summary-was-running".into(),
                state: "running".into(),
                kind: "summarize".into(),
                ..Default::default()
            });
            Ok(())
        })
        .unwrap();
    // The engine is fine, the AI is not: only the summary gate is in play.
    f.processor.ready.store(true, Ordering::SeqCst);
    let service = reopened(&f);
    service.recover_interrupted().unwrap();
    quiet();
    let d = service.read(id).unwrap();
    assert_eq!((kinds(&d), states(&d)), (vec!["summarize"], vec!["interrupted"]));
}
