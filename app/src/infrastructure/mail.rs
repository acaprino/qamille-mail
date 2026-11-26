//! Real accounts: IMAP download and flag sync, SMTP sending (with attachments).

use crate::attach;
use crate::model::{Account, AccountKind, Attachment, Email, Store};
use lettre::message::{Attachment as LAttachment, Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Message, SmtpTransport, Transport};
use mail_parser::MimeHeaders;

pub const IMAP_HOST: &str = "imap.gmail.com";
pub const IMAP_PORT: u16 = 993;
pub const SMTP_HOST: &str = "smtp.gmail.com";
pub const SMTP_PORT: u16 = 465;
pub const APP_PASSWORD_HELP: &str = "https://myaccount.google.com/apppasswords";

/// Sets up a Google account (Gmail or Workspace). Google shows app passwords with spaces.
pub fn gmail_account(id: u32, name: &str, email: &str, app_password: &str, color: [u8; 3]) -> Account {
    Account {
        provider_id:"gmail".into(),
        id,
        name: if name.trim().is_empty() { email.trim().to_string() } else { name.trim().to_string() },
        email: email.trim().to_string(),
        color,
        kind: AccountKind::Gmail,
        imap_host: IMAP_HOST.into(),
        imap_port: IMAP_PORT,
        smtp_host: SMTP_HOST.into(),
        smtp_port: SMTP_PORT,
        username: email.trim().to_string(),
        password: app_password.chars().filter(|c| !c.is_whitespace()).collect(),
        archive_folder: "[Gmail]/All Mail".into(),
        trash_folder: "[Gmail]/Trash".into(),
        fetch_count: 200,
        ..Default::default()
    }
}

pub fn looks_like_email(s: &str) -> bool {
    let t = s.trim();
    match t.split_once('@') {
        Some((a, b)) => !a.is_empty() && b.contains('.') && !t.contains(char::is_whitespace),
        None => false,
    }
}

#[path = "mail_extensions.rs"]
pub(super) mod extension;
use extension::GmailExtensions;
pub(super) type Session = extension::Session<native_tls::TlsStream<std::net::TcpStream>>;

pub(super) fn imap_quote(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) { return Err("Invalid message identifier".into()); }
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

// All Mail includes sent and archived mail. Trash and Junk are separate in Gmail.
pub(super) fn export_folders<T: std::io::Read + std::io::Write>(s: &mut imap::Session<T>, a: &Account, require_all_mail: bool) -> Result<Vec<String>, String> {
    let names = s.list(Some(""), Some("*")).map_err(|e| e.to_string())?;
    let mut folders = Vec::new();
    let mut all_mail = false;
    for name in names.iter() {
        all_mail |= name.attributes().iter().any(|attr| matches!(attr, imap::types::NameAttribute::Custom(role) if role.as_ref() == "\\All"));
        if name.attributes().iter().any(|attr| matches!(attr, imap::types::NameAttribute::Custom(role) if matches!(role.as_ref(), "\\All" | "\\Trash" | "\\Junk"))) {
            folders.push(name.name().to_owned());
        }
    }
    for folder in [&a.archive_folder, &a.trash_folder] {
        if !folder.is_empty() && !folders.contains(folder) && names.iter().any(|n| n.name() == folder) { folders.push(folder.clone()); }
    }
    all_mail |= !a.archive_folder.eq_ignore_ascii_case("INBOX") && !a.archive_folder.is_empty() && folders.contains(&a.archive_folder);
    if require_all_mail && !all_mail {
        return Err("Gmail All Mail is unavailable over IMAP. Enable it in Gmail before printing the complete conversation.".into());
    }
    if !folders.iter().any(|f| f.eq_ignore_ascii_case("INBOX")) { folders.push("INBOX".into()); }
    Ok(folders)
}

pub(super) fn locate_message<T: std::io::Read + std::io::Write>(s: &mut imap::Session<T>, folders: &[String], email: &Email) -> Result<(String, u32), String> {
    if let Some(global) = email.remote_global_id {
        for folder in folders {
            s.examine(folder).map_err(|e| e.to_string())?;
            let ids = s.uid_search(format!("X-GM-MSGID {global}")).map_err(|e| e.to_string())?;
            if ids.len() > 1 { return Err("Provider returned ambiguous message identities".into()); }
            if let Some(uid) = ids.into_iter().next() { return Ok((folder.clone(), uid)); }
        }
        return Err("Original provider message is no longer available".into());
    }
    // A verified mailbox generation and UID identify the exact message, even
    // when another delivery copied the same RFC Message-ID header.
    let locations = std::iter::once((email.remote_folder.as_deref().unwrap_or("INBOX"), email.uid, email.remote_uid_validity))
        .chain(email.remote_locations.iter().map(|location| (location.folder.as_str(), Some(location.uid), location.uid_validity)));
    for (folder, uid, generation) in locations {
        let (Some(uid), Some(generation)) = (uid, generation) else { continue };
        let mailbox = s.examine(folder).map_err(|e| e.to_string())?;
        if mailbox.uid_validity != Some(generation) { continue; }
        let fetched = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[HEADER])").map_err(|e| e.to_string())?;
        if fetched.iter().find(|message| message.uid == Some(uid)).and_then(|message| message.header())
            .and_then(|raw| parse_message_inner(raw, 0, email.account_id, None))
            .is_some_and(|candidate| candidate.message_id == email.message_id && candidate.subject == email.subject && candidate.from_addr == email.from_addr) {
            return Ok((folder.into(), uid));
        }
    }
    if !email.message_id.is_empty() {
        let query = format!("HEADER Message-ID {}", imap_quote(&format!("<{}>", email.message_id.trim_matches(['<', '>'])))?);
        let mut found = None;
        for folder in folders {
            s.examine(folder).map_err(|e| e.to_string())?;
            let ids = s.uid_search(&query).map_err(|e| e.to_string())?;
            for id in ids {
                let fetched = s.uid_fetch(id.to_string(), "BODY.PEEK[HEADER]").map_err(|e| e.to_string())?;
                if fetched.iter().filter_map(|m| m.header()).any(|raw| {
                    mail_parser::MessageParser::default().parse(raw).and_then(|m| m.message_id().map(str::to_owned))
                        .is_some_and(|id| id == email.message_id.trim_matches(['<', '>']))
                }) {
                    if found.is_some() {
                        return Err("More than one server message matches these headers. Sync the account to verify its exact location before retrying.".into());
                    }
                    found = Some((folder.clone(), id));
                }
            }
        }
        if let Some((folder, uid)) = found {
            // Searches may have examined another folder after finding this UID.
            s.examine(&folder).map_err(|e| e.to_string())?;
            return Ok((folder, uid));
        }
    } else if let Some(uid) = email.uid {
        // A UID is mailbox-specific; validate its content before trusting an old one.
        let source = email.remote_folder.as_deref().unwrap_or("INBOX");
        s.examine(source).map_err(|e| e.to_string())?;
        let fetched = s.uid_fetch(uid.to_string(), "BODY.PEEK[]").map_err(|e| e.to_string())?;
        if fetched.iter().filter_map(|m| m.body()).filter_map(|raw| parse_message_inner(raw, 0, email.account_id, None))
            .any(|e| e.from_addr == email.from_addr && e.subject == email.subject && e.date == email.date && e.body == email.body) {
            return Ok((source.into(), uid));
        }
    }
    Err(format!("Original message not found on the server: {}. Sync mail and retry.", email.subject))
}

// A mailbox generation, UID and provider ID verify the exact saved location.
// This avoids listing and searching unrelated folders for every click.
pub(super) fn locate_known_message<T: std::io::Read + std::io::Write>(
    s: &mut imap::Session<T>, email: &Email,
) -> Result<Option<(String, u32)>, String> {
    let Some(global) = email.remote_global_id else { return Ok(None) };
    let locations = std::iter::once((email.remote_folder.as_deref().unwrap_or("INBOX"), email.uid, email.remote_uid_validity))
        .chain(email.remote_locations.iter().map(|location| (location.folder.as_str(), Some(location.uid), location.uid_validity)));
    for (folder, uid, generation) in locations {
        let (Some(uid), Some(generation)) = (uid, generation) else { continue };
        let Ok(mailbox) = s.examine(folder) else { continue };
        if mailbox.uid_validity != Some(generation) { continue; }
        let ids = s.uid_search(format!("UID {uid} X-GM-MSGID {global}")).map_err(|e| e.to_string())?;
        if ids.len() == 1 && ids.contains(&uid) { return Ok(Some((folder.to_owned(), uid))); }
    }
    Ok(None)
}

pub fn download_originals(a: &Account, emails: &[Email]) -> Result<Vec<Vec<u8>>, String> {
    let mut s = connect(a)?;
    let result = (|| {
        let mut folders = None;
        let mut output = Vec::new();
        for email in emails {
            let (_, uid) = match locate_known_message(&mut s, email)? {
                Some(location) => location,
                None => {
                    if folders.is_none() { folders = Some(export_folders(&mut s, a, false)?); }
                    locate_message(&mut s, folders.as_ref().unwrap(), email)?
                }
            };
            let fetched = s.uid_fetch(uid.to_string(), "BODY.PEEK[]").map_err(|e| e.to_string())?;
            let raw = fetched.iter().find(|m| m.uid == Some(uid)).and_then(|m| m.body()).ok_or("The server did not return the complete message")?;
            output.push(raw.to_vec());
        }
        Ok(output)
    })();
    let _ = s.logout();
    result
}

#[cfg(test)]
fn gmail_thread_id(response: &[u8], uid: u32) -> Option<u64> {
    let response = std::str::from_utf8(response).ok()?;
    for line in response.lines().filter(|l| l.starts_with("* ") && l.contains(" FETCH (")) {
        let tokens: Vec<_> = line.split(|c: char| c.is_whitespace() || c == '(' || c == ')').filter(|s| !s.is_empty()).collect();
        if !tokens.windows(2).any(|pair| pair[0] == "UID" && pair[1].parse::<u32>() == Ok(uid)) { continue; }
        for pair in tokens.windows(2) {
            if pair[0] == "X-GM-THRID" { return pair[1].parse().ok(); }
        }
    }
    None
}

// imap 2.x rejects X-GM-THRID while reading FETCH, but exposes the complete
// unparsed response. Consume this dedicated session: its tagged completion may
// still be buffered, so it must never be reused for another command or logout.
#[cfg(test)]
fn fetch_thread_id<T: std::io::Read + std::io::Write>(mut s: imap::Session<T>, uid: u32) -> Result<u64, String> {
    let response = match s.run_command_and_read_response(format!("UID FETCH {uid} (X-GM-THRID)")) {
        Ok(response) => response,
        Err(imap::Error::Parse(imap::error::ParseError::Invalid(response))) => response,
        Err(error) => return Err(error.to_string()),
    };
    gmail_thread_id(&response, uid).ok_or_else(|| "The server did not provide a conversation ID".into())
}

pub fn download_thread(a: &Account, email: &Email) -> Result<Vec<Email>, String> {
    let mut s = connect(a)?;
    let metadata = (|| {
        let folders = export_folders(&mut s, a, true)?;
        let (_, uid) = locate_message(&mut s, &folders, email)?;
        Ok::<_, String>((folders, uid))
    })();
    let (folders, uid) = match metadata {
        Ok(metadata) => metadata,
        Err(error) => { let _ = s.logout(); return Err(error); }
    };
    let thread_id = s.extensions.thread_id(uid)?;
    let _ = s.logout();
    let mut s = connect(a)?;
    let result = (|| {
        let mut emails = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for folder in folders {
            s.examine(&folder).map_err(|e| e.to_string())?;
            let ids = s.uid_search(format!("X-GM-THRID {thread_id}")).map_err(|e| e.to_string())?;
            let mut ids: Vec<_> = ids.into_iter().collect();
            ids.sort_unstable();
            for batch in ids.chunks(50) {
                let set = batch.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
                let fetched = s.uid_fetch(set, "BODY.PEEK[]").map_err(|e| e.to_string())?;
                if fetched.len() != batch.len() { return Err("Conversation changed during download. Retry printing.".into()); }
                for message in fetched.iter() {
                    let raw = message.body().ok_or("A conversation message could not be downloaded")?;
                    use sha2::{Digest, Sha256};
                    if !seen.insert(Sha256::digest(raw).to_vec()) { continue; }
                    let mut parsed = parse_message_inner(raw, emails.len() as u64 + 1, a.id, None).ok_or("A conversation message could not be read")?;
                    parsed.folder = folder.clone();
                    emails.push(parsed);
                }
            }
        }
        if emails.is_empty() { return Err("The conversation is no longer available on the server".into()); }
        emails.sort_by_key(|e| (e.date, e.id));
        Ok(emails)
    })();
    let _ = s.logout();
    result
}

