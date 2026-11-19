//! A short-lived, read-only Gmail connection for message text, without attachments.
use super::mail;
use crate::model::{Account, Email};
use imap_proto::{BodyStructure, MessageSection, SectionPath};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

/// How long a pooled session may sit idle before it is dropped.
const IDLE_LIMIT: Duration = Duration::from_secs(90);
/// The most sessions kept per account. INV-003 allows 4 connections per account; the other two
/// are left for the worker and the sync.
const POOLED_PER_ACCOUNT: usize = 2;
/// The most reads in flight per account, so reader sockets (in use plus idle) stay at
/// `POOLED_PER_ACCOUNT` and INV-003 keeps the rest of the 4 for the worker and the sync.
const MAX_IN_FLIGHT: usize = POOLED_PER_ACCOUNT;

/// Read-only sessions pooled per account: `.0` the idle sessions, `.1` the reads in flight per
/// account, `.2` signalled when a read finishes. The locks only guard those maps: they are never
/// held while connecting or fetching (INV-001), so a slow read on one account cannot hold up
/// another. Generic over the session only so tests can swap the IMAP session for a fake.
pub struct MailReader<S = mail::Session>(
    Mutex<HashMap<u32, VecDeque<Connection<S>>>>,
    Mutex<HashMap<u32, usize>>,
    Condvar,
);
impl<S> Default for MailReader<S> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()), Mutex::new(HashMap::new()), Condvar::new())
    }
}
/// One of an account's `MAX_IN_FLIGHT` read slots, given back on drop (also on error or panic).
struct Permit<'a> {
    busy: &'a Mutex<HashMap<u32, usize>>,
    freed: &'a Condvar,
    account_id: u32,
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = busy.get_mut(&self.account_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                busy.remove(&self.account_id);
            }
        }
        drop(busy);
        self.freed.notify_all();
    }
}
struct Connection<S> {
    key: String,
    session: S,
    used: Instant,
}

fn connection_key(account: &Account) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                account.id,
                &account.email,
                &account.imap_host,
                account.imap_port,
                &account.username,
                &account.oauth_client_id,
                &account.password,
            ))
            .map_err(|e| e.to_string())?
        )
    ))
}

impl MailReader {
    pub fn read(&self, account: &Account, email: &Email) -> Result<Email, String> {
        self.read_with(account, email, mail::connect, read_message)
    }
}

impl<S> MailReader<S> {
    fn read_with(
        &self,
        account: &Account,
        email: &Email,
        connect: impl Fn(&Account) -> Result<S, String>,
        fetch: impl Fn(&mut S, &Account, &Email) -> Result<Email, String>,
    ) -> Result<Email, String> {
        let key = connection_key(account)?;
        // Held until the session is back in the pool, so at most MAX_IN_FLIGHT sockets per account.
        let _permit = self.acquire(account.id)?;
        let pooled = self.take_connection(account.id, &key, IDLE_LIMIT)?;
        let reused = pooled.is_some();
        let mut session = match pooled {
            Some(session) => session,
            None => connect(account)?,
        };
        let mut result = fetch(&mut session, account, email);
        if result.is_err() && reused {
            // A server may have closed an idle socket. Never retry a write here.
            drop(session);
            session = connect(account)?;
            result = fetch(&mut session, account, email);
        }
        if result.is_ok() {
            self.return_connection(account.id, key, session);
        }
        result
    }

