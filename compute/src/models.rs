//! Installation of EmbeddingGemma 2 (`onnx-community/embeddinggemma-2-ONNX`, ~288 MB) in
//! `<models>/embeddinggemma-2`, moved from the app's `rag.rs`. Files are checked against their
//! hash; an interrupted file resumes from the bytes already on disk when the server honours the
//! range request.

use crate::download;
#[cfg(test)]
use crate::download::hash_hex;
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

/// The embedding model's identifier in the protocol (D2) and in the index (D4).
pub const MODEL_ID: &str = "embeddinggemma-2";
pub const MODEL_MB: u64 = 288;
pub const MODEL_MISSING_MESSAGE: &str = "The semantic search model isn't installed yet. Install it in Settings.";

/// Pinned so that the hashes below stay valid.
const MODEL_BASE: &str = "https://huggingface.co/onnx-community/embeddinggemma-2-ONNX/resolve/daa72c51243991dfcaf9f9137d2c573d8f7790c0";

pub struct ModelFile {
    pub remote: &'static str,
    pub local: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const MODEL_FILES: [ModelFile; 5] = [
    ModelFile {
        remote: "onnx/model_q4f16.onnx",
        local: "model_q4f16.onnx",
        size: 495_298,
        sha256: "53feeced79582d661e30adeaa9829cea90d92b1c78347e26fd123773580f71d0",
    },
    ModelFile {
        remote: "onnx/model_q4f16.onnx_data",
        local: "model_q4f16.onnx_data",
        size: 156_862_464,
        sha256: "c39fbaa1fb4221f04beb82786a06b81999c514fd39e84fcd58ef79413c87fa56",
    },
    ModelFile {
        remote: "tokenizer.json",
        local: "tokenizer.json",
        size: 32_170_510,
        sha256: "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
    },
    ModelFile {
        remote: "onnx/vision_encoder_q4f16.onnx",
        local: "vision_encoder_q4f16.onnx",
        size: 150_693,
        sha256: "55975fba53928162eee37e9eea2a69965c10c0411ed14c24ea149efa55a8aa68",
    },
    ModelFile {
        remote: "onnx/vision_encoder_q4f16.onnx_data",
        local: "vision_encoder_q4f16.onnx_data",
        size: 97_614_336,
        sha256: "842f996aff092e300c966266ae0cc60771c6ca16282fee04ca83f0f159bc6b33",
    },
];

pub fn total_bytes() -> u64 {
    MODEL_FILES.iter().map(|f| f.size).sum()
}

/// `dir` is the model's own folder (`<models>/embeddinggemma-2`).
pub fn installed(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| {
        std::fs::metadata(dir.join(f.local))
            .map(|m| m.len() == f.size)
            .unwrap_or(false)
    })
}

/// Model download progress, as `GET /health` reports it.
#[derive(Clone, Debug, Default)]
pub struct ModelDownload {
    pub running: bool,
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
}

// One download at a time: the `.part` files are shared.
static DOWNLOAD_LOCK: Mutex<()> = Mutex::new(());
static DOWNLOAD_STATE: Mutex<ModelDownload> =
    Mutex::new(ModelDownload { running: false, done: 0, total: 0, error: None });

fn update_download(change: impl FnOnce(&mut ModelDownload)) {
    if let Ok(mut state) = DOWNLOAD_STATE.lock() {
        change(&mut state);
    }
}

pub fn download_status() -> ModelDownload {
    DOWNLOAD_STATE.lock().map(|state| state.clone()).unwrap_or_default()
}

/// Downloads the model into `dir`. `progress` receives bytes done and total.
pub fn download_model(
    dir: &Path,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    download_model_with(dir, &download::Options::resumable(), cancel, progress)
}

/// As [`download_model`], with the mirror (`models_mirror_url`) and the other options.
pub fn download_model_with(
    dir: &Path,
    options: &download::Options,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    let _single = DOWNLOAD_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if installed(dir) {
        return Ok(());
    }
    // Another process (a second server on the same `data/`) shares the `.part` files: one
    // download at a time across processes too; the second one then finds the model installed.
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("download.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    if installed(dir) {
        return Ok(());
    }
    update_download(|state| {
        state.running = true;
        state.error = None;
    });
    let report = |done: u64, total: u64| {
        update_download(|state| {
            state.done = done;
            state.total = total;
        });
        progress(done, total);
    };
    let result = download_files(dir, MODEL_BASE, &MODEL_FILES, options, cancel, &report);
    update_download(|state| {
        state.running = false;
        state.error = result.as_ref().err().cloned();
    });
    result
}

fn download_files(
    dir: &Path,
    base: &str,
    files: &[ModelFile],
    options: &download::Options,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let agent = download::agent();
    let mut done = 0u64;
    for f in files {
        let url = format!("{base}/{}", f.remote);
        let target = download::Target { name: f.local, size: f.size, sha256: f.sha256 };
        download::fetch(&agent, &url, dir, f.local, &target, options, cancel, &|got| progress(done + got, total))
            .map_err(String::from)?;
        done += f.size;
        progress(done, total);
    }
    Ok(())
}

