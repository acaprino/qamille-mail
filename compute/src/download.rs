//! The downloader shared by the model installers (`models`, `speech`, `llm_catalog`): one file
//! per call, checked for size and SHA-256 **before** it gets its final name (INV-39).
//!
//! Two modes: [`Options::resumable`] keeps `<local>.part` between attempts and resumes it with a
//! `Range` request; [`Options::one_shot`] uses a part file with a unique name, never resumes, and
//! removes it on any failure. Error texts never carry the URL, a query, a header or a token.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Space kept free beyond a download, like the app's own disk admission.
pub const PHYSICAL_RESERVE: u64 = 200_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    Cancelled,
    /// The connection dropped or the body ended early. The detail follows "Download interrupted: ".
    Interrupted(String),
    /// The request failed. The detail follows "Download failed: ".
    Failed(String),
    /// The body was longer than declared; carries the file name.
    TooLarge(String),
    /// Size or SHA-256 differ from the declared ones; carries the file name.
    WrongHash(String),
    NoSpace,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("Download cancelled."),
            DownloadError::Interrupted(detail) => write!(f, "Download interrupted: {detail}"),
            DownloadError::Failed(detail) => write!(f, "Download failed: {detail}"),
            DownloadError::TooLarge(file) => write!(f, "{file} exceeded its declared size; the download was stopped."),
            DownloadError::WrongHash(file) => write!(f, "{file} failed its integrity check; nothing was installed."),
            DownloadError::NoSpace => f.write_str("Not enough free disk space. The operation is paused; no mail was removed."),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<DownloadError> for String {
    fn from(error: DownloadError) -> String {
        error.to_string()
    }
}

/// What to fetch: `name` is the file name used in messages.
pub struct Target<'a> {
    pub name: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

pub const HF_HOST: &str = "huggingface.co";
pub const GITHUB_HOST: &str = "github.com";
/// Redirects followed by hand for one request.
pub const MAX_REDIRECTS: usize = 5;

/// A replacement for `https://huggingface.co` and `https://github.com` (`models_mirror_url`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    base: String,
    authority: String,
}

impl Mirror {
    /// `https`, or `http` only toward loopback, private addresses (`10/8`, `172.16/12`,
    /// `192.168/16`, `fc00::/7`) and names without dots; no credentials, query or fragment.
    pub fn parse(value: &str) -> Result<Mirror, &'static str> {
        const BAD: &str = "the mirror address is not allowed";
        let value = value.trim_end_matches('/');
        if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) || value.contains(['?', '#', '\\']) {
            return Err(BAD);
        }
        let (scheme, rest) = value.split_once("://").ok_or(BAD)?;
        let authority = rest.split('/').next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') {
            return Err(BAD);
        }
        let host = host_of(authority).ok_or(BAD)?;
        match scheme {
            "https" => {}
            "http" if is_internal_host(&host) => {}
            _ => return Err(BAD),
        }
        Ok(Mirror { base: value.to_owned(), authority: authority.to_ascii_lowercase() })
    }

    /// The mirror's `host[:port]`.
    pub fn host(&self) -> &str {
        &self.authority
    }

    /// `https://huggingface.co/...` and `https://github.com/...` with the mirror in place of the
    /// host and the path kept; every other address: `None`.
    pub fn map(&self, url: &str) -> Option<String> {
        for host in [HF_HOST, GITHUB_HOST] {
            let prefix = format!("https://{host}");
            if let Some(rest) = url.strip_prefix(&prefix)
                && (rest.is_empty() || rest.starts_with('/'))
            {
                return Some(format!("{}{rest}", self.base));
            }
        }
        None
    }
}

/// The host of an `authority` (no userinfo), lowercase, without port or brackets.
fn host_of(authority: &str) -> Option<String> {
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split_once(']')?.0
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn is_internal_host(host: &str) -> bool {
    use std::net::IpAddr;
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => host == "localhost" || !host.contains('.'),
    }
}

/// The Hugging Face token: never printed, never in an error.
#[derive(Clone)]
pub struct HfToken(String);

