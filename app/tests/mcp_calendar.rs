//! Calendar lifecycle against an isolated workspace; never contacts an account.
use serde_json::json;
use qamille::{
    application::{AppResult, ServiceContext, ports::SecretStore},
    infrastructure::{
        gateways::{GmailGateway, LocalAttachments, SystemClock},
        repository::MemoryRepository,
    },
    mcp::calendar,
    model::{Access, Store},
};
use std::sync::Arc;
struct NoSecrets;
impl SecretStore for NoSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        panic!("No secret writes in calendar lifecycle test")
    }
}

#[test]
fn local_lifecycle_revisions_profiles_import_and_concurrent_creation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("checks")
        .join(format!(
            "qamille-calendar-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    // This test binary has one test; no other thread reads the variable during setup.
    unsafe {
        std::env::set_var("QAMILLE_DATA", &root);
        std::env::set_var("QAMILLE_MCP_FILE_ROOTS", &root);
    }
    let imports = root.join("imports");
    std::fs::create_dir_all(&imports).unwrap();
    let source = imports.join("report.pdf");
    std::fs::write(&source, b"synthetic attachment").unwrap();
    let registered = qamille::mcp::files::register(&source).unwrap();
    let payload = qamille::mcp::files::resolve(registered["handle"].as_str().unwrap()).unwrap();
    assert_eq!(payload.file_name().unwrap(), "report.pdf");
    assert_eq!(std::fs::read(payload).unwrap(), b"synthetic attachment");
    let mut store = Store::default();
    store.settings.mcp_access = Access::Send;
    store.settings.time_zone = "Europe/Rome".into();
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(qamille::application::ports::NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(GmailGateway::new(root.join("attachments"))),
        secrets: Arc::new(NoSecrets),
        clock: Arc::new(SystemClock),
        attachments: Arc::new(LocalAttachments),
    });
    let args = json!({"calendar_id":"local","title":"Calendar synthetic","start":"2026-10-25","end":"2026-10-26","all_day":true,"idempotency_key":"day"});
    let created = calendar::call(context.clone(), "create_event", &args).unwrap();
    assert_eq!(
        calendar::call(context.clone(), "create_event", &args).unwrap(),
        created
    );
    assert_eq!(context.read().unwrap().events.len(), 1);
    let event = context.read().unwrap().events[0].clone();
    assert_eq!(event.end - event.start, 25 * 3600);
    let id = created["id"].as_str().unwrap();
    let updated = calendar::call(
        context.clone(),
        "update_event",
        &json!({"id":id,"expected_revision":created["revision"],"title":"Changed"}),
    )
    .unwrap();
    assert_eq!(updated["summary"], "Changed");
    assert_eq!(
        calendar::call(
            context.clone(),
            "update_event",
            &json!({"id":id,"expected_revision":created["revision"],"title":"Stale"})
        )
        .unwrap_err()
        .code,
        "revision_conflict"
    );
    let forbidden = vec!["remote@example.com".to_string()];
    assert_eq!(
        calendar::call_scoped(
            context.clone(),
            "read_event",
            &json!({"id":id}),
            Some(&forbidden)
        )
        .unwrap_err()
        .code,
        "permission_denied"
    );
    let ics = calendar::call(
        context.clone(),
        "export_events",
        &json!({"from":"2026-10-24","to":"2026-10-27"}),
    )
    .unwrap();
    assert!(ics.get("ics").is_none());
    let handle = ics["file"]["handle"].as_str().unwrap();
    let exported = qamille::mcp::files::resolve_scoped(handle, Some(&[])).unwrap();
    // Neither a descriptor nor its payload may be rebound into a new, unscoped handle.
    let descriptor = Store::data_dir().join("mcp-files").join(format!("{handle}.json"));
    for managed in [&descriptor, &exported] {
        assert_eq!(
            qamille::mcp::files::register(managed).unwrap_err().code,
            "permission_denied"
        );
    }
    assert!(
        std::fs::read_to_string(exported)
            .unwrap()
            .contains("DTSTART;VALUE=DATE:20261025")
    );
    let preview =
        calendar::call(context.clone(), "import_events", &json!({"handle":handle})).unwrap();
    assert_eq!(preview["count"], 1);
    assert_eq!(context.read().unwrap().events.len(), 1);
    let range = json!({"from":"2026-10-24","to":"2026-10-27","limit":1});
    assert_eq!(
        calendar::call(context.clone(), "list_events", &range).unwrap()["total"],
        1
    );
    let slots=calendar::call(context.clone(),"find_free_slots",&json!({"from":"2026-10-23","to":"2026-10-27","time_zone":"Europe/Rome","work_start":"09:00","work_end":"17:00","weekdays":[1,2,3,4,5]})).unwrap();
    assert_eq!(slots["slots"].as_array().unwrap().len(), 2);
    assert_eq!(slots["slots"][0]["start"], "2026-10-23T07:00:00+00:00");
    assert_eq!(slots["slots"][1]["start"], "2026-10-26T08:00:00+00:00");
    let mut jobs = Vec::new();
    for _ in 0..4 {
        let context = context.clone();
        jobs.push(std::thread::spawn(move||calendar::call(context,"create_event",&json!({"title":"Concurrent","start":"2026-11-02T09:00:00+01:00","end":"2026-11-02T10:00:00+01:00","idempotency_key":"race"}))));
    }
    let mut completed = 0;
    for job in jobs {
        match job.join().unwrap() {
            Ok(_) => completed += 1,
            Err(error) => assert_eq!(error.code, "operation_unknown"),
        }
    }
    assert!(completed >= 1);
    assert_eq!(context.read().unwrap().events.len(), 2);
    calendar::call(
        context.clone(),
        "delete_event",
        &json!({"id":id,"expected_revision":updated["revision"]}),
    )
    .unwrap();
    assert_eq!(context.read().unwrap().events.len(), 1);
    context
        .update(|s| {
            s.settings.mcp_access = Access::ReadOnly;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        calendar::call(context.clone(), "create_event", &args)
            .unwrap_err()
            .code,
        "permission_denied"
    );
    // Deliberate desktop edits are independent of external-assistant permissions.
    assert!(calendar::call_desktop(context.clone(),"create_event",&json!({"title":"Desktop","start":"2026-11-02T11:00:00+01:00","end":"2026-11-02T12:00:00+01:00","idempotency_key":"desktop"})).is_ok());
    assert!(root.join("calendar-state.json").exists());
}
