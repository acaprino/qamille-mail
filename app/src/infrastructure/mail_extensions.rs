//! Numeric Gmail FETCH extensions, with complete tagged-response consumption.
//!
//! imap 2.x cannot parse X-GM-MSGID. Keep its ordinary command parser, but
//! prevent its private buffer from reading past a line. Extension commands use
//! the same authenticated socket and a separate tag namespace.
use crate::model::Account;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

pub(crate) struct LineStream<T>(Arc<Mutex<BufReader<T>>>, Arc<AtomicBool>);
impl<T: Read> Read for LineStream<T> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.1.load(Ordering::Relaxed) {
            return Err(io::Error::other(
                "IMAP extension response was not completed",
            ));
        }
        let mut reader = self
            .0
            .lock()
            .map_err(|_| io::Error::other("IMAP stream lock failed"))?;
        let available = reader.fill_buf()?;
        let len = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1)
            .min(out.len());
        out[..len].copy_from_slice(&available[..len]);
        reader.consume(len);
        Ok(len)
    }
}
impl<T: Write> Write for LineStream<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.1.load(Ordering::Relaxed) {
            return Err(io::Error::other(
                "IMAP extension response was not completed",
            ));
        }
        self.0
            .lock()
            .map_err(|_| io::Error::other("IMAP stream lock failed"))?
            .get_mut()
            .write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("IMAP stream lock failed"))?
            .get_mut()
            .flush()
    }
}

