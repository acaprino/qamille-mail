//! A fake Hugging Face for the installation tests: a std-thread HTTP server on `127.0.0.1:0` that
//! serves one body for every path (as a mirror would), in slow chunks if asked, or fails.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Seen {
    pub path: String,
    pub authorization: bool,
    pub range: Option<String>,
}

struct Inner {
    body: Vec<u8>,
    chunk: usize,
    delay: Duration,
    /// An HTTP status to answer instead of the body (0: the body).
    status: u16,
    seen: Vec<Seen>,
}

pub struct FakeHf {
    /// `http://127.0.0.1:PORT`: a mirror address.
    pub url: String,
    inner: Arc<Mutex<Inner>>,
}

impl FakeHf {
    pub fn start(body: Vec<u8>) -> FakeHf {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let inner = Arc::new(Mutex::new(Inner { body, chunk: usize::MAX, delay: Duration::ZERO, status: 0, seen: Vec::new() }));
        let shared = Arc::clone(&inner);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || serve(stream, &shared));
            }
        });
        FakeHf { url, inner }
    }

    /// Serves the body in pieces of `chunk` bytes, `delay` apart.
    pub fn slow(&self, chunk: usize, delay: Duration) {
        let mut inner = self.inner.lock().unwrap();
        inner.chunk = chunk;
        inner.delay = delay;
    }

    /// Answers every request with this HTTP status (0: the body again).
    pub fn fail_with(&self, status: u16) {
        self.inner.lock().unwrap().status = status;
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.inner.lock().unwrap().seen.clone()
    }
}

fn serve(stream: std::net::TcpStream, inner: &Mutex<Inner>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
    let (mut authorization, mut range, mut length) = (false, None, 0_usize);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim_end().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("authorization:") {
            authorization = true;
        }
        if let Some(value) = lower.strip_prefix("range:") {
            range = Some(value.trim().to_owned());
        }
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut ignored = vec![0_u8; length];
    let _ = reader.read_exact(&mut ignored);
    let (body, chunk, delay, status) = {
        let mut inner = inner.lock().unwrap();
        inner.seen.push(Seen { path, authorization, range: range.clone() });
        (inner.body.clone(), inner.chunk, inner.delay, inner.status)
    };
    let mut stream = stream;
    if status != 0 {
        let _ = write!(stream, "HTTP/1.1 {status} Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    // A `Range: bytes=N-` request is answered with the rest (what a resume asks for).
    let start = range.as_deref().and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.split('-').next()).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0).min(body.len());
    let partial = range.is_some() && start > 0;
    let rest = &body[start..];
    let head = if partial {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
            rest.len(),
            body.len() - 1,
            body.len()
        )
    } else {
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", rest.len())
    };
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    for piece in rest.chunks(chunk.max(1)) {
        if stream.write_all(piece).is_err() || stream.flush().is_err() {
            return;
        }
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}
