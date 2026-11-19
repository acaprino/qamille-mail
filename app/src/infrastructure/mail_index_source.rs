//! Full-mailbox indexing uses read-only IMAP requests and keeps MIME in memory.
use crate::{
    application::{AppError, AppResult, compute, indexing_ports::*},
    domain::{
        indexing::FolderProgress,
        model::{Account, Attachment, Email},
    },
};
use mail_parser::MimeHeaders;
use super::mail::extension::GmailExtensions;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

/// Per-attachment memory budget of indexing (D2: the compute server takes 25 MiB).
const MAX_MEDIA_BYTES: usize = 25 * 1024 * 1024;

pub struct GmailIndexSource;
const UID_WINDOW: u32 = 2048;
const MAX_MIME: u32 = 32 * 1024 * 1024;

fn stopped(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(AppError::new("index_paused", "Indexing paused"))
    } else {
        Ok(())
    }
}
#[cfg(test)]
fn global_ids(response: &[u8]) -> Vec<u64> {
    let text = String::from_utf8_lossy(response);
    text.lines()
        .filter(|line| line.starts_with("* ") && line.to_ascii_uppercase().contains(" FETCH "))
        .filter_map(|line| {
            let mut words = line
                .split(|c: char| c.is_ascii_whitespace() || c == '(' || c == ')')
                .filter(|part| !part.is_empty());
            while let Some(word) = words.next() {
                if word.eq_ignore_ascii_case("X-GM-MSGID") {
                    return words.next()?.parse().ok();
                }
            }
            None
        })
        .collect()
}
impl IndexSource for GmailIndexSource {
    fn inventory_total(
        &self,
        account: &Account,
        folders: &[FolderProgress],
        cancel: &AtomicBool,
    ) -> AppResult<Option<u64>> {
        let mut session = super::mail::connect(account).map_err(AppError::external)?;
        let result = (|| {
            if !session
                .capabilities()
                .map_err(|e| AppError::external(e.to_string()))?
                .has_str("X-GM-EXT-1")
            {
                return Ok(None);
            }
            let mut unique = std::collections::HashSet::new();
            for folder in folders {
                stopped(cancel)?;
                let mailbox = session
                    .examine(&folder.name)
                    .map_err(|e| AppError::external(e.to_string()))?;
                if mailbox.exists > 2_000_000 {
                    return Ok(None);
                }
                let mut ids: Vec<_> = session
                    .uid_search("ALL")
                    .map_err(|e| AppError::external(e.to_string()))?
                    .into_iter()
                    .collect();
                ids.sort_unstable();
                for page in ids.chunks(500) {
                    stopped(cancel)?;
                    let sequence = page
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    let found = session.extensions.ids(&sequence).map_err(AppError::external)?;
                    if page.iter().any(|uid| !found.contains_key(uid)) {
                        return Err(AppError::external(
                            "Mailbox changed during census; the total will be recalculated",
                        ));
                    }
                    unique.extend(page.iter().filter_map(|uid| found.get(uid)).copied());
                    if unique.len() > 2_000_000 {
                        return Ok(None);
                    }
                }
            }
            Ok(Some(unique.len() as u64))
        })();
        let _ = session.logout();
        result
    }
    fn folders(&self, account: &Account) -> AppResult<Vec<FolderProgress>> {
        let mut session = super::mail::connect(account).map_err(AppError::external)?;
        let result = (|| {
            let names = session
                .list(Some(""), Some("*"))
                .map_err(|e| AppError::external(e.to_string()))?;
            let mut folders = Vec::new();
            for name in names.iter() {
                if name
                    .attributes()
                    .iter()
                    .any(|attribute| matches!(attribute, imap::types::NameAttribute::NoSelect))
                {
                    continue;
                }
                let role = name
                    .attributes()
                    .iter()
                    .find_map(|attribute| match attribute {
                        imap::types::NameAttribute::Custom(value) => match value.as_ref() {
                            "\\All" => Some("Archive"),
                            "\\Sent" => Some("Sent"),
                            "\\Drafts" => Some("Drafts"),
                            "\\Trash" => Some("Trash"),
                            "\\Junk" => Some("Spam"),
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap_or(if name.name().eq_ignore_ascii_case("INBOX") {
                        "Inbox"
                    } else {
                        name.name()
                    });
                folders.push(FolderProgress {
                    name: name.name().into(),
                    role: role.into(),
                    ..Default::default()
                });
            }
            folders.sort_by_key(|folder| {
                (
                    !folder.name.eq_ignore_ascii_case("INBOX"),
                    folder.name.clone(),
                )
            });
            if folders.is_empty() {
                return Err(AppError::external(
                    "No accessible IMAP folders; full indexing is unavailable",
                ));
            }
            Ok(folders)
        })();
        let _ = session.logout();
        result
    }
    fn open(&self, account: &Account) -> AppResult<Option<Box<dyn IndexSession>>> {
        let session = super::mail::connect(account).map_err(AppError::external)?;
        Ok(Some(Box::new(GmailIndexSession::new(session, account.clone())?)))
    }
}

/// How long one connection is reused, and for how many messages (W1.2).
const SESSION_SECONDS: u64 = 600;
const SESSION_MESSAGES: usize = 500;
/// Windows searched by one `next_batch` before it returns a short batch.
const WINDOWS_PER_BATCH: usize = 16;

/// One IMAP connection for many batches of one account.
pub struct GmailIndexSession<T: std::io::Read + std::io::Write> {
    session: Option<super::mail::extension::Session<T>>,
    account: Account,
    gmail: bool,
    /// Folder examined now, with its generation, message count and next UID.
    examined: Option<(String, Option<u32>, u32, Option<u32>)>,
    opened: std::time::Instant,
    read: usize,
    broken: bool,
}

impl<T: std::io::Read + std::io::Write> GmailIndexSession<T> {
    pub(super) fn new(mut session: super::mail::extension::Session<T>, account: Account) -> AppResult<Self> {
        let gmail = match session.capabilities() {
            Ok(capabilities) => capabilities.has_str("X-GM-EXT-1"),
            Err(error) => {
                let _ = session.logout();
                return Err(AppError::external(error.to_string()));
            }
        };
        Ok(Self { session: Some(session), account, gmail, examined: None, opened: std::time::Instant::now(), read: 0, broken: false })
    }
    fn imap(&mut self) -> AppResult<&mut super::mail::extension::Session<T>> {
        self.session.as_mut().ok_or_else(|| AppError::external("IMAP session closed"))
    }
    /// Examines `folder` unless it is the one examined already.
    fn examine(&mut self, folder: &str) -> AppResult<(Option<u32>, u32, Option<u32>)> {
        if let Some((name, validity, exists, next)) = &self.examined {
            if name == folder {
                return Ok((*validity, *exists, *next));
            }
        }
        let mailbox = self.imap()?.examine(folder).map_err(|e| AppError::external(e.to_string()))?;
        self.examined = Some((folder.to_string(), mailbox.uid_validity, mailbox.exists, mailbox.uid_next));
        Ok((mailbox.uid_validity, mailbox.exists, mailbox.uid_next))
    }
    fn batch(&mut self, folder: &FolderProgress, max: usize, cancel: &AtomicBool) -> AppResult<IndexBatch> {
        stopped(cancel)?;
        // A batch always reads a fresh count: new mail must be seen.
        self.examined = None;
        let (validity, exists, next) = self.examine(&folder.name)?;
        let generation = validity.ok_or_else(|| {
            AppError::external("Server omitted UIDVALIDITY; indexing cannot safely resume")
        })?;
        let mut progress = folder.clone();
        if progress.uid_validity != Some(generation) {
            progress.restart();
        }
        progress.uid_validity = Some(generation);
        progress.total = exists;
        let upper = next
            .ok_or_else(|| AppError::external("Server omitted UIDNEXT; indexing cannot safely paginate"))?
            .saturating_sub(1);
        if exists == 0 {
            progress.finish_empty(upper);
            return Ok(IndexBatch { progress, emails: vec![] });
        }
        // New mail first, then older mail from the newest downwards.
        let mut uids = Vec::new();
        for _ in 0..WINDOWS_PER_BATCH {
            let Some(window) = progress.plan(upper, UID_WINDOW) else { break };
            stopped(cancel)?;
            let listed = self
                .imap()?
                .uid_search(format!("UID {}:{}", window.start, window.end))
                .map_err(|e| AppError::external(e.to_string()))?;
            uids.extend(progress.advance_many(window, listed, upper, max - uids.len()));
            if uids.len() >= max || progress.complete {
                break;
            }
        }
        if uids.is_empty() {
            return Ok(IndexBatch { progress, emails: vec![] });
        }
        stopped(cancel)?;
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let gmail = if self.gmail { self.imap()?.extensions.metadata(&set).map_err(AppError::external)? } else { Default::default() };
        let account_id = self.account.id;
        let fetched = self
            .imap()?
            .uid_fetch(&set, "(UID FLAGS RFC822.SIZE INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER])")
            .map_err(|e| AppError::external(e.to_string()))?;
        let mut emails = Vec::new();
        // Walk order. A message whose headers cannot be read, a draft, or one
        // expunged since the search is passed over: the cursor already moved.
        for uid in &uids {
            let Some(row) = fetched.iter().find(|row| row.uid == Some(*uid)) else { continue };
            let metadata = gmail.get(uid);
            if self.gmail && metadata.and_then(|m| m.global_id).is_none() {
                // The batch is not saved: the same position is read again.
                return Err(AppError::external("Message changed during indexing; retrying this position"));
            }
            if let Some(email) = super::mail::metadata_row(row, account_id, &folder.name, Some(generation), &folder.role, metadata) {
                emails.push(email);
            }
        }
        self.read += emails.len();
        Ok(IndexBatch { progress, emails })
    }
    /// Contents of the messages of one folder, in groups under the download budget.
    fn folder_contents(&mut self, folder: &str, generation: Option<u32>, emails: &[&Email], cancel: &AtomicBool) -> AppResult<Vec<AppResult<IndexContent>>> {
        let (validity, _, _) = self.examine(folder)?;
        if generation.is_none() || validity != generation {
            return Ok(emails.iter().map(|_| Err(AppError::external("Mailbox identity changed; sync before indexing this message"))).collect());
        }
        let uids: Vec<u32> = emails.iter().filter_map(|email| email.uid).collect();
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let sizes: std::collections::HashMap<u32, u32> = if set.is_empty() { Default::default() } else {
            self.imap()?
                .uid_fetch(&set, "(UID RFC822.SIZE)")
                .map_err(|e| AppError::external(e.to_string()))?
                .iter()
                .filter_map(|row| Some((row.uid?, row.size?)))
                .collect()
        };
        // Each group is parsed and its MIME dropped before the next one arrives:
        // at most 32 MiB of bodies are in memory at a time.
        let mut parsed: std::collections::HashMap<u32, AppResult<IndexContent>> = std::collections::HashMap::new();
        let mut group: Vec<u32> = Vec::new();
        let mut group_bytes = 0u64;
        let wanted: Vec<u32> = uids.iter().copied().filter(|uid| sizes.get(uid).is_some_and(|size| *size <= MAX_MIME)).collect();
        for (index, uid) in wanted.iter().enumerate() {
            let size = u64::from(sizes[uid]);
            if !group.is_empty() && group_bytes + size > u64::from(MAX_MIME) {
                self.parse_group(&group, emails, &mut parsed, cancel)?;
                group.clear();
                group_bytes = 0;
            }
            group.push(*uid);
            group_bytes += size;
            if index + 1 == wanted.len() {
                self.parse_group(&group, emails, &mut parsed, cancel)?;
            }
        }
        Ok(emails
            .iter()
            .map(|email| {
                let uid = email.uid.ok_or_else(|| AppError::external("Message has no verified remote UID"))?;
                let size = sizes.get(&uid).ok_or_else(|| AppError::external("Message size unavailable"))?;
                if *size > MAX_MIME {
                    return Err(AppError::new("index_content_limit", "Message exceeds the 32 MiB temporary download budget; contents not indexed"));
                }
                parsed.remove(&uid).unwrap_or_else(|| Err(AppError::external("Message body unavailable")))
            })
            .collect())
    }
    /// Downloads one group of bodies and parses it right away.
    fn parse_group(
        &mut self,
        uids: &[u32],
        emails: &[&Email],
        parsed: &mut std::collections::HashMap<u32, AppResult<IndexContent>>,
        cancel: &AtomicBool,
    ) -> AppResult<()> {
        stopped(cancel)?;
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let fetched = self.imap()?.uid_fetch(&set, "(UID BODY.PEEK[])").map_err(|e| AppError::external(e.to_string()))?;
        for row in fetched.iter() {
            let (Some(uid), Some(raw)) = (row.uid, row.body()) else { continue };
            let Some(email) = emails.iter().find(|email| email.uid == Some(uid)) else { continue };
            let result = if raw.len() > MAX_MIME as usize {
                Err(AppError::new("index_content_limit", "Message exceeded its declared download size"))
            } else {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse_content(raw, email)))
                    .unwrap_or_else(|_| Err(AppError::new("index_panic", "Indexing stopped on this message: unreadable MIME")))
            };
            parsed.insert(uid, result);
        }
        Ok(())
    }
}

impl<T: std::io::Read + std::io::Write + Send> IndexSession for GmailIndexSession<T> {
    fn next_batch(&mut self, folder: &FolderProgress, max: usize, cancel: &AtomicBool) -> AppResult<IndexBatch> {
        let result = self.batch(folder, max.max(1), cancel);
        if result.as_ref().is_err_and(|e| e.code != "index_paused") {
            self.broken = true;
        }
        result
    }
    fn contents(&mut self, emails: &[Email], cancel: &AtomicBool) -> Vec<AppResult<IndexContent>> {
        let mut results: Vec<Option<AppResult<IndexContent>>> = emails.iter().map(|_| None).collect();
        let mut folders: Vec<(String, Option<u32>)> = Vec::new();
        for email in emails {
            let key = (email.remote_folder.clone().unwrap_or_else(|| "INBOX".into()), email.remote_uid_validity);
            if !folders.contains(&key) {
                folders.push(key);
            }
        }
        for (folder, generation) in folders {
            let members: Vec<usize> = emails
                .iter()
                .enumerate()
                .filter(|(_, e)| e.remote_folder.as_deref().unwrap_or("INBOX") == folder && e.remote_uid_validity == generation)
                .map(|(index, _)| index)
                .collect();
            let selected: Vec<&Email> = members.iter().map(|index| &emails[*index]).collect();
            match self.folder_contents(&folder, generation, &selected, cancel) {
                Ok(found) => {
                    for (index, result) in members.iter().zip(found) {
                        results[*index] = Some(result);
                    }
                }
                Err(error) => {
                    if error.code != "index_paused" {
                        self.broken = true;
                    }
                    for index in members {
                        results[index] = Some(Err(AppError::new(error.code, error.message.clone())));
                    }
                }
            }
        }
        results.into_iter().map(|result| result.unwrap_or_else(|| Err(AppError::external("Message body unavailable")))).collect()
    }
    fn expired(&self) -> bool {
        self.broken || self.read >= SESSION_MESSAGES || self.opened.elapsed().as_secs() >= SESSION_SECONDS
    }
}

impl<T: std::io::Read + std::io::Write> Drop for GmailIndexSession<T> {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = session.logout();
        }
    }
}

