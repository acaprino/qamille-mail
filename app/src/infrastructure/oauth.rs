//! Google sign-in with OAuth 2.0 for desktop apps: the browser opens Google's page, Google returns
//! to a one-time local address, and the code is exchanged using PKCE. Qamille never sees the
//! Google password. Only the refresh token is stored, in the portable credential vault.

use crate::model::Account;
use crate::secret;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
/// The mail and address scopes of the mail sign-in. Calendar and contacts are asked separately
/// (`CALENDAR_SCOPE`, the contacts scope of `google_people`), each with its own consent.
pub const SCOPE: &str = "https://mail.google.com https://www.googleapis.com/auth/userinfo.email";
/// Calendar consent is requested explicitly, separately from mail sign-in.
pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.freebusy";
pub const CONSOLE_URL: &str = "https://console.cloud.google.com/apis/credentials";
pub const CLIENT_SECRET_KEY: &str = "google-client-secret";
const WAIT_SECS: u64 = 300; // estimated
const REFRESH_MARGIN_SECS: u64 = 60; // estimated

#[derive(Clone, Default, PartialEq)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires_in: u64,
    /// The permissions Google really granted, which can be fewer than were asked for.
    pub scope: String,
}

/// The tokens are secrets: `Debug` shows only that they exist.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("access", &"<hidden>")
            .field("refresh", &"<hidden>")
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Google lets people untick single permissions, so a finished sign-in may hold less than was asked for.
/// A reply that lists nothing cannot be judged. The address permission is skipped because Google
/// may spell it `email`.
fn check_granted(requested: &str, granted: &str) -> Result<(), String> {
    if granted.trim().is_empty() {
        return Ok(());
    }
    let norm = |s: &str| s.trim_end_matches('/').to_string();
    let have: Vec<String> = granted.split_whitespace().map(norm).collect();
    let missing: Vec<&str> = requested
        .split_whitespace()
        .filter(|s| !s.ends_with("userinfo.email"))
        .filter(|s| !have.contains(&norm(s)))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(if missing.iter().any(|s| s.starts_with("https://mail.google.com")) {
        "You didn't give Qamille access to Gmail. Try again and tick the Gmail box on Google's permissions page.".to_string()
    } else {
        "Google didn't grant every permission Qamille asked for. Try again and tick all the boxes on Google's permissions page.".to_string()
    })
}

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

pub fn pkce_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn random_text() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(base64url(&b))
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%').then(|| s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match (hex, b[i]) {
            (Some(v), _) => {
                out.push(v);
                i += 3;
            }
            (None, b'+') => {
                out.push(b' ');
                i += 1;
            }
            (None, c) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn auth_url(client_id: &str, redirect: &str, state: &str, challenge: &str) -> String {
    auth_url_with_scope(client_id, redirect, state, challenge, SCOPE, "")
}

/// `login_hint` makes Google's page start on that account instead of the browser's current one.
pub fn auth_url_with_scope(client_id: &str, redirect: &str, state: &str, challenge: &str, scope: &str, login_hint: &str) -> String {
    let hint = match login_hint.trim() {
        "" => String::new(),
        h => format!("&login_hint={}", encode(h)),
    };
    format!(
        "{AUTH_URL}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent{hint}",
        encode(client_id),
        encode(redirect),
        encode(scope),
        encode(state),
        encode(challenge)
    )
}

/// The query of a request line, as name/value pairs.
fn redirect_params(request_line: &str) -> Result<HashMap<String, String>, String> {
    let target = request_line.split_whitespace().nth(1).ok_or("Google's reply couldn't be read.")?;
    let query = target.split_once('?').map(|x| x.1).unwrap_or("");
    Ok(query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), decode(v))).collect())
}

