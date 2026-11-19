//! Two computers of the same person against the real `qamille-server` (wave 7, spec «Verifica»,
//! EX-36..EX-39). Ignored by default: it needs the server executable.
//!
//! ```text
//! QAMILLE_SERVER_BIN=../server/target/debug/qamille-server \
//! cargo test --jobs 1 --no-default-features --test sync_two_apps -- --ignored --nocapture
//! ```
//!
//! The server runs on a temporary folder with `QAMILLE_SYNC_KEY` from `sync keygen` and an
//! administrator key; the administrator API creates the user; each «computer» is a repository, a
//! vault and a state file of its own, signed in through the real `HttpServerAccount`, and
//! synchronizing through the real `HttpSyncServer`. Google is a scripted endpoint.
#[path = "support/http_fake.rs"]
mod http_fake;
#[path = "support/sync_fakes.rs"]
mod fakes;

use fakes::*;
use http_fake::{Fake, reply};
use qamille::{
    application::{
        ports::SecretStore,
        server_account::{ConnectOutcome, Credentials, SERVER_SESSION},
        sync::{RevokeOutcome, SyncMode},
    },
    domain::{server_account::ServerSession, sync::item_id},
    infrastructure::{server_account::HttpServerAccount, sync_http::HttpSyncServer},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const ADMIN_KEY: &str = "e2e-admin-key-0123456789abcdefghijklmnop";
const PASSWORD: &str = "una password lunga e mia";
const CLIENT: &str = "personal.apps.googleusercontent.com";

fn binary() -> PathBuf {
    PathBuf::from(std::env::var("QAMILLE_SERVER_BIN").expect("QAMILLE_SERVER_BIN: the qamille-server executable"))
}
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Running {
    child: Option<Child>,
    dir: PathBuf,
    port: u16,
    admin_port: u16,
    key: String,
    agent: ureq::Agent,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
impl Running {
    fn start() -> Running {
        let dir = std::env::temp_dir().join(format!("qamille-sync-e2e-{}-{}", std::process::id(), free_port()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = {
            let output = Command::new(binary()).args(["sync", "keygen"]).output().unwrap();
            String::from_utf8_lossy(&output.stdout).lines().next().unwrap().trim().to_string()
        };
        assert_eq!(key.len(), 64, "sync keygen prints the key first");
        let agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().into();
        let mut server = Running { child: None, dir, port: free_port(), admin_port: free_port(), key, agent };
        server.launch();
        server
    }
    fn command(&self) -> Command {
        let mut command = Command::new(binary());
        command
            .env("QAMILLE_DATA_DIR", &self.dir)
            .env("QAMILLE_BIND", format!("127.0.0.1:{}", self.port))
            .env("QAMILLE_ADMIN_BIND", format!("127.0.0.1:{}", self.admin_port))
            .env("QAMILLE_ADMIN_KEY", ADMIN_KEY)
            .env("QAMILLE_SYNC_KEY", &self.key);
        command
    }
    fn launch(&mut self) {
        let log = std::fs::File::create(self.dir.join("server.log")).unwrap();
        self.child = Some(self.command().arg("serve").stdout(Stdio::null()).stderr(log).spawn().unwrap());
        let started = Instant::now();
        loop {
            if self.agent.get(&format!("{}/health", self.url())).call().is_ok_and(|r| r.status() == 200) {
                return;
            }
            assert!(started.elapsed() < Duration::from_secs(30), "the server did not start: {}", std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default());
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    fn admin(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = format!("http://127.0.0.1:{}{path}", self.admin_port);
        let auth = format!("Bearer {ADMIN_KEY}");
        let response = match (method, body) {
            ("GET", _) => self.agent.get(&url).header("Authorization", &auth).call(),
            (_, Some(body)) => self.agent.post(&url).header("Authorization", &auth).send_json(body),
            (_, None) => self.agent.post(&url).header("Authorization", &auth).send_empty(),
        };
        let mut response = response.unwrap();
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
    /// A user with a temporary password.
    fn user(&self, name: &str) -> String {
        let (status, created) = self.admin("POST", "/admin/api/users", Some(json!({ "username": name })));
        assert_eq!(status, 201, "{created}");
        created["temporary_password"].as_str().unwrap().to_string()
    }
    /// `qamille-server sync reset --yes` with the server stopped, then started again.
    fn reset(&mut self) {
        self.stop();
        let output = self.command().args(["sync", "reset", "--yes"]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        self.launch();
    }
}

/// A computer signed in as `user` on the real server (the first one changes the temporary password).
fn computer(server: &Running, user: &str, temporary: Option<&str>, name: &str) -> App {
    let info = Arc::new(FakeAccountServer { sync: Mutex::new(true) });
    let app = App::build(Arc::new(HttpSyncServer), Arc::new(HttpServerAccount), info);
    app.accounts.record_consent(&server.url()).unwrap();
    app.sync.record_consent(&server.url()).unwrap();
    let signed = match temporary {
        Some(temporary) => {
            let first = Credentials { identifier: user, password: temporary, device_name: name };
            assert!(matches!(app.accounts.connect(&server.url(), &first).unwrap(), ConnectOutcome::PasswordChangeRequired));
            app.accounts.change_temporary_password(&server.url(), &first, PASSWORD, false).unwrap()
        }
        None => app.accounts.connect(&server.url(), &Credentials { identifier: user, password: PASSWORD, device_name: name }).unwrap(),
    };
    assert!(matches!(signed, ConnectOutcome::Connected { .. }));
    let settings = app.store().settings;
    assert!(settings.mailbox_sync && settings.mailbox_sync_consent.as_deref() == Some(server.url().as_str()), "the consent was recorded from /v1/info");
    app
}
fn pair(server: &Running) -> (App, App) {
    let temporary = server.user("mario");
    let first = computer(server, "mario", Some(&temporary), "Fisso");
    let second = computer(server, "mario", None, "Portatile");
    (first, second)
}
fn device_id(app: &App) -> String {
    let session: ServerSession = serde_json::from_str(&app.vault.get(SERVER_SESSION).unwrap()).unwrap();
    session.device_id
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn a_gmail_and_an_imap_mailbox_appear_on_the_second_app_after_its_login() {
    let server = Running::start();
    let (a, b) = pair(&server);
    a.add_gmail("giulia@gmail.com", "1//refresh-token-of-giulia", CLIENT);
    a.add_imap("ufficio@example.com", "app-password-of-giulia");
    assert_eq!(a.sync.sync_now().unwrap().uploaded, 2);
    assert_eq!(b.sync.sync_now().unwrap().downloaded, 2);
    let gmail = b.account("giulia@gmail.com").unwrap();
    assert_eq!((gmail.oauth_client_id.as_str(), gmail.imap_host.as_str()), (CLIENT, "imap.gmail.com"));
    assert_eq!(b.token("giulia@gmail.com"), "1//refresh-token-of-giulia");
    let imap = b.account("ufficio@example.com").unwrap();
    assert_eq!(b.vault.get(&format!("provider:fastmail:{}", imap.id)).unwrap(), "app-password-of-giulia");
    assert_eq!(b.sync.status().unwrap().mode, SyncMode::Idle);
    // The first computer may be off: the second works alone from here.
    assert_eq!(b.sync.sync_now().unwrap().uploaded, 0);
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn a_changed_token_arrives_on_the_other_app_without_doing_anything_and_an_edit_merges_by_parts() {
    let server = Running::start();
    let (a, b) = pair(&server);
    let id = a.add_gmail("giulia@gmail.com", "1//old", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    // EX-37: Google asks for the sign-in again on the laptop; the desktop gets the new token.
    b.advance(60);
    b.vault.set("gmail:giulia@gmail.com", "1//renewed").unwrap();
    // Meanwhile the desktop renames the mailbox and sends first: a conflict for the laptop.
    a.advance(30);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == id).unwrap().name = "Casa".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    a.sync.sync_now().unwrap();
    for app in [&a, &b] {
        assert_eq!(app.token("giulia@gmail.com"), "1//renewed");
        assert_eq!(app.account("giulia@gmail.com").unwrap().name, "Casa");
    }
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn the_same_mailbox_on_both_apps_is_one_item() {
    let server = Running::start();
    let (a, b) = pair(&server);
    a.add_gmail("giulia@gmail.com", "1//first", CLIENT);
    b.add_gmail("Giulia@Gmail.com", "1//second", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    a.sync.sync_now().unwrap();
    assert_eq!(a.store().accounts.len(), 1);
    assert_eq!(b.store().accounts.len(), 1);
    assert_eq!(a.token("giulia@gmail.com"), b.token("giulia@gmail.com"));
    let id = item_id("gmail", "giulia@gmail.com");
    assert_eq!(a.sync.status().unwrap().mailboxes.len(), 1);
    assert_eq!(id.len(), 64);
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn a_removal_on_one_app_asks_on_the_other_and_a_disabled_mailbox_sends_nothing() {
    let server = Running::start();
    let (a, b) = pair(&server);
    let id = a.add_gmail("giulia@gmail.com", "1//t", CLIENT);
    let kept = a.add_gmail("tenuta@gmail.com", "1//kept", CLIENT);
    a.sync.set_mailbox(kept, false).unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert!(b.account("tenuta@gmail.com").is_none(), "a mailbox switched off is not sent");
    let removed = a.account_service.remove_account(id).unwrap();
    a.sync.after_local_removal(&removed.provider_id, &removed.email).unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let status = b.sync.status().unwrap();
    assert_eq!(status.removals.len(), 1, "{status:?}");
    assert_eq!(b.account("giulia@gmail.com").unwrap().email, "giulia@gmail.com", "nothing deleted without asking");
    b.sync.resolve_removal(&status.removals[0].item_id, true, &b.account_service).unwrap();
    assert!(b.account("giulia@gmail.com").is_none());
    assert_eq!(b.token("giulia@gmail.com"), "");
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn after_a_server_reset_the_apps_reupload_without_notices() {
    let mut server = Running::start();
    let (a, b) = pair(&server);
    a.add_gmail("giulia@gmail.com", "1//t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    server.reset();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert!(a.sync.status().unwrap().removals.is_empty() && b.sync.status().unwrap().removals.is_empty());
    assert_eq!(a.store().accounts.len(), 1);
    assert_eq!(b.store().accounts.len(), 1);
    // The mailbox is on the server again for a third computer.
    let c = computer(&server, "mario", None, "Ufficio");
    assert_eq!(c.sync.sync_now().unwrap().downloaded, 1);
    assert_eq!(c.token("giulia@gmail.com"), "1//t");
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn a_disabled_device_gets_401_and_the_tokens_are_revoked_with_a_scripted_google() {
    let server = Running::start();
    let (a, b) = pair(&server);
    let id = a.add_gmail("giulia@gmail.com", "1//stolen-token", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    // EX-38: the laptop is lost; the desktop disables it and revokes the access.
    a.accounts.revoke_device(&device_id(&b)).unwrap();
    assert_eq!(b.sync.sync_now().unwrap_err().code, "unauthorized", "INV-37");
    let google = Fake::start(vec![reply(500, "oops"), reply(200, "{}")]);
    *a.revoker.google.lock().unwrap() = Some(format!("{}/revoke", google.url));
    let failed = a.sync.revoke_all().unwrap();
    assert_eq!(failed[0].outcome, RevokeOutcome::Failed, "a revocation that fails is reported as not done");
    assert_eq!(a.token("giulia@gmail.com"), "1//stolen-token");
    let done = a.sync.revoke_all().unwrap();
    assert_eq!(done[0].outcome, RevokeOutcome::Revoked);
    assert_eq!(google.requests().len(), 2);
    assert_eq!(google.requests()[1].body, "token=1%2F%2Fstolen-token");
    assert_eq!(a.token("giulia@gmail.com"), "");
    // The mailbox is signed in again here, and the new token reaches the other computers.
    let c = computer(&server, "mario", None, "Portatile nuovo");
    a.advance(60);
    a.vault.set("gmail:giulia@gmail.com", "1//new-token").unwrap();
    a.sync.sync_now().unwrap();
    c.sync.sync_now().unwrap();
    assert_eq!(c.token("giulia@gmail.com"), "1//new-token");
    let _ = id;
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn the_server_database_and_its_log_do_not_contain_the_tokens() {
    let server = Running::start();
    let (a, _b) = pair(&server);
    a.add_gmail("giulia@gmail.com", "1//very-distinctive-token-0451", CLIENT);
    a.add_imap("ufficio@example.com", "very-distinctive-password-0451");
    a.sync.sync_now().unwrap();
    let mut bytes = 0usize;
    let mut found_user = false;
    for entry in std::fs::read_dir(&server.dir).unwrap() {
        let path = entry.unwrap().path();
        let Ok(content) = std::fs::read(&path) else { continue };
        bytes += content.len();
        let text = String::from_utf8_lossy(&content);
        assert!(!text.contains("very-distinctive"), "{} holds a credential in clear", path.display());
        assert!(!text.contains(&server.key), "{} holds the key of the server", path.display());
        found_user |= text.contains("mario");
    }
    assert!(bytes > 0 && found_user, "the scan reads the database (it finds the user name)");
}

#[test]
#[ignore = "needs QAMILLE_SERVER_BIN"]
fn without_the_key_the_server_does_not_declare_sync_and_the_app_sends_nothing() {
    let mut server = Running::start();
    server.stop();
    server.key.clear();
    // Same data folder, no key: `command()` still sets the variable, but empty counts as off.
    server.launch();
    let (status, info) = (200, server.agent.get(&format!("{}/v1/info", server.url())).call().unwrap().body_mut().read_json::<Value>().unwrap());
    assert_eq!(status, 200);
    assert!(info.get("sync").is_none(), "{info}");
    let temporary = server.user("mario");
    let app = App::build(Arc::new(HttpSyncServer), Arc::new(HttpServerAccount), Arc::new(FakeAccountServer { sync: Mutex::new(true) }));
    app.accounts.record_consent(&server.url()).unwrap();
    app.sync.record_consent(&server.url()).unwrap();
    assert!(!app.store().settings.mailbox_sync, "the consent counts only if the server declares sync");
    let _ = temporary;
}
