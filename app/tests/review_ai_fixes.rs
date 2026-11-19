use qamille::{ai, model::{AiProvider, Email, Settings}};
use std::{io::{Read, Write}, net::TcpListener, thread, time::{Duration, Instant}};

fn complete_request(request: &[u8]) -> bool {
    let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else { return false; };
    let header = String::from_utf8_lossy(&request[..end]);
    if let Some(length) = header.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
    }) { return request.len() >= end + 4 + length; }
    assert!(header.to_ascii_lowercase().contains("transfer-encoding: chunked"));
    let mut position = end + 4;
    loop {
        let Some(line_end) = request[position..].windows(2).position(|bytes| bytes == b"\r\n") else { return false; };
        let size = usize::from_str_radix(std::str::from_utf8(&request[position..position + line_end]).unwrap(), 16).unwrap();
        position += line_end + 2;
        if size == 0 { return request[position..].starts_with(b"\r\n"); }
        if request.len() < position + size + 2 { return false; }
        position += size + 2;
    }
}

/// Real HTTP/provider boundary: neither the parser nor complete() is mocked.
fn provider(text: &str) -> (Settings, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let body = serde_json::json!({"choices":[{"message":{"content":text}}]}).to_string();
    let worker = thread::spawn(move || {
        for _ in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                    Err(error) => panic!("Test provider: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if complete_request(&request) { break; }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    (Settings { ai_provider: AiProvider::OpenAiCompatible, ai_base_url: url, ai_key: "test-only".into(), ..Default::default() }, worker)
}

#[test]
fn malformed_provider_json_uses_classification_and_meeting_fallbacks() {
    let email = Email { subject: "Invoice".into(), body: "Please review the invoice by Friday.".into(), ..Default::default() };
    let baseline = ai::classify(&Settings::default(), &email, &[]);
    let summary = ai::summarize_meeting(&Settings::default(), "Please review the invoice by Friday.", "Send the report.");
    for text in ["} {", "è🙂 } testo {", "{unfinished", "just text", "{invalid json}", ""] {
        let (settings, worker) = provider(text);
        assert_eq!(ai::classify(&settings, &email, &[]), baseline, "{text:?}");
        assert_eq!(ai::summarize_meeting(&settings, "Please review the invoice by Friday.", "Send the report."), summary, "{text:?}");
        worker.join().unwrap();
    }
}

#[test]
fn valid_json_with_unicode_and_fences_remains_supported() {
    let (settings, worker) = provider("Risposta 🙂\n```json\n{\"category\":\"priority\",\"labels\":[\"Lavoro\"],\"summary\":\"Risultato\",\"key_points\":[\"Punto\"],\"action_items\":[\"Azione\"]}\n```");
    let (category, labels) = ai::classify(&settings, &Email::default(), &[]);
    assert_eq!(category, qamille::model::Category::Priority);
    assert_eq!(labels, vec!["Lavoro"]);
    assert_eq!(ai::summarize_meeting(&settings, "transcript", "notes"), ("Risultato".into(), vec!["Punto".into()], vec!["Azione".into()]));
    worker.join().unwrap();
}