pub(crate) trait GmailExtensions {
    fn ids(&self, sequence: &str) -> Result<HashMap<u32, u64>, String>;
}
pub(crate) struct Extensions<T> {
    stream: Arc<Mutex<BufReader<T>>>,
    counter: Arc<Mutex<u64>>,
    poisoned: Arc<AtomicBool>,
}
impl<T> Clone for Extensions<T> {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.clone(),
            counter: self.counter.clone(),
            poisoned: self.poisoned.clone(),
        }
    }
}
impl<T: Read + Write> Extensions<T> {
    /// Sends one command and consumes every response up to its tagged completion.
    fn run(&self, command: &str, mut untagged: impl FnMut(&[&str])) -> Result<(), String> {
        self.exchange(command, None, |line| {
            let Ok(line) = std::str::from_utf8(line) else { return Err("Invalid Gmail extension response".to_string()) };
            let words: Vec<_> = line
                .split(|c: char| c.is_ascii_whitespace() || c == '(' || c == ')')
                .filter(|p| !p.is_empty())
                .collect();
            untagged(&words);
            Ok(())
        })
    }
    /// Reads one response line, with the bytes of any literal it carries.
    fn read_line(stream: &mut BufReader<T>, total: &mut usize) -> Result<Vec<u8>, String> {
        let mut line = Vec::new();
        loop {
            let start = line.len();
            let read = stream
                .by_ref()
                .take(64 * 1024 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|e| e.to_string())?;
            *total += read;
            if read == 0 {
                return Err("Gmail closed the extension response before completion".into());
            }
            if !line.ends_with(b"\r\n") {
                return Err("Gmail extension response ended with an incomplete line".into());
            }
            if read > 64 * 1024 || *total > 8 * 1024 * 1024 {
                return Err("Gmail extension response exceeded its safety limit".into());
            }
            // A literal's bytes may hold CRLF or a tag; keep them inside this response.
            let Some(size) = literal_size(&line[start..]) else { return Ok(line) };
            if *total + size > 8 * 1024 * 1024 {
                return Err("Gmail extension response exceeded its safety limit".into());
            }
            let before = line.len();
            stream.by_ref().take(size as u64).read_to_end(&mut line).map_err(|e| e.to_string())?;
            if line.len() - before != size {
                return Err("Gmail closed the extension response before completion".into());
            }
            *total += size;
        }
    }
    /// Sends `command`, followed by `literal` as an IMAP literal (`{n}`, after the
    /// server's `+` continuation) when there is one, and hands every untagged
    /// line, literals included, to `untagged` until the tagged completion.
    fn exchange(
        &self,
        command: &str,
        literal: Option<&[u8]>,
        mut untagged: impl FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut counter = self
            .counter
            .lock()
            .map_err(|_| "IMAP extension lock failed")?;
        if self.poisoned.swap(true, Ordering::Relaxed) {
            return Err("IMAP extension response was not completed".into());
        }
        *counter += 1;
        let tag = format!("gmail{}", *counter);
        let mut stream = self.stream.lock().map_err(|_| "IMAP stream lock failed")?;
        let mut total = 0;
        let started = std::time::Instant::now();
        let tagged = |line: &[u8]| {
            line.strip_prefix(tag.as_bytes()).filter(|rest| rest.first() == Some(&b' ')).map(|rest| {
                let status = String::from_utf8_lossy(rest).trim().to_ascii_uppercase();
                status.starts_with("OK")
            })
        };
        match literal {
            None => {
                write!(stream.get_mut(), "{tag} {command}\r\n").map_err(|e| e.to_string())?;
                stream.get_mut().flush().map_err(|e| e.to_string())?;
            }
            Some(bytes) => {
                write!(stream.get_mut(), "{tag} {command} {{{}}}\r\n", bytes.len()).map_err(|e| e.to_string())?;
                stream.get_mut().flush().map_err(|e| e.to_string())?;
                loop {
                    if started.elapsed() > std::time::Duration::from_secs(30) {
                        return Err("Gmail extension response did not complete in time".into());
                    }
                    let line = Self::read_line(&mut stream, &mut total)?;
                    if line.starts_with(b"+") {
                        break;
                    }
                    if let Some(ok) = tagged(&line) {
                        self.poisoned.store(false, Ordering::Relaxed);
                        let _ = ok;
                        return Err("Gmail rejected the extension request".into());
                    }
                    if !line.starts_with(b"* ") || line[2..].to_ascii_uppercase().starts_with(b"BYE") {
                        return Err("Unexpected Gmail extension completion".into());
                    }
                    untagged(&line)?;
                }
                stream.get_mut().write_all(bytes).map_err(|e| e.to_string())?;
                stream.get_mut().write_all(b"\r\n").map_err(|e| e.to_string())?;
                stream.get_mut().flush().map_err(|e| e.to_string())?;
            }
        }
        // A reply line the caller cannot read fails this command only: the rest
        // of the response is still consumed, so the session stays usable.
        let mut unreadable = None;
        loop {
            if started.elapsed() > std::time::Duration::from_secs(30) {
                return Err("Gmail extension response did not complete in time".into());
            }
            let line = Self::read_line(&mut stream, &mut total)?;
            if let Some(ok) = tagged(&line) {
                self.poisoned.store(false, Ordering::Relaxed);
                if !ok {
                    return Err("Gmail rejected the extension request".into());
                }
                return unreadable.map_or(Ok(()), Err);
            }
            if !line.starts_with(b"* ") || line[2..].to_ascii_uppercase().starts_with(b"BYE") {
                return Err("Unexpected Gmail extension completion".into());
            }
            if unreadable.is_none() {
                if let Err(error) = untagged(&line) {
                    unreadable = Some(error);
                }
            }
        }
    }
    fn numeric_fetch(&self, sequence: &str, attribute: &str) -> Result<HashMap<u32, u64>, String> {
        if sequence.is_empty()
            || !sequence
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b',' || b == b':')
        {
            return Err("Invalid Gmail UID sequence".into());
        }
        let mut ids = HashMap::new();
        let mut parse_error = None;
        self.run(&format!("UID FETCH {sequence} (UID {attribute})"), |words| {
            if !words
                .get(2)
                .is_some_and(|s| s.eq_ignore_ascii_case("FETCH"))
            {
                return;
            }
            let value = |key: &str| {
                words
                    .windows(2)
                    .find(|p| p[0].eq_ignore_ascii_case(key))
                    .map(|p| p[1])
            };
            if let Some(global) = value(attribute) {
                match (
                    value("UID").and_then(|v| v.parse::<u32>().ok()),
                    global.parse::<u64>(),
                ) {
                    (Some(uid), Ok(global)) if global != 0 => {
                        if ids.insert(uid, global).is_some_and(|old| old != global) {
                            parse_error = Some("Conflicting Gmail message identities".to_string());
                        }
                    }
                    _ => parse_error = Some("Invalid Gmail message identity".to_string()),
                }
            }
        })?;
        parse_error.map_or(Ok(ids), Err)
    }
    /// Adds (`+`) or removes (`-`) already-quoted Gmail labels. imap 2.x cannot parse the X-GM-LABELS reply.
    pub(crate) fn store_labels(&self, uid: u32, change: char, labels: &str) -> Result<(), String> {
        if !matches!(change, '+' | '-') || labels.chars().any(char::is_control) {
            return Err("Invalid Gmail label change".into());
        }
        self.run(&format!("UID STORE {uid} {change}X-GM-LABELS ({labels})"), |_| {})
    }
    pub(crate) fn thread_id(&self, uid: u32) -> Result<u64, String> {
        self.numeric_fetch(&uid.to_string(), "X-GM-THRID")?
            .remove(&uid)
            .ok_or_else(|| "Gmail omitted the conversation ID".into())
    }
    /// Gmail search (`X-GM-RAW`) in the selected mailbox. Text that is not plain
    /// ASCII travels as a literal, which the crate's command parser cannot send.
    pub(crate) fn raw_search(&self, query: &str) -> Result<Vec<u32>, String> {
        if query.trim().is_empty() || query.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
            return Err("Invalid Gmail search".into());
        }
        let plain = query.is_ascii() && !query.chars().any(|c| c == '"' || c == '\\' || c.is_ascii_control());
        let mut uids = Vec::new();
        let mut collect = |line: &[u8]| -> Result<(), String> {
            let text = String::from_utf8_lossy(line);
            let mut words = text.split_ascii_whitespace();
            if words.next() == Some("*") && words.next().is_some_and(|w| w.eq_ignore_ascii_case("SEARCH")) {
                for word in words {
                    uids.push(word.parse::<u32>().map_err(|_| "Invalid Gmail search response".to_string())?);
                }
            }
            Ok(())
        };
        if plain {
            self.exchange(&format!("UID SEARCH CHARSET UTF-8 X-GM-RAW \"{query}\""), None, &mut collect)?;
        } else {
            self.exchange("UID SEARCH CHARSET UTF-8 X-GM-RAW", Some(query.as_bytes()), &mut collect)?;
        }
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }
    /// Identity, conversation and labels of a group of messages in one request.
    pub(crate) fn metadata(&self, sequence: &str) -> Result<HashMap<u32, GmailMetadata>, String> {
        if sequence.is_empty()
            || !sequence.bytes().all(|b| b.is_ascii_digit() || b == b',' || b == b':')
        {
            return Err("Invalid Gmail UID sequence".into());
        }
        let mut output = HashMap::new();
        self.exchange(&format!("UID FETCH {sequence} (UID X-GM-MSGID X-GM-THRID X-GM-LABELS)"), None, |line| {
            if let Some((uid, metadata)) = parse_fetch_metadata(line) {
                output.insert(uid, metadata);
            }
            Ok(())
        })?;
        Ok(output)
    }
}