/// Reads Google's return request line. Answers with the code, or says why there is none.
pub fn parse_redirect(request_line: &str, state: &str) -> Result<String, String> {
    let params = redirect_params(request_line)?;
    if params.get("state").map(String::as_str) != Some(state) {
        return Err("The reply didn't match this sign-in, so it was ignored. Please try again.".into());
    }
    if let Some(e) = params.get("error") {
        return Err(if e == "access_denied" { "Sign-in was cancelled in the browser, or this address isn't a test user of your Google project.".to_string() } else { format!("Google said: {e}") });
    }
    params.get("code").filter(|c| !c.is_empty()).cloned().ok_or("Google's reply had no sign-in code.".into())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).http_status_as_error(false).build().into()
}

fn parse_tokens(status: u16, v: &Value) -> Result<Tokens, String> {
    if status >= 400 || v["access_token"].as_str().is_none() {
        let code = v["error"].as_str().unwrap_or("unknown_error");
        let text = v["error_description"].as_str().unwrap_or("");
        return Err(match code {
            "invalid_grant" => "Google no longer accepts this sign-in. Please sign in with Google again. With a project in Testing mode this happens every 7 days.".to_string(),
            "invalid_client" => "Google couldn't verify this version of Qamille. Contact the app developer.".to_string(),
            _ => format!("Google refused the sign-in: {code} {text}"),
        });
    }
    Ok(Tokens {
        access: v["access_token"].as_str().unwrap_or("").to_string(),
        refresh: v["refresh_token"].as_str().unwrap_or("").to_string(),
        expires_in: v["expires_in"].as_u64().unwrap_or(3600),
        scope: v["scope"].as_str().unwrap_or("").to_string(),
    })
}

fn post_token(url: &str, form: &[(&str, &str)]) -> Result<Tokens, String> {
    let mut resp = agent().post(url).send_form(form.iter().copied()).map_err(|e| format!("Couldn't reach Google: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp.body_mut().read_json().map_err(|e| format!("Google's reply couldn't be read: {e}"))?;
    parse_tokens(status, &v)
}

pub fn exchange_code(url: &str, client_id: &str, client_secret: &str, code: &str, verifier: &str, redirect: &str) -> Result<Tokens, String> {
    post_token(
        url,
        &[("grant_type", "authorization_code"), ("client_id", client_id), ("client_secret", client_secret), ("code", code), ("code_verifier", verifier), ("redirect_uri", redirect)],
    )
}

pub fn refresh(url: &str, client_id: &str, client_secret: &str, refresh_token: &str) -> Result<Tokens, String> {
    post_token(url, &[("grant_type", "refresh_token"), ("client_id", client_id), ("client_secret", client_secret), ("refresh_token", refresh_token)])
}

fn user_email(access: &str) -> Result<String, String> {
    let mut resp = agent().get(USERINFO_URL).header("Authorization", format!("Bearer {access}")).call().map_err(|e| format!("Couldn't reach Google: {e}"))?;
    let v: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
    v["email"].as_str().map(String::from).ok_or("Google didn't share the account's address.".into())
}

/// The longest request line read from a connection; the browser's is far shorter.
const MAX_LINE: u64 = 8 * 1024;
/// How long one connection has to send its request line.
const LINE_DEADLINE: Duration = Duration::from_secs(5);

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

/// Waits for the browser to come back to the local address. Returns the code. Each connection is
/// read on its own thread with a size and time limit, so a program on this computer that opens a
/// connection and stays silent (or sends endless bytes) cannot hold up the real return.
fn wait_for_code(listener: &TcpListener, state: &str, stopped: &dyn Fn() -> bool) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(WAIT_SECS);
    let (found_tx, found_rx) = std::sync::mpsc::channel::<Result<String, String>>();
    loop {
        if let Ok(result) = found_rx.try_recv() {
            return result;
        }
        if stopped() {
            return Err("Sign-in stopped.".into());
        }
        if Instant::now() > deadline {
            return Err("No answer from the browser after 5 minutes. Please try again.".into());
        }
        let (stream, _) = match listener.accept() {
            Ok(x) => x,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let state = state.to_string();
        let found_tx = found_tx.clone();
        std::thread::spawn(move || {
            let Some(line) = request_line(&stream) else { return };
            // Browsers also ask for an icon, and anything else on this computer can knock on the
            // port. Only a request carrying this sign-in's own state counts; the rest is answered
            // and ignored.
            let ours = redirect_params(&line).is_ok_and(|p| p.get("state").map(String::as_str) == Some(state.as_str()));
            let mut w = &stream;
            if !ours {
                let _ = w.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                return;
            }
            let result = parse_redirect(&line, &state);
            let page = if result.is_ok() { "You're signed in. You can close this tab and go back to Qamille." } else { "Sign-in didn't finish. Go back to Qamille to see why." };
            let body = format!("<!doctype html><meta charset=utf-8><title>Qamille</title><body style=\"font-family:sans-serif;margin:3em\">{page}");
            let _ = w.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            let _ = found_tx.send(result);
        });
    }
}

/// The whole sign-in: browser, return, tokens, address.
pub fn sign_in(client_id: &str, client_secret: &str, cancel: &AtomicBool, login_hint: &str) -> Result<(String, Tokens), String> {
    sign_in_with_scope(client_id, client_secret, cancel, SCOPE, login_hint)
}

fn with_sign_in_guard<T>(busy: &AtomicBool, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Google sign-in is already in progress. Complete it in your browser.".to_string())?;
    struct Release<'a>(&'a AtomicBool);
    impl Drop for Release<'_> {
        fn drop(&mut self) { self.0.store(false, Ordering::Release); }
    }
    let _release = Release(busy);
    action()
}

