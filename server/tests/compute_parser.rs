//! The sacrificial parser process (INV-11) behind `/qamille/v1/extract` and
//! `/qamille/v1/embeddings/image`, with the real `--parser-worker` and with programs that
//! die or hang. No model is installed: the image path stops at `model_missing`.
// The dying and hanging readers are shell programs: those tests are Unix only.
#![cfg_attr(not(unix), allow(unused_imports))]
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use qamille_server::compute::engine::OnnxBackend;
use qamille_server::compute::parser::{ParserCommand, ParserPool};
use qamille_server::compute::{ComputeAuth, ComputeState, DOCUMENT_UNREADABLE, fixed_router};
use support::{TempDir, TestResponse};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

fn real_parser() -> ParserCommand {
    ParserCommand {
        program: env!("CARGO_BIN_EXE_qamille-server").into(),
        args: vec!["--parser-worker".to_owned()],
    }
}

fn router(backend: Arc<OnnxBackend>) -> Router {
    fixed_router(ComputeState::new(backend, ComputeAuth::fixed(TOKEN), "fixed"))
}

async fn post(router: &Router, path: &str, content_type: &str, body: Vec<u8>) -> TestResponse {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    TestResponse { status, headers, body, text }
}

async fn extract(router: &Router, name: &str, file: &[u8]) -> TestResponse {
    let boundary = "parser-test";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f\"\r\n\r\n").as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    post(router, "/qamille/v1/extract", &format!("multipart/form-data; boundary={boundary}"), body).await
}

fn docx(text: &str) -> Vec<u8> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(format!("<w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>").as_bytes()).unwrap();
    zip.finish().unwrap().into_inner()
}

fn png() -> Vec<u8> {
    let image = image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8 * 8, y as u8 * 10, 90]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

#[tokio::test]
async fn documents_are_read_in_the_parser_process() {
    let models = TempDir::new();
    let backend = Arc::new(OnnxBackend::new(models.path(), real_parser()));
    let router = router(backend.clone());
    let word = extract(&router, "Fattura.DOCX", &docx("Fattura di ottobre &amp; novembre")).await;
    assert_eq!(word.status, 200, "{}", word.text);
    assert_eq!(word.body["text"], "Fattura di ottobre & novembre");
    assert!(backend.parser().running(), "a separate reader process answered");
    let text = extract(&router, "note.txt", "città".as_bytes()).await;
    assert_eq!(text.body["text"], "città");
    let broken = extract(&router, "broken.pdf", b"%PDF-1.4 not a document").await;
    assert_eq!(broken.status, 422);
    assert_eq!(broken.body["error"]["code"], "document_unreadable");
    assert!(broken.body["error"]["message"].as_str().unwrap().starts_with("PDF"));
    let binary = extract(&router, "blob.bin", &[0u8; 64]).await;
    assert_eq!(binary.body["error"]["message"], "Binary file, no text preview.");
    // The reader keeps working after a refused file.
    let after = extract(&router, "after.txt", b"ancora qui").await;
    assert_eq!(after.body["text"], "ancora qui");
}

#[tokio::test]
async fn images_are_decoded_by_the_parser_then_need_the_model() {
    let models = TempDir::new();
    let backend = Arc::new(OnnxBackend::new(models.path(), real_parser()));
    let router = router(backend.clone());
    let decoded = post(&router, "/qamille/v1/embeddings/image", "application/octet-stream", png()).await;
    assert_eq!(decoded.status, 503, "{}", decoded.text);
    assert_eq!(decoded.body["error"]["code"], "model_missing");
    let garbage = post(&router, "/qamille/v1/embeddings/image", "application/octet-stream", b"not an image".to_vec()).await;
    assert_eq!(garbage.status, 422);
    assert_eq!(garbage.body["error"]["code"], "document_unreadable");
}

#[cfg(unix)]
#[tokio::test]
async fn a_reader_that_dies_on_a_document_costs_only_that_document() {
    let models = TempDir::new();
    // A reader that dies at once, whatever it is sent: two processes, then the file is skipped.
    let dying = ParserCommand { program: "/bin/sh".into(), args: vec!["-c".to_owned(), "exit 3".to_owned()] };
    let backend = Arc::new(OnnxBackend::with_parser(models.path(), ParserPool::new(dying)));
    let router = router(backend);
    let started = Instant::now();
    let response = extract(&router, "crash.pdf", b"%PDF-1.7").await;
    assert_eq!(response.status, 422, "{}", response.text);
    assert_eq!(response.body["error"]["code"], "document_unreadable");
    assert_eq!(response.body["error"]["message"], DOCUMENT_UNREADABLE);
    assert!(started.elapsed() < Duration::from_secs(10));
    let image = post(&router, "/qamille/v1/embeddings/image", "application/octet-stream", png()).await;
    assert_eq!(image.body["error"]["code"], "document_unreadable");
    // The server itself is still there.
    let health = Request::builder().uri("/health").body(Body::empty()).unwrap();
    assert_eq!(router.clone().oneshot(health).await.unwrap().status(), 200);
}

