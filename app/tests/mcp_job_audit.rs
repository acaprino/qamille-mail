//! Final tool outcomes are audited once, including calls that outlive the IPC budget.
use serde_json::{Value, json};
use qamille::{
    application::{AppError, AppResult, ServiceContext, ports::*},
    infrastructure::{
        gateways::{LocalAttachments, SystemClock},
        repository::MemoryRepository,
    },
    mcp::server::Server,
    model::{Access, Account, Email, Store},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct NoSecrets;
impl SecretStore for NoSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        unreachable!()
    }
}
struct FailingMail;
impl MailGateway for FailingMail {
    fn load_body(&self, _: &Account, email: &Email) -> AppResult<Email> {
        if email.id == 7 {
            std::thread::sleep(Duration::from_secs(11));
        }
        Err(AppError::external("synthetic-private-provider-error"))
    }
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
fn call(server: &mut Server, id: u64, name: &str, arguments: Value) -> Value {
    server
        .handle(json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
        "params":{"name":name,"arguments":arguments}}))
        .unwrap()["result"]
        .clone()
}

#[test]
fn synchronous_and_asynchronous_failures_each_have_one_final_audit_record() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../checks")
        .join(format!(
            "mcp-job-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    // This binary has one test and no other environment readers during setup.
    unsafe {
        std::env::set_var("QAMILLE_DATA", &root);
    }
    let mut store = Store::default();
    store.settings.mcp_access = Access::ReadOnly;
    store.accounts.push(Account {
        id: 1,
        email: "synthetic@example.test".into(),
        ..Default::default()
    });
    for id in [7, 8] {
        store.emails.push(Email {
            id,
            account_id: 1,
            remote_only: true,
            ..Default::default()
        });
    }
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(FailingMail),
        secrets: Arc::new(NoSecrets),
        clock: Arc::new(SystemClock),
        attachments: Arc::new(LocalAttachments),
    });
    let mut server = Server::new(context, "default".into());
    server.handle(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{}}}),
    );
    server.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    assert_eq!(
        call(&mut server, 2, "read_email", json!({"id":8}))["isError"],
        true
    );
    let delayed = call(&mut server, 3, "read_email", json!({"id":7}));
    assert_eq!(delayed["structuredContent"]["meta"]["asynchronous"], true);
    let operation = delayed["structuredContent"]["operation_id"]
        .as_str()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = call(&mut server, 4, "get_operation", json!({"id":operation}));
        if state["structuredContent"]["status"] == "failed" {
            break;
        }
        assert!(Instant::now() < deadline, "delayed call did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = std::fs::read_to_string(root.join("mcp-activity.jsonl")).unwrap();
    let records: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let failures: Vec<_> = records
        .iter()
        .filter(|row| row["tool"] == "read_email")
        .collect();
    assert_eq!(
        failures.len(),
        2,
        "every failure must be recorded exactly once"
    );
    assert!(
        failures
            .iter()
            .all(|row| row["success"] == false && row["profile"] == "default")
    );
    assert!(!log.contains("synthetic-private-provider-error"));
    assert!(
        records
            .iter()
            .any(|row| row["tool"] == "get_operation" && row["success"] == true)
    );
    std::fs::remove_dir_all(root).unwrap();
}