/// One Google sign-in at a time, and a way for the app to stop it while it waits for the browser.
struct SignInControl {
    active: AtomicBool,
    cancel: AtomicBool,
}
impl SignInControl {
    const fn new() -> Self {
        Self { active: AtomicBool::new(false), cancel: AtomicBool::new(false) }
    }
    /// Runs `action` while holding the single sign-in slot. `action` gets the flag `cancel` raises.
    fn run<T>(&self, action: impl FnOnce(&AtomicBool) -> Result<T, String>) -> Result<T, String> {
        with_sign_in_guard(&self.active, || {
            self.cancel.store(false, Ordering::Release);
            action(&self.cancel)
        })
    }
    /// Stops the sign-in that is running; does nothing when none is, so it can't spoil the next one.
    fn cancel(&self) {
        if self.active.load(Ordering::Acquire) {
            self.cancel.store(true, Ordering::Release);
        }
    }
}
static SIGN_IN: SignInControl = SignInControl::new();

/// Stops the Google sign-in that is waiting for the browser, if there is one.
pub fn cancel_sign_in() {
    SIGN_IN.cancel();
}

pub fn sign_in_with_scope(client_id: &str, client_secret: &str, cancel: &AtomicBool, scope: &str, login_hint: &str) -> Result<(String, Tokens), String> {
    SIGN_IN.run(|cancelled| {
        let stopped = || cancel.load(Ordering::Relaxed) || cancelled.load(Ordering::Relaxed);
        sign_in_flow(client_id, client_secret, &stopped, scope, login_hint)
    })
}

fn sign_in_flow(client_id: &str, client_secret: &str, stopped: &dyn Fn() -> bool, scope: &str, login_hint: &str) -> Result<(String, Tokens), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let (state, verifier) = (random_text()?, random_text()?);
    open::that(auth_url_with_scope(client_id, &redirect, &state, &pkce_challenge(&verifier), scope, login_hint)).map_err(|e| format!("Couldn't open the browser: {e}"))?;
    let code = wait_for_code(&listener, &state, stopped)?;
    let tokens = exchange_code(TOKEN_URL, client_id, client_secret, &code, &verifier, &redirect)?;
    check_granted(scope, &tokens.scope)?;
    if tokens.refresh.is_empty() {
        return Err("Google didn't grant lasting access. Please try again.".into());
    }
    let email = user_email(&tokens.access)?;
    // A new authorization replaces the old one: an access token cached for this address belongs
    // to the previous refresh token.
    forget_access_token(&email);
    Ok((email, tokens))
}

/// The access tokens kept until shortly before they run out, by lower-case address.
static ACCESS_CACHE: Mutex<Option<HashMap<String, (String, Instant)>>> = Mutex::new(None);

