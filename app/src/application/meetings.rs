//! Meeting use cases. Capture and model workers never hold the store transaction.
use super::{AppError, AppResult, ServiceContext, meeting_ports::*};
use crate::domain::{meeting::*, model::Meeting};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type Controls = Arc<Mutex<HashMap<(u64, String), Arc<AtomicBool>>>>;
#[path = "meetings/job_control.rs"]
mod job_control;
#[path = "meetings/job_recovery.rs"]
mod job_recovery;

#[derive(Clone)]
pub struct MeetingService {
    context: Arc<ServiceContext>,
    repository: Arc<dyn MeetingRepository>,
    audio: Arc<dyn AudioCapture>,
    processor: Arc<dyn MeetingProcessor>,
    controls: Controls,
    interrupted: Arc<Mutex<std::collections::BTreeMap<(i64, u64), CaptureSnapshot>>>,
    pending_results: Arc<Mutex<HashMap<(u64, String), MeetingJobCheckpoint>>>,
    job_gate: Arc<Mutex<()>>,
    capture_gate: Arc<Mutex<()>>,
    /// Set by `cancel_all_jobs` when the app is closing: a job that stops because of it is
    /// left as it is on disk, so the next launch finds it unfinished and queues it again.
    shutting_down: Arc<AtomicBool>,
    /// Startup recovery is going through the meetings: until it is done, a job with a free
    /// lease is not yet known to be abandoned, see `set_recovering`.
    recovering: Arc<AtomicBool>,
    source_profile: Option<String>,
}

