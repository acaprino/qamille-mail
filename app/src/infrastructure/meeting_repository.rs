//! Per-meeting records with process locks, durable replacement and persistent tombstones.
use crate::{
    application::{
        AppError, AppResult,
        meeting_ports::{MeetingJobLease, MeetingRepository},
    },
    domain::meeting::*,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub struct FileMeetingRepository {
    root: PathBuf,
}

struct FileJobLease {
    _file: File,
    cancellation: PathBuf,
}
impl MeetingJobLease for FileJobLease {
    fn cancellation_requested(&self, job_id: &str) -> AppResult<bool> {
        match fs::read(&self.cancellation) {
            Ok(bytes) => Ok(bytes == job_id.as_bytes()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

pub fn merged_details(
    root: PathBuf,
    legacy: &[crate::model::Meeting],
) -> AppResult<Vec<MeetingDetail>> {
    let repository = FileMeetingRepository::new(root);
    let mut records: std::collections::HashMap<_, _> = legacy
        .iter()
        .cloned()
        .map(|m| (m.id, MeetingDetail::legacy(m)))
        .collect();
    for detail in repository.list()? {
        records.insert(detail.meeting.id, detail);
    }
    let mut details: Vec<_> = records.into_values().filter(|d| !d.deleted).collect();
    details.sort_by_key(|d| std::cmp::Reverse((d.meeting.date, d.meeting.id)));
    Ok(details)
}
impl FileMeetingRepository {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn map_audio_paths(
        &self,
        detail: &mut MeetingDetail,
        transform: fn(&str, &Path) -> AppResult<String>,
    ) -> AppResult<()> {
        let workspace = self.root.parent().unwrap_or(&self.root);
        for meeting in std::iter::once(&mut detail.meeting).chain(&mut detail.recovery_copies) {
            if let Some(path) = &mut meeting.audio_path {
                *path = transform(path, workspace)?;
            }
        }
        Ok(())
    }
    fn resolve_audio_paths(&self, mut detail: MeetingDetail) -> AppResult<MeetingDetail> {
        self.map_audio_paths(&mut detail, super::portable_files::resolve_path)?;
        Ok(detail)
    }
    fn lock(&self, id: u64) -> AppResult<File> {
        fs::create_dir_all(&self.root)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(format!("{id}.lock")))?;
        lock.lock()?;
        Ok(lock)
    }
    fn read_unlocked(&self, id: u64) -> AppResult<Option<MeetingDetail>> {
        let folder = self.folder(id);
        // This independent, synced marker wins over every older backup.
        if folder.join(".deleted").exists() {
            return Ok(Some(tombstone(id, 0)));
        }
        let primary = fs::read(folder.join("session.json"));
        if let Ok(bytes) = &primary {
            match decode_record(bytes, id) {
                Ok(detail) => return Ok(Some(self.resolve_audio_paths(detail)?)),
                Err(error) if error.code == "unsupported_meeting_schema" => return Err(error),
                Err(_) => {}
            }
        }
        for name in ["session.json.prev", "session.json.tmp"] {
            if let Ok(bytes) = fs::read(folder.join(name)) {
                if let Ok(mut detail) = decode_record(&bytes, id) {
                    detail.recovery_notice = Some(format!(
                        "Metadati recuperati dalla copia {name}. L'ultima modifica potrebbe essere incompleta; l'audio viene verificato separatamente."
                    ));
                    detail.reconcile_projection = true;
                    return Ok(Some(self.resolve_audio_paths(detail)?));
                }
            }
        }
        if !folder.exists() {
            return Ok(None);
        }
        let has_archive = primary.is_ok()
            || fs::read_dir(&folder)?.filter_map(Result::ok).any(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("capture.json")
                    || name == "legacy.json"
                    || name.ends_with(".wav")
                    || name.ends_with(".wav.tmp")
            });
        if !has_archive {
            return match primary {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(None),
            };
        }
        let meeting = fs::read(folder.join("legacy.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<crate::model::Meeting>(&bytes).ok())
            .filter(|meeting| meeting.id == id)
            .unwrap_or_else(|| crate::model::Meeting {
                id,
                title: format!("Registrazione recuperata #{id}"),
                date: crate::model::now_ts(),
                ..Default::default()
            });
        let mut recovered = MeetingDetail::legacy(meeting);
        recovered.request_id = format!("recovered-{id}");
        recovered.reconcile_projection = true;
        recovered.included_in_search = false;
        recovered.recovery_notice = Some("L'indice del meeting è danneggiato o mancante. I file originali sono conservati; la registrazione è ricostruita dai blocchi disponibili. Verifica titolo, appunti e versioni.".into());
        recovered.capture = match super::meeting_audio::load_capture(&folder) {
            Ok(Some(capture)) => capture,
            Ok(None) => CaptureSnapshot {
                meeting_id: Some(id),
                state: CaptureState::Interrupted,
                incomplete: true,
                error: Some(
                    "Nessun blocco audio confermato trovato; i file originali restano conservati."
                        .into(),
                ),
                ..Default::default()
            },
            Err(error) => CaptureSnapshot {
                meeting_id: Some(id),
                state: CaptureState::Interrupted,
                incomplete: true,
                error: Some(format!("Verifica audio non riuscita: {}", error.message)),
                ..Default::default()
            },
        };
        recovered.meeting.duration_secs = recovered.capture.saved_ms / 1000;
        Ok(Some(self.resolve_audio_paths(recovered)?))
    }
    fn write(&self, detail: &MeetingDetail) -> AppResult<()> {
        let folder = self.folder(detail.meeting.id);
        fs::create_dir_all(&folder)?;
        let sanitized;
        let detail = if detail.deleted {
            persist_deleted(&folder, detail.meeting.id)?;
            sanitized = tombstone(detail.meeting.id, detail.revision);
            &sanitized
        } else {
            detail
        };
        let mut persisted = detail.clone();
        persisted.refresh_flags();
        self.map_audio_paths(&mut persisted, super::portable_files::persistent_path)?;
        let current = folder.join("session.json");
        if let Ok(bytes) = fs::read(&current) {
            if decode_record(&bytes, detail.meeting.id).is_ok() {
                replace_synced(&folder, "session.json.prev", &bytes)?;
            } else {
                preserve_damaged(&folder, &bytes)?;
            }
        }
        crash_checkpoint(&folder, "after_backup");
        let tmp = folder.join("session.json.tmp");
        let mut file = File::create(&tmp)?;
        let bytes =
            serde_json::to_vec_pretty(&persisted).map_err(|e| AppError::storage(e.to_string()))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        crash_checkpoint(&folder, "after_stage_sync");
        fs::rename(tmp, folder.join("session.json"))?;
        crash_checkpoint(&folder, "after_publish");
        // Also protect the first record, before any later update exists.
        if !folder.join("session.json.prev").exists() && !detail.deleted {
            replace_synced(&folder, "session.json.prev", &bytes)?;
        }
        Ok(())
    }
}

#[cfg(not(test))]
fn crash_checkpoint(_: &Path, _: &str) {}
#[cfg(test)]
fn crash_checkpoint(folder: &Path, point: &str) {
    if std::env::var("QAMILLE_SESSION_CRASH_POINT")
        .ok()
        .as_deref()
        == Some(point)
        && std::env::var_os("QAMILLE_SESSION_CRASH_FOLDER")
            .is_some_and(|path| PathBuf::from(path) == folder)
    {
        fs::write(folder.join("test-crash-ready"), point).unwrap();
        loop {
            std::thread::park_timeout(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    #[test]
    #[ignore = "subprocess helper, invoked only by the process-kill tests"]
    fn session_crash_writer_process() {
        let folder = PathBuf::from(std::env::var_os("QAMILLE_SESSION_CRASH_FOLDER").unwrap());
        let repository = FileMeetingRepository::new(folder.parent().unwrap().to_path_buf());
        if std::env::var("QAMILLE_SESSION_CRASH_POINT").unwrap() == "job_lease_held" {
            let _lease = repository.claim_job(7).unwrap();
            panic!("The parent should kill this helper while it holds the lease");
        }
        if std::env::var("QAMILLE_SESSION_CRASH_POINT")
            .unwrap()
            .starts_with("job_result_")
        {
            repository
                .save_job_result(&MeetingJobCheckpoint {
                    schema_version: 1,
                    meeting_id: 7,
                    job_id: "ready".into(),
                    input_revision: 0,
                    created_at: 1,
                    transcript_version: None,
                    cancelled: false,
                    error: None,
                    output: Some(MeetingJobOutput::Summary(SummaryVersion {
                        summary: "Completed before crash".into(),
                        ..Default::default()
                    })),
                })
                .unwrap();
            panic!("The parent should kill this helper after syncing the result");
        }
        repository
            .update(7, &mut |detail| {
                detail.meeting.notes = "After confirmed update".into();
                detail.revision += 1;
                Ok(())
            })
            .unwrap();
        panic!("The parent should kill this helper at its checkpoint");
    }
    #[test]
    fn meeting_job_lease_is_released_after_process_kill() {
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "job-lease-crash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repository = FileMeetingRepository::new(root.clone());
        repository
            .create(&MeetingDetail::legacy(crate::model::Meeting {
                id: 7,
                ..Default::default()
            }))
            .unwrap();
        let folder = repository.folder(7);
        let helper = format!(
            "{}::session_crash_writer_process",
            module_path!().split_once("::").unwrap().1
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &helper, "--ignored", "--nocapture"])
            .env("QAMILLE_SESSION_CRASH_POINT", "job_lease_held")
            .env("QAMILLE_SESSION_CRASH_FOLDER", &folder)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !folder.join("test-crash-ready").exists() {
            if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Child failed to acquire job lease");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(repository.claim_job(7).err().unwrap().code, "job_running");
        child.kill().unwrap();
        child.wait().unwrap();
        let lease = repository.claim_job(7).unwrap();
        drop(lease);
        assert!(
            FileMeetingRepository::new(root.clone())
                .claim_job(7)
                .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn meeting_metadata_survives_process_kill() {
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "session-crash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for point in ["after_backup", "after_stage_sync", "after_publish"] {
            let repository = FileMeetingRepository::new(root.join(point));
            repository
                .create(&MeetingDetail::legacy(crate::model::Meeting {
                    id: 7,
                    notes: "Before update".into(),
                    ..Default::default()
                }))
                .unwrap();
            let folder = repository.folder(7);
            let helper = format!(
                "{}::session_crash_writer_process",
                module_path!().split_once("::").unwrap().1
            );
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &helper, "--ignored", "--nocapture"])
                .env("QAMILLE_SESSION_CRASH_POINT", point)
                .env("QAMILLE_SESSION_CRASH_FOLDER", &folder)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !folder.join("test-crash-ready").exists() {
                if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("Child failed to reach {point}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            child.kill().unwrap();
            assert!(!child.wait().unwrap().success());
            let reopened = FileMeetingRepository::new(root.join(point));
            let detail = reopened.read(7).unwrap().unwrap();
            assert_eq!(
                detail.meeting.notes,
                if point == "after_publish" {
                    "After confirmed update"
                } else {
                    "Before update"
                }
            );
            assert!(folder.join("session.json.prev").is_file());
            assert_eq!(
                reopened.read(7).unwrap().unwrap().meeting.notes,
                detail.meeting.notes
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn meeting_worker_result_survives_process_kill_before_and_after_publication() {
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "job-crash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for point in ["job_result_staged", "job_result_published"] {
            let repository = FileMeetingRepository::new(root.join(point));
            repository
                .create(&MeetingDetail::legacy(crate::model::Meeting {
                    id: 7,
                    ..Default::default()
                }))
                .unwrap();
            let folder = repository.folder(7);
            // The module path differs only for the isolated harness, so use the
            // test's exact fully qualified module path rather than a fixed prefix.
            let helper = format!(
                "{}::session_crash_writer_process",
                module_path!().split_once("::").unwrap().1
            );
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &helper, "--ignored", "--nocapture"])
                .env("QAMILLE_SESSION_CRASH_POINT", point)
                .env("QAMILLE_SESSION_CRASH_FOLDER", &folder)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !folder.join("test-crash-ready").exists() {
                if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("Child failed to reach {point}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            child.kill().unwrap();
            assert!(!child.wait().unwrap().success());
            let reopened = FileMeetingRepository::new(root.join(point));
            let (results, warnings) = reopened.job_results(7).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(results.len(), 1);
            assert!(
                matches!(&results[0].output, Some(MeetingJobOutput::Summary(summary)) if summary.summary == "Completed before crash")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}
impl MeetingRepository for FileMeetingRepository {
    fn unverified_audio(&self, id: u64, chunks: &[AudioChunk]) -> AppResult<Vec<String>> {
        let folder = self.folder(id);
        let mut unverified = vec![];
        for chunk in chunks {
            super::meeting_audio::chunk_path(&folder, &chunk.path)?;
            if !folder
                .join(format!("{}.sha256.json", chunk.path))
                .try_exists()?
            {
                unverified.push(chunk.path.clone());
            }
        }
        Ok(unverified)
    }
    fn claim_job(&self, id: u64) -> AppResult<std::sync::Arc<dyn MeetingJobLease>> {
        fs::create_dir_all(&self.root)?;
        // Keep the lock outside the removable meeting folder. Never unlink a
        // lock name: doing so could let two processes lock different file objects.
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join(format!("{id}.job.lock")))?;
        file.try_lock().map_err(|_| {
            AppError::new(
                "job_running",
                "Un'elaborazione è già in corso per questo meeting.",
            )
        })?;
        crash_checkpoint(&self.folder(id), "job_lease_held");
        Ok(std::sync::Arc::new(FileJobLease {
            _file: file,
            cancellation: self.root.join(format!("{id}.job.cancel")),
        }))
    }
    fn request_job_cancel(&self, id: u64, job_id: &str) -> AppResult<()> {
        let _guard = self.lock(id)?;
        let detail = self
            .read_unlocked(id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        if detail.deleted {
            return Err(AppError::missing("Meeting"));
        }
        let job = detail
            .jobs
            .iter()
            .find(|j| j.id == job_id)
            .ok_or_else(|| AppError::missing("Lavoro"))?;
        if matches!(job.state.as_str(), "queued" | "running") {
            replace_synced(&self.root, &format!("{id}.job.cancel"), job_id.as_bytes())?;
        }
        Ok(())
    }
    fn save_job_result(&self, checkpoint: &MeetingJobCheckpoint) -> AppResult<()> {
        let _guard = self.lock(checkpoint.meeting_id)?;
        let detail = self
            .read_unlocked(checkpoint.meeting_id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        if detail.deleted {
            return Err(AppError::missing("Meeting"));
        }
        let folder = self.folder(checkpoint.meeting_id).join("job-results");
        fs::create_dir_all(&folder)?;
        let payload =
            serde_json::to_string(checkpoint).map_err(|e| AppError::storage(e.to_string()))?;
        let envelope = serde_json::to_vec(&JobEnvelope {
            sha256: digest(payload.as_bytes()),
            payload,
        })
        .map_err(|e| AppError::storage(e.to_string()))?;
        let name = job_result_name(&checkpoint.job_id);
        let staging = folder.join(format!("{name}.writing"));
        let mut file = File::create(&staging)?;
        file.write_all(&envelope)?;
        file.sync_all()?;
        drop(file);
        crash_checkpoint(&self.folder(checkpoint.meeting_id), "job_result_staged");
        fs::rename(staging, folder.join(name))?;
        crash_checkpoint(&self.folder(checkpoint.meeting_id), "job_result_published");
        Ok(())
    }
    fn job_results(&self, id: u64) -> AppResult<(Vec<MeetingJobCheckpoint>, Vec<String>)> {
        let _guard = self.lock(id)?;
        let folder = self.folder(id).join("job-results");
        if !folder.exists() || self.folder(id).join(".deleted").exists() {
            return Ok((vec![], vec![]));
        }
        let mut results = vec![];
        let mut warnings = vec![];
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            // A fully synced staging file is also recoverable if publication was interrupted.
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") && !name.ends_with(".json.writing") {
                continue;
            }
            let parsed = (|| -> AppResult<MeetingJobCheckpoint> {
                let envelope: JobEnvelope = serde_json::from_slice(&fs::read(entry.path())?)
                    .map_err(|_| AppError::storage("Risultato AI incompleto"))?;
                if digest(envelope.payload.as_bytes()) != envelope.sha256 {
                    return Err(AppError::storage("Checksum del risultato AI non valido"));
                }
                let checkpoint: MeetingJobCheckpoint = serde_json::from_str(&envelope.payload)
                    .map_err(|_| AppError::storage("Formato del risultato AI non valido"))?;
                let expected = job_result_name(&checkpoint.job_id);
                if checkpoint.schema_version != 1
                    || checkpoint.meeting_id != id
                    || (name != expected && name != format!("{expected}.writing"))
                {
                    return Err(AppError::storage("Identità del risultato AI non valida"));
                }
                Ok(checkpoint)
            })();
            match parsed {
                Ok(checkpoint) => results.push(checkpoint),
                Err(_) => warnings.push(format!("Risultato AI {name} danneggiato: il file è conservato; puoi riprovare l'elaborazione.")),
            }
        }
        results.sort_by(|a, b| (a.created_at, &a.job_id).cmp(&(b.created_at, &b.job_id)));
        Ok((results, warnings))
    }
    fn clear_job_result(&self, id: u64, job_id: &str) -> AppResult<()> {
        let _guard = self.lock(id)?;
        // Retire the journal only after both session copies contain its terminal
        // state. Otherwise a damaged primary could restore a still-running job
        // from the previous copy after the only result checkpoint was removed.
        let bytes = fs::read(self.folder(id).join("session.json"))?;
        let detail = decode_record(&bytes, id)?;
        if !detail.deleted
            && !detail.jobs.iter().any(|job| {
                job.id == job_id
                    && matches!(job.state.as_str(), "completed" | "failed" | "cancelled")
            })
        {
            return Err(AppError::storage(
                "Risultato AI non ancora confermato nell'indice",
            ));
        }
        if !detail.deleted {
            replace_synced(&self.folder(id), "session.json.prev", &bytes)?;
        }
        let folder = self.folder(id).join("job-results");
        for name in [
            job_result_name(job_id),
            format!("{}.writing", job_result_name(job_id)),
        ] {
            match fs::remove_file(folder.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    fn list(&self) -> AppResult<Vec<MeetingDetail>> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let mut result = vec![];
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<u64>().ok())
            else {
                continue;
            };
            match self.read(id) {
                Ok(Some(detail)) => result.push(detail),
                Ok(None) => {}
                Err(error) => {
                    // One inaccessible or newer record must not hide all other audio.
                    let mut notice = MeetingDetail::legacy(crate::model::Meeting {
                        id,
                        title: format!("Meeting #{id} da verificare"),
                        ..Default::default()
                    });
                    notice.included_in_search = false;
                    notice.recovery_notice = Some(error.message.clone());
                    notice.capture.state = CaptureState::Interrupted;
                    notice.capture.error = Some(error.message);
                    result.push(notice);
                }
            }
        }
        Ok(result)
    }
    fn read(&self, id: u64) -> AppResult<Option<MeetingDetail>> {
        let _guard = self.lock(id)?;
        Ok(self.read_unlocked(id)?.map(|mut detail| {
            detail.refresh_flags();
            detail
        }))
    }
    fn create(&self, detail: &MeetingDetail) -> AppResult<()> {
        if detail.schema_version != 2 {
            return Err(AppError::validation("Formato meeting non supportato."));
        }
        let _guard = self.lock(detail.meeting.id)?;
        if self.read_unlocked(detail.meeting.id)?.is_some() {
            return Err(AppError::new("conflict", "Questo meeting esiste già."));
        }
        if detail.request_id.is_empty() && detail.capture.meeting_id.is_none() {
            // Keep the first legacy record before its compatibility projection changes.
            let folder = self.folder(detail.meeting.id);
            fs::create_dir_all(&folder)?;
            let backup = folder.join("legacy.json");
            if !backup.exists() {
                let mut persisted = detail.clone();
                self.map_audio_paths(&mut persisted, super::portable_files::persistent_path)?;
                let staging = folder.join("legacy.json.tmp");
                let mut file = File::create(&staging)?;
                file.write_all(
                    &serde_json::to_vec_pretty(&persisted.meeting)
                        .map_err(|e| AppError::storage(e.to_string()))?,
                )?;
                file.sync_all()?;
                drop(file);
                fs::rename(staging, backup)?;
            }
        }
        self.write(detail)
    }
    fn update(
        &self,
        id: u64,
        edit: &mut dyn FnMut(&mut MeetingDetail) -> AppResult<()>,
    ) -> AppResult<MeetingDetail> {
        let _guard = self.lock(id)?;
        let mut detail = self
            .read_unlocked(id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        edit(&mut detail)?;
        if detail.meeting.id != id || detail.schema_version != 2 {
            return Err(AppError::validation(
                "Identità o formato meeting non modificabile.",
            ));
        }
        detail.refresh_flags();
        self.write(&detail)?;
        Ok(detail)
    }
    fn folder(&self, id: u64) -> PathBuf {
        self.root.join(id.to_string())
    }
    fn capture(&self, id: u64) -> AppResult<Option<CaptureSnapshot>> {
        super::meeting_audio::load_capture(&self.folder(id))
    }
    fn recover_capture(&self, id: u64) -> AppResult<CaptureSnapshot> {
        let _guard = self.lock(id)?;
        let detail = self
            .read_unlocked(id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        if detail.deleted {
            return Err(AppError::missing("Meeting"));
        }
        super::meeting_audio::recover_capture(&self.folder(id), id)
    }
    fn remove_audio(&self, id: u64) -> AppResult<()> {
        let _job_lease = self.claim_job(id)?;
        let _guard = self.lock(id)?;
        let detail = self
            .read_unlocked(id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        if !detail.deleted {
            return Err(AppError::validation(
                "Confermare prima la rimozione del meeting.",
            ));
        }
        let folder = self.folder(id);
        let capture_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(folder.join(".capture.lock"))?;
        capture_lock.try_lock().map_err(|_| {
            AppError::new(
                "recording_busy",
                "La registrazione è ancora attiva; l'audio non è stato rimosso.",
            )
        })?;
        persist_deleted(&folder, id)?;
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "session.json" || name == ".deleted" || name == ".capture.lock" {
                continue;
            }
            // Do not follow junctions/symlinks when removing this application's own archive.
            let kind = entry.file_type()?;
            if kind.is_file() || kind.is_symlink() {
                fs::remove_file(entry.path())?;
            } else if kind.is_dir() {
                remove_owned_tree(&entry.path())?;
            }
        }
        match fs::remove_file(self.root.join(format!("{id}.job.cancel"))) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
    fn discard_audio(&self, id: u64) -> AppResult<u64> {
        // No claim_job: the lease is an exclusive OS lock and the caller already holds it.
        let _guard = self.lock(id)?;
        let detail = self
            .read_unlocked(id)?
            .ok_or_else(|| AppError::missing("Meeting"))?;
        if detail.deleted {
            return Err(AppError::missing("Meeting"));
        }
        super::meeting_audio::discard_audio(&self.folder(id))
    }
    fn audio_bytes(&self, id: u64) -> AppResult<u64> {
        super::meeting_audio::audio_bytes(&self.folder(id))
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JobEnvelope {
    sha256: String,
    payload: String,
}
fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
fn job_result_name(job_id: &str) -> String {
    format!("{}.json", digest(job_id.as_bytes()))
}

fn decode_record(bytes: &[u8], id: u64) -> AppResult<MeetingDetail> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| AppError::storage("Indice meeting danneggiato."))?;
    if value["schema_version"]
        .as_u64()
        .is_some_and(|version| version != 2)
    {
        return Err(AppError::new(
            "unsupported_meeting_schema",
            "Formato meeting non supportato: il file originale non viene modificato.",
        ));
    }
    let detail: MeetingDetail = serde_json::from_value(value)
        .map_err(|_| AppError::storage("Indice meeting incompleto."))?;
    if detail.schema_version != 2 || detail.meeting.id != id {
        return Err(AppError::storage("Identificativo meeting incoerente."));
    }
    Ok(detail)
}

fn tombstone(id: u64, revision: u64) -> MeetingDetail {
    let mut detail = MeetingDetail::legacy(crate::model::Meeting {
        id,
        ..Default::default()
    });
    detail.deleted = true;
    detail.revision = revision;
    detail.included_in_search = false;
    detail
}

fn persist_deleted(folder: &Path, id: u64) -> AppResult<()> {
    if !folder.join(".deleted").exists() {
        replace_synced(folder, ".deleted", id.to_string().as_bytes())?;
    }
    Ok(())
}

fn replace_synced(folder: &Path, name: &str, bytes: &[u8]) -> AppResult<()> {
    let temporary = folder.join(format!("{name}.writing"));
    let mut file = File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, folder.join(name))?;
    Ok(())
}

fn preserve_damaged(folder: &Path, bytes: &[u8]) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let name = format!("session.damaged-{:x}.json", Sha256::digest(bytes));
    let path = folder.join(name);
    if !path.exists() {
        let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

pub(crate) fn remove_owned_tree(path: &Path) -> AppResult<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && !entry.file_type()?.is_symlink() {
            remove_owned_tree(&entry.path())?;
        } else {
            remove_file_or_link(&entry.path())?;
        }
    }
    fs::remove_dir(path)?;
    Ok(())
}

/// Removes a file, or a link itself without following it. A Windows junction or directory
/// symlink is refused by `remove_file` and needs `remove_dir`, which unlinks it and leaves
/// its target alone. The original error is reported when both fail.
pub(crate) fn remove_file_or_link(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(first) => fs::remove_dir(path).map_err(|_| first),
    }
}

/// Prevent the two UI processes from keeping independent writable store snapshots.
pub struct WorkspaceUiLock {
    _file: File,
}
impl WorkspaceUiLock {
    pub fn acquire(data: &Path) -> AppResult<Self> {
        fs::create_dir_all(data)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data.join("ui.lock"))?;
        file.try_lock().map_err(|_| {
            AppError::new(
                "workspace_busy",
                "Questa cartella è già aperta in un'altra interfaccia Qamille.",
            )
        })?;
        Ok(Self { _file: file })
    }
}
