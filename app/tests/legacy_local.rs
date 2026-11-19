//! The first start after the local compute mode (wave 2, compito 5): a server left running is
//! stopped only when it answers with the recorded token, the old files go, the person is told
//! once and may free the space of the models, only after confirming.
use qamille::{
    application::{
        AppResult, ServiceContext,
        legacy_local::{LegacyLocalPort, LegacyLocalService},
        ports::{AttachmentGateway, Clock, MailGateway, NullSink, OutgoingMessage, RemoteAction, SecretStore, SyncBatch},
    },
    domain::model::{Account, Attachment, Store},
    infrastructure::{legacy_local::{FileSystemLegacyLocal, process_alive}, repository::MemoryRepository},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

struct Vault;
impl SecretStore for Vault {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok(String::new())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct NoMail;
impl MailGateway for NoMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> i64 {
        1_800_000_000
    }
}
struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qamille-legacy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn service(data: &Path) -> LegacyLocalService {
    let context = Arc::new(ServiceContext {
        compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        repository: Arc::new(MemoryRepository::new(Store::default())),
        mail: Arc::new(NoMail),
        secrets: Arc::new(Vault),
        clock: Arc::new(Fixed),
        attachments: Arc::new(NoFiles),
        events: Arc::new(NullSink),
    });
    let port: Arc<dyn LegacyLocalPort> = Arc::new(FileSystemLegacyLocal::new(data));
    LegacyLocalService::new(context, port)
}
fn write_models(data: &Path) {
    for folder in ["models", "speech-models"] {
        let dir = data.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        for n in 0..3 {
            std::fs::write(dir.join(format!("m{n}.bin")), vec![7u8; 4096]).unwrap();
        }
    }
}
fn write_record(data: &Path, pid: u32, port: u16, token: &str) {
    let record = serde_json::json!({"pid": pid, "port": port, "token": token, "version": "1", "started_at": "then", "program": null});
    std::fs::write(data.join("compute-server.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::write(data.join("compute-server.lock"), b"").unwrap();
}
/// A server that answers `/health` with the model list only to this token.
fn fake_server(token: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let body = if request.contains(&format!("Bearer {token}")) { r#"{"status":"ok","version":"1","models":[]}"# } else { r#"{"status":"ok","version":"1"}"# };
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    port
}
fn sleeper() -> Child {
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("ping");
        c.args(["-n", "60", "127.0.0.1"]);
        c
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("sleep");
        c.arg("60");
        c
    };
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
}

#[test]
fn a_fresh_data_folder_shows_no_notice() {
    let data = data_dir("fresh");
    let status = service(&data).startup().unwrap();
    assert!(!status.show && !status.had_server && status.models_bytes == 0);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_discovery_file_and_models_show_the_notice_once() {
    let data = data_dir("notice");
    write_models(&data);
    write_record(&data, u32::MAX - 7, 9, "x");
    let service = service(&data);
    let status = service.startup().unwrap();
    assert!(status.show && status.had_server && status.models_bytes > 0);
    assert!(!data.join("compute-server.json").exists() && !data.join("compute-server.lock").exists());
    assert!(status.guide_url.ends_with("docs/server-sul-pc.md"));
    service.dismiss().unwrap();
    assert!(!service.status().unwrap().show);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_server_that_answers_with_the_recorded_token_is_stopped() {
    let data = data_dir("stopped");
    let token = "t".repeat(64);
    let port = fake_server(Box::leak(token.clone().into_boxed_str()));
    let mut child = sleeper();
    assert!(process_alive(child.id()));
    write_record(&data, child.id(), port, &token);
    service(&data).startup().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let exited = child.try_wait().unwrap().is_some();
    let _ = child.kill();
    assert!(exited, "the recorded server was stopped");
    assert!(!data.join("compute-server.json").exists());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn a_pid_that_does_not_answer_with_the_recorded_token_is_not_killed() {
    let data = data_dir("notkilled");
    let mut child = sleeper();
    // Nothing listens on the recorded port.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    write_record(&data, child.id(), closed, &"a".repeat(64));
    service(&data).startup().unwrap();
    assert!(child.try_wait().unwrap().is_none() && process_alive(child.id()), "closed port: not killed");
    // Something answers, but not to the recorded token.
    let port = fake_server("another-token-entirely-another-token-entirely-0000");
    write_record(&data, child.id(), port, &"b".repeat(64));
    service(&data).startup().unwrap();
    assert!(child.try_wait().unwrap().is_none() && process_alive(child.id()), "other token: not killed");
    assert!(process_alive(std::process::id()));
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn free_space_needs_confirmation() {
    let data = data_dir("free");
    write_models(&data);
    let service = service(&data);
    assert_eq!(service.free_space(false).unwrap_err().message, "confirmation_required");
    assert!(data.join("models").exists() && data.join("speech-models").exists());
    assert!(service.free_space(true).unwrap() > 0);
    assert!(!data.join("models").exists() && !data.join("speech-models").exists());
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn the_index_survives_freeing_the_models() {
    let data = data_dir("index");
    write_models(&data);
    std::fs::write(data.join("rag_index.bin"), b"index").unwrap();
    std::fs::write(data.join("store.db"), b"store").unwrap();
    service(&data).free_space(true).unwrap();
    assert!(data.join("rag_index.bin").exists() && data.join("store.db").exists());
    let _ = std::fs::remove_dir_all(&data);
}