pub fn parse_content(raw: &[u8], original: &Email) -> AppResult<IndexContent> {
    let message = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| AppError::new("index_content_unreadable", "Unreadable MIME message"))?;
    let mut email = super::mail::parse_message_inner(raw, original.id, original.account_id, None)
        .ok_or_else(|| AppError::new("index_content_unreadable", "Unreadable message"))?;
    email.folder = original.folder.clone();
    email.uid = original.uid;
    email.remote_folder = original.remote_folder.clone();
    email.remote_uid_validity = original.remote_uid_validity;
    email.remote_content_hash = format!("{:x}", Sha256::digest(raw));
    email.remote_global_id = original.remote_global_id;
    // Historical indexing may encounter the provider Sent copy before normal
    // sync. Full decoded payload proof lets its content-phase dedup retain the
    // original local Sent ID; matching headers alone never set this proof.
    let outgoing_parts = message.attachments().map(|part| (
        part.attachment_name().unwrap_or("attachment").to_string(),
        format!("{:x}", Sha256::digest(part.contents())),
    )).collect::<Vec<_>>();
    email.outgoing_fingerprint = crate::domain::mail_identity::outgoing_fingerprint(&email, &outgoing_parts);
    email.remote_only = false;
    email.attachments.clear();
    let mut media = Vec::new();
    let mut warnings = Vec::new();
    let mut extracted_bytes = 0;
    for (index, part) in message.parts.iter().enumerate() {
        if index >= 1000 {
            warnings
                .push("Message has more than 1000 MIME parts; remaining parts were skipped".into());
            break;
        }
        let mime = part
            .content_type()
            .map(|kind| {
                format!(
                    "{}/{}",
                    kind.ctype(),
                    kind.subtype().unwrap_or("octet-stream")
                )
            })
            .unwrap_or_default();
        let image = mime.starts_with("image/");
        let attached = part.attachment_name().is_some()
            || part
                .content_disposition()
                .is_some_and(|value| value.is_attachment());
        if !image && !attached {
            continue;
        }
        let name = part
            .attachment_name()
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!("inline-{index}.{}", mime.split('/').nth(1).unwrap_or("bin"))
            });
        let bytes = part.contents();
        let attachment_id = email.id.saturating_mul(1000).saturating_add(index as u64);
        let mut attachment = Attachment {
            id: attachment_id,
            name: name.clone(),
            mime: mime.clone(),
            size: bytes.len() as u64,
            path: None,
            inline_text: None,
        };
        if bytes.len() > MAX_MEDIA_BYTES {
            warnings.push(format!(
                "{name}: exceeds the 25 MiB per-attachment memory budget"
            ));
        } else if image
            && !matches!(
                mime.as_str(),
                "image/png"
                    | "image/jpeg"
                    | "image/jpg"
                    | "image/gif"
                    | "image/webp"
                    | "image/bmp"
                    | "image/x-ms-bmp"
                    | "image/tiff"
                    | "image/x-tiff"
            )
        {
            warnings.push(format!("{name}: unsupported image format {mime}"));
        } else if image {
            media.push(IndexMedia {
                attachment_id,
                mime,
                bytes: bytes.to_vec(),
            });
        } else if extracted_bytes >= 8 * 1024 * 1024 {
            warnings.push(format!(
                "{name}: combined extracted text exceeds the 8 MiB message budget"
            ));
        } else {
            // Documents are read by the compute server's parser process; a pause there says
            // nothing about this message and must not cost it an attempt (INV-3).
            let extracted = super::attach::document_text(&name, bytes).and_then(|text| {
                if text.len() > 4 * 1024 * 1024 {
                    Err(AppError::new(compute::codes::DOCUMENT_UNREADABLE, "Extracted text exceeds the 4 MiB working budget"))
                } else {
                    Ok(text)
                }
            });
            match extracted {
                Ok(text) if extracted_bytes + text.len() > 8 * 1024 * 1024 => warnings.push(
                    format!("{name}: combined extracted text exceeds the 8 MiB message budget"),
                ),
                Ok(text) if !text.trim().is_empty() => {
                    extracted_bytes += text.len();
                    attachment.inline_text = Some(text);
                }
                Ok(_) => warnings.push(format!(
                    "{name}: no extractable text; content coverage is partial"
                )),
                Err(error) if compute::is_transient(error.code) => {
                    return Err(AppError::new("index_transient", error.message));
                }
                Err(error) => warnings.push(format!("{name}: {}", error.message)),
            }
        }
        email.attachments.push(attachment);
    }
    Ok(IndexContent {
        email,
        media,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Wire(std::io::Cursor<Vec<u8>>, std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Read for Wire {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(out)
        }
    }
    impl std::io::Write for Wire {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.1.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn session(script: &str) -> (GmailIndexSession<Wire>, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let input = format!("* OK hello\r\na1 OK login\r\n* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\na2 OK caps\r\n{script}");
        let imap = super::super::mail::extension::Session::new(Wire(std::io::Cursor::new(input.into_bytes()), written.clone()), &Account::default()).unwrap();
        (GmailIndexSession::new(imap, Account { id: 4, ..Default::default() }).unwrap(), written)
    }
    fn header(uid: u32) -> String {
        format!("From: a@example.com\r\nSubject: Old {uid}\r\nDate: Mon, 1 Jan 2018 10:00:00 +0000\r\nMessage-ID: <old{uid}@example.com>\r\n\r\n")
    }
    #[test]
    fn a_batch_is_one_search_and_one_fetch_with_gmail_labels_in_bulk() {
        let fetch = |seq: u32, uid: u32| { let h = header(uid); format!("* {seq} FETCH (UID {uid} FLAGS () RFC822.SIZE 900 BODY[HEADER] {{{}}}\r\n{h})\r\n", h.len()) };
        let script = [
            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT 31] next\r\na3 OK examined\r\n".to_string(),
            "* SEARCH 10 20 30\r\na4 OK searched\r\n".into(),
            "* 1 FETCH (UID 30 X-GM-MSGID 300 X-GM-THRID 1 X-GM-LABELS (\\Inbox \"Clienti\"))\r\n* 2 FETCH (UID 20 X-GM-MSGID 200 X-GM-THRID 2 X-GM-LABELS (\\Draft))\r\n* 3 FETCH (UID 10 X-GM-MSGID 100 X-GM-THRID 3 X-GM-LABELS ())\r\ngmail1 OK done\r\n".into(),
            format!("{}{}{}a5 OK fetched\r\n", fetch(1, 10), fetch(2, 20), fetch(3, 30)),
        ].concat();
        let (mut session, written) = session(&script);
        let folder = FolderProgress { name: "[Gmail]/All".into(), role: "Archive".into(), ..Default::default() };
        let batch = session.next_batch(&folder, 25, &AtomicBool::new(false)).unwrap();
        let commands = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert_eq!(commands.matches("UID SEARCH").count(), 1, "{commands}");
        assert!(commands.contains("UID FETCH 30,20,10 (UID FLAGS"), "{commands}");
        assert!(!commands.contains("BODY.PEEK[]"), "metadata only: {commands}");
        assert!(batch.progress.complete);
        assert_eq!(batch.emails.iter().map(|e| e.uid.unwrap()).collect::<Vec<_>>(), [30, 10], "newest first, the draft passed over");
        assert_eq!((batch.emails[0].folder.as_str(), batch.emails[0].labels.clone()), ("Inbox", vec!["Clienti".to_string()]));
        assert_eq!(batch.emails[1].folder, "Archive");
        assert_eq!(batch.emails[0].remote_global_id, Some(300));
        assert!(!session.expired());
    }
    #[test]
    fn contents_come_in_groups_and_an_oversized_message_is_set_aside() {
        let raw = |uid: u32| format!("{}Body of {uid}\r\n", header(uid));
        let body = |seq: u32, uid: u32| { let r = raw(uid); format!("* {seq} FETCH (UID {uid} BODY[] {{{}}}\r\n{r})\r\n", r.len()) };
        let script = [
            "* 3 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT 31] next\r\na3 OK examined\r\n".to_string(),
            format!("* 1 FETCH (UID 10 RFC822.SIZE 100)\r\n* 2 FETCH (UID 20 RFC822.SIZE {})\r\n* 3 FETCH (UID 30 RFC822.SIZE 200)\r\na4 OK sizes\r\n", 40 * 1024 * 1024),
            format!("{}{}a5 OK bodies\r\n", body(1, 10), body(3, 30)),
        ].concat();
        let (mut session, written) = session(&script);
        let email = |id: u64, uid: u32| Email { id, account_id: 4, uid: Some(uid), remote_folder: Some("[Gmail]/All".into()), remote_uid_validity: Some(7), ..Default::default() };
        let results = session.contents(&[email(1, 10), email(2, 20), email(3, 30)], &AtomicBool::new(false));
        let commands = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("UID FETCH 10,30 (UID BODY.PEEK[])"), "one request for the group: {commands}");
        assert!(results[0].as_ref().unwrap().email.body.contains("Body of 10"));
        assert_eq!(results[1].as_ref().err().unwrap().code, "index_content_limit");
        assert!(results[2].as_ref().unwrap().email.body.contains("Body of 30"));
        assert_eq!(results[2].as_ref().unwrap().email.id, 3);
    }
    #[test]
    fn census_reads_only_numeric_global_ids_from_fetch_responses() {
        assert_eq!(global_ids(b"* 1 FETCH (X-GM-MSGID 1278455344230334865)\r\n* 2 FETCH (UID 55 X-GM-MSGID 99)\r\nA1 OK X-GM-MSGID 444\r\n"),[1278455344230334865,99]);
        assert!(global_ids(b"* OK X-GM-MSGID 55\r\n* 1 FETCH (X-GM-MSGID invalid)\r\n").is_empty());
    }
    #[test]
    fn inline_image_bytes_reach_the_visual_encoder_without_writing_files() {
        let raw = b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Image message\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/plain\r\n\r\nComplete body\r\n--parts\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\nContent-ID: <picture>\r\nContent-Transfer-Encoding: base64\r\n\r\nAQIDBA==\r\n--parts--\r\n";
        let original = Email {
            id: 7,
            account_id: 2,
            uid: Some(42),
            remote_uid_validity: Some(9),
            remote_folder: Some("Archive".into()),
            ..Default::default()
        };
        let content = parse_content(raw, &original).unwrap();
        assert!(content.email.body.contains("Complete body"));
        assert_eq!(content.email.to, ["recipient@example.com"]);
        assert_eq!(content.email.uid, Some(42));
        assert_eq!(content.email.remote_uid_validity, Some(9));
        assert_eq!(content.media.len(), 1);
        assert_eq!(content.media[0].bytes, [1, 2, 3, 4]);
        assert_eq!(content.media[0].mime, "image/png");
        assert_eq!(
            content.email.attachments[0].id,
            content.media[0].attachment_id
        );
        assert!(content.email.attachments[0].path.is_none());
        assert!(content.email.attachments[0].inline_text.is_none());
        assert!(content.warnings.is_empty());
        assert_eq!(
            content.email.remote_content_hash,
            format!("{:x}", Sha256::digest(raw))
        );
    }
}