pub(super) fn connect(a: &Account) -> Result<Session, String> {
    let session = extension::Session::new(connect_stream(a)?, a)?;
    // A signed-in connection is the proof that this account is reachable again:
    // wake its queued confirmations instead of letting them wait for their backoff
    // (ADR-0007 v3, decision 8 / EX-009).
    crate::application::connectivity::note_success(a.id);
    Ok(session)
}

pub(super) fn connect_stream(a: &Account) -> Result<native_tls::TlsStream<std::net::TcpStream>, String> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let tls = native_tls::TlsConnector::builder().build().map_err(|e| e.to_string())?;
    let port = if a.imap_port == 0 { 993 } else { a.imap_port };
    let addresses = (a.imap_host.as_str(), port).to_socket_addrs().map_err(|e| format!("IMAP connect: {e}"))?;
    let mut last_error = "IMAP server has no address".to_string();
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(15)) {
            Ok(stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
                stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
                return tls.connect(&a.imap_host, stream).map_err(|e| format!("IMAP TLS: {e}"));
            }
            Err(error) => last_error = format!("IMAP connect: {error}"),
        }
    }
    Err(last_error)
}

pub(super) fn authenticate<T: std::io::Read + std::io::Write>(client: imap::Client<T>, a: &Account) -> Result<imap::Session<T>, String> {
    let user = if a.username.is_empty() { a.email.as_str() } else { a.username.as_str() };
    if a.oauth_client_id.is_empty() {
        return client.login(user, &a.password).map_err(|e| format!("IMAP login: {}", e.0));
    }
    let token = crate::oauth::access_token(a)?;
    client.authenticate("XOAUTH2", &XOAuth2 { user: user.to_string(), token }).map_err(|e| format!("IMAP login: {}", e.0))
}

struct XOAuth2 {
    user: String,
    token: String,
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;
    /// On failure Google sends a challenge with the reason and expects an empty answer.
    fn process(&self, challenge: &[u8]) -> String {
        if challenge.is_empty() { crate::oauth::xoauth2_string(&self.user, &self.token) } else { String::new() }
    }
}

/// Gmail names its system folders in the account's language, so ask for them by role.
pub fn gmail_folders(a: &Account) -> Result<(Option<String>, Option<String>), String> {
    let mut s = connect(a)?;
    let out = list_folders(&mut s);
    let _ = s.logout();
    out
}

fn list_folders<T: std::io::Read + std::io::Write>(s: &mut imap::Session<T>) -> Result<(Option<String>, Option<String>), String> {
    let names = s.list(Some(""), Some("*")).map_err(|e| e.to_string())?;
    let (mut all, mut trash) = (None, None);
    for n in names.iter() {
        for attr in n.attributes() {
            if let imap::types::NameAttribute::Custom(c) = attr {
                match c.as_ref() {
                    "\\All" => all = Some(n.name().to_string()),
                    "\\Trash" => trash = Some(n.name().to_string()),
                    _ => {}
                }
            }
        }
    }
    Ok((all, trash))
}

