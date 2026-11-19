//! A tiny HTTP server for the tests of adapters: answers scripted responses, one per connection,
//! and records what it was asked.
#![allow(dead_code)]
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

#[derive(Clone, Debug)]
pub struct Seen {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}
impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}
pub fn reply(status: u16, body: &str) -> Reply {
    Reply { status, headers: vec![], body: body.into() }
}
pub struct Fake {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    handle: Option<JoinHandle<()>>,
}
impl Fake {
    /// Serves `replies` in order, one per connection, then stops.
    pub fn start(replies: Vec<Reply>) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let handle = std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                let (head_end, length) = loop {
                    let n = stream.read(&mut buffer).unwrap_or(0);
                    if n == 0 {
                        break (raw.len(), 0);
                    }
                    raw.extend_from_slice(&buffer[..n]);
                    if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..pos]).to_lowercase();
                        let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                        break (pos + 4, length);
                    }
                };
                while raw.len() < head_end + length {
                    let n = stream.read(&mut buffer).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..n]);
                }
                let head = String::from_utf8_lossy(&raw[..head_end.min(raw.len())]).to_string();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or_default().to_string();
                let headers = lines
                    .filter(|l| !l.is_empty())
                    .filter_map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_string(), v.trim().to_string())))
                    .collect();
                let body = String::from_utf8_lossy(&raw[head_end.min(raw.len())..]).to_string();
                log.lock().unwrap().push(Seen { request_line, headers, body });
                let mut text = format!("HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                for (n, v) in &reply.headers {
                    text.push_str(&format!("{n}: {v}\r\n"));
                }
                text.push_str("\r\n");
                text.push_str(&reply.body);
                let _ = stream.write_all(text.as_bytes());
            }
        });
        Fake { url, seen, handle: Some(handle) }
    }
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        // Unblock a listener still waiting for a connection that never comes.
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}
