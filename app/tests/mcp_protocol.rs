//! End-to-end stdio protocol checks, always in a synthetic workspace.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new(access: qamille::model::Access) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../checks")
            .join(format!(
                "qamille-protocol-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&root).unwrap();
        let mut store = qamille::model::sample_store();
        store.settings.mcp_access = access;
        store.drafts.clear();
        store.undo_actions.clear();
        std::fs::write(root.join("store.json"), serde_json::to_vec(&store).unwrap()).unwrap();
        Self(root)
    }
    fn run(&self, requests: Vec<Value>) -> Vec<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_qamille-mcp"))
            .env("QAMILLE_DATA", &self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for request in requests {
            writeln!(input, "{request}").unwrap();
        }
        drop(input);
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("stdout must contain only JSON-RPC"))
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn legacy() -> Vec<Value> {
    vec![
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ]
}
#[test]
fn legacy_catalog_is_paginated_and_all_families_are_registered() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    let mut req = legacy();
    req.push(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    let responses = fixture.run(req);
    let first = &responses[1]["result"];
    assert_eq!(first["tools"].as_array().unwrap().len(), 40);
    let cursor = first["nextCursor"].as_str().unwrap();
    let mut req = legacy();
    req.push(json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"cursor":cursor}}));
    let next = fixture.run(req);
    let names: Vec<_> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .chain(next[1]["result"]["tools"].as_array().unwrap().iter())
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in [
        "search_emails",
        "create_draft",
        "send_draft",
        "list_calendars",
        "update_event",
        "read_meeting_segments",
        "get_operation",
    ] {
        assert!(names.contains(&name), "Missing {name}");
    }
}
#[test]
fn read_only_profile_rejects_writes_and_credential_overrides() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    let mut req = legacy();
    req.push(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_draft","arguments":{"account_id":1,"body":"Synthetic","idempotency_key":"test"}}}));
    req.push(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_accounts","arguments":{"password":"must-reject"}}}));
    let results = fixture.run(req);
    assert_eq!(
        results[1]["result"]["structuredContent"]["error"]["code"],
        "permission_denied"
    );
    assert_eq!(results[2]["error"]["code"], -32602);
    let store: qamille::model::Store =
        serde_json::from_slice(&std::fs::read(fixture.0.join("store.json")).unwrap()).unwrap();
    assert!(store.drafts.is_empty());
}
#[test]
fn modern_requests_require_metadata_and_reject_unknown_versions() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    let meta = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
    let responses=fixture.run(vec![json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":meta}}),json!({"jsonrpc":"2.0","id":2,"method":"ping","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01"}}}),json!({"jsonrpc":"2.0","id":3,"method":"tools/list"})]);
    assert_eq!(responses[0]["result"]["resultType"], "complete");
    assert!(
        responses[0]["result"]["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28"))
    );
    assert_eq!(responses[1]["error"]["code"], -32022);
    assert!(responses[2].get("error").is_some());
}
#[test]
fn missing_and_unknown_profiles_never_silently_inherit_grants() {
    let fixture = Fixture::new(qamille::model::Access::Send);
    std::fs::write(fixture.0.join("mcp-config.json"),r#"{"profiles":{"default":{"access":"ReadOnly","account_ids":[],"calendar_ids":[],"ai_execute":false,"permanent_delete":false}},"worker_autostart":false}"#).unwrap();
    let mut req = legacy();
    req.push(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_accounts","arguments":{}}}));
    let result = fixture.run(req);
    let data = &result[1]["result"]["structuredContent"];
    assert_eq!(result[1]["result"]["isError"], false);
    assert!(
        data.to_string().find("@example").is_none(),
        "Restricted profile leaked account data"
    );
}
#[test]
fn modern_client_identity_is_optional() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    let response=fixture.run(vec![json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}})]);
    assert_eq!(response[0]["result"]["resultType"], "complete");
}