/// What Gmail says about one message besides the standard FETCH items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GmailMetadata {
    pub global_id: Option<u64>,
    pub thread_id: Option<u64>,
    pub labels: Vec<String>,
}

#[derive(Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Atom(String),
    Text(Vec<u8>),
}

/// Splits an IMAP response line into atoms, strings, literals and parentheses.
fn tokens(line: &[u8]) -> Option<Vec<Token>> {
    let mut output = Vec::new();
    let mut i = 0;
    while i < line.len() {
        match line[i] {
            b' ' | b'\r' | b'\n' | b'\t' => i += 1,
            b'(' => { output.push(Token::Open); i += 1; }
            b')' => { output.push(Token::Close); i += 1; }
            b'"' => {
                let mut value = Vec::new();
                i += 1;
                loop {
                    match *line.get(i)? {
                        b'"' => { i += 1; break; }
                        b'\\' => { value.push(*line.get(i + 1)?); i += 2; }
                        byte => { value.push(byte); i += 1; }
                    }
                }
                output.push(Token::Text(value));
            }
            b'{' => {
                let close = i + line[i..].iter().position(|b| *b == b'}')?;
                let digits = std::str::from_utf8(&line[i + 1..close]).ok()?.trim_end_matches('+');
                let size: usize = digits.parse().ok()?;
                let start = close + 1 + line.get(close + 1..)?.starts_with(b"\r\n").then_some(2)?;
                output.push(Token::Text(line.get(start..start + size)?.to_vec()));
                i = start + size;
            }
            _ => {
                let start = i;
                while i < line.len() && !matches!(line[i], b' ' | b'(' | b')' | b'\r' | b'\n' | b'"') {
                    i += 1;
                }
                output.push(Token::Atom(String::from_utf8_lossy(&line[start..i]).into_owned()));
            }
        }
    }
    Some(output)
}