    /// Waits for one of the account's read slots. Only the small in-flight counter is locked,
    /// and the lock is released while waiting: no pool lock and no network call happen here.
    fn acquire(&self, account_id: u32) -> Result<Permit<'_>, String> {
        let mut busy = self.1.lock().map_err(|_| "Mail reader lock failed")?;
        while busy.get(&account_id).copied().unwrap_or(0) >= MAX_IN_FLIGHT {
            busy = self.2.wait(busy).map_err(|_| "Mail reader lock failed")?;
        }
        *busy.entry(account_id).or_insert(0) += 1;
        Ok(Permit {
            busy: &self.1,
            freed: &self.2,
            account_id,
        })
    }

    /// Takes this account's pooled session out of the pool, if it has one for these credentials.
    /// The lock is released before this returns; idle sessions are closed after releasing it too.
    /// Sessions of this account made with other credentials are useless and are dropped here, so
    /// they never add to the account's socket count.
    fn take_connection(
        &self,
        account_id: u32,
        key: &str,
        idle_limit: Duration,
    ) -> Result<Option<S>, String> {
        let (session, expired) = {
            let mut pools = self.0.lock().map_err(|_| "Mail reader lock failed")?;
            let mut expired = Vec::new();
            pools.retain(|id, pool| {
                let (fresh, stale): (VecDeque<_>, VecDeque<_>) = std::mem::take(pool)
                    .into_iter()
                    .partition(|item| {
                        item.used.elapsed() < idle_limit && (*id != account_id || item.key == key)
                    });
                expired.extend(stale);
                *pool = fresh;
                !pool.is_empty()
            });
            let session = pools.get_mut(&account_id).and_then(|pool| {
                let index = pool.iter().position(|item| item.key == key)?;
                pool.remove(index).map(|item| item.session)
            });
            if pools.get(&account_id).is_some_and(VecDeque::is_empty) {
                pools.remove(&account_id);
            }
            (session, expired)
        };
        drop(expired);
        Ok(session)
    }

    /// Hands a healthy session back to its account's pool, dropping the oldest beyond the cap.
    fn return_connection(&self, account_id: u32, key: String, session: S) {
        let evicted = {
            let Ok(mut pools) = self.0.lock() else {
                return;
            };
            let pool = pools.entry(account_id).or_default();
            let mut evicted = Vec::new();
            while pool.len() >= POOLED_PER_ACCOUNT {
                evicted.extend(pool.pop_front());
            }
            pool.push_back(Connection {
                key,
                session,
                used: Instant::now(),
            });
            evicted
        };
        drop(evicted);
    }
}

#[derive(Debug, PartialEq)]
struct TextPart {
    path: Vec<u32>,
    html: bool,
    octets: u32,
}
fn text_parts(structure: &BodyStructure<'_>, path: Vec<u32>) -> Vec<TextPart> {
    match structure {
        BodyStructure::Multipart { common, bodies, .. } => {
            if common
                .disposition
                .as_ref()
                .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"))
            {
                return vec![];
            }
            let mut parts = bodies.iter().enumerate().filter_map(|(index, body)| {
                let mut child = path.clone();
                child.push(index as u32 + 1);
                let parts = text_parts(body, child);
                (!parts.is_empty()).then_some(parts)
            });
            let Some(first) = parts.next() else {
                return vec![];
            };
            if common.ty.subtype.eq_ignore_ascii_case("alternative") {
                let mut alternatives: Vec<_> =
                    std::iter::once(first).chain(parts).flatten().collect();
                // Show the sender's HTML when present; plain text stays as the fallback.
                alternatives.sort_by_key(|part| !part.html);
                alternatives
            } else {
                first
            }
        }
        BodyStructure::Text { common, other, .. } => {
            let attachment = common
                .disposition
                .as_ref()
                .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"))
                || common
                    .ty
                    .params
                    .as_ref()
                    .is_some_and(|p| p.iter().any(|(key, _)| key.eq_ignore_ascii_case("name")));
            if attachment {
                return vec![];
            }
            let html = common.ty.subtype.eq_ignore_ascii_case("html");
            if !html && !common.ty.subtype.eq_ignore_ascii_case("plain") {
                return vec![];
            }
            vec![TextPart {
                path: if path.is_empty() { vec![1] } else { path },
                html,
                octets: other.octets,
            }]
        }
        // Attached RFC822 messages are not the enclosing message's body.
        _ => vec![],
    }
}

