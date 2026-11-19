//! Contacts over MCP: real sources, no destructive tool, access levels and profile scope.
use serde_json::{Value, json};
use qamille::domain::contacts::*;
use std::ops::Not;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(access: qamille::model::Access) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../checks").join(format!(
            "qamille-mcp-contacts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut store = qamille::model::sample_store();
        store.settings.mcp_access = access;
        store.drafts.clear();
        store.undo_actions.clear();
        std::fs::write(root.join("store.json"), serde_json::to_vec(&store).unwrap()).unwrap();
        Self(root)
    }
    fn seed(&self, cards: &[(&str, &str, &str)]) {
        let mut state = ContactsState::default();
        for (key, name, email) in cards {
            state.sources.push(SourceContact {
                key: key.to_string(),
                book: BookId::local(),
                fields: ContactFields { name: name.to_string(), emails: vec![Labeled { label: String::new(), value: email.to_string() }], ..Default::default() },
                ..Default::default()
            });
        }
        state.next_id = 100;
        std::fs::write(self.0.join("contacts.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    }
    fn state_exists(&self) -> bool {
        self.0.join("contacts.json").exists()
    }
    fn state(&self) -> ContactsState {
        serde_json::from_slice(&std::fs::read(self.0.join("contacts.json")).unwrap()).unwrap()
    }
    fn restrict_to_no_account(&self) {
        std::fs::write(
            self.0.join("mcp-config.json"),
            r#"{"profiles":{"default":{"access":"Triage","account_ids":[],"calendar_ids":[],"ai_execute":false,"permanent_delete":false}},"worker_autostart":false}"#,
        )
        .unwrap();
    }
    fn call(&self, name: &str, arguments: Value) -> Value {
        self.raw(name, arguments)["result"]["structuredContent"].clone()
    }
    fn raw(&self, name: &str, arguments: Value) -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_qamille-mcp"))
            .env("QAMILLE_DATA", &self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        for request in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
        ] {
            writeln!(input, "{request}").unwrap();
        }
        drop(input);
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        lines[1].clone()
    }
    fn tool_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut child = Command::new(env!("CARGO_BIN_EXE_qamille-mcp"))
                .env("QAMILLE_DATA", &self.0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            let list = match &cursor {
                Some(c) => json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"cursor":c}}),
                None => json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            };
            for request in [
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                list,
            ] {
                writeln!(input, "{request}").unwrap();
            }
            drop(input);
            let out = child.wait_with_output().unwrap();
            let lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            let result = &lines[1]["result"];
            names.extend(result["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()));
            match result["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => return names,
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn items(result: &Value) -> Vec<Value> {
    result["data"]["items"].as_array().cloned().unwrap_or_default()
}

#[test]
fn search_contacts_reports_real_sources() {
    let fixture = Fixture::new(qamille::model::Access::Triage);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it")]);
    let found = fixture.call("search_contacts", json!({"query": "ada"}));
    let ada = items(&found).into_iter().find(|i| i["name"] == "Ada Lovelace").unwrap_or_else(|| panic!("{found}"));
    assert_eq!(ada["address"], "ada@x.it");
    assert_eq!(ada["source"], "address_book");
    assert!(ada["sources"].to_string().contains("Locale"));
    let seen = fixture.call("search_contacts", json!({"query": "maria@client"}));
    let maria = items(&seen).into_iter().next().unwrap_or_else(|| panic!("{seen}"));
    assert_eq!(maria["source"], "observed_mail");
}

#[test]
fn address_books_and_a_single_contact_can_be_read() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it")]);
    let books = fixture.call("list_address_books", json!({}));
    assert!(items(&books).iter().any(|b| b["id"] == "local"), "{books}");
    let id = items(&fixture.call("search_contacts", json!({"query": "ada"})))[0]["id"].as_str().unwrap().to_string();
    let one = fixture.call("get_contact", json!({"id": id}));
    assert_eq!(one["data"]["fields"]["name"], "Ada Lovelace");
}

#[test]
fn find_duplicates_lists_groups_without_changing_anything() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it"), ("c2", "Ada L.", "ada@x.it"), ("c3", "Zed", "z@z.it")]);
    let before = std::fs::read(fixture.0.join("contacts.json")).unwrap();
    let found = fixture.call("find_duplicates", json!({"include_possible": true}));
    assert_eq!(items(&found).len(), 1, "{found}");
    assert_eq!(items(&found)[0]["members"].as_array().unwrap().len(), 2);
    assert_eq!(std::fs::read(fixture.0.join("contacts.json")).unwrap(), before);
}

#[test]
fn link_contacts_writes_nothing_remote() {
    let fixture = Fixture::new(qamille::model::Access::Triage);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it"), ("c2", "Augusta Byron", "augusta@y.it")]);
    let linked = fixture.call("link_contacts", json!({"keys": ["c1", "c2"]}));
    assert_eq!(linked["isError"], Value::Null, "{linked}");
    let state = fixture.state();
    assert_eq!(state.links.len(), 1);
    assert!(state.queue.is_empty(), "no remote write is queued");
    assert_eq!(state.sources.len(), 2, "no card is removed or rewritten");
    let found = fixture.call("search_contacts", json!({"query": "augusta"}));
    assert_eq!(items(&found).len(), 1);
    assert_eq!(items(&found)[0]["sources"].as_array().unwrap().len(), 2, "one person, two cards");
}

#[test]
fn no_tool_deletes_or_merges_contacts() {
    let fixture = Fixture::new(qamille::model::Access::Send);
    let names = fixture.tool_names();
    for expected in ["search_contacts", "list_address_books", "get_contact", "find_duplicates", "create_contact", "update_contact", "link_contacts"] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
    for name in &names {
        assert!(!(name.contains("contact") && (name.contains("delete") || name.contains("merge") || name.contains("remove"))), "{name}");
    }
}

#[test]
fn read_only_access_cannot_create_or_update() {
    let fixture = Fixture::new(qamille::model::Access::ReadOnly);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it")]);
    let before = std::fs::read(fixture.0.join("contacts.json")).unwrap();
    let created = fixture.call("create_contact", json!({"book": "local", "name": "Nuovo"}));
    assert_eq!(created["error"]["code"], "permission_denied", "{created}");
    let updated = fixture.call("update_contact", json!({"id": "c1", "name": "Altro"}));
    assert_eq!(updated["error"]["code"], "permission_denied", "{updated}");
    assert_eq!(std::fs::read(fixture.0.join("contacts.json")).unwrap(), before);
}

#[test]
fn create_and_update_work_in_the_local_book_and_demand_a_book() {
    let fixture = Fixture::new(qamille::model::Access::Triage);
    let missing = fixture.raw("create_contact", json!({"name": "Senza rubrica"}));
    assert!(missing["error"].is_object(), "the schema requires a book: {missing}");
    assert!(fixture.state_exists().not(), "nothing was written");
    let created = fixture.call("create_contact", json!({"book": "local", "name": "Carla Neri", "emails": ["carla@x.it"], "org": "Acme"}));
    let id = created["data"]["id"].as_str().unwrap_or_else(|| panic!("{created}")).to_string();
    assert_eq!(fixture.state().sources.len(), 1);
    let updated = fixture.call("update_contact", json!({"id": id, "title": "CEO", "emails": ["carla@x.it", "c.neri@acme.it"]}));
    assert_eq!(updated["data"]["fields"]["title"], "CEO", "{updated}");
    assert_eq!(updated["data"]["fields"]["org"], "Acme", "fields that were not provided stay");
    assert_eq!(updated["data"]["fields"]["emails"].as_array().unwrap().len(), 2);
    let write_to_google = fixture.call("create_contact", json!({"book": "google:1", "name": "Nessun consenso"}));
    assert!(write_to_google["error"].is_object(), "an MCP client cannot open the Google consent flow: {write_to_google}");
}

#[test]
fn a_restricted_profile_never_sees_the_local_book() {
    let fixture = Fixture::new(qamille::model::Access::Triage);
    fixture.seed(&[("c1", "Ada Lovelace", "ada@x.it")]);
    fixture.restrict_to_no_account();
    let found = fixture.call("search_contacts", json!({"query": "ada"}));
    assert!(items(&found).is_empty(), "{found}");
    let created = fixture.call("create_contact", json!({"book": "local", "name": "Fuori profilo"}));
    assert!(created["error"].is_object(), "{created}");
    assert_eq!(fixture.state().sources.len(), 1);
}