/// Compatibility entry point: the same per-account policy applies to native
/// and MCP callers. Normal synchronization only acquires metadata.
pub fn sync_account(a: &Account, attach_dir: &std::path::Path, first_id: u64) -> Result<Vec<Email>, String> {
    let mut store: Store = match std::fs::read(Store::path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| format!("Mail data cannot be read; the original file is unchanged: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store { accounts: vec![a.clone()], ..Default::default() },
        Err(error) => return Err(error.to_string()),
    };
    super::portable_files::resolve_store_paths(&mut store, &Store::data_dir()).map_err(|error| error.message)?;
    let measured = super::rag::message_index_bytes(&Store::data_dir(), &store).map_err(|error| error.to_string())?;
    let drafts = super::repository::draft_storage_bytes(&Store::data_dir(), &store).map_err(|error| error.message)?;
    let max_bytes = a.storage_policy.admission_limit_bytes().map(|limit| {
        limit.saturating_sub(crate::domain::storage::usage_with_storage_bytes(&store, a.id, &measured, &drafts).map_or(0, |usage| usage.used_bytes))
    });
    let outcome = super::mail_cache_download::sync_account_policy(a, attach_dir, &crate::application::ports::SyncRequest {
        max_bytes, known: &store.emails,
    })?;
    let mut emails = outcome.batch.emails;
    for (index, email) in emails.iter_mut().enumerate() { email.id = first_id.saturating_add(index as u64).saturating_add(1); }
    Ok(emails)
}

#[cfg(test)]
fn fetch_inbox<T: std::io::Read + std::io::Write>(s: &mut imap::Session<T>, a: &Account, attach_dir: &std::path::Path, first_id: u64) -> Result<Vec<Email>, String> {
    let mbox = s.select("INBOX").map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if mbox.exists > 0 {
        let count = a.fetch_count.clamp(10, 500);
        let start = mbox.exists.saturating_sub(count - 1).max(1);
        let msgs = s.fetch(format!("{}:{}", start, mbox.exists), "(UID FLAGS BODY.PEEK[])").map_err(|e| e.to_string())?;
        let mut next = first_id;
        for m in msgs.iter() {
            let Some(raw) = m.body() else { continue };
            next += 1;
            let seen = m.flags().iter().any(|f| matches!(f, imap::types::Flag::Seen));
            let flagged = m.flags().iter().any(|f| matches!(f, imap::types::Flag::Flagged));
            let answered = m.flags().iter().any(|f| matches!(f, imap::types::Flag::Answered));
            if let Some(mut e) = parse_message(raw, next, a.id, attach_dir) {
                e.uid = m.uid;
                e.read = seen;
                e.starred = flagged;
                e.replied = answered;
                out.push(e);
            }
        }
    }
    Ok(out)
}

/// Really signs in to Google for receiving and for sending, without changing anything.
pub fn check_login(a: &Account) -> Result<(), String> {
    let oauth = !a.oauth_client_id.is_empty();
    let mut s = connect(a).map_err(|e| friendly_login_error(&e, oauth))?;
    let _ = s.logout();
    match smtp_transport(a).and_then(|t| t.test_connection().map_err(|e| format!("SMTP: {e}"))) {
        Ok(true) => {
            // The sending transport connected and signed in: the account is reachable.
            crate::application::connectivity::note_success(a.id);
            Ok(())
        }
        Ok(false) => Err("Google didn't answer the sending check. Try again in a minute.".into()),
        Err(e) => Err(friendly_login_error(&e, oauth)),
    }
}

/// Makes an account from a finished Google sign-in.
pub fn oauth_account(id: u32, email: &str, client_id: &str, refresh_token: &str, color: [u8; 3]) -> Account {
    let mut a = gmail_account(id, "", email, "", color);
    a.password = refresh_token.to_string();
    a.oauth_client_id = client_id.to_string();
    a
}

/// Says what to do about a failed sign-in, keeping Google's own words at the end.
/// `oauth` is true for accounts connected with "Sign in with Google", which have no password to blame.
pub fn friendly_login_error(raw: &str, oauth: bool) -> String {
    let l = raw.to_lowercase();
    let hint = if l.contains("sign in with google again") || l.contains("client id") {
        return raw.trim().to_string();
    } else if l.contains("authenticationfailed") || l.contains("invalid credentials") || l.contains("username and password not accepted") || l.contains("535") || l.contains("application-specific password") {
        if oauth {
            "Google refused the Qamille sign-in. Sign in with Google again and tick the Gmail box on the permissions page."
        } else {
            "Google refused this address and password. Use an app password from Google's page, not your normal password; it needs 2-Step Verification turned on."
        }
    } else if l.contains("web login required") || l.contains("534") || l.contains("alert") {
        "Google wants you to confirm this sign-in in your browser. Open Gmail on the web once, then try again."
    } else if l.contains("too many") || l.contains("overquota") || l.contains("limit") {
        "Google is limiting this account for now. Wait a few minutes and try again."
    } else if l.contains("disabled") || l.contains("not enabled") {
        "IMAP access is switched off for this account. On a work account, your administrator can turn it on."
    } else if l.contains("connect") || l.contains("dns") || l.contains("timed out") || l.contains("resolve") || l.contains("os error") {
        "Couldn't reach Google. Check the internet connection and any firewall or antivirus."
    } else {
        "Sign-in failed."
    };
    format!("{hint} (Google said: {})", raw.trim())
}

/// Whether a provider failure proves that nothing was executed, so a queued
/// confirmation may be attempted again (ADR-0007 v3, decision 8 / INV-010).
///
/// This file writes the `IMAP connect: `, `IMAP TLS: `, `IMAP login: ` and
/// `SMTP: ` texts the rule reads, so the contract is exposed and tested here;
/// the rule itself is the confirmation queue's policy and lives in
/// `application::connectivity`, which the services may depend on. Anything
/// ambiguous stays definitive: a result lost after a mutation or a delivery
/// started is never retried (INV-008).
pub fn is_retryable_connect_error(message: &str) -> bool {
    crate::application::connectivity::is_retryable_connect_error(message)
}

pub fn parse_message(raw: &[u8], id: u64, account_id: u32, attach_dir: &std::path::Path) -> Option<Email> {
    let mut email = parse_message_inner(raw, id, account_id, Some(attach_dir))?;
    use sha2::{Digest, Sha256};
    email.remote_content_hash = format!("{:x}", Sha256::digest(raw));
    Some(email)
}

pub(super) fn parse_message_inner(raw: &[u8], id: u64, account_id: u32, attach_dir: Option<&std::path::Path>) -> Option<Email> {
    let msg = mail_parser::MessageParser::default().parse(raw)?;
    let addr_list = |a: Option<&mail_parser::Address>| -> Vec<String> {
        a.map(|x| x.iter().filter_map(|p| p.address().map(String::from)).collect()).unwrap_or_default()
    };
    let (from_name, from_addr) = msg
        .from()
        .and_then(|f| f.first())
        .map(|f| {
            let addr = f.address().unwrap_or("").to_string();
            (f.name().map(String::from).unwrap_or_else(|| addr.clone()), addr)
        })
        .unwrap_or_default();
    let mut body = msg.body_text(0).map(|t| t.to_string()).unwrap_or_default();
    if body.trim().is_empty() {
        if let Some(h) = msg.body_html(0) {
            body = attach::html_to_text(&h);
        }
    }
    // `body_html` would also convert a plain-only message; keep real HTML parts only.
    let body_html = match msg.html_part(0).map(|part| &part.body) {
        Some(mail_parser::PartType::Html(html)) if !html.trim().is_empty() => html.to_string(),
        _ => String::new(),
    };
    let mut attachments = Vec::new();
    for (i, part) in msg.attachments().enumerate() {
        let name = part.attachment_name().unwrap_or("attachment").to_string();
        let safe: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
        let bytes = part.contents();
        let path = attach_dir.and_then(|dir| {
            use sha2::{Digest, Sha256};
            let key = format!("{:x}", Sha256::digest(raw));
            let path = dir.join(format!("gmail-{account_id}-{key}-{i}-{safe}"));
            std::fs::create_dir_all(dir).ok()?;
            std::fs::write(&path, bytes).ok()?;
            Some(path.to_string_lossy().to_string())
        });
        let mime = part
            .content_type()
            .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        attachments.push(Attachment {
            id: id * 100 + i as u64,
            name,
            size: bytes.len() as u64,
            mime,
            path,
            inline_text: None,
        });
    }
    // The same headers the original reads for its machine-mail checks.
    let header = |name: &str| msg.header_raw(name).map(|v| v.trim().to_lowercase());
    let precedence = header("Precedence").unwrap_or_default();
    let bulk = header("List-Unsubscribe").is_some() || precedence == "bulk" || precedence == "list";
    let auto_submitted = header("Auto-Submitted").is_some_and(|v| !v.starts_with("no"));
    let calendar = msg.parts.iter().any(|p| p.content_type().is_some_and(|c| c.ctype().eq_ignore_ascii_case("text") && c.subtype().is_some_and(|x| x.eq_ignore_ascii_case("calendar"))))
        || attachments.iter().any(|a: &Attachment| a.name.to_lowercase().ends_with(".ics"));
    let message_id = msg.message_id().unwrap_or("").to_string();
    let in_reply_to = msg.in_reply_to().as_text().map(String::from);
    let thread = in_reply_to.clone().unwrap_or_else(|| message_id.clone());
    Some(Email {
        id,
        account_id,
        folder: "Inbox".into(),
        thread_id: thread,
        message_id,
        in_reply_to: in_reply_to.unwrap_or_default(),
        from_name,
        from_addr,
        reply_to_addresses: addr_list(msg.reply_to()),
        to: addr_list(msg.to()),
        cc: addr_list(msg.cc()),
        subject: msg.subject().unwrap_or("(no subject)").to_string(),
        body,
        body_html,
        date: msg.date().map(|d| d.to_timestamp()).unwrap_or(0),
        date_missing: msg.date().is_none(),
        attachments,
        bulk,
        auto_submitted,
        calendar,
        ..Default::default()
    })
}

/// Merge freshly downloaded mail into the store, keeping local task state for known messages.
pub fn merge(store: &mut Store, account_id: u32, fresh: Vec<Email>) -> Vec<u64> {
    let mut added = Vec::new();
    for mut e in fresh {
        if let Some(old) = store
            .emails
            .iter_mut()
            .find(|o| o.account_id == account_id && crate::domain::mail_identity::same_message(o, &e))
        {
            old.read = old.read || e.read;
            old.uid = e.uid;
            old.remote_folder = e.remote_folder;
            old.remote_uid_validity = e.remote_uid_validity;
            for location in e.remote_locations {
                if !old.remote_locations.contains(&location) { old.remote_locations.push(location); }
            }
            if !e.remote_only {
                old.body = e.body;
                old.attachments = e.attachments;
                old.remote_content_hash = e.remote_content_hash;
                old.remote_only = false;
                old.storage_evicted = false;
            }
            continue;
        }
        e.id = store.new_id();
        added.push(e.id);
        store.emails.push(e);
    }
    added
}

/// Push a local action to the server (best effort).
pub enum Remote {
    Seen(bool),
    Flag(bool),
    Move(String),
    /// Gmail archive: drops the Inbox label wherever the message is found.
    Archive,
    /// Adds a Gmail label: copying into a label's folder is how IMAP does it.
    Label(String),
    RemoveLabel(String),
    Spam,
}

pub fn push_action(a: &Account, uid: u32, action: Remote) -> Result<(), String> {
    let mut s = connect(a)?;
    s.select("INBOX").map_err(|e| e.to_string())?;
    let result = apply_session_action(&mut s, uid, action);
    let _ = s.logout();
    result
}

/// Resolve a mailbox-specific identity again before mutating a search result.
pub fn push_message_action(a: &Account, email: &Email, action: Remote) -> Result<(), String> {
    let mut s = connect(a)?;
    let result = push_message_action_in(&mut s, a, email, action);
    let _ = s.logout();
    result
}

fn push_message_action_in<T: std::io::Read + std::io::Write>(s: &mut extension::Session<T>, a: &Account, email: &Email, action: Remote) -> Result<(), String> {
    let (folder, uid) = match locate_known_message(&mut **s, email)? {
        Some(location) => location,
        None => {
            let mut folders = export_folders(&mut **s, a, true)?;
            if let Some(folder) = &email.remote_folder {
                folders.retain(|f| f != folder);
                folders.insert(0, folder.clone());
            }
            locate_message(&mut **s, &folders, email)?
        }
    };
    s.select(folder).map_err(|e| e.to_string())?;
    apply_session_action(s, uid, action)
}

fn apply_session_action<T: std::io::Read + std::io::Write>(s: &mut extension::Session<T>, uid: u32, action: Remote) -> Result<(), String> {
    match action {
        Remote::Archive => s.extensions.store_labels(uid, '-', "\\Inbox"),
        Remote::RemoveLabel(label) => s.extensions.store_labels(uid, '-', &imap_quote(&label)?),
        other => apply_uid_action(&mut **s, uid, other),
    }
}

/// Verify a stable Gmail identity while the source UID is still valid.
pub fn resolve_identity(a: &Account, email: &Email) -> Result<Email, String> {
    let mut s = connect(a)?;
    let result = resolve_identity_in(&mut s, a, email);
    let _ = s.logout();
    result
}

fn resolve_identity_in<T: std::io::Read + std::io::Write>(s: &mut extension::Session<T>, a: &Account, email: &Email) -> Result<Email, String> {
    // The saved location, mailbox generation and Gmail ID identify the message
    // exactly; only an unverified location needs the scan of every folder.
    let (folder, uid) = match locate_known_message(&mut **s, email)? {
        Some(location) => location,
        None => {
            let folders = export_folders(&mut **s, a, true)?;
            locate_message(&mut **s, &folders, email)?
        }
    };
    let generation = s.examine(&folder).map_err(|e| e.to_string())?.uid_validity;
    if email.remote_global_id.is_none() {
        let same = |known_folder: &str, known_uid: Option<u32>, known_generation: Option<u32>| {
            known_uid == Some(uid) && known_generation.is_some() && known_generation == generation
                && crate::domain::mail_identity::same_folder(known_folder, &folder)
        };
        let verified = same(email.remote_folder.as_deref().unwrap_or("INBOX"), email.uid, email.remote_uid_validity)
            || email.remote_locations.iter().any(|location| same(&location.folder, Some(location.uid), location.uid_validity));
        if !verified {
            if email.remote_content_hash.is_empty() { return Err("Sync this message to verify its provider identity before moving it".into()); }
            let fetched = s.uid_fetch(uid.to_string(), "BODY.PEEK[]").map_err(|e| e.to_string())?;
            let raw = fetched.iter().find(|row| row.uid == Some(uid)).and_then(|row| row.body()).ok_or("Provider message unavailable")?;
            use sha2::{Digest, Sha256};
            if format!("{:x}", Sha256::digest(raw)) != email.remote_content_hash { return Err("Provider content does not match this message; sync before moving it".into()); }
        }
    }
    let global = s.extensions.ids(&uid.to_string())?.remove(&uid)
        .ok_or("Gmail did not verify the message identity before moving")?;
    if email.remote_global_id.is_some_and(|old| old != global) { return Err("The provider message changed before moving".into()); }
    let mut resolved = email.clone();
    resolved.remote_global_id = Some(global);
    resolved.remote_folder = Some(folder.clone());
    resolved.uid = Some(uid);
    resolved.remote_uid_validity = generation;
    let location = crate::model::RemoteLocation { folder, uid, uid_validity: generation };
    if !resolved.remote_locations.contains(&location) { resolved.remote_locations.push(location); }
    Ok(resolved)
}

fn apply_uid_action<T:std::io::Read+std::io::Write>(s: &mut imap::Session<T>, uid: u32, action: Remote) -> Result<(), String> {
    let set = uid.to_string();
    if matches!(&action,Remote::Spam){
        let folders=s.list(Some(""),Some("*")).map_err(|e|e.to_string())?;
        let folder=folders.iter().find(|folder|folder.attributes().iter().any(|attr|matches!(attr,imap::types::NameAttribute::Custom(role) if role.as_ref()=="\\Junk"))).map(|folder|folder.name().to_string()).ok_or("The provider did not expose its spam folder over IMAP")?;
        return apply_uid_action(s,uid,Remote::Move(folder));
    }
    match action {
        Remote::Seen(true) => s.uid_store(&set, "+FLAGS (\\Seen)").map(|_| ()),
        Remote::Seen(false) => s.uid_store(&set, "-FLAGS (\\Seen)").map(|_| ()),
        Remote::Flag(true) => s.uid_store(&set, "+FLAGS (\\Flagged)").map(|_| ()),
        Remote::Flag(false) => s.uid_store(&set, "-FLAGS (\\Flagged)").map(|_| ()),
        Remote::Move(folder) => s.uid_mv(&set, &folder).or_else(|_| {
            s.uid_copy(&set, &folder)?;
            s.uid_store(&set, "+FLAGS (\\Deleted)")?;
            // Never expunge unrelated messages already marked Deleted by another client.
            s.uid_expunge(&set).map(|_| ())
        }),
        Remote::Label(label) => {
            let _ = s.create(&label);
            s.uid_copy(&set, &label)
        }
        Remote::Archive | Remote::RemoveLabel(_) => return Err("Gmail label changes need the extension session".into()),
        Remote::Spam=>unreachable!("Spam was resolved to the provider folder before dispatch"),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// One provider search page: Gmail's own search (`X-GM-RAW`) in All Mail, plus
/// Spam and Trash when the query asks for them. Metadata only: headers, flags,
/// structure and Gmail labels; never a body (INV-1).
pub fn search_account(
    a: &Account,
    request: &crate::application::ports::RemoteSearchRequest,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<crate::application::ports::RemoteSearchPage, String> {
    let mut s = connect(a)?;
    let result = search_in(&mut s, a, request, cancel);
    let _ = s.logout();
    result
}

pub(super) fn search_in<T: std::io::Read + std::io::Write>(
    s: &mut extension::Session<T>,
    a: &Account,
    request: &crate::application::ports::RemoteSearchRequest,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<crate::application::ports::RemoteSearchPage, String> {
    use crate::application::ports::{RemoteCursor, RemoteScope, RemoteSearchPage};
    use std::sync::atomic::Ordering;
    let stop = || if cancel.load(Ordering::Relaxed) { Err("Server search cancelled".to_string()) } else { Ok(()) };
    let names = s.list(Some(""), Some("*")).map_err(|e| e.to_string())?;
    let role = |wanted: &str| names.iter().find(|name| name.attributes().iter().any(|attr| matches!(attr, imap::types::NameAttribute::Custom(r) if r.as_ref() == wanted))).map(|n| n.name().to_string());
    let all = role("\\All")
        .or_else(|| names.iter().find(|n| n.name() == a.archive_folder && !a.archive_folder.eq_ignore_ascii_case("INBOX")).map(|n| n.name().to_string()))
        .ok_or("Enable \"All Mail\" in Gmail (Settings, Labels, Show in IMAP) to search the server")?;
    let mut folders = vec![(all, "Archive")];
    for scope in &request.scopes {
        let found = match scope { RemoteScope::Spam => role("\\Junk").map(|f| (f, "Spam")), RemoteScope::Trash => role("\\Trash").map(|f| (f, "Trash")) };
        if let Some(found) = found { folders.push(found); }
    }
    // Every folder is searched: the total counts all of them, and a cursor names one.
    let mut found = Vec::new();
    let mut mailbox_total = 0;
    for (index, (folder, _)) in folders.iter().enumerate() {
        stop()?;
        let mailbox = s.examine(folder).map_err(|e| e.to_string())?;
        if index == 0 { mailbox_total = mailbox.exists; }
        let mut uids = s.extensions.raw_search(&request.raw)?;
        uids.reverse();
        found.push((mailbox.uid_validity.unwrap_or(0), uids));
    }
    let total = found.iter().map(|(_, uids)| uids.len() as u32).sum();
    let (start, before) = request.cursor.as_ref()
        .and_then(|c| folders.iter().position(|(f, _)| *f == c.folder).filter(|i| found[*i].0 == c.uid_validity).map(|i| (i, c.before_uid)))
        .unwrap_or((0, u32::MAX));
    let size = request.page_size.clamp(1, 100);
    let mut page: Vec<(usize, u32)> = Vec::new();
    for (index, (_, uids)) in found.iter().enumerate().skip(start) {
        let limit = if index == start { before } else { u32::MAX };
        page.extend(uids.iter().filter(|uid| **uid < limit).take(size - page.len()).map(|uid| (index, *uid)));
        if page.len() >= size { break; }
    }
    let next = page.last().and_then(|(index, last)| {
        if found[*index].1.iter().any(|uid| uid < last) {
            return Some(RemoteCursor { folder: folders[*index].0.clone(), uid_validity: found[*index].0, before_uid: *last });
        }
        (index + 1..folders.len()).find(|i| !found[*i].1.is_empty())
            .map(|i| RemoteCursor { folder: folders[i].0.clone(), uid_validity: found[i].0, before_uid: u32::MAX })
    });
    let mut emails = Vec::new();
    for (index, (folder, logical)) in folders.iter().enumerate() {
        let uids: Vec<u32> = page.iter().filter(|(i, _)| *i == index).map(|(_, uid)| *uid).collect();
        if uids.is_empty() { continue; }
        stop()?;
        let mailbox = s.examine(folder).map_err(|e| e.to_string())?;
        emails.extend(fetch_metadata(s, a.id, folder, mailbox.uid_validity, logical, &uids)?);
    }
    emails.sort_by_key(|e: &Email| (std::cmp::Reverse(e.date), std::cmp::Reverse(e.uid)));
    Ok(RemoteSearchPage { emails, total, mailbox_total, next })
}

/// Metadata-only rows for `uids` of the examined `folder`: headers, flags,
/// structure and Gmail labels in two requests, never a body (INV-1). A message
/// deleted meanwhile is simply missing; a draft is left out.
pub(super) fn fetch_metadata<T: std::io::Read + std::io::Write>(
    s: &mut extension::Session<T>,
    account_id: u32,
    folder: &str,
    uid_validity: Option<u32>,
    logical: &str,
    uids: &[u32],
) -> Result<Vec<Email>, String> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let rows = s.uid_fetch(&set, "(UID FLAGS RFC822.SIZE INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER])").map_err(|e| e.to_string())?;
    let gmail = s.extensions.metadata(&set)?;
    Ok(rows
        .iter()
        .filter_map(|row| metadata_row(row, account_id, folder, uid_validity, logical, row.uid.and_then(|uid| gmail.get(&uid))))
        .collect())
}

/// One history batch (W3.1): the messages of `range` in Gmail's All Mail.
pub fn history_batch(
    a: &Account,
    request: &crate::application::ports::HistoryRequest,
) -> Result<crate::application::ports::HistoryBatch, String> {
    let mut s = connect(a)?;
    let result = history_in(&mut s, a, request);
    let _ = s.logout();
    result
}

pub(super) fn history_in<T: std::io::Read + std::io::Write>(
    s: &mut extension::Session<T>,
    a: &Account,
    request: &crate::application::ports::HistoryRequest,
) -> Result<crate::application::ports::HistoryBatch, String> {
    let names = s.list(Some(""), Some("*")).map_err(|e| e.to_string())?;
    let folder = names
        .iter()
        .find(|name| name.attributes().iter().any(|attr| matches!(attr, imap::types::NameAttribute::Custom(r) if r.as_ref() == "\\All")))
        .map(|n| n.name().to_string())
        .or_else(|| names.iter().find(|n| n.name() == a.archive_folder && !a.archive_folder.eq_ignore_ascii_case("INBOX")).map(|n| n.name().to_string()))
        .ok_or("Enable \"All Mail\" in Gmail (Settings, Labels, Show in IMAP) to bring older mail into the list")?;
    // Marking read needs a writable mailbox; everything else only examines.
    let mailbox = if request.progress.mark_read_before.is_some() { s.select(&folder) } else { s.examine(&folder) }
        .map_err(|e| e.to_string())?;
    let uid_validity = mailbox.uid_validity.ok_or("Gmail omitted UIDVALIDITY; the history cannot safely resume")?;
    let uid_next = mailbox.uid_next.ok_or("Gmail omitted UIDNEXT; the history cannot safely paginate")?;
    let mut progress = request.progress.clone();
    if progress.uid_validity != Some(uid_validity) {
        progress.restart(uid_validity);
    }
    let range = progress.plan(uid_next, request.batch_uids);
    let mut batch = crate::application::ports::HistoryBatch { folder: folder.clone(), uid_validity, uid_next, exists: mailbox.exists, range, ..Default::default() };
    if let Some(before) = request.progress.mark_read_before {
        batch.marked_read = mark_inbox_read_before(s, before)?;
    }
    if let Some(range) = range {
        let mut uids: Vec<u32> = s
            .uid_search(format!("UID {}:{}", range.start, range.end))
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|uid| (range.start..=range.end).contains(uid))
            .collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        batch.emails = fetch_metadata(s, a.id, &folder, Some(uid_validity), "Archive", &uids)?;
    } else if progress.before_uid > 0 && uid_next > 1 {
        // The walk is done (or waits for space): look again at the newest
        // messages, whose labels may have changed on the web (W3.2).
        batch.refresh = label_window(s, uid_next)?;
    }
    // Spam and Trash are not in All Mail: their newest messages, when asked.
    for role in &request.roles {
        let attribute = match role.as_str() { "Spam" => "\\Junk", "Trash" => "\\Trash", _ => continue };
        let Some(name) = names.iter().find(|n| n.attributes().iter().any(|attr| matches!(attr, imap::types::NameAttribute::Custom(r) if r.as_ref() == attribute))) else { continue };
        let name = name.name().to_string();
        let examined = s.examine(&name).map_err(|e| e.to_string())?;
        let mut uids: Vec<u32> = s.uid_search("ALL").map_err(|e| e.to_string())?.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(REFRESH_UIDS as usize);
        batch.extra.extend(fetch_metadata(s, a.id, &name, examined.uid_validity, role, &uids)?);
        batch.roles_read.push(role.clone());
    }
    Ok(batch)
}

/// The choice of the unread notice: Inbox messages still unread and older than
/// `before` are marked read on Gmail with one search and silent STOREs.
fn mark_inbox_read_before<T: std::io::Read + std::io::Write>(s: &mut extension::Session<T>, before: i64) -> Result<u32, String> {
    let day = chrono::DateTime::from_timestamp(before, 0).ok_or("Invalid date for marking old mail read")?.format("%Y/%m/%d");
    let uids = s.extensions.raw_search(&format!("in:inbox is:unread before:{day}"))?;
    for chunk in uids.chunks(500) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        s.uid_store(&set, "+FLAGS.SILENT (\\Seen)").map_err(|e| e.to_string())?;
    }
    Ok(uids.len() as u32)
}

/// UIDs of the newest All Mail messages read again for their labels (W3.2).
pub(super) const REFRESH_UIDS: u32 = 500;

/// Flags and labels of the newest `REFRESH_UIDS` UIDs of the examined folder:
/// two small FETCHes, no header and no body.
fn label_window<T: std::io::Read + std::io::Write>(
    s: &mut extension::Session<T>,
    uid_next: u32,
) -> Result<Vec<crate::application::ports::LabelState>, String> {
    let top = uid_next.saturating_sub(1);
    let set = format!("{}:{top}", top.saturating_sub(REFRESH_UIDS - 1).max(1));
    let rows = s.uid_fetch(&set, "(UID FLAGS)").map_err(|e| e.to_string())?;
    let gmail = s.extensions.metadata(&set)?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let metadata = gmail.get(&row.uid?)?;
            let flags = row.flags();
            Some(crate::application::ports::LabelState {
                global_id: metadata.global_id?,
                labels: metadata.labels.clone(),
                read: flags.iter().any(|f| matches!(f, imap::types::Flag::Seen)),
                starred: flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged)),
                replied: flags.iter().any(|f| matches!(f, imap::types::Flag::Answered)),
            })
        })
        .collect())
}