/// Downloads one file into `dir`, resuming `<local>.part` when the server
/// honours the range. A `.part` survives interruptions; only a size or hash
/// failure deletes it. `progress` receives the bytes of this file on disk.
#[cfg(test)]
fn fetch_verified(
    agent: &ureq::Agent,
    url: &str,
    dir: &Path,
    f: &ModelFile,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    let target = download::Target { name: f.local, size: f.size, sha256: f.sha256 };
    download::fetch(agent, url, dir, f.local, &target, &download::Options::resumable(), cancel, progress).map_err(String::from)
}

#[cfg(test)]
mod download_resume_tests {
    use super::*;
    use std::io::BufRead;
    use std::sync::Arc;

    /// Serves `payload`; the first response stops after `cut` bytes, as a
    /// dropped connection would. Records each request's `Range` header.
    fn serve(payload: Arc<Vec<u8>>, cut: usize, honour_range: bool, ranges: Arc<Mutex<Vec<Option<String>>>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        break;
                    }
                    let line = line.trim_end().to_ascii_lowercase();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("range: ") {
                        range = Some(value.to_string());
                    }
                }
                ranges.lock().unwrap().push(range.clone());
                let len = payload.len();
                let start = range
                    .as_deref()
                    .filter(|_| honour_range)
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.strip_suffix('-'))
                    .and_then(|n| n.parse::<usize>().ok());
                let head = match start {
                    Some(s) => format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {s}-{}/{len}\r\nConnection: close\r\n\r\n", len - s, len - 1),
                    None => format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"),
                };
                stream.write_all(head.as_bytes()).unwrap();
                let body = &payload[start.unwrap_or(0)..];
                let body = if index == 0 { &body[..cut] } else { body };
                let _ = stream.write_all(body);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });
        format!("http://{addr}/model.bin")
    }

    #[test]
    fn interrupted_model_download_resumes_from_the_bytes_on_disk() {
        for honour_range in [true, false] {
            let payload: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
            let mut hash = Sha256::new();
            hash.update(&payload);
            let sha256: &'static str = Box::leak(hash_hex(hash).into_boxed_str());
            let file = ModelFile { remote: "model.bin", local: "model.bin", size: payload.len() as u64, sha256 };
            let ranges = Arc::new(Mutex::new(Vec::new()));
            let url = serve(Arc::new(payload.clone()), 120_000, honour_range, ranges.clone());
            let dir = std::env::temp_dir()
                .join(format!("qamille-model-resume-{}-{honour_range}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
            let cancel = AtomicBool::new(false);

            let first = fetch_verified(&agent, &url, &dir, &file, &cancel, &|_| {});
            assert!(first.unwrap_err().contains("interrupted"));
            let kept = std::fs::metadata(dir.join("model.bin.part")).unwrap().len();
            assert!(kept > 0 && kept <= 120_000, "kept {kept} bytes");

            fetch_verified(&agent, &url, &dir, &file, &cancel, &|_| {}).unwrap();
            assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), payload);
            assert!(!dir.join("model.bin.part").exists());
            let ranges = ranges.lock().unwrap().clone();
            assert_eq!(ranges[0], None, "the first attempt starts from zero");
            assert_eq!(ranges[1].as_deref(), Some(format!("bytes={kept}-").as_str()));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_partial_install_is_not_installed() {
        let dir = std::env::temp_dir().join(format!("qamille-model-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!installed(&dir));
        std::fs::write(dir.join(MODEL_FILES[0].local), b"short").unwrap();
        assert!(!installed(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;
    use std::io::BufRead;
    use std::sync::Arc;

    #[test]
    fn the_embedding_download_goes_through_the_mirror() {
        let files: &'static [ModelFile] = {
            let payloads = [b"first file".to_vec(), b"second file, longer".to_vec()];
            let list: Vec<ModelFile> = payloads
                .iter()
                .enumerate()
                .map(|(i, bytes)| {
                    let mut hash = Sha256::new();
                    hash.update(bytes);
                    let sha256: &'static str = Box::leak(hash_hex(hash).into_boxed_str());
                    let name: &'static str = Box::leak(format!("onnx/file{i}.bin").into_boxed_str());
                    ModelFile { remote: name, local: Box::leak(format!("file{i}.bin").into_boxed_str()), size: bytes.len() as u64, sha256 }
                })
                .collect();
            Box::leak(list.into_boxed_slice())
        };
        let bodies: Arc<Vec<Vec<u8>>> = Arc::new(vec![b"first file".to_vec(), b"second file, longer".to_vec()]);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths: Arc<Mutex<Vec<String>>> = Arc::default();
        let (log, served) = (paths.clone(), bodies.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line.trim_end().is_empty() {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
                let body = if path.ends_with("file0.bin") { &served[0] } else { &served[1] };
                log.lock().unwrap().push(path);
                let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes());
                let _ = stream.write_all(body);
            }
        });
        let dir = std::env::temp_dir().join(format!("qamille-model-mirror-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let options = download::Options::resumable().with_mirror(download::Mirror::parse(&format!("http://127.0.0.1:{port}")).ok());
        download_files(&dir, MODEL_BASE, files, &options, &AtomicBool::new(false), &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("file1.bin")).unwrap(), bodies[1]);
        let paths = paths.lock().unwrap().clone();
        let prefix = "/onnx-community/embeddinggemma-2-ONNX/resolve/daa72c51243991dfcaf9f9137d2c573d8f7790c0/onnx/";
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.starts_with(prefix)), "{paths:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