impl HfToken {
    /// Visible ASCII, 1 to 256 characters.
    pub fn new(value: &str) -> Option<HfToken> {
        let ok = (1..=256).contains(&value.len()) && value.bytes().all(|b| (0x21..=0x7e).contains(&b));
        ok.then(|| HfToken(value.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for HfToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HfToken(<nascosto>)")
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub resume: bool,
    pub mirror: Option<Mirror>,
    pub token: Option<HfToken>,
    /// The only host that gets the token; only tests change it (they also lift the `https` rule
    /// for a host that is not `huggingface.co`).
    pub token_host: &'static str,
    /// The catalog entry asks for a token.
    pub gated: bool,
}

impl Options {
    /// `<local>.part` survives interruptions and resumes with `Range` (models, LLM catalog).
    pub fn resumable() -> Options {
        Options { resume: true, mirror: None, token: None, token_host: HF_HOST, gated: false }
    }

    /// No `Range`, no shared part file: a unique one, removed on failure (whisper).
    pub fn one_shot() -> Options {
        Options { resume: false, ..Options::resumable() }
    }

    pub fn with_mirror(self, mirror: Option<Mirror>) -> Options {
        Options { mirror, ..self }
    }
}

/// The agent the installers use: the timeouts of the model download.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_recv_body(Some(Duration::from_secs(3600)))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into()
}

pub fn hash_hex(hash: Sha256) -> String {
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Hashes the first `len` bytes of an interrupted download.
pub fn hash_prefix(path: &Path, len: u64) -> Result<Sha256, String> {
    let mut hash = Sha256::new();
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?.take(len);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(hash);
        }
        hash.update(&buf[..n]);
    }
}

/// Fails when `dir`'s disk, where the platform tells, cannot take `bytes` plus the reserve.
pub fn ensure_space(dir: &Path, bytes: u64) -> Result<(), DownloadError> {
    if crate::memory::available_disk_bytes(dir).is_some_and(|free| free < bytes.saturating_add(PHYSICAL_RESERVE)) {
        return Err(DownloadError::NoSpace);
    }
    Ok(())
}

/// A name no other file of this process or another one uses.
fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn io(error: impl std::fmt::Display) -> DownloadError {
    DownloadError::Failed(error.to_string())
}

/// Downloads `url` into `dir/<local>`. The body goes to `dir/<local>.part` (or a unique part with
/// [`Options::one_shot`]) and is checked for size and SHA-256 before the `rename`. A file already
/// there with the right size is not downloaded again. `progress` receives the bytes of the file on
/// disk, the resumed prefix included.
#[allow(clippy::too_many_arguments)]
pub fn fetch(
    agent: &ureq::Agent,
    url: &str,
    dir: &Path,
    local: &str,
    target: &Target<'_>,
    options: &Options,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), DownloadError> {
    let final_path = dir.join(local);
    if std::fs::metadata(&final_path).is_ok_and(|m| m.len() == target.size) {
        return Ok(());
    }
    let part = if options.resume {
        dir.join(format!("{local}.part"))
    } else {
        dir.join(format!("{local}.{}.part", unique_suffix()))
    };
    let result = fetch_into(agent, url, &part, &final_path, target, options, cancel, progress);
    if result.is_err() && !options.resume {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn fetch_into(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    final_path: &Path,
    target: &Target<'_>,
    options: &Options,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), DownloadError> {
    let dir = final_path.parent().unwrap_or(Path::new("."));
    let mut have = if options.resume { std::fs::metadata(part).map(|m| m.len()).unwrap_or(0) } else { 0 };
    if have >= target.size {
        // Complete but never renamed (closed right after the last byte), or corrupt: the
        // integrity check decides.
        if have == target.size && hash_hex(hash_prefix(part, target.size).map_err(io)?) == target.sha256 {
            return std::fs::rename(part, final_path).map_err(io);
        }
        let _ = std::fs::remove_file(part);
        have = 0;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(DownloadError::Cancelled);
    }
    ensure_space(dir, target.size - have)?;
    let mut resp = open(agent, url, options, (have > 0).then_some(have))?;
    let status = resp.status().as_u16();
    if status != 200 && status != 206 {
        return Err(DownloadError::Failed(format!("HTTP status {status}")));
    }
    let resumed = have > 0
        && status == 206
        && resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(&format!("bytes {have}-")));
    let (mut out, mut hash, mut got) = if resumed {
        let out = std::fs::OpenOptions::new().append(true).open(part).map_err(io)?;
        (out, hash_prefix(part, have).map_err(io)?, have)
    } else if options.resume {
        // No usable range: start this file again from the first byte.
        (std::fs::File::create(part).map_err(io)?, Sha256::new(), 0)
    } else {
        (std::fs::OpenOptions::new().write(true).create_new(true).open(part).map_err(io)?, Sha256::new(), 0)
    };
    progress(got);
    let mut reader = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            // The `.part` file stays on disk (resumable mode), so the next attempt resumes.
            return Err(DownloadError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| DownloadError::Interrupted(e.to_string()))?;
        if n == 0 {
            break;
        }
        if got.saturating_add(n as u64) > target.size {
            drop(out);
            let _ = std::fs::remove_file(part);
            return Err(DownloadError::TooLarge(target.name.to_owned()));
        }
        out.write_all(&buf[..n]).map_err(io)?;
        hash.update(&buf[..n]);
        got += n as u64;
        progress(got);
    }
    out.sync_all().map_err(io)?;
    drop(out);
    if got < target.size {
        // Connection closed early: keep the bytes so the next attempt resumes.
        return Err(DownloadError::Interrupted(format!("{} is incomplete.", target.name)));
    }
    if hash_hex(hash) != target.sha256 {
        let _ = std::fs::remove_file(part);
        return Err(DownloadError::WrongHash(target.name.to_owned()));
    }
    std::fs::rename(part, final_path).map_err(io)
}

/// `(scheme, authority, path-and-query)` of an absolute `http(s)` address.
fn split_url(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    (!authority.is_empty() && !authority.contains('@')).then_some((scheme, authority, tail))
}

/// Whether a redirect from `from` to `to` may be followed: never from `https` to `http` (unless
/// a mirror serves it, which is internal), and with a mirror only to the mirror's own host.
fn redirect_allowed(from: &str, to: &str, mirror: Option<&Mirror>) -> Result<(), DownloadError> {
    let refused = || DownloadError::Failed("redirect refused".to_owned());
    let (from_scheme, _, _) = split_url(from).ok_or_else(refused)?;
    let (to_scheme, to_authority, _) = split_url(to).ok_or_else(refused)?;
    match mirror {
        Some(mirror) if !to_authority.eq_ignore_ascii_case(mirror.host()) => Err(refused()),
        Some(_) => Ok(()),
        None if from_scheme == "https" && to_scheme == "http" => Err(refused()),
        None => Ok(()),
    }
}

/// Resolves a `Location` against the address it answered.
fn resolve_location(current: &str, location: &str) -> Option<String> {
    let (scheme, authority, path) = split_url(current)?;
    if location.starts_with("http://") || location.starts_with("https://") {
        Some(location.to_owned())
    } else if let Some(rest) = location.strip_prefix("//") {
        Some(format!("{scheme}://{rest}"))
    } else if location.starts_with('/') {
        Some(format!("{scheme}://{authority}{location}"))
    } else if location.is_empty() {
        None
    } else {
        let path = path.split(['?', '#']).next().unwrap_or("");
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        Some(format!("{scheme}://{authority}{dir}/{location}"))
    }
}

/// Sends the request and follows redirects by hand: the `Range` goes on every hop, the token on
/// the first request only, and only to `options.token_host` over `https` with no mirror.
fn open(agent: &ureq::Agent, url: &str, options: &Options, range_from: Option<u64>) -> Result<ureq::http::Response<ureq::Body>, DownloadError> {
    if options.gated && options.token.is_none() {
        return Err(DownloadError::Failed("this model needs a Hugging Face token, and none is configured".to_owned()));
    }
    let mut current = match &options.mirror {
        Some(mirror) => mirror.map(url).ok_or_else(|| DownloadError::Failed("the address is not served by the mirror".to_owned()))?,
        None => url.to_owned(),
    };
    for hop in 0..=MAX_REDIRECTS {
        let (scheme, authority, _) = split_url(&current).ok_or_else(|| DownloadError::Failed("the address is not valid".to_owned()))?;
        let mut request = agent.get(&current);
        if let Some(have) = range_from {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let secure = scheme == "https" || options.token_host != HF_HOST;
        if hop == 0
            && options.gated
            && options.mirror.is_none()
            && secure
            && host_of(authority).as_deref() == Some(options.token_host)
            && let Some(token) = &options.token
        {
            request = request.header("Authorization", format!("Bearer {}", token.expose()));
        }
        let resp = request.call().map_err(|e| DownloadError::Failed(without_url(&e.to_string(), &current)))?;
        if !matches!(resp.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(resp);
        }
        let next = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .and_then(|location| resolve_location(&current, location))
            .ok_or_else(|| DownloadError::Failed("redirect without a usable location".to_owned()))?;
        redirect_allowed(&current, &next, options.mirror.as_ref())?;
        current = next;
    }
    Err(DownloadError::Failed("too many redirects".to_owned()))
}

/// An error text with the address taken out, in case a library quotes it.
fn without_url(text: &str, url: &str) -> String {
    text.replace(url, "<url>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::sync::{Arc, Mutex};

    type Ranges = Arc<Mutex<Vec<Option<String>>>>;

    /// Serves `payload`; the first response stops after `cut` bytes (a dropped connection).
    /// Records each request's `Range` header.
    fn serve_bytes(payload: Arc<Vec<u8>>, cut: usize, honour_range: bool, ranges: Ranges) -> String {
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
                let _ = stream.write_all(head.as_bytes());
                let body = &payload[start.unwrap_or(0)..];
                let body = if index == 0 { &body[..cut.min(body.len())] } else { body };
                let _ = stream.write_all(body);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });
        format!("http://{addr}/model.bin")
    }

    /// Answers every request with `status` and an empty body.
    fn serve_status(status: u16) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line.trim_end().is_empty() {
                        break;
                    }
                }
                let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
            }
        });
        format!("http://{addr}/secret/path?token=abc")
    }

    fn payload(len: u32) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn sha(bytes: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(bytes);
        hash_hex(hash)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qamille-download-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(url: &str, dir: &Path, bytes: &[u8], options: &Options, cancel: &AtomicBool, progress: &dyn Fn(u64)) -> Result<(), DownloadError> {
        let sha256 = sha(bytes);
        let target = Target { name: "model.bin", size: bytes.len() as u64, sha256: &sha256 };
        fetch(&agent(), url, dir, "model.bin", &target, options, cancel, progress)
    }

    #[test]
    fn resumes_from_the_part_file_with_a_range_header() {
        let bytes = payload(300_000);
        let dir = temp_dir("resume");
        let ranges: Ranges = Arc::default();
        let url = serve_bytes(Arc::new(bytes.clone()), 120_000, true, ranges.clone());
        let cancel = AtomicBool::new(false);
        let first = run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|_| {});
        assert!(matches!(first, Err(DownloadError::Interrupted(_))), "{first:?}");
        let kept = std::fs::metadata(dir.join("model.bin.part")).unwrap().len();
        assert!(kept > 0 && kept <= 120_000, "kept {kept}");
        run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), bytes);
        assert!(!dir.join("model.bin.part").exists());
        let ranges = ranges.lock().unwrap().clone();
        assert_eq!(ranges[0], None);
        assert_eq!(ranges[1].as_deref(), Some(format!("bytes={kept}-").as_str()));
    }

    #[test]
    fn a_server_that_ignores_range_restarts_from_zero() {
        let bytes = payload(300_000);
        let dir = temp_dir("norange");
        let url = serve_bytes(Arc::new(bytes.clone()), 120_000, false, Arc::default());
        let cancel = AtomicBool::new(false);
        assert!(run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|_| {}).is_err());
        run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), bytes);
        assert!(!dir.join("model.bin.part").exists());
    }

    #[test]
    fn a_wrong_hash_deletes_the_part_and_installs_nothing() {
        let bytes = payload(50_000);
        let dir = temp_dir("wronghash");
        let url = serve_bytes(Arc::new(bytes.clone()), 50_000, true, Arc::default());
        let target = Target { name: "model.bin", size: 50_000, sha256: &"0".repeat(64) };
        let result = fetch(&agent(), &url, &dir, "model.bin", &target, &Options::resumable(), &AtomicBool::new(false), &|_| {});
        assert_eq!(result, Err(DownloadError::WrongHash("model.bin".into())));
        assert!(!dir.join("model.bin").exists() && !dir.join("model.bin.part").exists());
    }

    #[test]
    fn a_longer_body_than_declared_is_stopped_and_the_part_removed() {
        let bytes = payload(200_000);
        let dir = temp_dir("toolarge");
        let url = serve_bytes(Arc::new(bytes.clone()), 200_000, true, Arc::default());
        let target = Target { name: "model.bin", size: 100_000, sha256: &sha(&bytes[..100_000]) };
        let result = fetch(&agent(), &url, &dir, "model.bin", &target, &Options::resumable(), &AtomicBool::new(false), &|_| {});
        assert_eq!(result, Err(DownloadError::TooLarge("model.bin".into())));
        assert!(!dir.join("model.bin").exists() && !dir.join("model.bin.part").exists());
    }

    #[test]
    fn a_shorter_body_keeps_the_part_for_the_next_attempt() {
        let bytes = payload(200_000);
        let dir = temp_dir("short");
        let url = serve_bytes(Arc::new(bytes.clone()), 60_000, true, Arc::default());
        let result = run(&url, &dir, &bytes, &Options::resumable(), &AtomicBool::new(false), &|_| {});
        assert!(matches!(result, Err(DownloadError::Interrupted(_))), "{result:?}");
        assert!(dir.join("model.bin.part").exists());
        assert!(!dir.join("model.bin").exists());
    }

    #[test]
    fn a_complete_part_is_hashed_and_renamed_without_a_request() {
        let bytes = payload(80_000);
        let dir = temp_dir("completepart");
        std::fs::write(dir.join("model.bin.part"), &bytes).unwrap();
        // Nothing listens here: any request would fail.
        run("http://127.0.0.1:1/model.bin", &dir, &bytes, &Options::resumable(), &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), bytes);
        assert!(!dir.join("model.bin.part").exists());
    }

    #[test]
    fn an_existing_file_of_the_right_size_is_not_downloaded_again() {
        let bytes = payload(40_000);
        let dir = temp_dir("exists");
        std::fs::write(dir.join("model.bin"), &bytes).unwrap();
        let ranges: Ranges = Arc::default();
        let url = serve_bytes(Arc::new(bytes.clone()), 40_000, true, ranges.clone());
        run(&url, &dir, &bytes, &Options::resumable(), &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(ranges.lock().unwrap().len(), 0, "no request was made");
    }

    #[test]
    fn cancel_keeps_the_part_and_stops_between_chunks() {
        let bytes = payload(3_000_000);
        let dir = temp_dir("cancel");
        let url = serve_bytes(Arc::new(bytes.clone()), 3_000_000, true, Arc::default());
        let cancel = AtomicBool::new(false);
        let result = run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|got| {
            if got > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        assert_eq!(result, Err(DownloadError::Cancelled));
        let kept = std::fs::metadata(dir.join("model.bin.part")).unwrap().len();
        assert!(kept > 0 && kept < 3_000_000, "kept {kept}");
        assert!(!dir.join("model.bin").exists());
    }

    #[test]
    fn progress_reports_bytes_on_disk_including_the_resumed_prefix() {
        let bytes = payload(300_000);
        let dir = temp_dir("progress");
        let url = serve_bytes(Arc::new(bytes.clone()), 100_000, true, Arc::default());
        let cancel = AtomicBool::new(false);
        let _ = run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|_| {});
        let kept = std::fs::metadata(dir.join("model.bin.part")).unwrap().len();
        let seen = Mutex::new(Vec::new());
        run(&url, &dir, &bytes, &Options::resumable(), &cancel, &|got| seen.lock().unwrap().push(got)).unwrap();
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen[0], kept, "the first value is the prefix, never 0");
        assert_eq!(*seen.last().unwrap(), 300_000);
    }

    #[test]
    fn one_shot_never_sends_a_range_header_and_uses_its_own_part_name() {
        let bytes = Arc::new(payload(200_000));
        let dir = temp_dir("oneshot");
        let ranges: Ranges = Arc::default();
        let url = serve_bytes(bytes.clone(), 200_000, true, ranges.clone());
        // A stale shared part file must not be picked up.
        std::fs::write(dir.join("model.bin.part"), b"stale").unwrap();
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (url, dir, bytes) = (url.clone(), dir.clone(), bytes.clone());
                std::thread::spawn(move || run(&url, &dir, &bytes, &Options::one_shot(), &AtomicBool::new(false), &|_| {}))
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert!(ranges.lock().unwrap().iter().all(|r| r.is_none()));
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), *bytes);
        assert_eq!(std::fs::read(dir.join("model.bin.part")).unwrap(), b"stale");
    }

    #[test]
    fn a_non_success_status_is_a_failure_without_the_url_in_the_text() {
        for status in [404, 500] {
            let dir = temp_dir(&format!("status{status}"));
            let url = serve_status(status);
            let result = run(&url, &dir, b"x", &Options::resumable(), &AtomicBool::new(false), &|_| {});
            let Err(error @ DownloadError::Failed(_)) = result else { panic!("{result:?}") };
            for text in [error.to_string(), format!("{error:?}")] {
                assert!(!text.contains("secret") && !text.contains("token=") && !text.contains("127.0.0.1"), "{text}");
            }
        }
    }

    #[test]
    fn the_error_texts_are_the_ones_the_app_already_knows() {
        assert_eq!(DownloadError::Cancelled.to_string(), "Download cancelled.");
        assert_eq!(DownloadError::Interrupted("f.bin is incomplete.".into()).to_string(), "Download interrupted: f.bin is incomplete.");
        assert_eq!(DownloadError::TooLarge("f.bin".into()).to_string(), "f.bin exceeded its declared size; the download was stopped.");
        assert_eq!(DownloadError::WrongHash("f.bin".into()).to_string(), "f.bin failed its integrity check; nothing was installed.");
        assert_eq!(DownloadError::NoSpace.to_string(), "Not enough free disk space. The operation is paused; no mail was removed.");
        assert_eq!(DownloadError::Failed("boom".into()).to_string(), "Download failed: boom");
        assert_eq!(String::from(DownloadError::Cancelled), "Download cancelled.");
    }

    // --- Task 2: mirror, token, manual redirects -------------------------------------------

    #[derive(Clone)]
    enum Reply {
        Bytes(Arc<Vec<u8>>),
        Redirect(String),
        Status(u16),
    }

    #[derive(Clone, Debug)]
    struct Seen {
        path: String,
        authorization: Option<String>,
    }

    struct Fake {
        url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Fake {
        fn hits(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
        fn authorizations(&self) -> Vec<Option<String>> {
            self.seen.lock().unwrap().iter().map(|s| s.authorization.clone()).collect()
        }
        fn port(&self) -> String {
            self.url.rsplit(':').next().unwrap().to_owned()
        }
    }

    /// A loopback server answering each request with `reply(path, index)`.
    fn fake(reply: impl Fn(&str, usize) -> Reply + Send + 'static) -> Fake {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut authorization = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line.trim_end().is_empty() {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        authorization = Some(line.split_once(':').unwrap().1.trim().to_owned());
                    }
                }
                log.lock().unwrap().push(Seen { path: path.clone(), authorization });
                match reply(&path, index) {
                    Reply::Bytes(bytes) => {
                        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).as_bytes());
                        let _ = stream.write_all(&bytes);
                    }
                    Reply::Redirect(to) => {
                        let _ = stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
                    }
                    Reply::Status(code) => {
                        let _ = stream.write_all(format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
                    }
                }
                let _ = stream.flush();
            }
        });
        Fake { url: format!("http://127.0.0.1:{port}"), seen }
    }

    fn bytes_reply(bytes: &[u8]) -> impl Fn(&str, usize) -> Reply + Send + 'static + use<> {
        let bytes = Arc::new(bytes.to_vec());
        move |_, _| Reply::Bytes(bytes.clone())
    }

    fn token(value: &str) -> HfToken {
        HfToken::new(value).unwrap()
    }

    fn run_with(url: &str, dir: &Path, bytes: &[u8], options: &Options) -> Result<(), DownloadError> {
        run(url, dir, bytes, options, &AtomicBool::new(false), &|_| {})
    }

    #[test]
    fn the_mirror_replaces_the_host_and_keeps_the_path() {
        let mirror = Mirror::parse("https://mirror.example.com/hf/").unwrap();
        assert_eq!(
            mirror.map("https://huggingface.co/a/b/resolve/rev/f.gguf").as_deref(),
            Some("https://mirror.example.com/hf/a/b/resolve/rev/f.gguf")
        );
        assert_eq!(
            mirror.map("https://github.com/x/y/releases/download/t/z.zip").as_deref(),
            Some("https://mirror.example.com/hf/x/y/releases/download/t/z.zip")
        );
        assert_eq!(mirror.host(), "mirror.example.com");
    }

    #[test]
    fn a_url_of_another_host_is_not_mirrored() {
        let server = fake(bytes_reply(b"x"));
        let mirror = Mirror::parse(&server.url).unwrap();
        for url in ["https://example.com/a/b", "https://huggingface.co.evil.com/a", "https://evilhuggingface.co/a", "http://huggingface.co/a"] {
            assert_eq!(mirror.map(url), None, "{url}");
        }
        let dir = temp_dir("notmirrored");
        let options = Options { mirror: Some(mirror), ..Options::resumable() };
        let result = run_with("https://example.com/model.bin", &dir, b"x", &options);
        assert!(matches!(result, Err(DownloadError::Failed(_))), "{result:?}");
        assert_eq!(server.hits(), 0);
    }

    #[test]
    fn the_mirror_refuses_credentials_query_fragment_and_public_http() {
        for value in [
            "https://u:p@h.example.com",
            "https://h.example.com/?a=b",
            "https://h.example.com/p?a=b",
            "https://h.example.com/#x",
            "http://example.com",
            "http://8.8.8.8",
            "ftp://h.example.com",
            "file:///etc",
            "https://h .example.com",
            "https://",
            "",
            "   ",
        ] {
            assert!(Mirror::parse(value).is_err(), "{value:?} must be refused");
        }
    }

    #[test]
    fn the_mirror_accepts_internal_http() {
        for value in [
            "http://127.0.0.1:9",
            "http://10.0.0.5",
            "http://172.16.3.4",
            "http://192.168.1.2:8080",
            "http://[fd00::1]",
            "http://[::1]:80",
            "http://localhost:3000",
            "http://miospecchio:8080/modelli",
            "https://mirror.example.com",
        ] {
            assert!(Mirror::parse(value).is_ok(), "{value:?} must be accepted");
        }
        assert!(Mirror::parse("http://172.32.0.1").is_err(), "172.32 is public");
    }

    #[test]
    fn a_mirror_serving_other_bytes_is_refused() {
        let wanted = payload(50_000);
        let same_length: Vec<u8> = wanted.iter().map(|b| b ^ 1).collect();
        let shorter = wanted[..30_000].to_vec();
        for (name, served) in [("same", same_length), ("shorter", shorter)] {
            let server = fake(bytes_reply(&served));
            let dir = temp_dir(&format!("othermirror-{name}"));
            let options = Options { mirror: Mirror::parse(&server.url).ok(), ..Options::resumable() };
            let result = run_with("https://huggingface.co/a/b/resolve/rev/model.bin", &dir, &wanted, &options);
            assert!(result.is_err(), "{name}: {result:?}");
            if name == "same" {
                assert_eq!(result, Err(DownloadError::WrongHash("model.bin".into())));
            }
            assert!(!dir.join("model.bin").exists(), "{name}: nothing installed");
            assert_eq!(server.seen.lock().unwrap()[0].path, "/a/b/resolve/rev/model.bin");
        }
    }

    #[test]
    fn the_hf_token_goes_only_to_huggingface_co() {
        let bytes = payload(20_000);
        let gated = |host: &'static str| Options { gated: true, token: Some(token("hf_TESTTOKEN")), token_host: host, ..Options::resumable() };
        let server = fake(bytes_reply(&bytes));
        run_with(&format!("{}/f.bin", server.url), &temp_dir("tok-ok"), &bytes, &gated("127.0.0.1")).unwrap();
        assert_eq!(server.authorizations(), vec![Some("Bearer hf_TESTTOKEN".to_owned())]);

        let other = fake(bytes_reply(&bytes));
        run_with(&format!("{}/f.bin", other.url), &temp_dir("tok-host"), &bytes, &gated("huggingface.co")).unwrap();
        assert_eq!(other.authorizations(), vec![None], "another host never gets the token");

        let free = fake(bytes_reply(&bytes));
        let not_gated = Options { gated: false, token: Some(token("hf_TESTTOKEN")), token_host: "127.0.0.1", ..Options::resumable() };
        run_with(&format!("{}/f.bin", free.url), &temp_dir("tok-free"), &bytes, &not_gated).unwrap();
        assert_eq!(free.authorizations(), vec![None], "an entry that is not gated never sends it");
    }

    #[test]
    fn the_token_is_not_sent_after_a_redirect_to_another_host() {
        let bytes = payload(20_000);
        let cdn = fake(bytes_reply(&bytes));
        let target = format!("http://localhost:{}/cdn/f.bin", cdn.port());
        let hf = fake(move |_, _| Reply::Redirect(target.clone()));
        let dir = temp_dir("tok-redirect");
        let options = Options { gated: true, token: Some(token("hf_TESTTOKEN")), token_host: "127.0.0.1", ..Options::resumable() };
        run_with(&format!("{}/f.bin", hf.url), &dir, &bytes, &options).unwrap();
        assert_eq!(hf.authorizations(), vec![Some("Bearer hf_TESTTOKEN".to_owned())]);
        assert_eq!(cdn.authorizations(), vec![None], "the redirect target never sees the token");
        assert_eq!(std::fs::read(dir.join("model.bin")).unwrap(), bytes);
    }

    #[test]
    fn the_token_is_not_sent_to_the_mirror() {
        let bytes = payload(20_000);
        let server = fake(bytes_reply(&bytes));
        let options = Options {
            gated: true,
            token: Some(token("hf_TESTTOKEN")),
            token_host: "127.0.0.1",
            mirror: Mirror::parse(&server.url).ok(),
            ..Options::resumable()
        };
        run_with("https://huggingface.co/a/b/resolve/rev/f.bin", &temp_dir("tok-mirror"), &bytes, &options).unwrap();
        assert_eq!(server.authorizations(), vec![None]);
    }

    #[test]
    fn a_gated_entry_without_a_token_fails_before_any_request() {
        let server = fake(bytes_reply(b"x"));
        let options = Options { gated: true, token: None, token_host: "127.0.0.1", ..Options::resumable() };
        let result = run_with(&format!("{}/f.bin", server.url), &temp_dir("gated-no-token"), b"x", &options);
        assert!(matches!(result, Err(DownloadError::Failed(_))), "{result:?}");
        assert_eq!(server.hits(), 0);
    }

    #[test]
    fn the_token_is_never_in_an_error_or_a_debug() {
        const SECRET: &str = "hf_SECRETTOKEN0123";
        let unauthorized = fake(|_, _| Reply::Status(401));
        let missing = fake(|_, _| Reply::Status(404));
        let options = Options { gated: true, token: Some(token(SECRET)), token_host: "127.0.0.1", ..Options::resumable() };
        for url in [format!("{}/f.bin", unauthorized.url), format!("{}/f.bin", missing.url), "http://127.0.0.1:1/f.bin".to_owned()] {
            let result = run_with(&url, &temp_dir("tok-secret"), b"x", &options);
            let error = result.unwrap_err();
            for text in [error.to_string(), format!("{error:?}"), format!("{options:?}"), format!("{:?}", options.token)] {
                assert!(!text.contains(SECRET), "{text}");
            }
        }
        assert_eq!(format!("{:?}", token(SECRET)), "HfToken(<nascosto>)");
    }

    #[test]
    fn a_redirect_off_the_mirror_host_is_refused() {
        let other = fake(bytes_reply(b"x"));
        let target = format!("http://localhost:{}/f.bin", other.port());
        let mirror = fake(move |_, _| Reply::Redirect(target.clone()));
        let options = Options { mirror: Mirror::parse(&mirror.url).ok(), ..Options::resumable() };
        let result = run_with("https://huggingface.co/a/f.bin", &temp_dir("mirror-redirect"), b"x", &options);
        assert!(matches!(result, Err(DownloadError::Failed(_))), "{result:?}");
        assert_eq!(other.hits(), 0);
    }

    #[test]
    fn a_redirect_loop_stops_at_the_limit() {
        let server = fake(|_, _| Reply::Redirect("/again".to_owned()));
        let result = run_with(&format!("{}/f.bin", server.url), &temp_dir("loop"), b"x", &Options::resumable());
        assert!(matches!(result, Err(DownloadError::Failed(_))), "{result:?}");
        assert_eq!(server.hits(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn an_https_to_http_redirect_is_refused_without_a_mirror() {
        assert!(redirect_allowed("https://huggingface.co/a", "http://cdn.example.com/b", None).is_err());
        assert!(redirect_allowed("https://huggingface.co/a", "https://cdn.example.com/b", None).is_ok());
        assert!(redirect_allowed("http://127.0.0.1:1/a", "http://127.0.0.1:1/b", None).is_ok());
        let mirror = Mirror::parse("http://10.0.0.5:8080").unwrap();
        assert!(redirect_allowed("http://10.0.0.5:8080/a", "http://10.0.0.5:8080/b", Some(&mirror)).is_ok());
        assert!(redirect_allowed("http://10.0.0.5:8080/a", "http://10.0.0.6:8080/b", Some(&mirror)).is_err());
    }
}
