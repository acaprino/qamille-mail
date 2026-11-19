//! A Google event is found again in the local calendar copy that Qamille projected for it.
//! Both tests read one shared synthetic `calendar-state.json`; the workspace is selected once
//! (through `QAMILLE_DATA`) before any application code runs, and no account is contacted.
use serde_json::json;
use qamille::mcp::calendar;
use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

/// The workspace folder of this test process, prepared the first time a test asks for it.
fn workspace() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::var_os("QAMILLE_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-data"));
        let root = base.join(format!("calendar-event-link-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Two projections of the same Google id, one per account, plus an unrelated one.
        let state = json!({
            "remote": {
                "1:primary:meeting-1": {
                    "local_id": 42, "account_id": 1, "calendar_id": "primary",
                    "event": {"id": "meeting-1", "summary": "Weekly sync"}
                },
                "2:team@example.com:meeting-1": {
                    "local_id": 77, "account_id": 2, "calendar_id": "team@example.com",
                    "event": {"id": "meeting-1", "summary": "Other account"}
                },
                "1:primary:other": {
                    "local_id": 43, "account_id": 1, "calendar_id": "primary",
                    "event": {"id": "other", "summary": "Another event"}
                }
            }
        });
        std::fs::write(root.join("calendar-state.json"), serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        // SAFETY: every test of this binary asks for the workspace first, and `OnceLock` runs
        // this closure once while the others wait, so nothing reads the variable concurrently.
        unsafe { std::env::set_var("QAMILLE_DATA", &root) };
        root
    })
}

#[test]
fn google_event_resolves_to_its_local_copy() {
    workspace();
    assert_eq!(calendar::local_event_id_for(1, "meeting-1"), Some(42));
    assert_eq!(calendar::local_event_id_for(1, "other"), Some(43));
    assert_eq!(
        calendar::local_event_id_for(2, "meeting-1"),
        Some(77),
        "the same Google id on another account is another event"
    );
}

#[test]
fn unknown_google_event_resolves_to_none() {
    workspace();
    assert_eq!(calendar::local_event_id_for(1, "missing"), None);
    assert_eq!(calendar::local_event_id_for(3, "meeting-1"), None, "an account with no projection");
    assert_eq!(calendar::local_event_id_for(1, ""), None, "an empty id never matches");
}