/// A metadata-only row from a FETCH of headers, flags and structure, placed by its
/// Gmail labels. `logical` is the role of the folder searched: Spam and Trash
/// keep it whatever the labels say.
pub(super) fn metadata_row(
    row: &imap::types::Fetch,
    account_id: u32,
    folder: &str,
    uid_validity: Option<u32>,
    logical: &str,
    gmail: Option<&extension::GmailMetadata>,
) -> Option<Email> {
    let uid = row.uid?;
    let mut email = parse_message_inner(row.header()?, 0, account_id, None)?;
    let placement = gmail.map(|g| crate::domain::gmail_labels::classify(&g.labels));
    if placement.as_ref().is_some_and(|p| p.skip) { return None; }
    email.remote_only = true;
    email.body.clear();
    email.body_html.clear();
    email.uid = Some(uid);
    email.remote_folder = Some(folder.to_string());
    email.remote_uid_validity = uid_validity;
    email.remote_global_id = gmail.and_then(|g| g.global_id);
    let flags = row.flags();
    email.read = flags.iter().any(|f| matches!(f, imap::types::Flag::Seen));
    email.replied = flags.iter().any(|f| matches!(f, imap::types::Flag::Answered));
    email.starred = flags.iter().any(|f| matches!(f, imap::types::Flag::Flagged));
    email.folder = logical.to_string();
    if let Some(placement) = placement {
        if !matches!(logical, "Spam" | "Trash") { email.folder = placement.folder; }
        email.labels = placement.labels;
        email.starred |= placement.starred;
    }
    if let Some(structure) = row.bodystructure() {
        super::mail_cache_download::attachment_metadata(structure, &mut email.attachments);
    }
    if email.date_missing {
        if let Some(date) = row.internal_date() {
            email.date = date.timestamp();
            email.date_missing = false;
        }
    }
    // When the server received it, whatever its Date header says: whether it is new mail.
    email.received_at = row.internal_date().map(|date| date.timestamp());
    Some(email)
}

#[cfg(test)]
mod search_transport_tests {
    use super::*;
    #[test]
    fn a_missing_message_date_is_explicit_instead_of_becoming_today() {
        let email = parse_message_inner(b"From: sender@example.com\r\nSubject: no date\r\n\r\nBody", 1, 1, None).unwrap();
        assert!(email.date_missing);
        assert_eq!(email.date, 0);
    }
    #[test]
    fn replies_use_the_ordered_reply_to_mailboxes_when_present() {
        let raw = b"From: robot@test.example\r\nReply-To: desk@test.example, Ticket <agent@test.example>\r\n\r\nRequest";
        let email = super::parse_message_inner(raw, 1, 1, None).unwrap();
        assert_eq!(email.reply_recipients(), &["desk@test.example", "agent@test.example"]);
        let plain = super::parse_message_inner(b"From: robot@test.example\r\n\r\nText", 2, 1, None).unwrap();
        assert_eq!(plain.reply_recipients(), &["robot@test.example"]);
    }

    #[test]
    fn parse_keeps_the_in_reply_to_header() {
        let reply = parse_message_inner(
            b"From: a@x\r\nMessage-ID: <child@x>\r\nIn-Reply-To: <parent@x>\r\nSubject: Re: s\r\n\r\nBody",
            1, 1, None,
        ).unwrap();
        assert_eq!(reply.in_reply_to, "parent@x");
        assert_eq!(reply.thread_id, "parent@x", "thread_id keeps its meaning");
        let root = parse_message_inner(b"From: a@x\r\nMessage-ID: <root@x>\r\nSubject: s\r\n\r\nBody", 2, 1, None).unwrap();
        assert_eq!(root.in_reply_to, "");
        assert_eq!(root.thread_id, "root@x");
    }
}

pub struct Outgoing {
    pub message_id: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// When set, sent together with the plain text so every mail program can show it.
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachments: Vec<String>,
}

