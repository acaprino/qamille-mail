//! The app's half of a browser sign-in on a Qamille server (spec «Vendita Professional», V3).
//!
//! A one-time local address waits for the browser; the browser opens
//! `{server}/v1/auth/{provider}/start?port&state&code_challenge`; the server does the provider's
//! part (Google's client secret never reaches this app) and sends the browser back here with a
//! single-use code or an error. The code is worth nothing without the verifier made here (PKCE,
//! RFC 7636), so another program on this computer that catches it cannot use it.
use crate::application::server_account::{BrowserTicket, ServerAddress};
use crate::application::{AppError, AppResult};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long the browser has to come back.
const WAIT: Duration = Duration::from_secs(300);
/// The longest request line read from a connection; the browser's is far shorter.
const MAX_LINE: u64 = 8 * 1024;
/// How long one connection may take to send its request line.
const LINE_DEADLINE: Duration = Duration::from_secs(5);

fn base64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if c.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if c.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

fn random_text() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AppError::new("internal", "No randomness available"))?;
    Ok(base64url(&bytes))
}

/// The start address on the server; `reauth` asks for a confirmation code (`mode=reauth`).
/// `terms` is the version of the terms the person accepted (read from `/v1/info`): it only
/// travels on a sign-in, and the server keeps it in the signed `state` of a new account.
pub fn start_url(server: &str, provider: &str, port: u16, state: &str, verifier: &str, reauth: bool, terms: Option<&str>) -> String {
    format!(
        "{}/v1/auth/{provider}/start?port={port}&state={state}&code_challenge={}{}{}",
        server.trim_end_matches('/'),
        super::oauth::pkce_challenge(verifier),
        if reauth { "&mode=reauth" } else { "" },
        terms.map(|version| format!("&terms={}", encode(version))).unwrap_or_default()
    )
}

/// Percent-encoding of a version text: letters, digits and `-._~` stay, the rest is escaped.
fn encode(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

/// Percent-decoding on bytes: any input decodes, none panics.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(high << 4 | low);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query of the request line `GET /?… HTTP/1.1`.
fn params(request_line: &str) -> HashMap<String, String> {
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map(|x| x.1).unwrap_or_default();
    query.split('&').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (k.to_string(), decode(v))).collect()
}

/// What the server's return means for this app.
pub fn outcome(params: &HashMap<String, String>, state: &str) -> AppResult<String> {
    if params.get("state").map(String::as_str) != Some(state) {
        return Err(AppError::new("invalid_code", "The browser came back from another sign-in. Try again"));
    }
    if let Some(error) = params.get("error") {
        return Err(match error.as_str() {
            "cancelled" => AppError::new("cancelled", "Sign-in was cancelled in the browser"),
            "no_account" => AppError::new("no_account", "No Qamille account uses this Google address. Ask the administrator, or create an account"),
            "access_denied" => AppError::new("access_denied", "This account cannot sign in here. Ask the administrator"),
            "email_not_verified" => AppError::new("email_not_verified", "Google did not confirm this address"),
            "link_required" => AppError::new("link_required", "An account already uses this address. Sign in with its password"),
            "terms_required" => AppError::new("terms_required", "The Terms and the privacy notice must be accepted to create the account"),
            _ => AppError::new("provider_error", "Google did not complete the sign-in. Try again"),
        });
    }
    params
        .get("code")
        .filter(|code| code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit()))
        .cloned()
        .ok_or_else(|| AppError::new("provider_error", "Google did not complete the sign-in. Try again"))
}

fn page(ok: bool) -> String {
    let text = if ok {
        "Accesso completato. Puoi chiudere questa scheda e tornare a Qamille. / Signed in: you can close this tab and go back to Qamille."
    } else {
        "L'accesso non è finito: torna a Qamille per sapere perché. / Sign-in did not finish: go back to Qamille to see why."
    };
    let body = format!("<!doctype html><meta charset=utf-8><title>Qamille</title><body style=\"font-family:sans-serif;margin:3em\">{text}");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The request line of one connection, at most [`MAX_LINE`] bytes within [`LINE_DEADLINE`].
fn request_line(stream: &TcpStream) -> Option<String> {
    let _ = stream.set_nonblocking(false);
    let deadline = Instant::now() + LINE_DEADLINE;
    let mut reader = BufReader::new(stream.take(MAX_LINE));
    let mut line = Vec::new();
    loop {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
        let _ = stream.set_read_timeout(Some(left));
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return None,
            Ok(_) if line.ends_with(b"\n") => return String::from_utf8(line).ok(),
            Ok(_) => return None,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => continue,
            Err(_) => return None,
        }
    }
}

