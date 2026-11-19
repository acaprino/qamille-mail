//! Filesystem tests use only owned temporary fixtures, never the user's workspace.
use qamille::application::{AppError, meeting_ports::MeetingRepository};
use qamille::domain::{meeting::MeetingDetail, model::Meeting};
use qamille::infrastructure::meeting_repository::{FileMeetingRepository, WorkspaceUiLock};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    repository: FileMeetingRepository,
}
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "qamille-meeting-storage-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            repository: FileMeetingRepository::new(root.join("meetings")),
            root,
        }
    }
    fn detail(id: u64) -> MeetingDetail {
        MeetingDetail::legacy(Meeting {
            id,
            title: format!("Meeting {id}"),
            notes: "Original note".into(),
            ..Default::default()
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn durable_update_is_visible_after_reopening() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fixture
        .repository
        .update(7, &mut |meeting| {
            meeting.meeting.notes = "Updated note — è salvata".into();
            meeting.revision += 1;
            Ok(())
        })
        .unwrap();
    let reopened = FileMeetingRepository::new(fixture.root.join("meetings"));
    let saved = reopened.read(7).unwrap().unwrap();
    assert_eq!(saved.meeting.notes, "Updated note — è salvata");
    assert_eq!(saved.revision, 1);
    assert!(!reopened.folder(7).join("session.json.tmp").exists());
    let legacy: Meeting =
        serde_json::from_slice(&fs::read(reopened.folder(7).join("legacy.json")).unwrap()).unwrap();
    assert_eq!(legacy.notes, "Original note");
}

#[test]
fn failed_mutation_rolls_back_without_rewriting_the_file() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let path = fixture.repository.folder(7).join("session.json");
    let before = fs::read(&path).unwrap();
    let error = fixture
        .repository
        .update(7, &mut |meeting| {
            meeting.meeting.notes = "Must not be saved".into();
            meeting.revision = 123;
            Err(AppError::new(
                "revision_conflict",
                "Changed in another view",
            ))
        })
        .unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn corrupt_session_recovers_backup_and_preserves_original_bytes() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let path = fixture.repository.folder(7).join("session.json");
    let broken = b"{\"schema_version\":2, interrupted write";
    fs::write(&path, broken).unwrap();
    let recovered = fixture.repository.read(7).unwrap().unwrap();
    assert_eq!(recovered.meeting.notes, "Original note");
    assert!(recovered.recovery_notice.is_some());
    assert!(fixture.repository.create(&Fixture::detail(7)).is_err());
    assert_eq!(fixture.repository.list().unwrap().len(), 1);
    assert_eq!(fs::read(&path).unwrap(), broken);
    fixture.repository.update(7, &mut |_| Ok(())).unwrap();
    assert!(
        fs::read_dir(fixture.repository.folder(7))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("session.damaged-")
                && fs::read(entry.path()).unwrap() == broken)
    );
    assert!(serde_json::from_slice::<MeetingDetail>(&fs::read(path).unwrap()).is_ok());
}

#[test]
fn missing_primary_uses_last_confirmed_backup() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fs::remove_file(fixture.repository.folder(7).join("session.json")).unwrap();
    let recovered = fixture.repository.read(7).unwrap().unwrap();
    assert_eq!(recovered.meeting.notes, "Original note");
    assert!(recovered.recovery_notice.is_some());
}

#[test]
fn unsupported_record_does_not_hide_other_meetings_or_get_overwritten() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fixture.repository.create(&Fixture::detail(8)).unwrap();
    let path = fixture.repository.folder(7).join("session.json");
    fs::write(&path, br#"{"schema_version":99,"meeting":{"id":7}}"#).unwrap();
    assert_eq!(
        fixture.repository.read(7).unwrap_err().code,
        "unsupported_meeting_schema"
    );
    let all = fixture.repository.list().unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter()
            .find(|d| d.meeting.id == 7)
            .unwrap()
            .recovery_notice
            .is_some()
    );
    assert_eq!(
        all.iter()
            .find(|d| d.meeting.id == 8)
            .unwrap()
            .meeting
            .notes,
        "Original note"
    );
    assert!(fixture.repository.update(7, &mut |_| Ok(())).is_err());
}