/// Drops the cached access token of `email`: the account was removed or authorized again.
pub fn forget_access_token(email: &str) {
    if let Ok(mut g) = ACCESS_CACHE.lock() {
        if let Some(map) = g.as_mut() {
            map.remove(&email.trim().to_lowercase());
        }
    }
}

/// The access token kept for an address is no longer good (its refresh token changed or was revoked).
pub fn forget_access(email: &str) {
    forget_access_token(email);
}

pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// Asks Google to revoke a token (and every access derived from it). A token Google already does
/// not know counts as revoked. The token is never in the error; no redirect is followed.
pub fn revoke(url: &str, token: &str) -> Result<(), String> {
    const FAILED: &str = "Google did not revoke the access";
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).max_redirects(0).http_status_as_error(false).build().into();
    let mut response = agent.post(url).send_form([("token", token)]).map_err(|_| FAILED.to_string())?;
    match response.status().as_u16() {
        200 => Ok(()),
        400 => {
            let body: Value = response.body_mut().with_config().limit(1 << 16).read_json().unwrap_or(Value::Null);
            if body["error"] == "invalid_token" { Ok(()) } else { Err(FAILED.into()) }
        }
        _ => Err(FAILED.into()),
    }
}

/// A valid access token for an account, renewed when it is about to run out.
pub fn access_token(a: &Account) -> Result<String, String> {
    let key = a.email.trim().to_lowercase();
    if let Ok(g) = ACCESS_CACHE.lock() {
        if let Some((t, until)) = g.as_ref().and_then(|m| m.get(&key)) {
            if Instant::now() < *until {
                return Ok(t.clone());
            }
        }
    }
    if a.password.is_empty() {
        return Err("Please sign in with Google again.".into());
    }
    let client_secret = super::google_oauth_config::secret_for_client(&a.oauth_client_id, &secret::get(CLIENT_SECRET_KEY));
    let tokens = refresh(TOKEN_URL, &a.oauth_client_id, &client_secret, &a.password)?;
    let until = Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(REFRESH_MARGIN_SECS));
    if let Ok(mut g) = ACCESS_CACHE.lock() {
        g.get_or_insert_with(HashMap::new).insert(key, (tokens.access.clone(), until));
    }
    Ok(tokens.access)
}