#[cfg(unix)]
#[tokio::test]
async fn a_reader_that_hangs_gets_one_budget_only() {
    let models = TempDir::new();
    let hanging = ParserCommand { program: "/bin/sh".into(), args: vec!["-c".to_owned(), "cat > /dev/null".to_owned()] };
    let backend = Arc::new(OnnxBackend::with_parser(models.path(), ParserPool::with_timeout(hanging, Duration::from_millis(700))));
    let router = router(backend);
    let started = Instant::now();
    let response = extract(&router, "loop.pdf", b"%PDF-1.7").await;
    assert_eq!(response.body["error"]["code"], "document_unreadable");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(700) && elapsed < Duration::from_millis(1400), "one budget, not two: {elapsed:?}");
}

#[tokio::test]
async fn a_reader_that_dies_once_is_replaced_and_the_file_read() {
    // The first process is killed before the request: the pool notices, starts a fresh one and
    // the document is read (one repetition, D2).
    let models = TempDir::new();
    let backend = Arc::new(OnnxBackend::new(models.path(), real_parser()));
    let router = router(backend.clone());
    assert_eq!(extract(&router, "a.txt", b"uno").await.body["text"], "uno");
    let pid = backend.parser().pid().expect("a reader process");
    #[cfg(unix)]
    let killed = std::process::Command::new("kill").arg("-9").arg(pid.to_string()).status().unwrap();
    #[cfg(windows)]
    let killed = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status().unwrap();
    assert!(killed.success());
    std::thread::sleep(Duration::from_millis(200));
    let again = extract(&router, "b.txt", b"due").await;
    assert_eq!(again.status, 200, "{}", again.text);
    assert_eq!(again.body["text"], "due");
    assert_ne!(backend.parser().pid(), Some(pid), "a fresh process");
}

#[cfg(unix)]
#[test]
fn shutdown_kills_a_reader_stuck_in_a_file() {
    // At the end of `serve --local` a blocking task may still hold the backend: the reader is
    // stopped explicitly, without waiting for its budget.
    let hanging = ParserCommand { program: "/bin/sh".into(), args: vec!["-c".to_owned(), "cat > /dev/null".to_owned()] };
    let pool = Arc::new(ParserPool::with_timeout(hanging, Duration::from_secs(60)));
    let worker = {
        let pool = Arc::clone(&pool);
        std::thread::spawn(move || pool.request("extract", "loop.pdf", b"%PDF-1.7"))
    };
    let began = Instant::now();
    std::thread::sleep(Duration::from_millis(300));
    pool.shutdown();
    let result = worker.join().unwrap();
    assert!(result.is_err());
    assert!(began.elapsed() < Duration::from_secs(5), "stopped at once, not after its budget");
    assert!(!pool.running(), "and no new reader is started");
}

#[cfg(unix)]
#[test]
fn a_parser_worker_ends_with_the_server_it_watches() {
    // `--parser-worker --parent <pid>`: when that process is gone the worker exits, even though
    // its stdin stays open (as with a reader stuck in a file). A shell stands for the server.
    use std::io::BufRead;
    let mut server = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"exec 3<&0; "$0" --parser-worker --parent $$ <&3 & echo $!; wait"#)
        .arg(env!("CARGO_BIN_EXE_qamille-server"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let _stdin = server.stdin.take();
    let mut line = String::new();
    std::io::BufReader::new(server.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let worker = std::path::Path::new("/proc").join(line.trim());
    std::thread::sleep(Duration::from_millis(500));
    assert!(worker.exists(), "alive while its server lives");
    server.kill().unwrap();
    server.wait().unwrap();
    let began = Instant::now();
    while worker.exists() && !std::fs::read_to_string(worker.join("stat")).unwrap_or_default().contains(") Z ") {
        assert!(began.elapsed() < Duration::from_secs(10), "the worker outlived its server");
        std::thread::sleep(Duration::from_millis(100));
    }
}