#[test]
fn deletion_marker_prevents_resurrection_from_older_backups() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fixture
        .repository
        .update(7, &mut |d| {
            d.deleted = true;
            Ok(())
        })
        .unwrap();
    fs::write(fixture.repository.folder(7).join("session.json"), b"broken").unwrap();
    assert!(fixture.repository.read(7).unwrap().unwrap().deleted);
    fixture.repository.remove_audio(7).unwrap();
    assert!(fixture.repository.folder(7).join(".deleted").is_file());
    assert!(
        !fixture
            .repository
            .folder(7)
            .join("session.json.prev")
            .exists()
    );
    assert!(fixture.repository.read(7).unwrap().unwrap().deleted);
}

#[test]
fn cleanup_refuses_an_active_capture_lock() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fixture
        .repository
        .update(7, &mut |d| {
            d.deleted = true;
            Ok(())
        })
        .unwrap();
    let folder = fixture.repository.folder(7);
    fs::write(folder.join("audio.wav"), b"must survive").unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(folder.join(".capture.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(
        fixture.repository.remove_audio(7).unwrap_err().code,
        "recording_busy"
    );
    assert_eq!(fs::read(folder.join("audio.wav")).unwrap(), b"must survive");
}

#[test]
fn failed_backup_write_cannot_replace_the_primary_record() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let folder = fixture.repository.folder(7);
    let before = fs::read(folder.join("session.json")).unwrap();
    fs::create_dir(folder.join("session.json.prev.writing")).unwrap();
    assert!(
        fixture
            .repository
            .update(7, &mut |d| {
                d.meeting.notes = "must not commit".into();
                Ok(())
            })
            .is_err()
    );
    assert_eq!(fs::read(folder.join("session.json")).unwrap(), before);
}

#[test]
fn raw_audio_survives_loss_of_all_session_metadata() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let folder = fixture.repository.folder(7);
    for name in ["session.json", "session.json.prev", "legacy.json"] {
        fs::write(folder.join(name), b"damaged").unwrap();
    }
    let audio = folder.join("microphone-00000000-0000000000000000.wav");
    let mut writer = hound::WavWriter::create(
        &audio,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..800 {
        writer.write_sample(123_i16).unwrap();
    }
    writer.finalize().unwrap();
    let before = fs::read(&audio).unwrap();
    let recovered = fixture.repository.read(7).unwrap().unwrap();
    assert!(recovered.recovery_notice.is_some());
    assert_eq!(recovered.capture.chunks.len(), 1);
    assert_eq!(recovered.capture.saved_ms, 100);
    assert_eq!(fs::read(audio).unwrap(), before);
    fixture.repository.update(7, &mut |_| Ok(())).unwrap();
    assert_eq!(
        fixture
            .repository
            .read(7)
            .unwrap()
            .unwrap()
            .capture
            .chunks
            .len(),
        1
    );
}

#[test]
fn duplicate_id_never_replaces_existing_meeting() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let mut replacement = Fixture::detail(7);
    replacement.meeting.title = "Unexpected replacement".into();
    assert!(fixture.repository.create(&replacement).is_err());
    assert_eq!(
        fixture.repository.read(7).unwrap().unwrap().meeting.title,
        "Meeting 7"
    );
    assert_eq!(fixture.repository.list().unwrap().len(), 1);
}