impl MeetingService {
    pub fn new(
        context: Arc<ServiceContext>,
        repository: Arc<dyn MeetingRepository>,
        audio: Arc<dyn AudioCapture>,
        processor: Arc<dyn MeetingProcessor>,
    ) -> Self {
        Self {
            context,
            repository,
            audio,
            processor,
            interrupted: Arc::default(),
            controls: Arc::new(Mutex::new(HashMap::new())),
            pending_results: Arc::new(Mutex::new(HashMap::new())),
            job_gate: Arc::new(Mutex::new(())),
            capture_gate: Arc::new(Mutex::new(())),
            shutting_down: Arc::new(AtomicBool::new(false)),
            recovering: Arc::new(AtomicBool::new(false)),
            source_profile: None,
        }
    }
    /// Trusted MCP adapters attach the origin before starting a durable job.
    pub fn with_source_profile(mut self, profile: String) -> Self {
        self.source_profile = Some(profile);
        self
    }
    /// The desktop sets this while startup recovery runs. Recovery is what re-queues the jobs a
    /// closed app cut short, and it only re-queues those it finds unfinished: a window that
    /// asks about a meeting first must not mark them interrupted before recovery reaches them.
    /// Off by default (MCP and tests have no such phase); shared by every clone of the service.
    pub fn set_recovering(&self, recovering: bool) {
        self.recovering.store(recovering, Ordering::SeqCst);
    }
    fn now(&self) -> i64 {
        self.context.clock.now()
    }
    fn id(&self, prefix: &str) -> String {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(
            "{prefix}-{}-{}-{}",
            self.context.clock.now_ms(),
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }
    pub fn list(&self) -> AppResult<Vec<MeetingDetail>> {
        let mut entries: HashMap<u64, MeetingDetail> = self
            .context
            .read()?
            .meetings
            .into_iter()
            .map(|m| {
                let mut detail = MeetingDetail::legacy(m);
                detail.refresh_flags();
                (detail.meeting.id, detail)
            })
            .collect();
        for detail in self.repository.list()? {
            let detail = self.reconcile_job_results(detail);
            entries.insert(detail.meeting.id, detail);
        }
        let mut result: Vec<_> = entries.into_values().filter(|m| !m.deleted).collect();
        result.sort_by_key(|m| std::cmp::Reverse((m.meeting.date, m.meeting.id)));
        Ok(result)
    }
    pub fn read(&self, id: u64) -> AppResult<MeetingDetail> {
        if let Some(detail) = self.repository.read(id)? {
            return if detail.deleted {
                Err(AppError::missing("Meeting"))
            } else {
                Ok(self.reconcile_job_results(detail))
            };
        }
        self.context
            .read()?
            .meetings
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| {
                let mut detail = MeetingDetail::legacy(m);
                detail.refresh_flags();
                detail
            })
            .ok_or_else(|| AppError::missing("Meeting"))
    }
    pub fn list_page(
        &self,
        offset: usize,
        limit: usize,
        from_date: Option<&str>,
        to_date: Option<&str>,
        state: Option<&str>,
    ) -> AppResult<MeetingListPage> {
        use chrono::TimeZone;
        let parse_date = |value: Option<&str>| -> AppResult<Option<chrono::NaiveDate>> {
            value
                .filter(|v| !v.is_empty())
                .map(|v| {
                    chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d")
                        .map_err(|_| AppError::validation("Data non valida."))
                })
                .transpose()
        };
        let from = parse_date(from_date)?;
        let to = parse_date(to_date)?;
        let state = state.filter(|s| !s.is_empty());
        let mut entries = self.list()?;
        entries.retain(|d| {
            let date = chrono::Local
                .timestamp_opt(d.meeting.date, 0)
                .single()
                .map(|t| t.date_naive());
            let state_name = match d.capture.state {
                CaptureState::Idle => "idle",
                CaptureState::Preparing => "preparing",
                CaptureState::Recording => "recording",
                CaptureState::Paused => "paused",
                CaptureState::Saving => "saving",
                CaptureState::Saved => "saved",
                CaptureState::Interrupted => "interrupted",
            };
            from.is_none_or(|v| date.is_some_and(|date| date >= v))
                && to.is_none_or(|v| date.is_some_and(|date| date <= v))
                && state.is_none_or(|v| state_name == v)
        });
        let total = entries.len();
        let limit = limit.clamp(1, 100);
        let items = entries
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|mut d| {
                d.meeting.transcript.clear();
                d.meeting.notes.clear();
                d.meeting.summary.clear();
                d.meeting.key_points.clear();
                d.meeting.action_items.clear();
                d.meeting.audio_path = None;
                d.capture.chunks.clear();
                d.capture.gaps.clear();
                d.capture.unverified_chunks.clear();
                d.transcripts.clear();
                d.notes.clear();
                d.note_history.clear();
                d.recovery_copies.clear();
                d.summaries.clear();
                d.legacy_transcript = None;
                d.legacy_notes = None;
                if d.jobs.len() > 3 {
                    d.jobs.drain(..d.jobs.len() - 3);
                }
                d
            })
            .collect();
        Ok(MeetingListPage {
            items,
            total,
            next_offset: (offset.saturating_add(limit) < total)
                .then_some(offset.saturating_add(limit)),
        })
    }
    fn ensure(&self, id: u64) -> AppResult<()> {
        if self.repository.read(id)?.is_none() {
            let detail = self.read(id)?;
            if let Err(e) = self.repository.create(&detail) {
                if e.code != "conflict" {
                    return Err(e);
                }
            }
        }
        Ok(())
    }
    /// The indicator needs only the newest interrupted recording and its two clocks.
    /// Recovery seeds the index once; each persisted capture transition updates it.
    pub fn interrupted_capture(&self) -> Option<CaptureSnapshot> {
        self.interrupted.lock().unwrap_or_else(|e| e.into_inner())
            .last_key_value().map(|(_, capture)| capture.clone())
    }

    fn index_capture(&self, detail: &MeetingDetail) {
        let key = (detail.meeting.date, detail.meeting.id);
        let mut index = self.interrupted.lock().unwrap_or_else(|e| e.into_inner());
        if !detail.deleted && detail.capture.state == CaptureState::Interrupted {
            index.insert(key, CaptureSnapshot {
                meeting_id: Some(detail.meeting.id),
                state: CaptureState::Interrupted,
                elapsed_ms: detail.capture.elapsed_ms,
                saved_ms: detail.capture.saved_ms,
                ..Default::default()
            });
        } else if detail.deleted {
            // Tombstones do not retain the recording date.
            index.retain(|(_, id), _| *id != detail.meeting.id);
        } else {
            index.remove(&key);
        }
    }

    fn project(&self, detail: &MeetingDetail) -> AppResult<()> {
        self.context.update(|s| {
            s.next_id = s.next_id.max(detail.meeting.id.saturating_add(1));
            let mut latest = self.repository.read(detail.meeting.id)?;
            if latest
                .as_ref()
                .is_some_and(|d| d.reconcile_projection && !d.deleted)
            {
                let compatible = s
                    .meetings
                    .iter()
                    .find(|m| m.id == detail.meeting.id)
                    .cloned();
                latest = Some(self.repository.update(detail.meeting.id, &mut |d| {
                    if let Some(copy) = &compatible {
                        // The compatibility store may contain the only surviving
                        // copy of a newer edit after a meeting manifest is damaged.
                        if !d.recovery_copies.iter().any(|existing| {
                            serde_json::to_value(existing).ok() == serde_json::to_value(copy).ok()
                        }) {
                            d.recovery_copies.push(copy.clone());
                        }
                    }
                    d.reconcile_projection = false;
                    Ok(())
                })?);
            }
            let detail = latest.as_ref().unwrap_or(detail);
            self.index_capture(detail);
            if detail.deleted {
                s.meetings.retain(|m| m.id != detail.meeting.id);
            } else if let Some(m) = s.meetings.iter_mut().find(|m| m.id == detail.meeting.id) {
                *m = detail.meeting.clone();
            } else {
                s.meetings.push(detail.meeting.clone());
            }
            Ok(())
        })
    }
    fn edit(
        &self,
        id: u64,
        revision: u64,
        mut change: impl FnMut(&mut MeetingDetail) -> AppResult<()>,
    ) -> AppResult<MeetingDetail> {
        self.ensure(id)?;
        let detail = self.repository.update(id, &mut |d| {
            if d.deleted { return Err(AppError::missing("Meeting")); }
            if d.revision != revision { return Err(AppError::new("revision_conflict", "Il meeting è stato modificato. La tua bozza è conservata: rileggi la versione aggiornata.")); }
            change(d)?;
            d.revision += 1;
            Ok(())
        })?;
        self.project(&detail)?;
        Ok(detail)
    }
    pub fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        self.audio.devices()
    }
    /// The recorder's live state, read without reconciling or saving anything.
    pub fn capture_state(&self) -> AppResult<CaptureState> {
        Ok(self.audio.snapshot()?.state)
    }
    pub fn capture_status(&self) -> AppResult<CaptureSnapshot> {
        let mut snapshot = self.audio.snapshot()?;
        let persisted: AppResult<Option<CaptureSnapshot>> = (|| {
            if let Some(id) = snapshot.meeting_id {
                if let Some(old) = self.repository.read(id)? {
                    // A recovered, stopped capture keeps its durable state even while the
                    // audio worker still exposes the terminal error from this session.
                    if old.capture.state == CaptureState::Saved
                        && matches!(
                            snapshot.state,
                            CaptureState::Saved | CaptureState::Interrupted
                        )
                    {
                        return Ok(Some(old.capture));
                    }
                    if !old.deleted
                        && (snapshot.saved_ms.saturating_sub(old.capture.saved_ms) >= 10_000
                            || old.capture.state != snapshot.state
                            || old.capture.error != snapshot.error
                            || old.capture.incomplete != snapshot.incomplete
                            || old.capture.gaps.len() != snapshot.gaps.len()
                            || old.capture.unverified_chunks != snapshot.unverified_chunks
                            || old.capture.warnings != snapshot.warnings)
                    {
                        let d = self.repository.update(id, &mut |d| {
                            d.capture = snapshot.clone();
                            d.meeting.duration_secs = snapshot.elapsed_ms / 1000;
                            Ok(())
                        })?;
                        self.project(&d)?;
                    }
                } else {
                    return Err(AppError::storage(
                        "Indice della registrazione non disponibile",
                    ));
                }
            }
            Ok(None)
        })();
        match persisted {
            Ok(Some(recovered)) => return Ok(recovered),
            Ok(None) => {}
            Err(error) => {
                let warning = format!(
                    "Aggiornamento dell'indice non riuscito: {}. I blocchi audio già scritti restano recuperabili.",
                    error.message
                );
                snapshot.warnings.push(warning.clone());
                snapshot.error = Some(match snapshot.error {
                    Some(original) => format!("{original} {warning}"),
                    None => warning,
                });
            }
        }
        Ok(snapshot)
    }
    pub fn start(&self, request: StartMeeting) -> AppResult<MeetingDetail> {
        if request.request_id.trim().is_empty() || request.request_id.len() > 200 {
            return Err(AppError::validation("Identificativo di avvio non valido."));
        }
        if request.microphone.is_none() && request.system.is_none() {
            return Err(AppError::validation("Scegli almeno una sorgente audio."));
        }
        let _gate = self
            .capture_gate
            .lock()
            .map_err(|_| AppError::external("Recorder non disponibile"))?;
        if let Some(existing) = self
            .repository
            .list()?
            .into_iter()
            .find(|d| d.request_id == request.request_id && !d.deleted)
        {
            return Ok(existing);
        }
        if matches!(
            self.audio.snapshot()?.state,
            CaptureState::Preparing
                | CaptureState::Recording
                | CaptureState::Paused
                | CaptureState::Saving
        ) {
            return Err(AppError::new(
                "recording_busy",
                "Una registrazione è già in corso.",
            ));
        }
        let title = if request.title.trim().is_empty() {
            format!(
                "Riunione del {}",
                chrono::Local::now().format("%d/%m/%Y %H:%M")
            )
        } else {
            request.title.trim().chars().take(300).collect()
        };
        let meeting = self.context.update(|s| {
            if let Some(event_id) = request.event_id {
                if !s.events.iter().any(|e| e.id == event_id) {
                    return Err(AppError::missing("Evento"));
                }
            }
            let m = Meeting {
                id: s.new_id(),
                title,
                date: self.now(),
                event_id: request.event_id,
                ..Default::default()
            };
            s.meetings.push(m.clone());
            Ok(m)
        })?;
        let id = meeting.id;
        let mut detail = MeetingDetail::legacy(meeting);
        detail.request_id = request.request_id;
        detail.capture.meeting_id = Some(id);
        detail.capture.state = CaptureState::Preparing;
        self.repository.create(&detail)?;
        match self.audio.start(
            CaptureRequest {
                meeting_id: id,
                microphone: request.microphone,
                system: request.system,
            },
            self.repository.folder(id),
        ) {
            Ok(snapshot) => {
                match self.repository.update(id, &mut |d| {
                    d.capture = snapshot.clone();
                    Ok(())
                }) {
                    Ok(saved) => detail = saved,
                    Err(error) => {
                        detail.capture = snapshot;
                        detail.capture.error = Some(format!(
                            "Registrazione attiva, ma l'indice non è stato aggiornato: {}. Usa Termina per fermarla.",
                            error.message
                        ));
                    }
                }
            }
            Err(e) => {
                let interrupted = self.repository.update(id, &mut |d| {
                    d.capture.state = CaptureState::Interrupted;
                    d.capture.error = Some(e.message.clone());
                    Ok(())
                })?;
                self.index_capture(&interrupted);
                return Err(e);
            }
        }
        Ok(detail)
    }
    pub fn pause(&self) -> AppResult<CaptureSnapshot> {
        self.audio.pause()?;
        self.capture_status()
    }
    pub fn resume(&self) -> AppResult<CaptureSnapshot> {
        self.audio.resume()?;
        self.capture_status()
    }
    /// Ends the capture and, when it ended saved, starts the transcription by itself (and
    /// the summary after it, see `advance_pipeline`). Nothing starts when the engine cannot
    /// work yet: `readiness` says why and the person can start it from the meeting later.
    pub fn stop(&self) -> AppResult<CaptureSnapshot> {
        // `stop` on a recorder that is not recording answers with its last, stale snapshot:
        // that must not queue the same meeting again. The look is only for that: when the
        // recorder cannot report, "Termina" still stops it (and just starts nothing by itself).
        let was_live = self
            .audio
            .snapshot()
            .map(|snapshot| {
                matches!(
                    snapshot.state,
                    CaptureState::Preparing
                        | CaptureState::Recording
                        | CaptureState::Paused
                        | CaptureState::Saving
                )
            })
            .unwrap_or(false);
        self.audio.stop()?;
        let snapshot = self.capture_status()?;
        if was_live && snapshot.state == CaptureState::Saved {
            if let Some(id) = snapshot.meeting_id {
                let _ = self.auto_transcribe(id);
            }
        }
        Ok(snapshot)
    }
    pub fn recover_interrupted(&self) -> AppResult<()> {
        // What to carry on once the loop below has released its leases (a job started while
        // the loop holds a meeting's lease would find it taken):
        // (meeting, transcript) where a saved result made that transcript the active one, and
        let mut resumed_transcripts: Vec<(u64, String)> = Vec::new();
        // the transcription and summary jobs this pass interrupted, as they were.
        let mut interrupted_jobs: Vec<(u64, MeetingJob)> = Vec::new();
        for listed in self.repository.list()? {
            self.index_capture(&listed);
            let job_lease = match self.repository.claim_job(listed.meeting.id) {
                Ok(lease) => lease,
                // Another desktop/MCP runtime is still working. Do not mark its
                // live job interrupted or inspect an import while it writes.
                Err(_) => continue,
            };
            let old = match self.repository.read(listed.meeting.id) {
                Ok(Some(detail)) => detail,
                // Unsupported/inaccessible records remain visible as notices in the list.
                _ => continue,
            };
            // This runtime holds the job lease: settling the audio must not claim it again.
            let transcript_before = old.active_transcript.clone();
            let old = self.reconcile_job_results_with(old, true);
            let id = old.meeting.id;
            if old.deleted {
                drop(job_lease);
                let _ = self.repository.remove_audio(id);
                let _ = self.project(&old);
                continue;
            }
            if old.active_transcript != transcript_before {
                if let Some(transcript) = &old.active_transcript {
                    resumed_transcripts.push((id, transcript.clone()));
                }
            }
            let mut flipped: Vec<(u64, MeetingJob)> = Vec::new();
            let persisted = self.repository.capture(id);
            let d = self.repository.update(id, &mut |d| {
                match &persisted {
                    Ok(Some(snapshot)) => d.capture = snapshot.clone(),
                    Ok(None) => {},
                    Err(error) => {
                        d.recovery_notice = Some(format!("Verifica dell'audio non completata: {}. I file sono conservati; riprova Recupera audio.", error.message));
                        d.capture.state = CaptureState::Interrupted;
                        d.capture.incomplete = true;
                    }
                }
                if matches!(
                    d.capture.state,
                    CaptureState::Recording
                        | CaptureState::Paused
                        | CaptureState::Saving
                        | CaptureState::Preparing
                ) {
                    d.capture.state = CaptureState::Interrupted;
                    d.capture.error = Some(
                        "La registrazione è stata interrotta. Puoi recuperare i blocchi salvati."
                            .into(),
                    );
                }
                d.meeting.duration_secs = d
                    .capture
                    .chunks
                    .iter()
                    .map(|c| c.end_ms)
                    .max()
                    .unwrap_or(d.meeting.duration_secs * 1000)
                    / 1000;
                // The edit can run again: start from nothing each time.
                flipped.clear();
                for job in &mut d.jobs {
                    if matches!(job.state.as_str(), "queued" | "running") {
                        job.state = "interrupted".into();
                        job.error = Some("Elaborazione interrotta: riprova per continuare.".into());
                        if matches!(job.kind.as_str(), "transcribe" | "summarize") {
                            flipped.push((id, job.clone()));
                        }
                    }
                }
                Ok(())
            });
            if let Ok(d) = d {
                let _ = self.project(&d);
                interrupted_jobs.append(&mut flipped);
            }
        }
        // Every lease is released: carry on the work that a closed app cut short. Only what
        // this pass interrupted, never an old meeting that simply has no transcript.
        if !interrupted_jobs.is_empty() {
            // Settings that cannot be read count as "not ready": nothing starts.
            if let Ok(settings) = self.context.read().and_then(|store| self.context.settings(&store)) {
                for (id, job) in interrupted_jobs {
                    // A job is judged by the options it was started with, not by today's
                    // defaults: `retry_job` runs it with those.
                    let can_run = if job.kind == "transcribe" {
                        let options = job.options.clone().unwrap_or_default();
                        self.processor.transcription_blocker(&options, &settings).is_none()
                    } else {
                        self.processor.summary_ready(&settings)
                    };
                    if can_run {
                        let _ = self.retry_job(id, &job.id);
                    }
                }
            }
        }
        for (id, transcript) in resumed_transcripts {
            let _ = self.advance_pipeline(id, &transcript);
        }
        Ok(())
    }
    /// Recovers the saved audio blocks of an interrupted recording, then transcribes them
    /// like a recording that was stopped normally.
    pub fn recover(&self, id: u64) -> AppResult<MeetingDetail> {
        let recovered = self.recover_locked(id)?;
        // The gate and the lease are released: a job can claim them now.
        if recovered.capture.state == CaptureState::Saved {
            let _ = self.auto_transcribe(id);
        }
        Ok(self.read(id).unwrap_or(recovered))
    }
    fn recover_locked(&self, id: u64) -> AppResult<MeetingDetail> {
        let _gate = self
            .capture_gate
            .lock()
            .map_err(|_| AppError::external("Recorder non disponibile"))?;
        let snapshot = self.audio.snapshot()?;
        if snapshot.meeting_id == Some(id)
            && matches!(
                snapshot.state,
                CaptureState::Preparing
                    | CaptureState::Recording
                    | CaptureState::Paused
                    | CaptureState::Saving
            )
        {
            return Err(AppError::new(
                "recording_busy",
                "Termina prima la registrazione.",
            ));
        }
        self.ensure(id)?;
        let _job_lease = self.repository.claim_job(id)?;
        let recovered = self.repository.recover_capture(id)?;
        let d = self.repository.update(id, &mut |d| {
            if d.deleted {
                return Err(AppError::missing("Meeting"));
            }
            d.capture = recovered.clone();
            d.meeting.duration_secs = d.capture.elapsed_ms / 1000;
            Ok(())
        })?;
        self.project(&d)?;
        Ok(d)
    }
    pub fn save_notes(&self, id: u64, text: String, revision: u64) -> AppResult<MeetingDetail> {
        if text.len() > 2_000_000 {
            return Err(AppError::validation("Appunti troppo lunghi."));
        }
        self.edit(id, revision, |d| {
            d.meeting.notes = text.clone();
            if let Some(note) = d.notes.iter_mut().find(|n| n.id == "free") {
                d.note_history.push(note.clone());
                note.text = text.clone();
                note.revision += 1;
            } else {
                d.notes.push(MeetingNote {
                    id: "free".into(),
                    text: text.clone(),
                    at_ms: None,
                    revision: 1,
                });
            }
            Ok(())
        })
    }
    pub fn add_note(
        &self,
        id: u64,
        text: String,
        at_ms: Option<u64>,
        revision: u64,
    ) -> AppResult<MeetingDetail> {
        let note_id = self.id("note");
        let capture = self.audio.snapshot()?;
        self.edit(id, revision, |d| {
            let duration = if capture.meeting_id == Some(id) {
                capture.elapsed_ms
            } else {
                d.capture.elapsed_ms
            };
            if at_ms.is_some_and(|t| t > duration) {
                return Err(AppError::validation("Posizione audio non valida."));
            }
            d.notes.push(MeetingNote {
                id: note_id.clone(),
                text: text.clone(),
                at_ms,
                revision: 1,
            });
            Ok(())
        })
    }
    pub fn update(
        &self,
        id: u64,
        title: String,
        event_id: Option<u64>,
        included: bool,
        revision: u64,
    ) -> AppResult<MeetingDetail> {
        if title.trim().is_empty() {
            return Err(AppError::validation("Inserisci un titolo."));
        }
        let events: Vec<u64> = self.context.read_shared()?.events.iter().map(|e| e.id).collect();
        self.edit(id, revision, |d| {
            // Only a link that changes must point at an event that exists: a meeting whose
            // event was deleted or cancelled keeps its (dangling) link and can still be renamed.
            if let Some(event) = event_id {
                if d.meeting.event_id != Some(event) && !events.contains(&event) {
                    return Err(AppError::missing("Evento"));
                }
            }
            d.meeting.title = title.trim().chars().take(300).collect();
            d.meeting.event_id = event_id;
            d.included_in_search = included;
            Ok(())
        })
    }
    pub fn delete(&self, id: u64) -> AppResult<()> {
        let _gate = self
            .capture_gate
            .lock()
            .map_err(|_| AppError::external("Recorder non disponibile"))?;
        let snapshot = self.audio.snapshot()?;
        if snapshot.meeting_id == Some(id)
            && matches!(
                snapshot.state,
                CaptureState::Preparing
                    | CaptureState::Recording
                    | CaptureState::Paused
                    | CaptureState::Saving
            )
        {
            return Err(AppError::new(
                "recording_busy",
                "Termina prima la registrazione.",
            ));
        }
        let controls = self
            .controls
            .lock()
            .map_err(|_| AppError::external("Lavori non disponibili"))?;
        let pending: Vec<_> = controls.iter().filter(|((m, _), _)| *m == id).collect();
        if !pending.is_empty() {
            for (_, stop) in pending {
                stop.store(true, Ordering::Relaxed);
            }
            return Err(AppError::new(
                "job_running",
                "Annullamento in corso. Riprova a eliminare quando il lavoro è terminato.",
            ));
        }
        self.ensure(id)?;
        let job_lease = self.repository.claim_job(id)?;
        let d = self.repository.update(id, &mut |d| {
            d.deleted = true;
            d.revision += 1;
            d.notes.clear();
            d.note_history.clear();
            d.recovery_copies.clear();
            d.legacy_notes = None;
            d.legacy_transcript = None;
            d.transcripts.clear();
            d.summaries.clear();
            d.jobs.clear();
            d.capture.chunks.clear();
            d.meeting.notes.clear();
            d.meeting.transcript.clear();
            d.meeting.summary.clear();
            d.meeting.key_points.clear();
            d.meeting.action_items.clear();
            Ok(())
        })?;
        drop(controls);
        if let Ok(mut pending) = self.pending_results.lock() {
            pending.retain(|(meeting_id, _), _| *meeting_id != id);
        }
        self.project(&d)?;
        // The tombstone now prevents a new worker. Cleanup takes its own lease
        // so direct repository cleanup and startup retries use the same guard.
        drop(job_lease);
        self.repository.remove_audio(id)
    }
    pub fn correct_segment(
        &self,
        id: u64,
        version: &str,
        segment: &str,
        text: String,
        revision: u64,
    ) -> AppResult<MeetingDetail> {
        let new_id = self.id("transcript");
        self.edit(id, revision, |d| {
            let mut transcript = d
                .transcripts
                .iter()
                .find(|t| t.id == version)
                .cloned()
                .ok_or_else(|| AppError::missing("Trascrizione"))?;
            let s = transcript
                .segments
                .iter_mut()
                .find(|s| s.id == segment)
                .ok_or_else(|| AppError::missing("Passaggio"))?;
            s.text = text.clone();
            transcript.id = new_id.clone();
            transcript.created_at = self.now();
            d.meeting.transcript = transcript
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            d.active_transcript = Some(new_id.clone());
            d.transcripts.push(transcript);
            Ok(())
        })
    }
    pub fn toggle_action(
        &self,
        id: u64,
        action_id: &str,
        done: bool,
        revision: u64,
    ) -> AppResult<MeetingDetail> {
        let new_id = self.id("summary");
        self.edit(id, revision, |d| {
            let mut summary = d
                .summary()
                .cloned()
                .ok_or_else(|| AppError::missing("Verbale"))?;
            let action = summary
                .actions
                .iter_mut()
                .find(|a| a.id == action_id)
                .ok_or_else(|| AppError::missing("Attività"))?;
            action.done = done;
            summary.id = new_id.clone();
            summary.created_at = self.now();
            d.active_summary = Some(new_id.clone());
            d.summaries.push(summary);
            Ok(())
        })
    }
    pub fn search(&self, query: &str) -> AppResult<Vec<MeetingHit>> {
        search_details(&self.list()?, query, 100)
    }
    /// Deletes the recorded audio of a meeting and records that it is gone. The caller holds
    /// the job lease. Returns the bytes freed and the meeting as stored afterwards.
    ///
    /// `recording_busy` means a status poll or a recorder holds the capture lock; a poll
    /// lets go within moments, so it is retried briefly before giving up. The files go first:
    /// the other way round would record audio as gone while it is still on disk. If recording
    /// it then fails, the meeting still looks cleanable while the process stays up, and the
    /// next pass finds nothing left to delete and records it. After a crash in that window
    /// that does not hold: the restart resets the capture from the emptied manifest, the
    /// verdict becomes Pending and `audio_removed` is never set. That is benign (there is no
    /// audio, `has_audio` is false), just not recorded. `revision` stays: removing audio is
    /// not a content edit.
    fn drop_audio(&self, id: u64) -> AppResult<(u64, MeetingDetail)> {
        const TRIES: u32 = 5;
        let mut tries = 1;
        let freed = loop {
            match self.repository.discard_audio(id) {
                Ok(freed) => break freed,
                Err(e) if e.code == "recording_busy" && tries < TRIES => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        };
        let detail = self.repository.update(id, &mut |d| {
            if d.deleted {
                return Err(AppError::missing("Meeting"));
            }
            // External audio (`meeting.audio_path`) is not ours: it is never touched.
            d.audio_removed = true;
            d.audio_kept = None;
            // load_capture falls back to session.json when capture.json is gone, so the
            // chunk list must not survive here either. Same "nothing left to report" as the
            // manifest the repository rewrites.
            d.capture.chunks.clear();
            d.capture.missing_chunks.clear();
            d.capture.unverified_chunks.clear();
            d.capture.gaps.clear();
            d.capture.warnings.clear();
            d.capture.incomplete = false;
            d.capture.error = None;
            Ok(())
        })?;
        let _ = self.project(&detail);
        Ok((freed, detail))
    }
    /// Applies the decision on a meeting's recorded audio once a transcript is saved:
    /// removes it, or records why it is kept. Returns the bytes freed.
    ///
    /// Removal needs the job lease. `lease_held` says the caller owns it (the worker that
    /// just finished, or the startup recovery); otherwise it is claimed here and a meeting
    /// another job holds is left alone: the cleanup action finishes it later. It never takes
    /// `job_gate` and never starts a job.
    fn settle_audio(&self, id: u64, lease_held: bool) -> AppResult<u64> {
        let Some(detail) = self.repository.read(id)? else {
            return Ok(0);
        };
        match audio_verdict(&detail) {
            AudioVerdict::Pending => Ok(0),
            AudioVerdict::Keep(reason) => {
                if detail.audio_kept.as_deref() != Some(reason) {
                    self.repository.update(id, &mut |d| {
                        // Decide again on what is stored now, not on what was read.
                        if let AudioVerdict::Keep(reason) = audio_verdict(d) {
                            d.audio_kept = Some(reason.into());
                        }
                        Ok(())
                    })?;
                }
                Ok(0)
            }
            AudioVerdict::Remove => {
                let _lease = if lease_held {
                    None
                } else {
                    match self.repository.claim_job(id) {
                        Ok(lease) => Some(lease),
                        Err(_) => return Ok(0),
                    }
                };
                if !lease_held
                    && !matches!(
                        self.repository.read(id)?,
                        Some(d) if audio_verdict(&d) == AudioVerdict::Remove
                    )
                {
                    // Another runtime changed the meeting between the read and the claim.
                    return Ok(0);
                }
                match self.drop_audio(id) {
                    Ok((freed, _)) => Ok(freed),
                    // A recorder still owns the folder: leave the audio and the meeting as
                    // they are, the cleanup action finishes it.
                    Err(e) if e.code == "recording_busy" => Ok(0),
                    Err(e) => Err(e),
                }
            }
        }
    }
    /// The manual action for audio that was kept (see `MeetingDetail::audio_kept`).
    pub fn discard_audio(&self, id: u64) -> AppResult<MeetingDetail> {
        let wanted = |d: &MeetingDetail| d.audio_kept.is_some() && !d.audio_removed && !d.deleted;
        let no_audio = || AppError::validation("Non c'è audio da eliminare.");
        if !wanted(&self.read(id)?) {
            return Err(no_audio());
        }
        let _lease = self.repository.claim_job(id)?;
        match self.repository.read(id)? {
            Some(d) if wanted(&d) => {}
            _ => return Err(no_audio()),
        }
        self.drop_audio(id).map(|(_, detail)| detail)
    }
    /// What `audio_cleanup` would free: meetings whose transcript made the audio useless.
    pub fn audio_cleanup_preview(&self) -> AppResult<AudioCleanup> {
        let mut total = AudioCleanup::default();
        for detail in self.repository.list()? {
            if audio_verdict(&detail) != AudioVerdict::Remove {
                continue;
            }
            total.meetings += 1;
            total.bytes += self.repository.audio_bytes(detail.meeting.id).unwrap_or(0);
        }
        Ok(total)
    }
    /// Deletes the audio of every meeting that could have lost it already. A meeting a job
    /// holds, or whose recorder is busy, is skipped: the next sweep takes it.
    pub fn audio_cleanup(&self) -> AppResult<AudioCleanup> {
        let mut total = AudioCleanup::default();
        for listed in self.repository.list()? {
            if audio_verdict(&listed) != AudioVerdict::Remove {
                continue;
            }
            let id = listed.meeting.id;
            let Ok(_lease) = self.repository.claim_job(id) else {
                continue;
            };
            if !matches!(
                self.repository.read(id),
                Ok(Some(d)) if audio_verdict(&d) == AudioVerdict::Remove
            ) {
                continue;
            }
            if let Ok((freed, _)) = self.drop_audio(id) {
                total.meetings += 1;
                total.bytes += freed;
            }
        }
        Ok(total)
    }
    pub fn audio_range(&self, id: u64, start_ms: u64, duration_ms: u64) -> AppResult<Vec<u8>> {
        if duration_ms == 0 || duration_ms > 30_000 {
            return Err(AppError::validation(
                "Intervallo audio massimo: 30 secondi.",
            ));
        }
        let d = self.read(id)?;
        if d.capture.chunks.is_empty() {
            return Err(AppError::new(
                "audio_unavailable",
                "Importa o ritrascrivi il vecchio file per ascoltarlo nel player.",
            ));
        }
        if d.capture.requires_receipts {
            let end_ms = start_ms.saturating_add(duration_ms);
            let selected: Vec<_> = d
                .capture
                .chunks
                .iter()
                .filter(|c| c.start_ms < end_ms && c.end_ms > start_ms)
                .cloned()
                .collect();
            self.note_unverified_audio(id, &d.capture, &selected)?;
        }
        self.audio.audio_range(
            &self.repository.folder(id),
            &d.capture.chunks,
            start_ms,
            duration_ms,
        )
    }
    fn note_unverified_audio(
        &self,
        id: u64,
        capture: &CaptureSnapshot,
        chunks: &[AudioChunk],
    ) -> AppResult<()> {
        if !capture.requires_receipts {
            return Ok(());
        }
        let unverified = self.repository.unverified_audio(id, chunks)?;
        if unverified
            .iter()
            .any(|path| !capture.unverified_chunks.contains(path))
        {
            self.repository.update(id, &mut |detail| {
                    if detail.deleted { return Err(AppError::missing("Meeting")); }
                    for path in &unverified {
                        if !detail.capture.unverified_chunks.contains(path) { detail.capture.unverified_chunks.push(path.clone()); }
                    }
                    detail.capture.incomplete = true;
                    let notice = "Some audio blocks have no integrity receipt; audio is kept but its integrity cannot be verified";
                    if !detail.capture.warnings.iter().any(|w| w == notice) { detail.capture.warnings.push(notice.into()); }
                    detail.capture.error = Some(detail.capture.warnings.join("; "));
                    Ok(())
                })?;
        }
        Ok(())
    }
    pub fn export(&self, id: u64, format: &str) -> AppResult<String> {
        self.processor.export(&self.read(id)?, format)
    }
    pub fn export_selected(
        &self,
        id: u64,
        format: &str,
        transcript_version: Option<&str>,
        summary_version: Option<&str>,
        sections: Option<&[String]>,
    ) -> AppResult<String> {
        let mut detail = self.read(id)?;
        if let Some(version) = transcript_version.filter(|s| !s.is_empty()) {
            let selected = detail
                .transcripts
                .iter()
                .find(|t| t.id == version)
                .ok_or_else(|| AppError::missing("Versione trascrizione"))?;
            detail.meeting.transcript = selected
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            detail.active_transcript = Some(version.into());
        }
        if let Some(version) = summary_version.filter(|s| !s.is_empty()) {
            let selected = detail
                .summaries
                .iter()
                .find(|s| s.id == version)
                .ok_or_else(|| AppError::missing("Versione sintesi"))?;
            detail.meeting.summary = selected.summary.clone();
            detail.meeting.key_points = selected.key_points.clone();
            detail.meeting.action_items = selected.actions.iter().map(|a| a.text.clone()).collect();
            detail.active_summary = Some(version.into());
        }
        if let Some(sections) = sections {
            if sections.is_empty()
                || sections
                    .iter()
                    .any(|s| !matches!(s.as_str(), "summary" | "notes" | "transcript"))
            {
                return Err(AppError::validation(
                    "Scegli almeno un contenuto da esportare.",
                ));
            }
            // References stay verifiable even when their full section is omitted.
            if !sections.iter().any(|s| s == "summary") {
                detail.active_summary = None;
                detail.summaries.clear();
                detail.meeting.summary.clear();
                detail.meeting.key_points.clear();
                detail.meeting.action_items.clear();
            }
            if !sections.iter().any(|s| s == "transcript") {
                detail.active_transcript = None;
                detail.meeting.transcript.clear();
            }
            if !sections.iter().any(|s| s == "notes") {
                detail
                    .note_history
                    .extend(std::mem::take(&mut detail.notes));
                detail.meeting.notes.clear();
            }
        }
        let references: Vec<_> = detail
            .summary()
            .map(|s| {
                s.decisions
                    .iter()
                    .flat_map(|d| d.sources.clone())
                    .chain(s.actions.iter().flat_map(|a| a.sources.clone()))
                    .collect()
            })
            .unwrap_or_default();
        detail.transcripts.retain(|t| {
            detail.active_transcript.as_ref() == Some(&t.id)
                || references
                    .iter()
                    .any(|r| r.kind == "segment" && r.version == t.id)
        });
        // A cited version outside the exported transcript only needs its cited passages.
        for t in &mut detail.transcripts {
            if detail.active_transcript.as_ref() != Some(&t.id) {
                t.segments.retain(|s| {
                    references
                        .iter()
                        .any(|r| r.kind == "segment" && r.version == t.id && r.id == s.id)
                });
            }
        }
        detail
            .summaries
            .retain(|s| detail.active_summary.as_ref() == Some(&s.id));
        detail.note_history.retain(|n| {
            references
                .iter()
                .any(|r| r.kind == "note" && r.id == n.id && r.version == n.revision.to_string())
        });
        if !references.iter().any(|r| r.kind == "legacy_transcript") {
            detail.legacy_transcript = None;
        }
        if !references.iter().any(|r| r.kind == "legacy_note") {
            detail.legacy_notes = None;
        }
        detail.jobs.clear();
        detail.recovery_copies.clear();
        detail.capture.chunks.clear();
        detail.meeting.audio_path = None;
        self.processor.export(&detail, format)
    }
    pub fn models(&self) -> AppResult<Vec<SpeechModel>> {
        self.processor.models()
    }
    /// The app is closing: stops every job. A job that stops because
    /// of this is not recorded as cancelled (that would be final): it stays queued or running
    /// on disk, and the next launch's `recover_interrupted` finds it and starts it again.
    pub fn cancel_all_jobs(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Ok(controls) = self.controls.lock() {
            for stop in controls.values() {
                stop.store(true, Ordering::Relaxed);
            }
        }
    }
    pub fn transcribe(&self, id: u64, mut options: TranscribeOptions) -> AppResult<MeetingDetail> {
        // Callers (an assistant, an older window) may still use the names `local` and `remote`.
        options.provider = canonical_engine(&options.provider).to_owned();
        if !matches!(options.provider.as_str(), ENGINE_COMPUTE | ENGINE_EXTERNAL) {
            return Err(AppError::validation("Provider non valido."));
        }
        self.start_job(id, "transcribe", Some(options), None, None, self.source_profile.clone())
    }
    pub fn summarize(&self, id: u64, language: String) -> AppResult<MeetingDetail> {
        self.start_job(id, "summarize", None, Some(language), None, self.source_profile.clone())
    }
    /// What the engine chosen in Settings can do right now.
    pub fn readiness(&self) -> AppResult<MeetingReadiness> {
        let settings = self.context.settings(&self.context.read()?)?;
        Ok(MeetingReadiness {
            transcription_blocker: self
                .processor
                .transcription_blocker(&settings.meeting_options(), &settings),
            summary_ready: self.processor.summary_ready(&settings),
        })
    }
    /// `transcribe` with the engine, model and language chosen in Settings. While the engine
    /// cannot work it says why, instead of queueing a job that is bound to fail.
    pub fn transcribe_default(&self, id: u64) -> AppResult<MeetingDetail> {
        let settings = self.context.settings(&self.context.read()?)?;
        let options = settings.meeting_options();
        if let Some(blocker) = self.processor.transcription_blocker(&options, &settings) {
            return Err(AppError::validation(blocker));
        }
        self.transcribe(id, options)
    }
    /// `summarize` in the meeting language of Settings, or Italian when the summary does not
    /// support that one (the transcription language also allows `auto` and any other code).
    pub fn summarize_default(&self, id: u64) -> AppResult<MeetingDetail> {
        let settings = self.context.settings(&self.context.read()?)?;
        self.summarize(id, summary_language(&settings.meeting_language).into())
    }
    /// Starts the transcription with the engine chosen in Settings, if that engine can work
    /// and nothing tried to transcribe this meeting before. A blocked engine is not an
    /// error: the meeting simply waits, see `readiness`. A second stop, or a recovery of a
    /// meeting that already has its text, must not queue it again; the person's own
    /// "Trascrivi" (`transcribe_default`) is not held back by this.
    fn auto_transcribe(&self, id: u64) -> AppResult<()> {
        let detail = self.read(id)?;
        if detail.transcript().is_some()
            || detail.has_text
            || detail.jobs.iter().any(|j| j.kind == "transcribe")
        {
            return Ok(());
        }
        let settings = self.context.settings(&self.context.read()?)?;
        let options = settings.meeting_options();
        if self.processor.transcription_blocker(&options, &settings).is_some() {
            // No server (or no model there) for the server's engine: the recording is saved and
            // flagged, and `resume_waiting` transcribes it when there is one. The external
            // engine, with the person's own key, is not flagged: it waits for the person.
            if canonical_engine(&options.provider) == ENGINE_COMPUTE {
                self.repository.update(id, &mut |d| {
                    d.transcription_waiting = true;
                    Ok(())
                })?;
            }
            return Ok(());
        }
        self.start_job(id, "transcribe", Some(options), None, None, self.source_profile.clone())?;
        Ok(())
    }
    /// Starts the transcription of every flagged recording (`transcription_waiting`) that has its
    /// audio and no text, once the engine can transcribe; returns how many it started. The old
    /// recordings left without a transcript on purpose carry no flag and never start by themselves.
    pub fn resume_waiting(&self) -> AppResult<usize> {
        let mut started = 0;
        for listed in self.repository.list()? {
            if !listed.transcription_waiting || listed.deleted || !listed.has_audio || listed.has_text {
                continue;
            }
            let settings = self.context.settings(&self.context.read()?)?;
            let options = settings.meeting_options();
            if self.processor.transcription_blocker(&options, &settings).is_some() {
                // The same engine answers for every recording: nothing else can start now.
                break;
            }
            let id = listed.meeting.id;
            // Busy (another job, another runtime holding the lease): left flagged for the next pass.
            if self.start_job(id, "transcribe", Some(options), None, None, self.source_profile.clone()).is_ok() {
                started += 1;
            }
        }
        Ok(started)
    }
    /// After the transcript of job `finished_job` is saved, queues its summary: when that
    /// transcript is the active one, complete and with words, no summary was written from
    /// it, the AI is configured, and no summary job was tried since the transcript's own job
    /// (a failed one is not retried by itself: the person can ask again). A job that ended
    /// without saving a transcript (failed, cancelled) does not summarize an older one.
    ///
    /// Only for desktop work. A transcription started through MCP, or by an MCP-scoped
    /// service, stays what the client asked for: sending its text to the AI provider is a
    /// separate tool (`summarize_meeting`) with its own grant.
    ///
    /// It starts a job, so it must run with no job lease and no `job_gate` held and after
    /// the previous worker left `controls`; otherwise `start_job` answers `job_running`.
    fn advance_pipeline(&self, id: u64, finished_job: &str) -> AppResult<()> {
        let detail = self.read(id)?;
        let Some(transcript) = detail
            .transcript()
            .filter(|t| t.id == finished_job)
            .filter(|t| t.complete && t.segments.iter().any(|s| !s.text.trim().is_empty()))
        else {
            return Ok(());
        };
        if detail
            .summary()
            .is_some_and(|s| s.transcript_version.as_deref() == Some(transcript.id.as_str()))
        {
            return Ok(());
        }
        let origin = detail.jobs.iter().position(|j| j.id == transcript.id);
        if self.source_profile.is_some()
            || origin.is_some_and(|i| detail.jobs[i].source_profile.is_some())
        {
            return Ok(());
        }
        let later = &detail.jobs[origin.map_or(0, |i| i + 1)..];
        if later.iter().any(|j| j.kind == "summarize") {
            return Ok(());
        }
        let settings = self.context.settings(&self.context.read()?)?;
        if !self.processor.summary_ready(&settings) {
            return Ok(());
        }
        let language = summary_language(&settings.meeting_language);
        self.start_job(id, "summarize", None, Some(language.into()), None, None)?;
        Ok(())
    }
    pub fn import(&self, source: PathBuf) -> AppResult<MeetingDetail> {
        let title = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Audio importato")
            .chars()
            .take(300)
            .collect();
        let meeting = self.context.update(|s| {
            let m = Meeting {
                id: s.new_id(),
                title,
                date: self.now(),
                ..Default::default()
            };
            s.meetings.push(m.clone());
            Ok(m)
        })?;
        let id = meeting.id;
        let mut detail = MeetingDetail::legacy(meeting);
        detail.capture.meeting_id = Some(id);
        self.repository.create(&detail)?;
        self.start_job(id, "import", None, None, Some(source), self.source_profile.clone())
    }
    fn start_job(
        &self,
        id: u64,
        kind: &str,
        options: Option<TranscribeOptions>,
        language: Option<String>,
        source: Option<PathBuf>,
        source_profile: Option<String>,
    ) -> AppResult<MeetingDetail> {
        let _gate = self
            .capture_gate
            .lock()
            .map_err(|_| AppError::external("Recorder non disponibile"))?;
        self.ensure(id)?;
        self.capture_status()?;
        let job_lease = self.repository.claim_job(id)?;
        let input = self.read(id)?;
        if matches!(
            input.capture.state,
            CaptureState::Recording
                | CaptureState::Paused
                | CaptureState::Preparing
                | CaptureState::Saving
        ) {
            return Err(AppError::new(
                "recording_busy",
                "Termina la registrazione prima di elaborare l'audio.",
            ));
        }
        if kind == "transcribe" && input.audio_removed {
            return Err(AppError::new(
                "audio_removed",
                "L'audio di questa riunione è stato eliminato dopo la trascrizione.",
            ));
        }
        if kind == "summarize"
            && input.transcript().is_none()
            && input.meeting.transcript.trim().is_empty()
            && input.meeting.notes.trim().is_empty()
            && input.notes.is_empty()
        {
            return Err(AppError::validation(
                "Aggiungi una trascrizione o degli appunti.",
            ));
        }
        let settings = self.context.settings(&self.context.read()?)?;
        if kind == "summarize" || kind == "transcribe" {
            self.processor.authorize_mcp_ai(source_profile.as_deref(), settings.mcp_access)?;
        }
        let job_id = self.id(kind);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut controls = self
            .controls
            .lock()
            .map_err(|_| AppError::external("Lavori non disponibili"))?;
        if controls.keys().any(|(m, _)| *m == id) {
            return Err(AppError::new(
                "job_running",
                "Un'elaborazione è già in corso per questo meeting.",
            ));
        }
        let job = MeetingJob {
            id: job_id.clone(),
            kind: kind.into(),
            state: "queued".into(),
            input_revision: input.revision,
            progress: 0,
            error: None,
            created_at: self.now(),
            options: options.clone(),
            language: language.clone(),
            source_profile: source_profile.clone(),
        };
        let result = self.repository.update(id, &mut |d| {
            if d.deleted {
                return Err(AppError::missing("Meeting"));
            }
            // Holding the OS lease proves these jobs no longer have a worker.
            // A dead process must not leave a permanent 'running' barrier.
            for old in &mut d.jobs {
                if matches!(old.state.as_str(), "queued" | "running") {
                    old.state = "interrupted".into();
                    old.error = Some("Elaborazione interrotta: riprova per continuare.".into());
                }
            }
            d.jobs.push(job.clone());
            if job.kind == "transcribe" {
                d.transcription_waiting = false;
            }
            Ok(())
        })?;
        controls.insert((id, job_id.clone()), cancel.clone());
        drop(controls);
        let this = self.clone();
        let kind = kind.to_string();
        let worker_lease = job_lease.clone();
        let spawn_failure = MeetingJobCheckpoint {
            schema_version: 1,
            meeting_id: id,
            job_id: job_id.clone(),
            input_revision: input.revision,
            created_at: self.now(),
            transcript_version: input.active_transcript.clone(),
            cancelled: false,
            output: None,
            error: Some("Impossibile avviare l'elaborazione. Puoi riprovare.".into()),
        };
        let spawned = std::thread::Builder::new()
            .name(format!("meeting-{kind}"))
            .spawn(move || {
                let progress = |p: u32| {
                    let result = this.repository.update(id, &mut |d| {
                        if d.deleted {
                            return Err(AppError::missing("Meeting"));
                        }
                        if let Some(j) = d.jobs.iter_mut().find(|j| j.id == job_id) {
                            if j.state == "cancelled" {
                                cancel.store(true, Ordering::Relaxed);
                            } else {
                                j.state = "running".into();
                                j.progress = p.min(99);
                            }
                        }
                        Ok(())
                    });
                    if result.is_err() {
                        cancel.store(true, Ordering::Relaxed);
                    }
                };
                progress(0);
                let folder = this.repository.folder(id);
                let authorize = || {
                    if source_profile.is_none() { return Ok(()); }
                    let maximum = this.context.read()?.settings.mcp_access;
                    this.processor.authorize_mcp_ai(source_profile.as_deref(), maximum)
                };
                let cancellation_watch = job_control::CancellationWatch::start(
                    worker_lease.clone(),
                    job_id.clone(),
                    cancel.clone(),
                );
                let outcome: AppResult<MeetingJobOutput> = match &cancellation_watch {
                    Err(error) => Err(error.clone()),
                    Ok(_) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if kind == "import" {
                            return this
                                .processor
                                .import_audio(
                                    source.as_deref().ok_or_else(|| {
                                        AppError::validation("File non disponibile")
                                    })?,
                                    &folder,
                                    &cancel,
                                    &progress,
                                )
                                .map(MeetingJobOutput::Audio);
                        }
                        if kind == "summarize" {
                            return this
                                .processor
                                .summarize_authorized(
                                    &input,
                                    &settings,
                                    language.as_deref().unwrap_or("it"),
                                    &cancel,
                                    &progress,
                                    &authorize,
                                )
                                .map(MeetingJobOutput::Summary);
                        }
                        let mut chunks = input.capture.chunks.clone();
                        if chunks.is_empty() {
                            let source = input.meeting.audio_path.as_ref().ok_or_else(|| {
                                AppError::new("audio_unavailable", "Audio non disponibile.")
                            })?;
                            chunks = this.processor.import_audio(
                                &PathBuf::from(source),
                                &folder,
                                &cancel,
                                &|_| {},
                            )?;
                            this.repository.update(id, &mut |d| {
                                d.capture.chunks = chunks.clone();
                                d.capture.elapsed_ms =
                                    chunks.iter().map(|c| c.end_ms).max().unwrap_or(0);
                                d.capture.saved_ms = d.capture.elapsed_ms;
                                d.capture.state = CaptureState::Saved;
                                Ok(())
                            })?;
                        }
                        this.note_unverified_audio(id, &input.capture, &chunks)?;
                        this.processor
                            .transcribe_authorized(
                                &folder,
                                &chunks,
                                &options.clone().unwrap_or_default(),
                                &settings,
                                &cancel,
                                &progress,
                                &authorize,
                            )
                            .map(MeetingJobOutput::Transcript)
                    }))
                    .unwrap_or_else(|_| {
                        Err(AppError::new(
                            "worker",
                            "L'elaborazione si è interrotta. Il risultato precedente è conservato.",
                        ))
                    }),
                };
                let (output, error) = match outcome {
                    Ok(output) => (Some(output), None),
                    Err(error) => (None, Some(error.message)),
                };
                // Stopped by the app closing: nothing is committed, not even a read of the
                // meeting follows, so the job stays as the process leaves it. A person's own
                // cancel is already `cancelled` on disk, and a job that ended its work before
                // the close is committed as usual.
                let cancelled = cancel.load(Ordering::Relaxed);
                let abandoned = cancelled && this.shutting_down.load(Ordering::SeqCst);
                if !abandoned {
                    this.finish_job(MeetingJobCheckpoint {
                        schema_version: 1,
                        meeting_id: id,
                        job_id: job_id.clone(),
                        input_revision: input.revision,
                        created_at: this.now(),
                        transcript_version: input.active_transcript.clone(),
                        cancelled,
                        output,
                        error,
                    });
                }
                drop(cancellation_watch);
                drop(worker_lease);
                if let Ok(mut c) = this.controls.lock() {
                    c.remove(&(id, job_id.clone()));
                }
                // The lease and the `controls` entry are gone: the next job can start now.
                if kind == "transcribe" && !abandoned {
                    let _ = this.advance_pipeline(id, &job_id);
                }
            });
        if spawned.is_err() {
            self.finish_job(spawn_failure.clone());
            if let Ok(mut controls) = self.controls.lock() {
                controls.remove(&(id, spawn_failure.job_id));
            }
            return Err(AppError::new(
                "worker",
                "Impossibile avviare l'elaborazione. Puoi riprovare.",
            ));
        }
        Ok(result)
    }
    pub fn cancel_job(&self, id: u64, job_id: &str) -> AppResult<MeetingDetail> {
        if let Some(cancel) = self
            .controls
            .lock()
            .map_err(|_| AppError::external("Lavori non disponibili"))?
            .get(&(id, job_id.into()))
        {
            cancel.store(true, Ordering::Relaxed);
        }
        self.repository.request_job_cancel(id, job_id)?;
        self.repository.update(id, &mut |d| {
            let j = d
                .jobs
                .iter_mut()
                .find(|j| j.id == job_id)
                .ok_or_else(|| AppError::missing("Lavoro"))?;
            if matches!(j.state.as_str(), "queued" | "running") {
                j.state = "cancelled".into();
            }
            Ok(())
        })
    }
    pub fn retry_job(&self, id: u64, job_id: &str) -> AppResult<MeetingDetail> {
        let d = self.read(id)?;
        let job = d
            .jobs
            .iter()
            .find(|j| j.id == job_id)
            .ok_or_else(|| AppError::missing("Lavoro"))?;
        if !matches!(job.state.as_str(), "failed" | "interrupted" | "cancelled") {
            return Err(AppError::validation(
                "Questo lavoro non richiede un nuovo tentativo.",
            ));
        }
        if job.kind == "import" {
            return Err(AppError::validation(
                "Scegli nuovamente il file con Importa audio.",
            ));
        }
        self.start_job(
            id,
            &job.kind,
            job.options.clone(),
            job.language.clone(),
            None,
            job.source_profile.clone().or_else(|| self.source_profile.clone()),
        )
    }
}