/// Waits on `listener` for the return carrying `state`; anything else knocking is answered 404.
/// Each connection is read on its own thread with a size and time limit, so a program on this
/// computer that opens a connection and stays silent cannot hold up the browser's return.
pub fn wait_for_return(listener: &TcpListener, state: &str, cancel: &AtomicBool, wait: Duration) -> AppResult<String> {
    listener.set_nonblocking(true).map_err(|_| AppError::new("internal", "The local sign-in address could not be opened"))?;
    let deadline = Instant::now() + wait;
    let (found_tx, found_rx) = mpsc::channel::<AppResult<String>>();
    loop {
        if let Ok(result) = found_rx.try_recv() {
            return result;
        }
        if cancel.load(Ordering::Acquire) {
            return Err(AppError::new("cancelled", "Sign-in stopped"));
        }
        if Instant::now() > deadline {
            return Err(AppError::new("timeout", "No answer from the browser after 5 minutes. Try again"));
        }
        let (stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => return Err(AppError::new("internal", "The local sign-in address stopped")),
        };
        let state = state.to_string();
        let found_tx = found_tx.clone();
        std::thread::spawn(move || {
            let Some(line) = request_line(&stream) else { return };
            let found = params(&line);
            let mut writer = &stream;
            if found.get("state").map(String::as_str) != Some(state.as_str()) {
                let _ = writer.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                return;
            }
            let result = outcome(&found, &state);
            let _ = writer.write_all(page(result.is_ok()).as_bytes());
            let _ = found_tx.send(result);
        });
    }
}

/// The whole wait: a local address, the browser on the server's start page, the code back.
pub fn sign_in(address: &ServerAddress, provider: &str, reauth: bool, terms: Option<&str>, cancel: &AtomicBool) -> AppResult<BrowserTicket> {
    sign_in_with(address, provider, reauth, terms, cancel, |url| open::that(url).map_err(|_| AppError::new("browser", "The browser could not be opened")))
}

/// As [`sign_in`], with the browser injected (tests).
pub fn sign_in_with(address: &ServerAddress, provider: &str, reauth: bool, terms: Option<&str>, cancel: &AtomicBool, open: impl FnOnce(&str) -> AppResult<()>) -> AppResult<BrowserTicket> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| AppError::new("internal", "The local sign-in address could not be opened"))?;
    let port = listener.local_addr().map_err(|_| AppError::new("internal", "The local sign-in address could not be opened"))?.port();
    let (state, verifier) = (random_text()?, random_text()?);
    open(&start_url(&address.url, provider, port, &state, &verifier, reauth, terms))?;
    let code = wait_for_return(&listener, &state, cancel, WAIT)?;
    Ok(BrowserTicket { code, verifier })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_start_address_carries_only_the_challenge() {
        let url = start_url("https://q.example/", "google", 50000, "st4te", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", false, None);
        assert_eq!(url, "https://q.example/v1/auth/google/start?port=50000&state=st4te&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert!(start_url("https://q.example", "google", 50000, "s", "v", true, None).ends_with("&mode=reauth"));
        // The accepted version of the terms travels escaped, and only when there is one.
        let with = start_url("https://q.example", "google", 1, "s", "v", false, Some("2026-12-01 a&b"));
        assert!(with.ends_with("&terms=2026-12-01%20a%26b"), "{with}");
        assert!(!start_url("https://q.example", "google", 1, "s", "v", false, None).contains("terms"));
    }

    #[test]
    fn returns_are_read_and_refusals_explained() {
        let code = "a".repeat(64);
        let p = params(&format!("GET /?code={code}&state=s HTTP/1.1"));
        assert_eq!(outcome(&p, "s").unwrap(), code);
        assert_eq!(outcome(&p, "other").unwrap_err().code, "invalid_code");
        assert_eq!(outcome(&params("GET /?error=no_account&state=s HTTP/1.1"), "s").unwrap_err().code, "no_account");
        assert_eq!(outcome(&params("GET /?error=cancelled&state=s HTTP/1.1"), "s").unwrap_err().code, "cancelled");
        assert_eq!(outcome(&params("GET /?code=short&state=s HTTP/1.1"), "s").unwrap_err().code, "provider_error");
        assert_eq!(outcome(&params("GET /?error=link_required&state=s HTTP/1.1"), "s").unwrap_err().code, "link_required");
        assert_eq!(outcome(&params("GET /?error=terms_required&state=s HTTP/1.1"), "s").unwrap_err().code, "terms_required");
        // Malformed escapes, also before a multi-byte character, decode without panicking.
        assert_eq!(decode("%"), "%");
        assert_eq!(decode("%a"), "%a");
        assert_eq!(decode("%aé"), "%aé");
        assert_eq!(decode("%zz%41"), "%zzA");
    }

    #[test]
    fn only_the_return_with_our_state_counts() {
        let address = ServerAddress { url: "https://q.example".into(), ca_pem: None };
        let cancel = AtomicBool::new(false);
        let ticket = sign_in_with(&address, "google", false, None, &cancel, |url| {
            let port: u16 = url.split("port=").nth(1).unwrap().split('&').next().unwrap().parse().unwrap();
            let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string();
            std::thread::spawn(move || {
                // A program that connects and stays silent does not hold up the browser.
                let _silent = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                // Something else knocks first, then the browser comes back.
                let mut other = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                other.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
                let mut answer = String::new();
                let _ = other.read_to_string(&mut answer);
                assert!(answer.starts_with("HTTP/1.1 404"));
                let mut browser = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(browser, "GET /?code={}&state={state} HTTP/1.1\r\n\r\n", "b".repeat(64)).unwrap();
                let mut answer = String::new();
                let _ = browser.read_to_string(&mut answer);
                assert!(answer.contains("Accesso completato"));
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(ticket.code, "b".repeat(64));
        assert!(ticket.verifier.len() >= 43);
        assert!(!format!("{ticket:?}").contains(&ticket.code));
    }

    #[test]
    fn cancel_stops_the_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(wait_for_return(&listener, "s", &cancel, WAIT).unwrap_err().code, "cancelled");
    }
}