pub fn split_addrs(s: &str) -> Vec<String> {
    s.split([',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

pub fn build_message(a: &Account, from_name: &str, out: &Outgoing) -> Result<Message, String> {
    // Built from its two parts: a name like "Rossi, Mario" is not valid inside a typed-out address.
    let address = a.email.parse().map_err(|e| format!("From: {e}"))?;
    let from = Mailbox::new(Some(from_name.trim().to_string()).filter(|n| !n.is_empty()), address);
    let mut b = Message::builder().from(from).subject(out.subject.clone());
    if let Some(id)=&out.message_id { b=b.message_id(Some(id.clone())); }
    for t in &out.to {
        b = b.to(t.parse().map_err(|e| format!("To {t}: {e}"))?);
    }
    for c in &out.cc {
        b = b.cc(c.parse().map_err(|e| format!("Cc {c}: {e}"))?);
    }
    // lettre strips Bcc from the serialized headers while retaining its envelope recipients.
    for c in &out.bcc {
        b = b.bcc(c.parse().map_err(|e| format!("Bcc {c}: {e}"))?);
    }
    if let Some(r) = &out.in_reply_to {
        if !r.is_empty() {
            let id = if r.starts_with('<') { r.clone() } else { format!("<{r}>") };
            b = b.in_reply_to(id.clone()).references(id);
        }
    }
    if out.attachments.is_empty() {
        return match &out.html {
            Some(h) => b.multipart(MultiPart::alternative_plain_html(out.body.clone(), h.clone())),
            None => b.header(ContentType::TEXT_PLAIN).body(out.body.clone()),
        }
        .map_err(|e| e.to_string());
    }
    let mut mp = match &out.html {
        Some(h) => MultiPart::mixed().multipart(MultiPart::alternative_plain_html(out.body.clone(), h.clone())),
        None => MultiPart::mixed().singlepart(SinglePart::plain(out.body.clone())),
    };
    for p in &out.attachments {
        let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        let name = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        mp = mp.singlepart(attachment_part(name, bytes));
    }
    b.multipart(mp).map_err(|e| e.to_string())
}

/// An attached `.eml` travels as `message/rfc822` so mail programs show it as a
/// message. RFC 2046 forbids base64 for that type, so a message that is not
/// 7-bit clean goes as a plain file, exactly as before.
fn attachment_part(name: String, bytes: Vec<u8>) -> SinglePart {
    use lettre::message::{Body, header::ContentTransferEncoding};
    let bytes = if crate::domain::attached_message::is_message(&name, "") {
        match Body::new_with_encoding(bytes, ContentTransferEncoding::SevenBit) {
            Ok(body) => {
                return LAttachment::new(name).body(body, ContentType::parse("message/rfc822").unwrap());
            }
            Err(bytes) => bytes,
        }
    } else {
        bytes
    };
    LAttachment::new(name).body(bytes, ContentType::parse("application/octet-stream").unwrap())
}

pub fn send(a: &Account, from_name: &str, out: &Outgoing) -> Result<(), String> {
    deliver(a, from_name, out).map_err(|e| e.to_string())
}

/// Failures before SMTP submission are safe to correct and retry.
pub fn deliver(a: &Account, from_name: &str, out: &Outgoing) -> crate::application::AppResult<()> {
    use crate::application::AppError;
    let msg = build_message(a, from_name, out).map_err(AppError::delivery_not_started)?;
    let transport = smtp_transport(a).map_err(AppError::delivery_not_started)?;
    // lettre connects, authenticates and delivers inside `send`, so a success is
    // the SMTP proof of connectivity that wakes this account's confirmations.
    transport.send(&msg).map_err(|e| AppError::external(e.to_string()))?;
    crate::application::connectivity::note_success(a.id);
    Ok(())
}

fn smtp_transport(a: &Account) -> Result<SmtpTransport, String> {
    let user = if a.username.is_empty() { a.email.clone() } else { a.username.clone() };
    let port = if a.smtp_port == 0 { 465 } else { a.smtp_port };
    let builder = if port == 465 { SmtpTransport::relay(&a.smtp_host) } else { SmtpTransport::starttls_relay(&a.smtp_host) };
    let builder = builder.map_err(|e| e.to_string())?.port(port).timeout(Some(std::time::Duration::from_secs(30)));
    Ok(if a.oauth_client_id.is_empty() {
        builder.credentials(Credentials::new(user, a.password.clone())).build()
    } else {
        builder.credentials(Credentials::new(user, crate::oauth::access_token(a)?)).authentication(vec![Mechanism::Xoauth2]).build()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Explicit read-only live mailbox diagnostic; requires QAMILLE_PROBE_ID and QAMILLE_DATA"]
    fn live_read_latency_probe() {
        use std::time::Instant;
        let root = std::env::var_os("QAMILLE_DATA").expect("Set the workspace explicitly");
        let id: u64 = std::env::var("QAMILLE_PROBE_ID").expect("Set a message ID explicitly").parse().unwrap();
        let store: crate::model::Store = serde_json::from_slice(&std::fs::read(std::path::PathBuf::from(root).join("store.json")).unwrap()).unwrap();
        let email = store.email(id).expect("Message not found");
        let mut account = store.account(email.account_id).unwrap().clone();
        account.password = crate::secret::get_result(&crate::secret::account_key(&account.email)).unwrap();
        assert!(!account.password.is_empty(), "Saved account credentials are required");
        if std::env::var_os("QAMILLE_PROBE_READER").is_some() {
            use crate::application::ports::MailGateway;
            let reader = super::super::gateways::GmailGateway::new(std::path::PathBuf::new());
            for (index, email) in std::iter::once(email).chain(store.emails.iter().filter(|other| other.account_id == account.id && other.id != id).take(1)).enumerate() {
                let phase = Instant::now();
                let loaded = reader.load_body_preview(&account, email).unwrap();
                eprintln!("mail_probe text_read={} elapsed_ms={} body_bytes={}", index + 1, phase.elapsed().as_millis(), loaded.body.len());
                if loaded.body.is_empty() {
                    let originals = super::download_originals(&account, std::slice::from_ref(email)).unwrap();
                    let original = super::parse_message_inner(&originals[0], email.id, email.account_id, None).unwrap();
                    eprintln!("mail_probe empty_body_original_bytes={}", original.body.len());
                    assert!(original.body.is_empty(), "Text reader omitted an existing message body");
                }
            }
            return;
        }
        let started = Instant::now();
        let mut session = super::connect(&account).expect("Connection failed");
        eprintln!("mail_probe connection_ms={}", started.elapsed().as_millis());
        let phase = Instant::now();
        let (_, uid) = match super::locate_known_message(&mut session, email).unwrap() {
            Some(location) => location,
            None => {
                let folders = super::export_folders(&mut session, &account, false).unwrap();
                super::locate_message(&mut session, &folders, email).unwrap()
            }
        };
        eprintln!("mail_probe location_ms={}", phase.elapsed().as_millis());
        let phase = Instant::now();
        let fetched = session.uid_fetch(uid.to_string(), "BODY.PEEK[]").unwrap();
        let raw = fetched.iter().find(|item| item.uid == Some(uid)).and_then(|item| item.body()).unwrap();
        eprintln!("mail_probe transfer_ms={} mime_bytes={}", phase.elapsed().as_millis(), raw.len());
        let phase = Instant::now();
        let parsed = super::parse_message_inner(raw, email.id, email.account_id, None).unwrap();
        eprintln!("mail_probe parse_ms={} body_bytes={} attachments={}", phase.elapsed().as_millis(), parsed.body.len(), parsed.attachments.len());
        let phase = Instant::now();
        let _ = session.logout();
        eprintln!("mail_probe logout_ms={} total_ms={}", phase.elapsed().as_millis(), started.elapsed().as_millis());
    }
    struct ScriptWire {
        input:std::io::Cursor<Vec<u8>>,
        output:std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl std::io::Read for ScriptWire {fn read(&mut self,bytes:&mut[u8])->std::io::Result<usize>{std::io::Read::read(&mut self.input,bytes)}}
    impl std::io::Write for ScriptWire {
        fn write(&mut self,bytes:&[u8])->std::io::Result<usize>{self.output.lock().unwrap().extend_from_slice(bytes);Ok(bytes.len())}
        fn flush(&mut self)->std::io::Result<()>{Ok(())}
    }
    fn scripted_session(script:&[u8])->(imap::Session<ScriptWire>,std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let output=std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let client=imap::Client::new(ScriptWire{input:std::io::Cursor::new(script.to_vec()),output:output.clone()});
        let session=client.login("sample","sample").map_err(|e|e.0).unwrap();(session,output)
    }
    #[test]
    fn online_read_verifies_saved_uid_without_listing_or_searching_other_folders() {
        let (mut session, output) = scripted_session(b"a1 OK logged in\r\n* OK [UIDVALIDITY 1] valid\r\na2 OK examined\r\n* SEARCH 7\r\na3 OK found\r\n");
        let email = crate::model::Email { uid: Some(7), remote_folder: Some("INBOX".into()), remote_uid_validity: Some(1), remote_global_id: Some(900), ..Default::default() };
        assert_eq!(super::locate_known_message(&mut session, &email).unwrap(), Some(("INBOX".into(), 7)));
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("EXAMINE \"INBOX\""), "{commands}");
        assert!(commands.contains("UID SEARCH UID 7 X-GM-MSGID 900"), "{commands}");
        assert!(!commands.contains("LIST"));
    }
    #[test]
    fn online_read_rejects_changed_mailbox_generations_and_missing_provider_identity() {
        let email = crate::model::Email { uid: Some(7), remote_uid_validity: Some(1), remote_global_id: Some(900), ..Default::default() };
        for (script, missing_identity) in [
            (b"a1 OK logged in\r\n* OK [UIDVALIDITY 2] changed\r\na2 OK examined\r\n".as_slice(), false),
            (b"a1 OK logged in\r\n* OK [UIDVALIDITY 1] valid\r\na2 OK examined\r\n* SEARCH\r\na3 OK absent\r\n".as_slice(), false),
            (b"a1 OK logged in\r\n".as_slice(), true),
        ] {
            let (mut session, _) = scripted_session(script);
            let mut email = email.clone();
            if missing_identity { email.remote_global_id = None; }
            assert!(super::locate_known_message(&mut session, &email).unwrap().is_none());
        }
    }
    #[test]
    fn move_fallback_expunge_is_limited_to_the_moved_uid() {
        let (mut session,output)=scripted_session(b"a1 OK logged in\r\na2 NO MOVE unavailable\r\na3 OK copied\r\na4 OK flagged\r\na5 OK expunged\r\n");
        super::apply_uid_action(&mut session,7,super::Remote::Move("Archive".into())).unwrap();
        let commands=String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID COPY 7"));assert!(commands.contains("UID STORE 7 +FLAGS (\\Deleted)"));assert!(commands.contains("UID EXPUNGE 7\r\n"));
        assert!(!commands.lines().any(|line|line.split_whitespace().nth(1)==Some("EXPUNGE")),"No mailbox-wide EXPUNGE may remove unrelated Deleted messages: {commands}");
    }
    fn gmail_session(script: &[u8]) -> (super::extension::Session<ScriptWire>, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let input = [b"* OK hello\r\na1 OK login\r\n".as_slice(), script].concat();
        let session = super::extension::Session::new(ScriptWire { input: std::io::Cursor::new(input), output: output.clone() }, &crate::model::Account::default()).unwrap();
        (session, output)
    }
    #[test]
    fn server_search_reads_one_page_of_metadata_from_all_mail_with_gmail_search() {
        let header = |n: u32| format!("From: Anna <anna@example.com>\r\nSubject: Fattura {n}\r\nDate: Mon, 1 Jan 2019 10:00:00 +0000\r\nMessage-ID: <m{n}@example.com>\r\n\r\n");
        let fetch = |seq: u32, uid: u32| { let h = header(uid); format!("* {seq} FETCH (UID {uid} FLAGS (\\Seen) RFC822.SIZE 1000 BODY[HEADER] {{{}}}\r\n{h})\r\n", h.len()) };
        let script = [
            "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/Tutti i messaggi\"\r\na2 OK listed\r\n".to_string(),
            "* 3000 EXISTS\r\n* OK [UIDVALIDITY 5] ok\r\na3 OK examined\r\n".into(),
            "+ go\r\n* SEARCH 4 9 12\r\ngmail1 OK found\r\n".into(),
            "* 3000 EXISTS\r\n* OK [UIDVALIDITY 5] ok\r\na4 OK examined\r\n".into(),
            format!("{}{}a5 OK fetched\r\n", fetch(1, 12), fetch(2, 9)),
            "* 1 FETCH (UID 12 X-GM-MSGID 112 X-GM-THRID 1 X-GM-LABELS (\\Inbox \"Clienti\"))\r\n* 2 FETCH (UID 9 X-GM-MSGID 109 X-GM-THRID 2 X-GM-LABELS ())\r\ngmail2 OK done\r\n".into(),
        ].concat();
        let (mut session, output) = gmail_session(script.as_bytes());
        let request = crate::application::ports::RemoteSearchRequest { raw: "fattura label:\"Clienti\"".into(), page_size: 2, ..Default::default() };
        let page = super::search_in(&mut session, &crate::model::Account { id: 3, ..Default::default() }, &request, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("X-GM-RAW {"), "{commands}");
        assert!(!commands.contains("BODY.PEEK[]"), "never a body: {commands}");
        assert!(commands.contains("UID FETCH 12,9 "), "the newest first: {commands}");
        assert_eq!((page.total, page.mailbox_total), (3, 3000));
        assert_eq!(page.next, Some(crate::application::ports::RemoteCursor { folder: "[Gmail]/Tutti i messaggi".into(), uid_validity: 5, before_uid: 9 }));
        assert_eq!(page.emails.len(), 2);
        let inbox = page.emails.iter().find(|e| e.uid == Some(12)).unwrap();
        assert_eq!((inbox.folder.as_str(), inbox.labels.clone(), inbox.remote_global_id), ("Inbox", vec!["Clienti".to_string()], Some(112)));
        assert!(inbox.remote_only && inbox.read && inbox.body.is_empty());
        assert_eq!(page.emails.iter().find(|e| e.uid == Some(9)).unwrap().folder, "Archive");
    }
    #[test]
    fn server_search_without_all_mail_says_how_to_enable_it() {
        let (mut session, _) = gmail_session(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\na2 OK listed\r\n");
        let request = crate::application::ports::RemoteSearchRequest { raw: "x".into(), page_size: 50, ..Default::default() };
        let error = super::search_in(&mut session, &crate::model::Account::default(), &request, &std::sync::atomic::AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("All Mail"), "{error}");
    }
    #[test]
    fn a_history_batch_reads_the_newest_uids_of_all_mail_as_metadata() {
        let header = |n: u32| format!("From: Anna <anna@example.com>\r\nSubject: Vecchia {n}\r\nDate: Mon, 1 Jan 2018 10:00:00 +0000\r\nMessage-ID: <h{n}@example.com>\r\n\r\n");
        let fetch = |seq: u32, uid: u32| { let h = header(uid); format!("* {seq} FETCH (UID {uid} FLAGS () RFC822.SIZE 900 BODY[HEADER] {{{}}}\r\n{h})\r\n", h.len()) };
        let script = [
            "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\r\na2 OK listed\r\n".to_string(),
            "* 950 EXISTS\r\n* OK [UIDVALIDITY 5] ok\r\n* OK [UIDNEXT 1001] next\r\na3 OK examined\r\n".into(),
            "* SEARCH 900 600 77\r\na4 OK found\r\n".into(),
            format!("{}{}a5 OK fetched\r\n", fetch(1, 900), fetch(2, 600)),
            "* 1 FETCH (UID 900 X-GM-MSGID 1900 X-GM-THRID 1 X-GM-LABELS (\\Inbox))\r\n* 2 FETCH (UID 600 X-GM-MSGID 1600 X-GM-THRID 2 X-GM-LABELS (\\Draft))\r\ngmail1 OK done\r\n".into(),
        ].concat();
        let (mut session, output) = gmail_session(script.as_bytes());
        let request = crate::application::ports::HistoryRequest { progress: Default::default(), batch_uids: 500, roles: Vec::new() };
        let batch = super::history_in(&mut session, &crate::model::Account { id: 3, ..Default::default() }, &request).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID SEARCH UID 501:1000"), "{commands}");
        assert!(commands.contains("UID FETCH 900,600 "), "UIDs outside the range are ignored, newest first: {commands}");
        assert!(!commands.contains("BODY.PEEK[]"), "never a body: {commands}");
        assert_eq!((batch.folder.as_str(), batch.uid_validity, batch.uid_next, batch.exists), ("[Gmail]/All Mail", 5, 1001, 950));
        assert_eq!(batch.range.map(|r| (r.start, r.end, r.older)), Some((501, 1000, true)));
        assert_eq!(batch.emails.len(), 1, "the draft is left out");
        let row = &batch.emails[0];
        assert_eq!((row.folder.as_str(), row.remote_global_id, row.remote_folder.as_deref()), ("Inbox", Some(1900), Some("[Gmail]/All Mail")));
        assert!(row.remote_only && !row.read && batch.refresh.is_empty());
    }
    #[test]
    fn a_finished_history_reads_back_the_labels_of_the_newest_messages() {
        let script = [
            "* LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/All Mail\"\r\na2 OK listed\r\n",
            "* 950 EXISTS\r\n* OK [UIDVALIDITY 5] ok\r\n* OK [UIDNEXT 1001] next\r\na3 OK examined\r\n",
            "* 1 FETCH (UID 999 FLAGS (\\Seen \\Flagged))\r\n* 2 FETCH (UID 998 FLAGS ())\r\na4 OK fetched\r\n",
            "* 1 FETCH (UID 999 X-GM-MSGID 1999 X-GM-THRID 1 X-GM-LABELS (\"Clienti\"))\r\n* 2 FETCH (UID 998 X-GM-MSGID 1998 X-GM-THRID 2 X-GM-LABELS (\\Inbox))\r\ngmail1 OK done\r\n",
        ].concat();
        let (mut session, output) = gmail_session(script.as_bytes());
        let progress = crate::domain::history::HistoryProgress { uid_validity: Some(5), before_uid: 1, highest_seen: 1000, complete: true, ..Default::default() };
        let request = crate::application::ports::HistoryRequest { progress, batch_uids: 500, roles: Vec::new() };
        let batch = super::history_in(&mut session, &crate::model::Account::default(), &request).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID FETCH 501:1000 (UID FLAGS)"), "{commands}");
        assert!(!commands.contains("BODY"), "no header, no body: {commands}");
        assert!(batch.range.is_none() && batch.emails.is_empty());
        let archived = batch.refresh.iter().find(|s| s.global_id == 1999).unwrap();
        assert_eq!((archived.labels.clone(), archived.read, archived.starred), (vec!["Clienti".to_string()], true, true));
        assert_eq!(batch.refresh.len(), 2);
    }
    #[test]
    fn a_history_batch_marks_old_inbox_mail_read_and_reads_spam_when_asked() {
        let script = [
            "* LIST (\\All) \"/\" \"[Gmail]/All Mail\"\r\n* LIST (\\Junk) \"/\" \"[Gmail]/Spam\"\r\na2 OK listed\r\n",
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 5] ok\r\n* OK [UIDNEXT 1] next\r\na3 OK [READ-WRITE] selected\r\n",
            "* SEARCH 7 9\r\ngmail1 OK found\r\n",
            "a4 OK stored\r\n",
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 2] ok\r\na5 OK examined\r\n",
            "* SEARCH\r\na6 OK none\r\n",
        ].concat();
        let (mut session, output) = gmail_session(script.as_bytes());
        let progress = crate::domain::history::HistoryProgress { mark_read_before: Some(1_767_225_600), ..Default::default() };
        let request = crate::application::ports::HistoryRequest { progress, batch_uids: 500, roles: vec!["Spam".into(), "Trash".into()] };
        let batch = super::history_in(&mut session, &crate::model::Account::default(), &request).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("SELECT \"[Gmail]/All Mail\""), "{commands}");
        assert!(commands.contains("in:inbox is:unread before:2026/01/01"), "{commands}");
        assert!(commands.contains("UID STORE 7,9 +FLAGS.SILENT (\\Seen)"), "{commands}");
        assert!(commands.contains("EXAMINE \"[Gmail]/Spam\""), "{commands}");
        assert_eq!((batch.marked_read, batch.roles_read.clone()), (2, vec!["Spam".to_string()]), "Trash is not exposed here");
    }
    fn saved_inbox_message() -> crate::model::Email {
        crate::model::Email { id: 71, uid: Some(7), remote_folder: Some("INBOX".into()), remote_uid_validity: Some(1), remote_global_id: Some(900), ..Default::default() }
    }
    #[test]
    fn done_verifies_the_saved_location_without_scanning_every_folder() {
        let (mut session, output) = gmail_session(b"* OK [UIDVALIDITY 1] valid\r\na2 OK examined\r\n* SEARCH 7\r\na3 OK found\r\n* OK [UIDVALIDITY 1] valid\r\na4 OK examined\r\n* 1 FETCH (UID 7 X-GM-MSGID 900)\r\ngmail1 OK fetched\r\n");
        let resolved = super::resolve_identity_in(&mut session, &crate::model::Account::default(), &saved_inbox_message()).unwrap();
        assert_eq!((resolved.remote_folder.as_deref(), resolved.uid, resolved.remote_uid_validity), (Some("INBOX"), Some(7), Some(1)));
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID SEARCH UID 7 X-GM-MSGID 900"), "{commands}");
        assert!(!commands.contains("LIST"), "Done must not list and search every folder: {commands}");
    }
    #[test]
    fn done_archives_from_the_verified_saved_location_without_listing_folders() {
        let (mut session, output) = gmail_session(b"* OK [UIDVALIDITY 1] valid\r\na2 OK examined\r\n* SEARCH 7\r\na3 OK found\r\n* OK [UIDVALIDITY 1] valid\r\na4 OK [READ-WRITE] selected\r\n* 1 FETCH (X-GM-LABELS () UID 7)\r\n* 1 EXPUNGE\r\ngmail1 OK removed\r\n");
        super::push_message_action_in(&mut session, &crate::model::Account::default(), &saved_inbox_message(), super::Remote::Archive).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("SELECT \"INBOX\""), "{commands}");
        assert!(commands.contains("gmail1 UID STORE 7 -X-GM-LABELS (\\Inbox)\r\n"), "{commands}");
        assert!(!commands.contains("LIST"), "Done must not list and search every folder: {commands}");
    }
    #[test]
    fn done_still_scans_the_mailbox_when_the_saved_location_changed() {
        let (mut session, output) = gmail_session(b"* OK [UIDVALIDITY 2] changed\r\na2 OK examined\r\n* LIST (\\HasNoChildren \\All) \"/\" \"[Gmail]/All Mail\"\r\na3 OK listed\r\n* OK [UIDVALIDITY 5] valid\r\na4 OK examined\r\n* SEARCH 31\r\na5 OK found\r\n* OK [UIDVALIDITY 5] valid\r\na6 OK examined\r\n* 3 FETCH (UID 31 X-GM-MSGID 900)\r\ngmail1 OK fetched\r\n");
        let resolved = super::resolve_identity_in(&mut session, &crate::model::Account::default(), &saved_inbox_message()).unwrap();
        assert_eq!((resolved.remote_folder.as_deref(), resolved.uid, resolved.remote_uid_validity), (Some("[Gmail]/All Mail"), Some(31), Some(5)));
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("LIST"), "{commands}");
    }
    #[test]
    fn archive_removes_the_gmail_inbox_label_and_reads_gmails_label_reply() {
        let (mut session, output) = gmail_session(b"* 1 FETCH (X-GM-LABELS (\\Important) UID 7)\r\n* 1 EXPUNGE\r\ngmail1 OK removed\r\na2 OK noop\r\n");
        super::apply_session_action(&mut session, 7, super::Remote::Archive).unwrap();
        session.noop().unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("gmail1 UID STORE 7 -X-GM-LABELS (\\Inbox)\r\n"), "{commands}");
        assert!(!commands.contains("MOVE") && !commands.contains("EXPUNGE"), "{commands}");
    }
    #[test]
    fn label_reply_with_a_literal_does_not_end_the_response_early() {
        let (mut session, _) = gmail_session(b"* 1 FETCH (X-GM-LABELS ({12}\r\ngmail1 OK x \\Important) UID 7)\r\ngmail1 OK removed\r\na2 OK noop\r\n");
        super::apply_session_action(&mut session, 7, super::Remote::Archive).unwrap();
        session.noop().unwrap();
    }
    #[test]
    fn rejected_archive_is_an_error() {
        let (mut session, _) = gmail_session(b"gmail1 NO not allowed\r\n");
        assert!(super::apply_session_action(&mut session, 7, super::Remote::Archive).is_err());
    }
    #[test]
    fn removing_gmail_label_quotes_value_and_rejects_command_injection() {
        let (mut session, output) = gmail_session(b"* 1 FETCH (X-GM-LABELS () UID 7)\r\ngmail1 OK removed\r\n");
        super::apply_session_action(&mut session, 7, super::Remote::RemoveLabel("A \"quoted\" \\ label".into())).unwrap();
        let commands = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID STORE 7 -X-GM-LABELS (\"A \\\"quoted\\\" \\\\ label\")\r\n"), "{commands}");
        let (mut session, output) = gmail_session(b"");
        assert!(super::apply_session_action(&mut session, 7, super::Remote::RemoveLabel("bad\r\nLOGOUT".into())).is_err());
        assert!(!String::from_utf8(output.lock().unwrap().clone()).unwrap().contains("UID STORE"));
    }
    #[test]
    fn spam_action_resolves_the_localized_provider_folder() {
        let(mut session,output)=scripted_session(b"a1 OK logged in\r\n* LIST (\\HasNoChildren \\Junk) \"/\" \"[Gmail]/Posta indesiderata\"\r\na2 OK listed\r\na3 OK moved\r\n");
        super::apply_uid_action(&mut session,7,super::Remote::Spam).unwrap();
        let commands=String::from_utf8(output.lock().unwrap().clone()).unwrap();assert!(commands.contains("UID MOVE 7 \"[Gmail]/Posta indesiderata\""));
    }
    #[test]
    fn blind_copy_is_in_envelope_and_never_in_delivered_headers() {
        let account=crate::model::Account{email:"from@example.test".into(),..Default::default()};
        let outgoing=super::Outgoing{message_id:Some("<stable@example.test>".into()),to:vec!["to@example.test".into()],cc:vec![],bcc:vec!["private@example.test".into()],subject:"Test".into(),body:"Synthetic".into(),html:None,in_reply_to:None,attachments:vec![]};
        let message=super::build_message(&account,"Sender",&outgoing).unwrap();
        assert!(message.envelope().to().iter().any(|address|address.to_string()=="private@example.test"));
        let headers=String::from_utf8(message.formatted()).unwrap();
        assert!(!headers.to_lowercase().contains("bcc:"));assert!(!headers.contains("private@example.test"));assert!(headers.contains("<stable@example.test>"));
    }
    #[test]
    fn gmail_thread_lookup_handles_the_actual_imap_parser_and_never_reuses_its_session() {
        #[derive(Debug)]
        struct Wire(std::io::Cursor<Vec<u8>>);
        impl std::io::Read for Wire {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> { std::io::Read::read(&mut self.0, bytes) }
        }
        impl std::io::Write for Wire {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { Ok(bytes.len()) }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let response = b"a1 OK logged in\r\n* 1 FETCH (X-GM-THRID 1278455344230334865 UID 9)\r\na2 OK done\r\n";
        let client = imap::Client::new(Wire(std::io::Cursor::new(response.to_vec())));
        let session = client.login("sample", "sample").unwrap();
        assert_eq!(super::fetch_thread_id(session, 9).unwrap(), 1278455344230334865);
    }
    #[test]
    fn export_identifiers_are_quoted_and_thread_response_is_read_as_u64() {
        assert!(super::imap_quote("id\r\nLOGOUT").is_err());
        assert_eq!(super::imap_quote("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
        assert_eq!(super::gmail_thread_id(b"* 1 FETCH (X-GM-THRID 1278455344230334865 UID 9)\r\nA1 OK done\r\n", 9), Some(1278455344230334865));
        assert_eq!(super::gmail_thread_id(b"* 1 FETCH (X-GM-THRID 100 UID 8)\r\n* 2 FETCH (UID 9 X-GM-THRID 200)\r\n", 9), Some(200));
        assert_eq!(super::gmail_thread_id(b"A1 OK X-GM-THRID 123\r\n", 9), None);
    }
    use super::*;

    #[test]
    fn machine_mail_headers_are_read() {
        let raw = b"From: News <news@x.org>\r\nTo: me@x.org\r\nSubject: Hi\r\nList-Unsubscribe: <mailto:u@x.org>\r\nAuto-Submitted: auto-generated\r\nContent-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\n";
        let e = parse_message(raw, 1, 1, &std::env::temp_dir()).unwrap();
        assert!(e.bulk && e.auto_submitted && e.calendar);
        let plain = b"From: Ann <a@x.org>\r\nTo: me@x.org\r\nSubject: Hi\r\nAuto-Submitted: no\r\n\r\nHello\r\n";
        let e = parse_message(plain, 2, 1, &std::env::temp_dir()).unwrap();
        assert!(!e.bulk && !e.auto_submitted && !e.calendar);
    }

    #[test]
    fn google_account_settings() {
        let a = gmail_account(1, "", " me@gmail.com ", "abcd efgh ijkl mnop", [0, 0, 0]);
        assert_eq!(a.imap_host, "imap.gmail.com");
        assert_eq!(a.smtp_host, "smtp.gmail.com");
        assert_eq!(a.password, "abcdefghijklmnop");
        assert_eq!(a.name, "me@gmail.com");
        assert_eq!(a.kind, AccountKind::Gmail);
        assert!(looks_like_email("me@gmail.com"));
        assert!(!looks_like_email("me"));
    }

    fn gmail_mail(subject: &str) -> String {
        format!("From: Maria <maria@example.com>\r\nTo: me@gmail.com\r\nSubject: {subject}\r\nMessage-ID: <{subject}@example.com>\r\n\r\nCiao\r\n")
    }

    /// A stand-in for Gmail's server, with replies written the way Gmail words them.
    fn fake_gmail() -> std::net::SocketAddr {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut w = stream.try_clone().unwrap();
            let mut r = BufReader::new(stream);
            w.write_all(b"* OK Gimap ready for requests from 127.0.0.1 x1\r\n").unwrap();
            let mut line = String::new();
            while { line.clear(); r.read_line(&mut line).unwrap_or(0) > 0 } {
                let tag = line.split(' ').next().unwrap().to_string();
                let upper = line.to_uppercase();
                let reply = if upper.contains(" LOGIN ") {
                    format!("* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS\r\n{tag} OK me@gmail.com authenticated (Success)\r\n")
                } else if upper.contains(" LIST ") {
                    format!("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n* LIST (\\HasChildren \\Noselect) \"/\" \"[Gmail]\"\r\n* LIST (\\All \\HasNoChildren) \"/\" \"[Gmail]/Tutti i messaggi\"\r\n* LIST (\\HasNoChildren \\Trash) \"/\" \"[Gmail]/Cestino\"\r\n{tag} OK Success\r\n")
                } else if upper.contains(" SELECT ") {
                    format!("* FLAGS (\\Answered \\Flagged \\Draft \\Deleted \\Seen $NotPhishing $Phishing)\r\n* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Draft \\Deleted \\Seen $NotPhishing $Phishing \\*)] Flags permitted.\r\n* OK [UIDVALIDITY 3] UIDs valid.\r\n* 2 EXISTS\r\n* 0 RECENT\r\n* OK [UIDNEXT 13] Predicted next UID.\r\n* OK [HIGHESTMODSEQ 991]\r\n{tag} OK [READ-WRITE] INBOX selected. (Success)\r\n")
                } else if upper.contains(" FETCH ") {
                    let (m1, m2) = (gmail_mail("Uno"), gmail_mail("Due"));
                    format!("* 1 FETCH (UID 11 FLAGS (\\Seen \\Flagged) BODY[] {{{}}}\r\n{m1})\r\n* 2 FETCH (UID 12 FLAGS () BODY[] {{{}}}\r\n{m2})\r\n{tag} OK Success\r\n", m1.len(), m2.len())
                } else {
                    format!("* BYE LOGOUT Requested\r\n{tag} OK 73 good day (Success)\r\n")
                };
                w.write_all(reply.as_bytes()).unwrap();
                if upper.contains(" LOGOUT") {
                    break;
                }
            }
        });
        addr
    }

    #[test]
    fn after_login_gmail_replies_are_understood() {
        let tcp = std::net::TcpStream::connect(fake_gmail()).unwrap();
        let mut client = imap::Client::new(tcp);
        client.read_greeting().unwrap();
        let mut s = client.login("me@gmail.com", "abcdabcdabcdabcd").map_err(|e| e.0.to_string()).unwrap();
        let (all, trash) = list_folders(&mut s).unwrap();
        assert_eq!(all.as_deref(), Some("[Gmail]/Tutti i messaggi"));
        assert_eq!(trash.as_deref(), Some("[Gmail]/Cestino"));
        let acc = gmail_account(1, "", "me@gmail.com", "x", [0, 0, 0]);
        let mails = fetch_inbox(&mut s, &acc, &std::env::temp_dir(), 100).unwrap();
        s.logout().unwrap();
        assert_eq!(mails.len(), 2);
        assert_eq!(mails[0].subject, "Uno");
        assert_eq!(mails[0].uid, Some(11));
        assert!(mails[0].read && mails[0].starred);
        assert_eq!(mails[1].subject, "Due");
        assert!(!mails[1].read);
    }

    #[test]
    fn oauth_sign_in_is_what_the_server_receives() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut w = stream.try_clone().unwrap();
            let mut r = BufReader::new(stream);
            w.write_all(b"* OK Gimap ready\r\n").unwrap();
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            assert!(first.contains("AUTHENTICATE XOAUTH2"), "{first}");
            let tag = first.split(' ').next().unwrap().to_string();
            w.write_all(b"+ \r\n").unwrap();
            let mut proof = String::new();
            r.read_line(&mut proof).unwrap();
            w.write_all(format!("{tag} OK me@gmail.com authenticated (Success)\r\n").as_bytes()).unwrap();
            proof
        });
        let mut client = imap::Client::new(std::net::TcpStream::connect(addr).unwrap());
        client.read_greeting().unwrap();
        let auth = XOAuth2 { user: "me@gmail.com".into(), token: "AT".into() };
        let session = client.authenticate("XOAUTH2", &auth).map_err(|e| e.0.to_string()).unwrap();
        drop(session);
        // Base64 of user=me@gmail.com, auth=Bearer AT, with the separators the protocol requires.
        assert_eq!(server.join().unwrap().trim(), "dXNlcj1tZUBnbWFpbC5jb20BYXV0aD1CZWFyZXIgQVQBAQ==");
    }

    #[test]
    fn attached_eml_files_are_sent_as_messages_when_seven_bit_clean() {
        let dir = std::env::temp_dir().join(format!("qamille-eml-send-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clean = dir.join("email-7-Avviso.eml");
        std::fs::write(&clean, b"From: a@example.com\r\nSubject: Avviso\r\n\r\nTesto\r\n").unwrap();
        let binary = dir.join("email-8-Binario.eml");
        std::fs::write(&binary, [b'S', b'u', b'b', 0xff, 0x00, b'\n']).unwrap();
        let report = dir.join("report.txt");
        std::fs::write(&report, b"plain").unwrap();
        let acc = gmail_account(1, "", "me@gmail.com", "x", [0, 0, 0]);
        let out = Outgoing { message_id: None, to: vec!["a@example.com".into()], cc: vec![], bcc: vec![], subject: "S".into(), body: "ciao".into(), html: None, in_reply_to: None, attachments: [&clean, &binary, &report].iter().map(|p| p.to_string_lossy().into_owned()).collect() };
        let raw = build_message(&acc, "Me", &out).unwrap().formatted();
        std::fs::remove_dir_all(&dir).unwrap();
        let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
        let types: Vec<_> = parsed
            .attachments()
            .map(|part| {
                let c = part.content_type().unwrap();
                format!("{}/{}", c.ctype(), c.subtype().unwrap_or(""))
            })
            .collect();
        assert_eq!(types, ["message/rfc822", "application/octet-stream", "application/octet-stream"]);
        let forwarded = parsed.attachment(0).unwrap().message().expect("parsed as a nested message");
        assert_eq!(forwarded.subject(), Some("Avviso"));
    }

    #[test]
    fn a_sender_name_with_punctuation_still_sends() {
        let acc = gmail_account(1, "", "me@gmail.com", "x", [0, 0, 0]);
        let out = Outgoing { message_id:None,to: vec!["a@example.com".into()], cc: vec![], bcc: vec![], subject: "S".into(), body: "ciao".into(), html: None, in_reply_to: None, attachments: vec![] };
        for name in ["Rossi, Mario", "Rossi; Mario", "Mario (Rossi)", "Mario <Rossi>", "Mario \"Il Capo\" Rossi", "Jörg Müller", ""] {
            let raw = String::from_utf8(build_message(&acc, name, &out).unwrap_or_else(|e| panic!("{name:?}: {e}")).formatted()).unwrap();
            let from = raw.lines().find(|l| l.starts_with("From:")).unwrap();
            assert!(from.contains("<me@gmail.com>") || from.trim_end() == "From: me@gmail.com", "{name:?}: {from}");
        }
        // However the name travels on the wire, the receiver must read it back whole.
        let formatted = build_message(&acc, "Rossi, Mario", &out).unwrap().formatted();
        let parsed = mail_parser::MessageParser::default().parse(&formatted).unwrap();
        let from = parsed.from().and_then(|f| f.first()).unwrap();
        assert_eq!(from.name(), Some("Rossi, Mario"));
        assert_eq!(from.address(), Some("me@gmail.com"));
    }

    #[test]
    fn formatted_mail_carries_both_versions() {
        let acc = gmail_account(1, "", "me@gmail.com", "x", [0, 0, 0]);
        let mut out = Outgoing { message_id:None,to: vec!["a@example.com".into()], cc: vec![], bcc: vec![], subject: "S".into(), body: "ciao".into(), html: Some("<b>ciao</b>".into()), in_reply_to: None, attachments: vec![] };
        let raw = String::from_utf8(build_message(&acc, "Me", &out).unwrap().formatted()).unwrap();
        assert!(raw.contains("multipart/alternative") && raw.contains("text/plain") && raw.contains("text/html"), "{raw}");
        out.html = None;
        let raw = String::from_utf8(build_message(&acc, "Me", &out).unwrap().formatted()).unwrap();
        assert!(raw.contains("text/plain") && !raw.contains("text/html"), "{raw}");
    }

    #[test]
    fn oauth_account_uses_the_refresh_token() {
        let a = oauth_account(3, "me@gmail.com", "cid", "RT", [0, 0, 0]);
        assert_eq!((a.password.as_str(), a.oauth_client_id.as_str(), a.imap_host.as_str()), ("RT", "cid", "imap.gmail.com"));
    }

    #[test]
    fn sign_in_errors_say_what_to_do() {
        assert!(friendly_login_error("IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)", false).contains("app password"));
        assert!(friendly_login_error("SMTP: permanent error (535): Username and Password not accepted", false).contains("app password"));
        assert!(friendly_login_error("IMAP connect: failed to lookup address", false).contains("internet"));
    }

    #[test]
    fn a_google_sign_in_refusal_never_talks_about_app_passwords() {
        for raw in ["IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)", "SMTP: permanent error (535): Username and Password not accepted"] {
            let e = friendly_login_error(raw, true);
            assert!(!e.to_lowercase().contains("app password"), "{e}");
            assert!(e.contains("Gmail box") && e.contains(raw), "{e}");
        }
        assert!(friendly_login_error("IMAP connect: failed to lookup address", true).contains("internet"));
    }

    /// Talks to Google: run by hand with `cargo test gmail_refuses -- --ignored`.
    #[test]
    #[ignore]
    fn gmail_refuses_a_wrong_password() {
        let a = gmail_account(1, "", "qamille-no-such-user-000@gmail.com", "wrong", [0, 0, 0]);
        let err = connect(&a).err().expect("login must fail");
        assert!(err.starts_with("IMAP login"), "{err}");
        let full = check_login(&a).expect_err("the check must fail");
        println!("{full}");
        assert!(full.contains("app password"), "{full}");
        // Sending has its own sign-in: it must reach Google and be refused too.
        let smtp = smtp_transport(&a).unwrap().test_connection().expect_err("sending sign-in must fail").to_string();
        println!("{smtp}");
        assert!(friendly_login_error(&smtp, false).contains("app password"), "{smtp}");
    }
}

/// The retry contract of the failure texts this file produces (ADR-0007 v3,
/// decision 8, INV-008/INV-010, EX-009/EX-010). A transient network failure must
/// keep a confirmation queued; anything the provider may already have executed,
/// and any refusal, must stay definitive.
#[cfg(test)]
mod retry_classification_tests {
    #[test]
    fn the_observed_offline_failure_is_retryable() {
        // The real error behind "Impossibile confermare: Segnato come fatto … os error 10060".
        let observed = "IMAP login: Impossibile stabilire la connessione. Una operazione di \
                        connessione non riuscita (os error 10060)";
        assert!(super::is_retryable_connect_error(observed), "{observed}");
        assert!(super::is_retryable_connect_error("IMAP connect: os error 10060"));
    }

    #[test]
    fn a_refused_sign_in_is_never_retryable() {
        for message in [
            "IMAP login: [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            "IMAP login: Invalid credentials (Failure)",
            "IMAP login: [ALERT] Please log in via your web browser: https://support.google.com/mail/answer/78754 (Failure)",
            "SMTP: permanent error (535): 5.7.8 Username and Password not accepted",
            "SMTP: permanent error (534): 5.7.9 Application-specific password required",
            "Google refused the Qamille sign-in. Sign in with Google again and tick the Gmail box on the permissions page. (Google said: IMAP login: no)",
            "Google refused this address and password. Use an app password from Google's page, not your normal password; it needs 2-Step Verification turned on. (Google said: IMAP login: no)",
            // A TLS trust refusal is a configuration failure, not a network one:
            // the same certificate reproduces it on every attempt (EX-010).
            "IMAP TLS: The certificate chain was issued by an authority that is not trusted. (os error -2146893016)",
            "tls error: the certificate has expired",
        ] {
            assert!(!super::is_retryable_connect_error(message), "{message}");
        }
    }

    #[test]
    fn address_tls_and_login_failures_before_any_command_are_retryable() {
        for message in [
            "IMAP connect: dns error: failed to lookup address information: Nodo sconosciuto. (os error 11001)",
            "IMAP connect: Connection refused (os error 10061)",
            "IMAP connect: Impossibile stabilire la connessione. (os error 10060)",
            "IMAP connect: failed to lookup address",
            "IMAP connect: Network is unreachable (os error 10051)",
            "IMAP server has no address",
            // The TLS phase runs before any command: its I/O failures prove the
            // server executed nothing. Certificate refusals are the exception:
            // the same trust configuration reproduces them, so they are pinned
            // as definitive below (EX-010).
            "IMAP TLS: handshake failure",
            "IMAP TLS: connection reset by peer (os error 10054)",
            "Couldn't reach Google: Dns Failed: failed to lookup address information (os error 11001)",
            "Connection error: dns error: no address found for smtp.gmail.com",
            "SMTP: Connection error: tcp connect error: connection refused (os error 10061)",
            "transient error (421): 4.7.0 Try again later",
        ] {
            assert!(super::is_retryable_connect_error(message), "{message}");
        }
    }

    #[test]
    fn a_refused_or_lost_result_after_a_command_started_is_never_retryable() {
        for message in [
            // The session broke while a mutation was in flight (imap::Error).
            "Connection Lost",
            "No Response: [TRYCREATE] UID MOVE failed",
            "Bad Response: b'NO [CANNOT] Message not found'",
            "connection reset by peer (os error 10054)",
            "An existing connection was forcibly closed by the remote host. (os error 10054)",
            "Una connessione esistente è stata interrotta forzatamente dall'host remoto. (os error 10054)",
            "timed out",
            "broken pipe (os error 32)",
            // Socket-level failures of an OPEN session (ENOTCONN/WSAENOTCONN,
            // WSAESHUTDOWN, WSAENOTSOCK, English and Italian wording) and an EOF
            // inside a response: a command in flight may already have run, so
            // the outcome is lost, never "nothing happened" (INV-008, RA-007).
            "Socket is not connected (os error 57)",
            "Transport endpoint is not connected (os error 107)",
            "A request to send or receive data was disallowed because the socket is not connected and (when sending on a datagram socket using a sendto call) no address was supplied. (os error 10057)",
            "Una richiesta di invio o ricezione dati è stata negata perché il socket non è connesso e (per l'invio su un socket datagramma usando una chiamata sendto) nessun indirizzo è stato fornito. (os error 10057)",
            "A request to send or receive data was disallowed because the socket had already been shut down in a previous call with a previous call to shutdown. (os error 10058)",
            "Una richiesta di invio o ricezione dati è stata negata perché il socket è già stato chiuso in una precedente chiamata a shutdown. (os error 10058)",
            "An operation was attempted on something that is not a socket (os error 10038)",
            "EOF while reading the IMAP response",
            // lettre reports these while an SMTP dialogue, possibly DATA, was open.
            "network error: connection reset by peer (os error 10054)",
            "response error: incomplete response",
            "internal client error: connection pool closed",
            "transport has been shut down",
            "permanent error (550): 5.1.1 Mailbox not found",
            // Definitive provider or local answers.
            "Original message not found on the server: Q4 proposal. Sync mail and retry.",
            "More than one server message matches these headers. Sync the account to verify its exact location before retrying.",
            "Sync this message before applying a server action",
            "Provider returned ambiguous message identities",
            "Gmail did not verify the message identity before moving",
            "The provider did not expose its spam folder over IMAP",
            "Delivery result is uncertain; do not resend automatically. Connection error: dns error",
            "Message not found",
            "   ",
        ] {
            assert!(!super::is_retryable_connect_error(message), "{message}");
        }
    }

    #[test]
    fn the_prefixes_written_here_are_the_ones_the_classifier_reads() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/infrastructure/mail.rs"
        ))
        .unwrap();
        // Each prefix probed with a failure text this file really produces
        // behind it: the raw I/O error for the IMAP phases, and lettre's own
        // display for SMTP ("SMTP: os error …" never leaves this file; a bare
        // socket error without connect evidence stays definitive by design).
        for (prefix, probe) in [
            ("IMAP connect: ", "IMAP connect: os error 10060"),
            ("IMAP TLS: ", "IMAP TLS: os error 10060"),
            ("IMAP login: ", "IMAP login: os error 10060"),
            (
                "SMTP: ",
                "SMTP: Connection error: tcp connect error: connection refused (os error 10061)",
            ),
        ] {
            assert!(
                source.contains(&format!("\"{prefix}")),
                "This file must keep writing the prefix the retry queue reads: {prefix}"
            );
            assert!(super::is_retryable_connect_error(probe), "{probe}");
        }
    }

    #[test]
    fn a_successful_connection_wakes_the_queue_of_that_account() {
        use crate::application::connectivity;
        let _guard = connectivity::test_guard();
        connectivity::note_success(11);
        assert_eq!(
            connectivity::drain_wakes(),
            std::collections::HashSet::from([11])
        );
        assert!(connectivity::drain_wakes().is_empty());
    }
}
