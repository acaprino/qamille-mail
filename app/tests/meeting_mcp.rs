//! One isolated test process exercises the public MCP adapter with synthetic data only.
//! Keep this file to one test: it selects QAMILLE_DATA before invoking application code.
use serde_json::{Value, json};
use qamille::{
    application::meeting_ports::MeetingRepository,
    domain::{
        meeting::{MeetingDetail, TranscriptSegment, TranscriptVersion},
        model::{Access, Meeting, Store},
    },
    infrastructure::meeting_repository::FileMeetingRepository,
    mcp,
};
use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    previous_data_dir: Option<OsString>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-data"));
        let root = base.join(format!("meeting-mcp-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let previous_data_dir = std::env::var_os("QAMILLE_DATA");
        // SAFETY: this integration-test binary has exactly one test and starts no
        // workers. Set the environment before any application code reads it.
        unsafe { std::env::set_var("QAMILLE_DATA", &root) };
        Self {
            root,
            previous_data_dir,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: the same single-test/no-workers invariant applies during cleanup.
        unsafe {
            match &self.previous_data_dir {
                Some(previous) => std::env::set_var("QAMILLE_DATA", previous),
                None => std::env::remove_var("QAMILLE_DATA"),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn meeting(id: u64, title: &str, marker: &str) -> Meeting {
    Meeting {
        id,
        title: title.into(),
        date: 1_700_000_000 + id as i64,
        duration_secs: 120,
        transcript: format!("{marker} transcript"),
        notes: format!("{marker} notes"),
        summary: format!("{marker} summary"),
        key_points: vec![format!("{marker} key point")],
        action_items: vec![format!("{marker} action")],
        ..Default::default()
    }
}

fn transcript(id: &str, count: usize, marker: &str) -> TranscriptVersion {
    TranscriptVersion {
        id: id.into(),
        created_at: 1_700_000_000,
        model: "synthetic-test-model".into(),
        language: "it".into(),
        complete: true,
        segments: (0..count)
            .map(|index| TranscriptSegment {
                id: format!("{id}-{index:03}"),
                start_ms: index as u64 * 1_000,
                end_ms: (index as u64 + 1) * 1_000,
                text: format!("{marker} passage {index:03}"),
                original_text: format!("{marker} original {index:03}"),
                source: "import".into(),
                confidence: Some(0.95),
            })
            .collect(),
    }
}

fn read(store: &mut Store, name: &str, arguments: Value) -> String {
    let (text, changed) = mcp::call(store, name, &arguments).unwrap();
    assert!(
        !changed,
        "read-only meeting tool {name} reported a mutation"
    );
    text
}

fn read_json(store: &mut Store, name: &str, arguments: Value) -> Value {
    serde_json::from_str(&read(store, name, arguments)).unwrap()
}

#[test]
fn meeting_tools_read_canonical_versions_exclude_tombstones_and_bound_pages() {
    let fixture = Fixture::new();
    assert_eq!(Store::data_dir(), fixture.root);
    let repository = FileMeetingRepository::new(fixture.root.join("meetings"));
    let mut store = Store::default();
    store.settings.mcp_access = Access::ReadOnly;
    // A stale in-memory legacy projection must never override its canonical record.
    store.meetings = vec![
        meeting(71, "Obsolete projection", "quartz-obsolete"),
        meeting(72, "Deleted legacy projection", "quartz-deleted"),
        meeting(73, "Excluded legacy projection", "quartz-hidden-legacy"),
        meeting(74, "Legacy-only meeting", "quartz-legacy-only"),
    ];
    let mut canonical =
        MeetingDetail::legacy(meeting(71, "Canonical V2 meeting", "quartz-current"));
    canonical.transcripts = vec![
        transcript("old-version", 3, "quartz-history"),
        transcript("active-version", 205, "quartz-current"),
    ];
    canonical.active_transcript = Some("active-version".into());
    canonical.notes.push(qamille::domain::meeting::MeetingNote { id: "marker".into(), text: "Appunto temporizzato".into(), at_ms: Some(1200), revision: 1 });
    canonical.transcripts[1].segments[103]
        .text
        .push_str(" quartz-pointer");
    repository.create(&canonical).unwrap();

    let mut deleted = MeetingDetail::legacy(meeting(72, "Deleted V2 meeting", "quartz-deleted"));
    deleted.deleted = true;
    deleted.transcripts = vec![transcript("deleted-version", 1, "quartz-deleted")];
    deleted.active_transcript = Some("deleted-version".into());
    repository.create(&deleted).unwrap();

    let mut excluded = MeetingDetail::legacy(meeting(73, "Excluded V2 meeting", "quartz-hidden"));
    excluded.included_in_search = false;
    repository.create(&excluded).unwrap();
    repository
        .create(&MeetingDetail::legacy(meeting(
            75,
            "V2-only meeting",
            "quartz-v2-only",
        )))
        .unwrap();
    let original_store = serde_json::to_value(&store).unwrap();
    let original_records: Vec<_> = [71, 72, 73, 75]
        .map(|id| {
            let path = repository.folder(id).join("session.json");
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .into();

    let list = read(&mut store, "list_meetings", json!({}));
    for title in [
        "Canonical V2 meeting",
        "Excluded V2 meeting",
        "Legacy-only meeting",
        "V2-only meeting",
    ] {
        assert!(list.contains(title), "missing {title} in {list}");
    }
    assert_eq!(
        list.lines().filter(|line| line.starts_with("71 |")).count(),
        1
    );
    assert!(!list.contains("projection"));
    assert!(!list.contains("Deleted"));

    let detail = read(
        &mut store,
        "read_meeting",
        json!({"id": 71, "transcript": true, "notes": true}),
    );
    for expected in [
        "Canonical V2 meeting",
        "quartz-current summary",
        "quartz-current key point",
        "quartz-current action",
        "quartz-current transcript",
        "quartz-current notes",
        "marker [1200 ms]: Appunto temporizzato",
    ] {
        assert!(detail.contains(expected), "missing {expected} in {detail}");
    }
    assert!(!detail.contains("quartz-obsolete"));
    let compact = read(&mut store, "read_meeting", json!({"id": 71}));
    assert!(!compact.contains("Transcript:"));
    assert!(!compact.contains("Notes:"));
    // Search exclusion controls discovery; an explicit user-selected read remains available.
    assert!(read(&mut store, "read_meeting", json!({"id": 73})).contains("quartz-hidden summary"));

    for query in [
        "quartz-obsolete",
        "quartz-deleted",
        "quartz-hidden",
        "quartz-history",
        "   ",
    ] {
        assert_eq!(
            read_json(&mut store, "search_meetings", json!({"query": query})),
            json!([])
        );
    }
    let hits = read_json(
        &mut store,
        "search_meetings",
        json!({"query": "QUARTZ-POINTER"}),
    );
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["meeting_id"], 71);
    assert_eq!(hits[0]["kind"], "transcript");
    assert_eq!(hits[0]["version"], "active-version");
    assert_eq!(hits[0]["segment_id"], "active-version-103");
    assert_eq!(hits[0]["at_ms"], 103_000);
    let capped_hits = read_json(
        &mut store,
        "search_meetings",
        json!({"query": "quartz-current"}),
    );
    assert_eq!(capped_hits.as_array().unwrap().len(), 100);
    assert!(
        capped_hits
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit["meeting_id"] == 71)
    );
    let legacy_hits = read_json(
        &mut store,
        "search_meetings",
        json!({"query": "quartz-legacy-only"}),
    );
    assert!(
        legacy_hits
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["meeting_id"] == 74
                && hit["kind"] == "transcript"
                && hit["version"].is_null())
    );

    let default_page = read_json(&mut store, "read_meeting_segments", json!({"id": 71}));
    assert_eq!(default_page["version"], "active-version");
    assert_eq!(default_page["segments"].as_array().unwrap().len(), 50);
    assert_eq!(default_page["total"], 205);
    assert_eq!(default_page["next_offset"], 50);
    let first = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "limit": 999}),
    );
    assert_eq!(first["segments"].as_array().unwrap().len(), 100);
    assert_eq!(first["segments"][0]["id"], "active-version-000");
    assert_eq!(first["next_offset"], 100);
    let second = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "offset": 100, "limit": 100}),
    );
    assert_eq!(second["segments"].as_array().unwrap().len(), 100);
    assert_eq!(second["segments"][0]["id"], "active-version-100");
    assert_eq!(second["segments"][99]["id"], "active-version-199");
    assert_eq!(second["next_offset"], 200);
    let tail = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "offset": 200, "limit": 100}),
    );
    assert_eq!(tail["segments"].as_array().unwrap().len(), 5);
    assert_eq!(tail["segments"][4]["id"], "active-version-204");
    assert!(tail["next_offset"].is_null());
    let beyond = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "offset": u64::MAX, "limit": u64::MAX}),
    );
    assert_eq!(beyond["segments"], json!([]));
    assert!(beyond["next_offset"].is_null());
    let minimum = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "limit": 0}),
    );
    assert_eq!(minimum["segments"].as_array().unwrap().len(), 1);
    assert_eq!(minimum["next_offset"], 1);

    let old = read_json(
        &mut store,
        "read_meeting_segments",
        json!({"id": 71, "version": "old-version", "offset": 1, "limit": 1}),
    );
    assert_eq!(old["version"], "old-version");
    assert_eq!(old["total"], 3);
    assert_eq!(old["segments"][0]["text"], "quartz-history passage 001");
    assert_eq!(old["next_offset"], 2);
    for (name, args) in [
        ("read_meeting", json!({"id": 72})),
        (
            "read_meeting_segments",
            json!({"id": 72, "version": "deleted-version"}),
        ),
        ("read_meeting_segments", json!({"id": 74})),
        (
            "read_meeting_segments",
            json!({"id": 71, "version": "missing-version"}),
        ),
        ("read_meeting", json!({})),
    ] {
        assert!(
            mcp::call(&mut store, name, &args).is_err(),
            "{name} unexpectedly accepted {args}"
        );
    }

    assert_eq!(serde_json::to_value(&store).unwrap(), original_store);
    for (path, original) in original_records {
        assert_eq!(
            fs::read(path).unwrap(),
            original,
            "read-only tools changed a canonical record"
        );
    }
    assert!(!fixture.root.join("store.json").exists());
}