pub(super) fn read_text<T: Read + Write>(
    session: &mut imap::Session<T>,
    uid: u32,
    email: &Email,
) -> Result<Email, String> {
    let fetched = session
        .uid_fetch(uid.to_string(), "(UID BODYSTRUCTURE)")
        .map_err(|e| e.to_string())?;
    let row = fetched
        .iter()
        .find(|item| item.uid == Some(uid))
        .ok_or("Gmail did not return this message")?;
    let structure = row
        .bodystructure()
        .ok_or("Gmail did not return the message structure")?;
    let parts = text_parts(structure, vec![]);
    #[cfg(test)]
    if std::env::var_os("QAMILLE_PROBE_READER").is_some() {
        fn outline(body: &BodyStructure<'_>) -> String {
            match body {
                BodyStructure::Multipart { common, bodies, .. } => format!(
                    "{}/{}[{}]",
                    common.ty.ty,
                    common.ty.subtype,
                    bodies.iter().map(outline).collect::<Vec<_>>().join(",")
                ),
                BodyStructure::Text { common, .. }
                | BodyStructure::Basic { common, .. }
                | BodyStructure::Message { common, .. } => {
                    format!(
                        "{}/{} attached={} named={}",
                        common.ty.ty,
                        common.ty.subtype,
                        common
                            .disposition
                            .as_ref()
                            .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment")),
                        common.ty.params.as_ref().is_some_and(|p| p
                            .iter()
                            .any(|(key, _)| key.eq_ignore_ascii_case("name")))
                    )
                }
            }
        }
        eprintln!(
            "mail_probe structure={} parts={parts:?}",
            outline(structure)
        );
    }
    let mut loaded = email.clone();
    loaded.body.clear();
    loaded.attachments.clear();
    super::mail_cache_download::attachment_metadata(structure, &mut loaded.attachments);
    super::mail_cache_download::assign_attachment_ids(email, &mut loaded.attachments);
    loaded.remote_only = false;
    // HTML alternatives come first. Fetch every part that fits the shared budget in
    // one round trip: HTML for display, plain text for summaries, replies and search.
    const BUDGET: u32 = 2 * 1024 * 1024;
    let mut used = 0u32;
    let wanted: Vec<_> = parts
        .iter()
        .filter(|part| {
            let fits = used.saturating_add(part.octets) <= BUDGET;
            if fits {
                used += part.octets;
            }
            fits
        })
        .collect();
    if wanted.is_empty() && !parts.is_empty() {
        return Err("Message text exceeds the 2 MiB reader budget. Download the original message to read it.".into());
    }
    if wanted.is_empty() {
        return Ok(loaded);
    }
    let dotted = |part: &TextPart| {
        part.path
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".")
    };
    let sections = wanted
        .iter()
        .map(|part| {
            let path = dotted(part);
            format!("BODY.PEEK[{path}.MIME] BODY.PEEK[{path}]")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let fetched = session
        .uid_fetch(uid.to_string(), format!("(UID {sections})"))
        .map_err(|e| e.to_string())?;
    let row = fetched
        .iter()
        .find(|item| item.uid == Some(uid))
        .ok_or("Gmail did not return the message text")?;
    let mut html_text = String::new();
    for part in wanted {
        let mime = row
            .section(&SectionPath::Part(
                part.path.clone(),
                Some(MessageSection::Mime),
            ))
            .ok_or("Gmail did not return the text encoding")?;
        let body = row
            .section(&SectionPath::Part(part.path.clone(), None))
            .ok_or("Gmail did not return the message text")?;
        let mut raw = mime.to_vec();
        if !raw.ends_with(b"\r\n\r\n") {
            raw.extend_from_slice(b"\r\n");
        }
        raw.extend_from_slice(body);
        #[cfg(test)]
        if std::env::var_os("QAMILLE_PROBE_READER").is_some() {
            let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
            eprintln!(
                "mail_probe encoded_bytes={} plain={} html={} attachments={}",
                body.len(),
                parsed.body_text(0).is_some(),
                parsed.body_html(0).is_some(),
                parsed.attachments().count()
            );
        }
        let parsed = mail::parse_message_inner(&raw, email.id, email.account_id, None)
            .ok_or("Gmail message text could not be read")?;
        if part.html {
            if loaded.body_html.is_empty() {
                loaded.body_html = parsed.body_html;
            }
            if html_text.trim().is_empty() {
                html_text = parsed.body;
            }
        } else if loaded.body.trim().is_empty() {
            loaded.body = parsed.body;
        }
    }
    if loaded.body.trim().is_empty() {
        loaded.body = html_text;
    }
    Ok(loaded)
}

fn read_message(
    session: &mut mail::Session,
    account: &Account,
    email: &Email,
) -> Result<Email, String> {
    let (_, uid) = match mail::locate_known_message(session, email)? {
        Some(location) => location,
        None => {
            let folders = mail::export_folders(session, account, false)?;
            mail::locate_message(session, &folders, email)?
        }
    };
    read_text(session, uid, email)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::Arc};
    struct Wire(io::Cursor<Vec<u8>>, Arc<Mutex<Vec<u8>>>);
    impl Read for Wire {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Write for Wire {
        fn write(&mut self, out: &[u8]) -> io::Result<usize> {
            self.1.lock().unwrap().extend_from_slice(out);
            Ok(out.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn text_reader_decodes_only_the_body_part_and_keeps_attachment_descriptors() {
        let structure = r#"(("TEXT" "PLAIN" ("CHARSET" "UTF-8") NIL NIL "QUOTED-PRINTABLE" 22 1)("APPLICATION" "PDF" ("NAME" "invoice.pdf") NIL NIL "BASE64" 1700000 NIL ("ATTACHMENT" ("FILENAME" "invoice.pdf")) NIL NIL) "MIXED")"#;
        let mime = "Content-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n";
        let body = "Ciao, citt=C3=A0!";
        let script = format!("a1 OK login\r\n* 1 FETCH (UID 9 BODYSTRUCTURE {structure})\r\na2 OK metadata\r\n* 1 FETCH (UID 9 BODY[1.MIME] {{{}}}\r\n{mime} BODY[1] {{{}}}\r\n{body})\r\na3 OK text\r\na4 OK noop\r\n", mime.len(), body.len());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let client =
            imap::Client::new(Wire(io::Cursor::new(script.into_bytes()), commands.clone()));
        let mut session = client.login("test", "test").map_err(|e| e.0).unwrap();
        let email = Email {
            attachments: vec![
                crate::model::Attachment {
                    id: 32,
                    name: "attachment".into(),
                    ..Default::default()
                },
                crate::model::Attachment {
                    id: 33,
                    name: "invoice.pdf".into(),
                    mime: "application/pdf".into(),
                    size: 1700000,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let loaded = read_text(&mut session, 9, &email).unwrap();
        assert_eq!(loaded.body, "Ciao, città!");
        assert!(loaded.body_html.is_empty(), "Plain-only mail is not converted to HTML");
        assert_eq!(
            loaded.attachments.len(),
            1,
            "Discard the header-only placeholder"
        );
        assert_eq!(loaded.attachments[0].id, 33);
        assert_eq!(loaded.attachments[0].size, 1700000);
        assert!(loaded.attachments[0].inline_text.is_none());
        session.noop().unwrap();
        let commands = String::from_utf8(commands.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("BODY.PEEK[1.MIME] BODY.PEEK[1]"));
        assert!(!commands.contains("BODY.PEEK[]"));
        assert!(!commands.contains("BODY.PEEK[2]"));
    }
    fn alternative_script(plain: &str, html: &str) -> String {
        let structure = format!(
            r#"(("TEXT" "PLAIN" ("CHARSET" "UTF-8") NIL NIL "7BIT" {} 1)("TEXT" "HTML" ("CHARSET" "UTF-8") NIL NIL "7BIT" {} 1) "ALTERNATIVE")"#,
            plain.len(),
            html.len()
        );
        let plain_mime = "Content-Type: text/plain; charset=UTF-8\r\n\r\n";
        let html_mime = "Content-Type: text/html; charset=UTF-8\r\n\r\n";
        format!(
            "a1 OK login\r\n* 1 FETCH (UID 9 BODYSTRUCTURE {structure})\r\na2 OK metadata\r\n* 1 FETCH (UID 9 BODY[2.MIME] {{{}}}\r\n{html_mime} BODY[2] {{{}}}\r\n{html} BODY[1.MIME] {{{}}}\r\n{plain_mime} BODY[1] {{{}}}\r\n{plain})\r\na3 OK text\r\n",
            html_mime.len(),
            html.len(),
            plain_mime.len(),
            plain.len()
        )
    }
    #[test]
    fn html_alternative_is_kept_for_display_beside_the_plain_text() {
        let html = "<p>Merch <b>nuovo</b></p>";
        let commands = Arc::new(Mutex::new(Vec::new()));
        let client = imap::Client::new(Wire(
            io::Cursor::new(alternative_script("Merch nuovo", html).into_bytes()),
            commands.clone(),
        ));
        let mut session = client.login("test", "test").map_err(|e| e.0).unwrap();
        let loaded = read_text(&mut session, 9, &Email::default()).unwrap();
        assert_eq!(loaded.body_html, html);
        assert_eq!(loaded.body, "Merch nuovo", "Summaries and replies keep the sender's plain text");
        let commands = String::from_utf8(commands.lock().unwrap().clone()).unwrap();
        assert!(
            commands.contains("(UID BODY.PEEK[2.MIME] BODY.PEEK[2] BODY.PEEK[1.MIME] BODY.PEEK[1])"),
            "Both parts arrive in one round trip, HTML first: {commands}"
        );
    }
    #[test]
    fn empty_plain_text_falls_back_to_html_in_an_alternative_message() {
        let html = "<p>Il messaggio completo.</p>";
        let client = imap::Client::new(Wire(
            io::Cursor::new(alternative_script("", html).into_bytes()),
            Arc::new(Mutex::new(vec![])),
        ));
        let mut session = client.login("test", "test").map_err(|e| e.0).unwrap();
        let loaded = read_text(&mut session, 9, &Email::default()).unwrap();
        assert_eq!(loaded.body.trim(), "Il messaggio completo.");
        assert_eq!(loaded.body_html, html);
    }
    #[test]
    fn oversized_html_leaves_the_plain_text_reader_working() {
        let structure = r#"(("TEXT" "PLAIN" ("CHARSET" "UTF-8") NIL NIL "7BIT" 5 1)("TEXT" "HTML" ("CHARSET" "UTF-8") NIL NIL "7BIT" 3000000 1) "ALTERNATIVE")"#;
        let mime = "Content-Type: text/plain; charset=UTF-8\r\n\r\n";
        let script = format!("a1 OK login\r\n* 1 FETCH (UID 9 BODYSTRUCTURE {structure})\r\na2 OK metadata\r\n* 1 FETCH (UID 9 BODY[1.MIME] {{{}}}\r\n{mime} BODY[1] {{5}}\r\nCiao!)\r\na3 OK text\r\n", mime.len());
        let commands = Arc::new(Mutex::new(Vec::new()));
        let client = imap::Client::new(Wire(io::Cursor::new(script.into_bytes()), commands.clone()));
        let mut session = client.login("test", "test").map_err(|e| e.0).unwrap();
        let loaded = read_text(&mut session, 9, &Email::default()).unwrap();
        assert_eq!(loaded.body, "Ciao!");
        assert!(loaded.body_html.is_empty());
        let commands = String::from_utf8(commands.lock().unwrap().clone()).unwrap();
        assert!(!commands.contains("BODY.PEEK[2]"), "The oversized HTML part is not transferred");
    }
    // The reader pool is exercised with fake sessions: a "connection" is just its serial number.
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    fn account(id: u32) -> Account {
        Account { id, email: format!("account{id}@example.com"), ..Default::default() }
    }
    fn ok(_: &mut u32, _: &Account, _: &Email) -> Result<Email, String> {
        Ok(Email::default())
    }

    #[test]
    fn reads_on_two_accounts_do_not_serialize() {
        let reader = MailReader::<u32>::default();
        let started = Instant::now();
        std::thread::scope(|scope| {
            for id in [1, 2] {
                let reader = &reader;
                scope.spawn(move || {
                    reader
                        .read_with(
                            &account(id),
                            &Email::default(),
                            |_| Ok(id),
                            |_, _, _| {
                                std::thread::sleep(Duration::from_millis(300));
                                Ok(Email::default())
                            },
                        )
                        .unwrap();
                });
            }
        });
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "A slow read on one account must not hold up another account: {elapsed:?}"
        );
    }

    #[test]
    fn the_same_account_reuses_its_pooled_connection() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let connect = |_: &Account| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1);
        for _ in 0..3 {
            reader.read_with(&account(1), &Email::default(), &connect, ok).unwrap();
        }
        assert_eq!(connects.load(Ordering::SeqCst), 1, "Later reads reuse the first connection");
    }

    #[test]
    fn pools_are_separate_per_account() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let connect = |_: &Account| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1);
        let seen = Mutex::new(Vec::new());
        let fetch = |session: &mut u32, account: &Account, _: &Email| {
            seen.lock().unwrap().push((account.id, *session));
            Ok(Email::default())
        };
        for id in [1, 2, 1, 2] {
            reader.read_with(&account(id), &Email::default(), &connect, &fetch).unwrap();
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(1, 1), (2, 2), (1, 1), (2, 2)],
            "Each account only ever sees its own connection"
        );
        assert_eq!(connects.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_stale_reused_connection_reconnects_once_and_the_new_one_is_pooled() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let connect = |_: &Account| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1);
        let server_closed_idle_socket = AtomicBool::new(false);
        let fetch = |session: &mut u32, _: &Account, _: &Email| {
            if *session == 1 && server_closed_idle_socket.load(Ordering::SeqCst) {
                Err("connection reset".to_string())
            } else {
                Ok(Email::default())
            }
        };
        reader.read_with(&account(1), &Email::default(), &connect, &fetch).unwrap();
        server_closed_idle_socket.store(true, Ordering::SeqCst);
        reader.read_with(&account(1), &Email::default(), &connect, &fetch).unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2, "One reconnect, not a retry loop");
        reader.read_with(&account(1), &Email::default(), &connect, &fetch).unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2, "The replacement connection is pooled");
    }

    #[test]
    fn a_failing_fresh_connection_is_not_retried_and_not_pooled() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let connect = |_: &Account| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1);
        let fetches = AtomicU32::new(0);
        let failing = |_: &mut u32, _: &Account, _: &Email| {
            fetches.fetch_add(1, Ordering::SeqCst);
            Err::<Email, _>("message not found".to_string())
        };
        assert!(reader.read_with(&account(1), &Email::default(), &connect, &failing).is_err());
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        assert_eq!(fetches.load(Ordering::SeqCst), 1, "A fresh connection is never retried");
        reader.read_with(&account(1), &Email::default(), &connect, ok).unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2, "The failed connection was dropped, not pooled");
    }

    impl<S> MailReader<S> {
        fn pooled(&self, account_id: u32) -> usize {
            self.0.lock().unwrap().get(&account_id).map_or(0, VecDeque::len)
        }
    }

    #[test]
    fn connecting_on_two_accounts_does_not_serialize_either() {
        let reader = MailReader::<u32>::default();
        let started = Instant::now();
        std::thread::scope(|scope| {
            for id in [1, 2] {
                let reader = &reader;
                scope.spawn(move || {
                    reader
                        .read_with(
                            &account(id),
                            &Email::default(),
                            |_| {
                                std::thread::sleep(Duration::from_millis(300));
                                Ok(id)
                            },
                            ok,
                        )
                        .unwrap();
                });
            }
        });
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "A slow connect on one account must not hold up another account: {elapsed:?}"
        );
    }

    fn wait_for(condition: impl Fn() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(started.elapsed() < Duration::from_secs(5), "timed out waiting");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn reads_in_flight_on_one_account_never_exceed_two() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let now = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    reader
                        .read_with(
                            &account(1),
                            &Email::default(),
                            |_| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1),
                            |_, _, _| {
                                peak.fetch_max(now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                                std::thread::sleep(Duration::from_millis(100));
                                now.fetch_sub(1, Ordering::SeqCst);
                                Ok(Email::default())
                            },
                        )
                        .unwrap();
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= MAX_IN_FLIGHT, "In flight: {}", peak.load(Ordering::SeqCst));
        let connects = connects.load(Ordering::SeqCst) as usize;
        assert!(connects <= POOLED_PER_ACCOUNT, "Reader sockets for one account: {connects}");
        assert_eq!(reader.pooled(1), connects, "Every healthy session went back to the pool");
        assert_eq!(reader.pooled(2), 0);
        assert!(reader.1.lock().unwrap().is_empty(), "Every slot was given back");
    }

    #[test]
    fn a_third_read_on_one_account_waits_while_another_account_is_not_delayed() {
        let reader = MailReader::<u32>::default();
        let release = AtomicBool::new(false);
        let inside = AtomicUsize::new(0);
        let third_started = AtomicBool::new(false);
        let other_done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            for id in 1..=2 {
                let (reader, release, inside) = (&reader, &release, &inside);
                scope.spawn(move || {
                    reader
                        .read_with(&account(1), &Email::default(), |_| Ok(id), |_, _, _| {
                            inside.fetch_add(1, Ordering::SeqCst);
                            wait_for(|| release.load(Ordering::SeqCst));
                            Ok(Email::default())
                        })
                        .unwrap();
                });
            }
            wait_for(|| inside.load(Ordering::SeqCst) == 2);
            scope.spawn(|| {
                reader
                    .read_with(&account(1), &Email::default(), |_| Ok(3), |_, _, _| {
                        third_started.store(true, Ordering::SeqCst);
                        Ok(Email::default())
                    })
                    .unwrap();
            });
            scope.spawn(|| {
                reader.read_with(&account(2), &Email::default(), |_| Ok(9), ok).unwrap();
                other_done.store(true, Ordering::SeqCst);
            });
            wait_for(|| other_done.load(Ordering::SeqCst));
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                !third_started.load(Ordering::SeqCst),
                "A third read on an account with two in flight waits for a slot"
            );
            release.store(true, Ordering::SeqCst);
        });
        assert!(third_started.load(Ordering::SeqCst), "The waiting read ran once a slot was free");
    }

    #[test]
    fn a_failed_read_gives_its_slot_back() {
        let reader = MailReader::<u32>::default();
        for _ in 0..(MAX_IN_FLIGHT * 3) {
            let failing = |_: &mut u32, _: &Account, _: &Email| Err::<Email, _>("gone".to_string());
            assert!(reader.read_with(&account(1), &Email::default(), |_| Ok(1), failing).is_err());
            assert!(reader.read_with(&account(1), &Email::default(), |_| Err("offline".into()), ok).is_err());
        }
        reader.read_with(&account(1), &Email::default(), |_| Ok(1), ok).unwrap();
    }

    #[test]
    fn a_changed_credential_does_not_reuse_the_pooled_session() {
        let reader = MailReader::<u32>::default();
        let connects = AtomicU32::new(0);
        let connect = |_: &Account| Ok(connects.fetch_add(1, Ordering::SeqCst) + 1);
        reader.read_with(&account(1), &Email::default(), &connect, ok).unwrap();
        let changed = Account { password: "new password".into(), ..account(1) };
        reader.read_with(&changed, &Email::default(), &connect, ok).unwrap();
        assert_eq!(connects.load(Ordering::SeqCst), 2, "New credentials get their own connection");
        assert_eq!(reader.pooled(1), 1, "The old credentials' session is closed, not kept");
    }

    #[test]
    fn an_idle_connection_expires_after_the_idle_limit() {
        let reader = MailReader::<u32>::default();
        let key = connection_key(&account(1)).unwrap();
        let put = |session| {
            reader.0.lock().unwrap().entry(1).or_default().push_back(Connection {
                key: key.clone(),
                session,
                used: Instant::now(),
            });
        };
        put(7);
        assert_eq!(reader.take_connection(1, &key, IDLE_LIMIT).unwrap(), Some(7), "Fresh sessions are reused");
        put(8);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(reader.take_connection(1, &key, Duration::from_millis(1)).unwrap(), None);
        assert_eq!(reader.pooled(1), 0, "The expired session is closed, not kept");
        assert_eq!(IDLE_LIMIT, Duration::from_secs(90));
    }
}