#[test]
fn identity_and_schema_cannot_change_inside_transaction() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    fixture.repository.create(&Fixture::detail(8)).unwrap();
    assert!(
        fixture
            .repository
            .update(7, &mut |meeting| {
                meeting.meeting.id = 8;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(
        fixture.repository.read(8).unwrap().unwrap().meeting.title,
        "Meeting 8"
    );
    assert!(
        fixture
            .repository
            .update(7, &mut |meeting| {
                meeting.schema_version = 99;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(
        fixture.repository.read(7).unwrap().unwrap().schema_version,
        2
    );
    let mut invalid = Fixture::detail(9);
    invalid.schema_version = 99;
    assert!(fixture.repository.create(&invalid).is_err());
    assert!(fixture.repository.read(9).unwrap().is_none());
}

#[test]
fn tombstone_cleanup_removes_owned_artifacts_but_preserves_external_original() {
    let fixture = Fixture::new();
    let external = fixture.root.join("user-original.wav");
    fs::write(&external, b"original outside managed meeting directory").unwrap();
    let mut detail = Fixture::detail(7);
    detail.meeting.audio_path = Some(external.to_string_lossy().into_owned());
    fixture.repository.create(&detail).unwrap();
    let owned = fixture.repository.folder(7);
    fs::write(owned.join("captured.wav"), b"owned audio").unwrap();
    fs::create_dir_all(owned.join("jobs")).unwrap();
    fs::write(owned.join("jobs").join("transcript.json"), b"{}").unwrap();
    assert!(fixture.repository.remove_audio(7).is_err());
    assert!(owned.join("captured.wav").exists());
    fixture
        .repository
        .update(7, &mut |meeting| {
            meeting.deleted = true;
            Ok(())
        })
        .unwrap();
    fixture.repository.remove_audio(7).unwrap();
    fixture.repository.remove_audio(7).unwrap();
    assert!(!owned.join("captured.wav").exists());
    assert!(!owned.join("jobs").exists());
    assert!(fixture.repository.read(7).unwrap().unwrap().deleted);
    assert_eq!(
        fs::read(external).unwrap(),
        b"original outside managed meeting directory"
    );
}

#[test]
fn independent_repository_instances_serialize_updates() {
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let root = Arc::new(fixture.root.join("meetings"));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let root = root.clone();
            std::thread::spawn(move || {
                let repository = FileMeetingRepository::new(root.as_ref().clone());
                for _ in 0..10 {
                    repository
                        .update(7, &mut |meeting| {
                            meeting.revision += 1;
                            Ok(())
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(fixture.repository.read(7).unwrap().unwrap().revision, 40);
}

#[test]
fn ui_workspace_lease_excludes_other_ui_until_released() {
    let fixture = Fixture::new();
    let first = WorkspaceUiLock::acquire(&fixture.root).unwrap();
    let second = WorkspaceUiLock::acquire(&fixture.root);
    assert!(second.is_err());
    assert_eq!(second.err().unwrap().code, "workspace_busy");
    drop(first);
    assert!(WorkspaceUiLock::acquire(&fixture.root).is_ok());
}

#[test]
fn recovered_capture_remains_saved_after_two_repository_reopens() {
    use qamille::domain::meeting::{AudioChunk, CaptureSnapshot, CaptureState};
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let folder = fixture.repository.folder(7);
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
        meeting_id: Some(7),
        state: CaptureState::Interrupted,
        elapsed_ms: 130,
        saved_ms: 100,
        chunks: vec![AudioChunk {
            source: "microphone".into(),
            path: "confirmed.wav".into(),
            sample_rate: 8000,
            channels: 1,
            start_ms: 0,
            end_ms: 100,
            sequence: 0,
        }],
        ..Default::default()
    };
    fs::write(
        folder.join("capture.json"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    let recovered = fixture.repository.recover_capture(7).unwrap();
    assert_eq!(recovered.state, CaptureState::Saved);
    assert_eq!(recovered.elapsed_ms, 100);
    for _ in 0..2 {
        let reopened = FileMeetingRepository::new(fixture.root.join("meetings"));
        let capture = reopened.capture(7).unwrap().unwrap();
        assert_eq!(capture.state, CaptureState::Saved);
        assert_eq!(capture.saved_ms, 100);
        assert_eq!(capture.elapsed_ms, 100);
        assert_eq!(capture.chunks.len(), 1);
    }
}

#[test]
fn missing_audio_cannot_be_marked_saved_by_recovery() {
    use qamille::domain::meeting::{AudioChunk, CaptureSnapshot, CaptureState};
    let fixture = Fixture::new();
    fixture.repository.create(&Fixture::detail(7)).unwrap();
    let snapshot = CaptureSnapshot {
        meeting_id: Some(7),
        state: CaptureState::Interrupted,
        chunks: vec![AudioChunk {
            source: "microphone".into(),
            path: "absent.wav".into(),
            sample_rate: 8000,
            channels: 1,
            end_ms: 100,
            ..Default::default()
        }],
        ..Default::default()
    };
    fs::write(
        fixture.repository.folder(7).join("capture.json"),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    assert!(fixture.repository.recover_capture(7).is_err());
    assert_eq!(
        fixture.repository.capture(7).unwrap().unwrap().state,
        CaptureState::Interrupted
    );
}
