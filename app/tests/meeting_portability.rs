use qamille::{
    application::meeting_ports::MeetingRepository,
    domain::meeting::{AudioChunk, CaptureSnapshot, MeetingDetail},
    infrastructure::meeting_repository::FileMeetingRepository,
    model::Meeting,
};
use std::{fs, path::{Path, PathBuf}};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../checks/meeting-portability");
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join(format!(
            "qamille-meeting-portability-{}-{stamp}", std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn data(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../checks/meeting-portability").canonicalize().unwrap();
        assert!(root.starts_with(base));
        assert!(root.file_name().unwrap().to_string_lossy().starts_with("qamille-meeting-portability-"));
        fs::remove_dir_all(root).unwrap();
    }
}

fn legacy(path: &Path) -> MeetingDetail {
    MeetingDetail::legacy(Meeting {
        id: 41,
        title: "Legacy meeting".into(),
        audio_path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    })
}

#[test]
fn legacy_audio_in_session_and_recovery_survives_moving_the_workspace() {
    let workspace = Workspace::new();
    let original = workspace.data("data-a");
    let moved = workspace.data("data-b");
    let audio = original.join("recordings/legacy.wav");
    fs::create_dir_all(audio.parent().unwrap()).unwrap();
    fs::write(&audio, b"legacy audio").unwrap();
    let repository = FileMeetingRepository::new(original.join("meetings"));
    repository.create(&legacy(&audio)).unwrap();

    let session: serde_json::Value = serde_json::from_slice(
        &fs::read(original.join("meetings/41/session.json")).unwrap()
    ).unwrap();
    let backup: serde_json::Value = serde_json::from_slice(
        &fs::read(original.join("meetings/41/legacy.json")).unwrap()
    ).unwrap();
    assert_eq!(session["meeting"]["audio_path"], "workspace:/recordings/legacy.wav");
    assert_eq!(backup["audio_path"], "workspace:/recordings/legacy.wav");

    fs::rename(&original, &moved).unwrap();
    let reopened = FileMeetingRepository::new(moved.join("meetings"));
    let loaded = reopened.read(41).unwrap().unwrap();
    let loaded_audio = PathBuf::from(loaded.meeting.audio_path.unwrap());
    assert_eq!(fs::read(&loaded_audio).unwrap(), b"legacy audio");
    assert_eq!(loaded_audio.canonicalize().unwrap(), moved.join("recordings/legacy.wav").canonicalize().unwrap());

    fs::remove_file(moved.join("meetings/41/session.json")).unwrap();
    fs::remove_file(moved.join("meetings/41/session.json.prev")).unwrap();
    let recovered = reopened.read(41).unwrap().unwrap();
    assert_eq!(fs::read(recovered.meeting.audio_path.unwrap()).unwrap(), b"legacy audio");
}

#[test]
fn external_legacy_audio_is_preserved_without_rebasing_or_copying() {
    let workspace = Workspace::new();
    let original = workspace.data("data-a");
    let moved = workspace.data("data-b");
    let external = workspace.0.join("user-audio.wav");
    fs::write(&external, b"external user audio").unwrap();
    let detail = legacy(&external);
    let repository = FileMeetingRepository::new(original.join("meetings"));
    repository.create(&detail).unwrap();
    fs::rename(&original, &moved).unwrap();

    let loaded = FileMeetingRepository::new(moved.join("meetings")).read(41).unwrap().unwrap();
    assert_eq!(loaded.meeting.audio_path, detail.meeting.audio_path);
    assert_eq!(fs::read(external).unwrap(), b"external user audio");
}

#[test]
fn recorded_meetings_keep_relative_chunk_paths_after_moving() {
    let workspace = Workspace::new();
    let original = workspace.data("data-a");
    let moved = workspace.data("data-b");
    let recording = original.join("meetings/42/microphone.wav");
    let mut detail = MeetingDetail::legacy(Meeting { id: 42, ..Default::default() });
    detail.capture = CaptureSnapshot {
        meeting_id: Some(42),
        chunks: vec![AudioChunk { path: "microphone.wav".into(), ..Default::default() }],
        ..Default::default()
    };
    FileMeetingRepository::new(original.join("meetings")).create(&detail).unwrap();
    fs::write(&recording, b"recorded audio").unwrap();
    fs::rename(&original, &moved).unwrap();

    let loaded = FileMeetingRepository::new(moved.join("meetings")).read(42).unwrap().unwrap();
    assert!(loaded.meeting.audio_path.is_none());
    assert_eq!(loaded.capture.chunks[0].path, "microphone.wav");
    assert_eq!(fs::read(moved.join("meetings/42").join(&loaded.capture.chunks[0].path)).unwrap(), b"recorded audio");
}