/// Reads `* n FETCH (UID u X-GM-MSGID m X-GM-THRID t X-GM-LABELS (...))`.
fn parse_fetch_metadata(line: &[u8]) -> Option<(u32, GmailMetadata)> {
    let tokens = tokens(line)?;
    let fetch = matches!(tokens.get(2), Some(Token::Atom(word)) if word.eq_ignore_ascii_case("FETCH"));
    if !matches!(tokens.first(), Some(Token::Atom(star)) if star == "*") || !fetch || tokens.get(3) != Some(&Token::Open) {
        return None;
    }
    let mut metadata = GmailMetadata::default();
    let mut uid = None;
    let mut i = 4;
    while let Some(Token::Atom(key)) = tokens.get(i) {
        let key = key.to_ascii_uppercase();
        i += 1;
        match tokens.get(i)? {
            Token::Open => {
                let mut values = Vec::new();
                i += 1;
                loop {
                    match tokens.get(i)? {
                        Token::Close => { i += 1; break; }
                        Token::Atom(value) => values.push(value.clone()),
                        Token::Text(value) => values.push(String::from_utf8_lossy(value).into_owned()),
                        Token::Open => return None,
                    }
                    i += 1;
                }
                if key == "X-GM-LABELS" {
                    metadata.labels = values.iter().map(|v| crate::domain::gmail_labels::decode_modified_utf7(v)).collect();
                }
            }
            Token::Atom(value) => {
                match key.as_str() {
                    "UID" => uid = value.parse().ok(),
                    "X-GM-MSGID" => metadata.global_id = value.parse().ok().filter(|v| *v != 0),
                    "X-GM-THRID" => metadata.thread_id = value.parse().ok().filter(|v| *v != 0),
                    _ => {}
                }
                i += 1;
            }
            Token::Text(_) => i += 1,
            Token::Close => return None,
        }
    }
    Some((uid?, metadata))
}
/// Size of an IMAP literal (`{n}` or `{n+}`) that ends a response line.
fn literal_size(line: &[u8]) -> Option<usize> {
    let line = line.strip_suffix(b"}\r\n")?;
    let open = line.iter().rposition(|b| *b == b'{')?;
    let digits = line[open + 1..].strip_suffix(b"+").unwrap_or(&line[open + 1..]);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

impl<T: Read + Write> GmailExtensions for Extensions<T> {
    fn ids(&self, sequence: &str) -> Result<HashMap<u32, u64>, String> {
        self.numeric_fetch(sequence, "X-GM-MSGID")
    }
}

pub(crate) struct Session<T: Read + Write> {
    inner: imap::Session<LineStream<T>>,
    pub(crate) extensions: Extensions<T>,
}
impl<T: Read + Write> Session<T> {
    pub(crate) fn new(stream: T, account: &Account) -> Result<Self, String> {
        let stream = Arc::new(Mutex::new(BufReader::new(stream)));
        let poisoned = Arc::new(AtomicBool::new(false));
        let mut client = imap::Client::new(LineStream(stream.clone(), poisoned.clone()));
        client.read_greeting().map_err(|e| e.to_string())?;
        let inner = super::authenticate(client, account)?;
        Ok(Self {
            inner,
            extensions: Extensions {
                stream,
                counter: Arc::new(Mutex::new(0)),
                poisoned,
            },
        })
    }
}
impl<T: Read + Write> Deref for Session<T> {
    type Target = imap::Session<LineStream<T>>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<T: Read + Write> DerefMut for Session<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Wire(io::Cursor<Vec<u8>>);
    impl Read for Wire {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Write for Wire {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn session(script: &str) -> Session<Wire> {
        Session::new(
            Wire(io::Cursor::new(
                format!("* OK hello\r\na1 OK login\r\n{script}").into_bytes(),
            )),
            &Account::default(),
        )
        .unwrap()
    }
    struct Recording(io::Cursor<Vec<u8>>, Arc<Mutex<Vec<u8>>>);
    impl Read for Recording {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Write for Recording {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.1.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn recorded(script: &str) -> (Session<Recording>, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        let session = Session::new(
            Recording(io::Cursor::new(format!("* OK hello\r\na1 OK login\r\n{script}").into_bytes()), written.clone()),
            &Account::default(),
        )
        .unwrap();
        (session, written)
    }
    fn sent(written: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8_lossy(&written.lock().unwrap()).into_owned()
    }
    #[test]
    fn plain_gmail_search_travels_as_a_quoted_string() {
        let (mut session, written) = recorded("* SEARCH 12 3 7\r\ngmail1 OK done\r\na2 OK noop\r\n");
        assert_eq!(session.extensions.raw_search("from:anna fattura").unwrap(), [3, 7, 12]);
        assert!(sent(&written).contains("gmail1 UID SEARCH CHARSET UTF-8 X-GM-RAW \"from:anna fattura\"\r\n"));
        session.noop().unwrap();
    }
    #[test]
    fn non_ascii_or_quoted_search_travels_as_a_literal_after_the_continuation() {
        let query = "perché label:\"Clienti\"";
        let (mut session, written) = recorded("+ go ahead\r\n* SEARCH 5\r\ngmail1 OK done\r\na2 OK noop\r\n");
        assert_eq!(session.extensions.raw_search(query).unwrap(), [5]);
        let text = sent(&written);
        assert!(text.contains(&format!("gmail1 UID SEARCH CHARSET UTF-8 X-GM-RAW {{{}}}\r\n{query}\r\n", query.len())), "{text}");
        session.noop().unwrap();
    }
    #[test]
    fn an_empty_search_result_and_a_refusal_are_told_apart() {
        let (mut session, _) = recorded("* SEARCH\r\ngmail1 OK done\r\na2 OK noop\r\ngmail2 NO bad query\r\na3 OK noop\r\n+ go\r\ngmail3 BAD nope\r\na4 OK noop\r\n");
        assert!(session.extensions.raw_search("nothing").unwrap().is_empty());
        session.noop().unwrap();
        assert!(session.extensions.raw_search("broken").is_err());
        session.noop().unwrap();
        assert!(session.extensions.raw_search("caffè").is_err());
        session.noop().unwrap();
    }
    #[test]
    fn an_unreadable_search_reply_fails_the_command_but_not_the_session() {
        let (mut session, _) = recorded("* SEARCH 1 (MODSEQ 9)\r\ngmail1 OK done\r\na2 OK noop\r\n* SEARCH 4\r\ngmail2 OK done\r\n");
        assert!(session.extensions.raw_search("x").is_err());
        session.noop().unwrap();
        assert_eq!(session.extensions.raw_search("y").unwrap(), [4]);
    }
    #[test]
    fn a_literal_refused_before_the_continuation_leaves_the_session_usable() {
        let (mut session, _) = recorded("gmail1 NO charset\r\na2 OK noop\r\n");
        assert!(session.extensions.raw_search("caffè").is_err());
        session.noop().unwrap();
    }
    #[test]
    fn labels_ids_and_threads_arrive_in_one_request() {
        let script = concat!(
            "* 1 FETCH (UID 9 X-GM-MSGID 42 X-GM-THRID 41 X-GM-LABELS (\\Inbox \"Clienti\" \"2024\"))\r\n",
            "* 2 FETCH (X-GM-LABELS (\"\\\\Important\" \"Progetto X\" Fatture/&AOg-) UID 10 X-GM-THRID 7 X-GM-MSGID 43)\r\n",
            "* 3 FETCH (UID 11 X-GM-MSGID 44 X-GM-THRID 44 X-GM-LABELS ())\r\n",
            "* 4 FETCH (UID 12 X-GM-MSGID 45 X-GM-THRID 45 X-GM-LABELS ({6}\r\na b\r\nc \\Sent))\r\n",
            "gmail1 OK done\r\na2 OK noop\r\n",
        );
        let (mut session, written) = recorded(script);
        let metadata = session.extensions.metadata("9:12").unwrap();
        assert!(sent(&written).contains("UID FETCH 9:12 (UID X-GM-MSGID X-GM-THRID X-GM-LABELS)"));
        assert_eq!(metadata[&9], GmailMetadata { global_id: Some(42), thread_id: Some(41), labels: vec!["\\Inbox".into(), "Clienti".into(), "2024".into()] });
        assert_eq!(metadata[&10].labels, ["\\Important", "Progetto X", "Fatture/è"]);
        assert_eq!(metadata[&10].thread_id, Some(7));
        assert!(metadata[&11].labels.is_empty());
        assert_eq!(metadata[&12].labels, ["a b\r\nc", "\\Sent"]);
        session.noop().unwrap();
    }
    #[test]
    fn gmail_census_and_message_fetch_drain_completion_before_next_session_command() {
        let mut session = session(
            "* 4 EXISTS\r\n* 1 FETCH (UID 9 X-GM-MSGID 18446744073709551614)\r\n* 2 FETCH (X-GM-MSGID 42 UID 10)\r\ngmail1 OK fetched\r\na2 OK noop\r\n* 1 FETCH (UID 9 X-GM-MSGID 18446744073709551614)\r\ngmail2 OK fetched\r\na3 OK noop\r\n",
        );
        assert_eq!(session.extensions.ids("9,10").unwrap().len(), 2);
        session.noop().unwrap();
        assert_eq!(
            session.extensions.ids("9").unwrap()[&9],
            18446744073709551614
        );
        session.noop().unwrap();
    }
    #[test]
    fn rejected_or_malformed_extension_is_not_a_successful_identity() {
        let mut session = session(
            "* 1 FETCH (UID 9 X-GM-MSGID nope)\r\ngmail1 OK fetched\r\na2 OK noop\r\ngmail2 NO unavailable\r\na3 OK noop\r\n",
        );
        assert!(session.extensions.ids("9").is_err());
        session.noop().unwrap();
        assert!(session.extensions.ids("9").is_err());
        session.noop().unwrap();
    }
    #[test]
    fn extension_without_tagged_completion_fails() {
        let mut session = session("* 1 FETCH (UID 9 X-GM-MSGID 42)\r\n");
        assert!(session.extensions.ids("9").is_err());
        assert!(session.noop().is_err());
    }
    #[test]
    fn ordinary_body_literals_preserve_newlines_and_command_boundaries() {
        let raw = "From: sender@example.com\r\nSubject: Literal\r\n\r\nfirst\r\n* not a response\r\nlast\n";
        let mut session = session(&format!(
            "* 1 FETCH (UID 9 BODY[] {{{}}}\r\n{raw})\r\na2 OK fetched\r\n* 1 FETCH (UID 9 X-GM-MSGID 42)\r\ngmail1 OK fetched\r\na3 OK noop\r\n",
            raw.len()
        ));
        let fetched = session.uid_fetch("9", "BODY.PEEK[]").unwrap();
        assert_eq!(fetched.iter().next().unwrap().body(), Some(raw.as_bytes()));
        assert_eq!(session.extensions.ids("9").unwrap()[&9], 42);
        session.noop().unwrap();
    }
}