/// The summary is written in one of these; any other transcription language (`auto`, `pt`)
/// gets an Italian summary.
fn summary_language(language: &str) -> &str {
    match language {
        "it" | "en" | "de" | "es" | "fr" => language,
        _ => "it",
    }
}

pub fn search_details(
    details: &[MeetingDetail],
    query: &str,
    limit: usize,
) -> AppResult<Vec<MeetingHit>> {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let mut hits = vec![];
    for d in details
        .iter()
        .filter(|d| !d.deleted && d.included_in_search)
    {
        let mut add = |kind: &str, text: &str, at_ms, version: Option<String>, segment_id| {
            let lower = text.to_lowercase();
            if hits.len() < limit && terms.iter().all(|t| lower.contains(t)) {
                let found = lower.find(&terms[0]).unwrap_or(0);
                let offset = lower[..found].chars().count().saturating_sub(100);
                let excerpt: String = text.chars().skip(offset).take(450).collect();
                hits.push(MeetingHit {
                    meeting_id: d.meeting.id,
                    title: d.meeting.title.clone(),
                    kind: kind.into(),
                    text: excerpt,
                    at_ms,
                    version,
                    segment_id,
                });
            }
        };
        add("title", &d.meeting.title, None, None, None);
        if let Some(t) = d.transcript() {
            for s in &t.segments {
                add(
                    "transcript",
                    &s.text,
                    Some(s.start_ms),
                    Some(t.id.clone()),
                    Some(s.id.clone()),
                );
            }
        } else {
            add("transcript", &d.meeting.transcript, None, None, None);
        }
        add(
            "notes",
            &d.meeting.notes,
            None,
            d.notes
                .iter()
                .find(|n| n.id == "free")
                .map(|n| n.revision.to_string()),
            Some("free".into()),
        );
        for n in d.notes.iter().filter(|n| n.id != "free") {
            add(
                "note",
                &n.text,
                n.at_ms,
                Some(n.revision.to_string()),
                Some(n.id.clone()),
            );
        }
        add(
            "summary",
            &d.meeting.summary,
            None,
            d.active_summary.clone(),
            None,
        );
    }
    Ok(hits)
}