/// What IMAP and SMTP sign-in with OAuth send to Google.
pub fn xoauth2_string(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn pkce_matches_the_standard_example() {
        // RFC 7636 appendix B.
        assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn sign_in_address_asks_for_lasting_mail_access() {
        let u = auth_url("id.apps", "http://127.0.0.1:5000", "st", "ch");
        assert!(u.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(u.contains("client_id=id.apps"));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5000"));
        assert!(u.contains("https%3A%2F%2Fmail.google.com"));
        assert!(u.contains("access_type=offline"));
        assert!(u.contains("code_challenge=ch&code_challenge_method=S256"));
    }

    #[test]
    fn the_browser_is_told_which_account_to_use() {
        let u = auth_url_with_scope("id.apps", "http://127.0.0.1:5000", "st", "ch", SCOPE, " a.caprino@tri-tech.it ");
        assert!(u.contains("&login_hint=a.caprino%40tri-tech.it"), "{u}");
        assert!(!auth_url_with_scope("id.apps", "http://127.0.0.1:5000", "st", "ch", SCOPE, "  ").contains("login_hint"));
        assert!(!auth_url("id.apps", "http://127.0.0.1:5000", "st", "ch").contains("login_hint"));
    }

    #[test]
    fn return_address_is_checked() {
        assert_eq!(parse_redirect("GET /?state=abc&code=4%2F0Ax&scope=x HTTP/1.1", "abc").unwrap(), "4/0Ax");
        assert!(parse_redirect("GET /?state=other&code=x HTTP/1.1", "abc").unwrap_err().contains("didn't match"));
        assert!(parse_redirect("GET /?state=abc&error=access_denied HTTP/1.1", "abc").unwrap_err().contains("cancelled"));
    }

    #[test]
    fn the_permissions_google_granted_are_kept() {
        let t = parse_tokens(200, &serde_json::json!({"access_token":"AT","refresh_token":"RT","scope":"https://www.googleapis.com/auth/userinfo.email openid"})).unwrap();
        assert_eq!(t.scope, "https://www.googleapis.com/auth/userinfo.email openid");
    }

    #[test]
    fn a_sign_in_without_the_gmail_box_is_refused_with_the_reason() {
        let e = check_granted(SCOPE, "https://www.googleapis.com/auth/userinfo.email").unwrap_err();
        assert!(e.contains("Gmail") && e.contains("tick"), "{e}");
    }

    #[test]
    fn a_full_sign_in_passes_however_google_spells_the_scopes() {
        assert!(check_granted(SCOPE, "https://mail.google.com/ https://www.googleapis.com/auth/userinfo.email").is_ok());
        assert!(check_granted(SCOPE, "email https://mail.google.com").is_ok(), "the short name for the address permission must not count as missing");
    }

    #[test]
    fn a_reply_that_lists_no_permissions_cannot_be_judged() {
        assert!(check_granted(SCOPE, "").is_ok());
    }

    #[test]
    fn any_other_missing_permission_is_named_generically() {
        let e = check_granted(CALENDAR_SCOPE, "https://www.googleapis.com/auth/userinfo.email").unwrap_err();
        assert!(e.contains("every permission") && !e.contains("Gmail"), "{e}");
    }

    #[test]
    fn token_errors_say_what_to_do() {
        let e = parse_tokens(400, &serde_json::json!({"error": "invalid_grant"})).unwrap_err();
        assert!(e.contains("sign in with Google again"));
        assert!(parse_tokens(401, &serde_json::json!({"error": "invalid_client"})).unwrap_err().contains("Contact the app developer"));
    }

    /// A one-request stand-in for Google's token address.
    fn fake_token_server(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/token", l.local_addr().unwrap().port());
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let t = String::from_utf8_lossy(&got).to_string();
                if let Some(p) = t.find("\r\n\r\n") {
                    let len: usize = t.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                    if got.len() >= p + 4 + len {
                        break;
                    }
                }
            }
            write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (url, h)
    }

    #[test]
    fn code_is_exchanged_with_its_proof() {
        let (url, h) = fake_token_server(r#"{"access_token":"AT","refresh_token":"RT","expires_in":3599}"#);
        let t = exchange_code(&url, "cid", "sec", "the/code", "verif", "http://127.0.0.1:9").unwrap();
        assert_eq!(t, Tokens { access: "AT".into(), refresh: "RT".into(), expires_in: 3599, scope: String::new() });
        let sent = h.join().unwrap();
        assert!(sent.contains("grant_type=authorization_code"));
        assert!(sent.contains("code_verifier=verif"));
        assert!(sent.contains("client_secret=sec"));
    }

    #[test]
    fn browser_return_reaches_the_app() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            let mut icon = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            icon.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /?state=abc&code=C1 HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut page = String::new();
            s.read_to_string(&mut page).unwrap();
            page
        });
        assert_eq!(wait_for_code(&l, "abc", &|| false).unwrap(), "C1");
        assert!(browser.join().unwrap().contains("signed in"));
    }

    #[test]
    fn a_stalled_or_endless_connection_does_not_hold_up_the_real_return() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            // Silent: never sends a byte.
            let _silent = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            // Endless: more than the limit with no end of line.
            let mut endless = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let _ = endless.write_all(&vec![b'a'; 32 * 1024]);
            let started = Instant::now();
            let mut real = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            real.write_all(b"GET /?state=abc&code=C1 HTTP/1.1\r\n\r\n").unwrap();
            let mut page = String::new();
            real.read_to_string(&mut page).unwrap();
            (started.elapsed(), page)
        });
        assert_eq!(wait_for_code(&l, "abc", &|| false).unwrap(), "C1");
        let (elapsed, page) = browser.join().unwrap();
        assert!(page.contains("signed in"), "{page}");
        assert!(elapsed < Duration::from_secs(3), "the stalled connections held the return for {elapsed:?}");
    }

    #[test]
    fn a_request_line_is_capped() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let client = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            let _ = s.write_all(&vec![b'a'; 64 * 1024]);
            std::thread::sleep(Duration::from_millis(300));
        });
        let (stream, _) = l.accept().unwrap();
        assert!(request_line(&stream).is_none(), "a line over the limit is dropped");
        client.join().unwrap();
    }

    #[test]
    fn tokens_never_show_in_debug() {
        let t = Tokens { access: "ACCESS-SECRET".into(), refresh: "REFRESH-SECRET".into(), expires_in: 5, scope: "mail".into() };
        let shown = format!("{t:?} {t:#?}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("Tokens"));
    }

    #[test]
    fn a_cached_access_token_can_be_forgotten() {
        let email = "Cache.Test@Example.com";
        ACCESS_CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(email.to_lowercase(), ("AT".into(), Instant::now() + Duration::from_secs(600)));
        let account = Account { email: email.into(), ..Default::default() };
        assert_eq!(access_token(&account).unwrap(), "AT");
        forget_access_token(" cache.test@example.com ");
        // Nothing cached and no refresh token to renew it with.
        assert!(access_token(&account).is_err());
    }

    #[test]
    fn stopping_ends_the_wait() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(wait_for_code(&l, "abc", &|| true).unwrap_err().contains("stopped"));
    }

    #[test]
    fn a_stray_request_does_not_end_the_wait() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            let mut stray = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stray.write_all(b"GET /?state=someone-else&code=X HTTP/1.1

Host: x



").unwrap();
            let mut stray_page = String::new();
            stray.read_to_string(&mut stray_page).unwrap();
            let mut real = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            real.write_all(b"GET /?state=abc&code=C1 HTTP/1.1

Host: x



").unwrap();
            let mut page = String::new();
            real.read_to_string(&mut page).unwrap();
            (stray_page, page)
        });
        assert_eq!(wait_for_code(&l, "abc", &|| false).unwrap(), "C1");
        let (stray_page, page) = browser.join().unwrap();
        assert!(!stray_page.contains("signed in"), "{stray_page}");
        assert!(page.contains("signed in"), "{page}");
    }

    #[test]
    fn cancelling_stops_a_sign_in_that_is_waiting_for_the_browser() {
        let control = SignInControl::new();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let result = std::thread::scope(|scope| {
            let waiting = scope.spawn(|| control.run(|cancelled| wait_for_code(&l, "abc", &|| cancelled.load(Ordering::Relaxed))));
            while !control.active.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
            control.cancel();
            waiting.join().unwrap()
        });
        assert!(result.unwrap_err().contains("stopped"));
        assert!(!control.active.load(Ordering::Acquire), "the lock is free for the next sign-in");
    }

    #[test]
    fn a_cancel_with_nothing_running_does_not_spoil_the_next_sign_in() {
        let control = SignInControl::new();
        control.cancel();
        let cancelled = control.run(|cancelled| Ok::<bool, String>(cancelled.load(Ordering::Relaxed))).unwrap();
        assert!(!cancelled);
    }

    #[test]
    fn concurrent_sign_in_is_rejected_and_guard_releases_after_failure() {
        let busy = AtomicBool::new(false);
        let error = with_sign_in_guard(&busy, || {
            let second: Result<(), String> = with_sign_in_guard(&busy, || panic!("second sign-in must not start"));
            assert_eq!(second.unwrap_err(), "Google sign-in is already in progress. Complete it in your browser.");
            Err::<(), _>("cancelled".into())
        }).unwrap_err();
        assert_eq!(error, "cancelled");
        assert!(with_sign_in_guard(&busy, || Ok(())).is_ok());
    }

    /// Talks to Google: run by hand with `cargo test google_rejects -- --ignored`.
    #[test]
    #[ignore]
    fn google_rejects_an_unknown_client() {
        let e = refresh(TOKEN_URL, "not-a-client.apps.googleusercontent.com", "x", "y").unwrap_err();
        println!("{e}");
        assert!(e.contains("Contact the app developer") || e.contains("refused"), "{e}");
    }
}
