//! Persistent Gmail notification sessions, separate from download/action sessions.
use crate::application::{AppError, AppResult, ports::MailWatch};
use crate::model::Account;
use imap::extensions::idle::SetReadTimeout;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

// Let the existing IMAP client authenticate and select, but keep read-ahead in
// our reader when that client is dropped. imap 2.x does not expose its transport.
struct CommandStream<'a, T>(&'a mut BufReader<T>);
impl<T: Read> Read for CommandStream<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.0.fill_buf()?;
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1)
            .min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.0.consume(count);
        Ok(count)
    }
}
impl<T: Write> Write for CommandStream<'_, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.get_mut().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.get_mut().flush()
    }
}

struct IdleWatch<T> {
    stream: BufReader<T>,
    tag: u64,
    partial: Vec<u8>,
}
impl<T: Read + Write + SetReadTimeout> IdleWatch<T> {
    fn new(stream: T, account: &Account) -> AppResult<Self> {
        let mut stream = BufReader::new(stream);
        {
            let mut client = imap::Client::new(CommandStream(&mut stream));
            client.read_greeting().map_err(external)?;
            let mut session =
                super::mail::authenticate(client, account).map_err(AppError::external)?;
            if !session.capabilities().map_err(external)?.has_str("IDLE") {
                return Err(AppError::new(
                    "idle_unsupported",
                    "This mail server does not support IDLE; periodic sync remains active",
                ));
            }
            session.select("INBOX").map_err(external)?;
            // A signed-in notification session is proof the account is reachable
            // again: wake its queued confirmations like every other successful
            // connection does (ADR-0007 v3, decision 8 / EX-009).
            crate::application::connectivity::note_success(account.id);
        }
        Ok(Self {
            stream,
            tag: 0,
            partial: Vec::new(),
        })
    }
    fn write(&mut self, command: &str) -> AppResult<()> {
        self.stream
            .get_mut()
            .write_all(command.as_bytes())
            .map_err(io_error)?;
        self.stream.get_mut().flush().map_err(io_error)
    }
    // Keep partial lines across a read timeout and bound both response size and
    // total command time, including keepalives and the tagged DONE completion.
    fn line(&mut self, deadline: Instant) -> AppResult<Option<String>> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        self.stream
            .get_mut()
            .set_read_timeout(Some(remaining))
            .map_err(external)?;
        let limit = (64 * 1024 + 1 - self.partial.len()) as u64;
        let read = self
            .stream
            .by_ref()
            .take(limit)
            .read_until(b'\n', &mut self.partial);
        if self.partial.len() > 64 * 1024 {
            return Err(AppError::external(
                "Gmail notification response is too large",
            ));
        }
        match read {
            Ok(0) => Err(AppError::external("Gmail notification connection closed")),
            Ok(_) => Ok(Some(
                String::from_utf8_lossy(&std::mem::take(&mut self.partial))
                    .trim_end()
                    .to_string(),
            )),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(io_error(e)),
        }
    }
}
impl<T: Read + Write + SetReadTimeout + Send> MailWatch for IdleWatch<T> {
    fn wait(&mut self, timeout: Duration) -> AppResult<bool> {
        self.tag += 1;
        let tag = format!("notify{}", self.tag);
        self.write(&format!("{tag} IDLE\r\n"))?;
        let mut changed = false;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            let line = self
                .line(deadline)?
                .ok_or_else(|| AppError::external("Gmail did not start IDLE in time"))?;
            if line.starts_with('+') {
                break;
            }
            changed |= mailbox_change(&line)?;
        }
        let deadline = Instant::now() + timeout;
        while !changed {
            let Some(line) = self.line(deadline)? else {
                break;
            };
            changed |= mailbox_change(&line)?;
        }
        self.write("DONE\r\n")?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            let line = self
                .line(deadline)?
                .ok_or_else(|| AppError::external("Gmail did not finish IDLE in time"))?;
            if let Some(status) = line.strip_prefix(&format!("{tag} ")) {
                if status
                    .split_whitespace()
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("OK"))
                {
                    return Ok(changed);
                }
                return Err(AppError::external("Gmail rejected the IDLE completion"));
            }
            changed |= mailbox_change(&line)?;
        }
    }
}
fn mailbox_change(line: &str) -> AppResult<bool> {
    let mut words = line.split_whitespace();
    if words.next() != Some("*") {
        return Err(AppError::external("Unexpected Gmail notification response"));
    }
    let kind = words.next().unwrap_or_default();
    if kind.eq_ignore_ascii_case("BYE") {
        return Err(AppError::external("Gmail notification session expired"));
    }
    Ok(!kind.eq_ignore_ascii_case("OK"))
}
fn external(error: imap::Error) -> AppError {
    AppError::external(format!("Gmail notifications: {error}"))
}
fn io_error(error: io::Error) -> AppError {
    AppError::external(format!("Gmail notifications: {error}"))
}

pub fn watch(account: &Account) -> AppResult<Box<dyn MailWatch>> {
    let stream = super::mail::connect_stream(account).map_err(AppError::external)?;
    Ok(Box::new(IdleWatch::new(stream, account)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::MailWatch;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    // This server only speaks the commands used by the notification connection.
    fn server(updates: &[&str]) -> (TcpStream, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let updates: Vec<String> = updates.iter().map(|s| s.to_string()).collect();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            socket.write_all(b"* OK ready\r\n").unwrap();
            let mut idle_tag = String::new();
            let supports_idle = updates.first().is_none_or(|update| update != "no-idle");
            let mut updates = updates.into_iter();
            let mut during_done = false;
            let mut partial = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap();
                if line.contains(" LOGIN ") {
                    socket
                        .write_all(format!("{tag} OK login\r\n").as_bytes())
                        .unwrap();
                } else if line.contains(" CAPABILITY") {
                    let idle = if supports_idle { " IDLE" } else { "" };
                    socket
                        .write_all(
                            format!("* CAPABILITY IMAP4rev1{idle}\r\n{tag} OK capabilities\r\n")
                                .as_bytes(),
                        )
                        .unwrap();
                } else if line.contains(" SELECT ") {
                    assert!(line.contains("INBOX"));
                    socket
                        .write_all(format!("* 1 EXISTS\r\n{tag} OK selected\r\n").as_bytes())
                        .unwrap();
                } else if line.contains(" IDLE") {
                    idle_tag = tag.to_string();
                    let update = updates.next();
                    if update.as_deref() == Some("before-continuation") {
                        socket.write_all(b"* 2 EXISTS\r\n+ idling\r\n").unwrap();
                        continue;
                    }
                    socket.write_all(b"+ idling\r\n").unwrap();
                    if let Some(update) = update {
                        if update == "disconnect" {
                            break;
                        }
                        if update == "during-done" {
                            during_done = true;
                        } else if update == "partial" {
                            socket.write_all(b"* 2 EX").unwrap();
                            partial = true;
                        } else {
                            socket.write_all(update.as_bytes()).unwrap();
                        }
                    }
                } else if line.trim() == "DONE" {
                    if during_done {
                        socket.write_all(b"* 2 EXISTS\r\n").unwrap();
                        during_done = false;
                    }
                    if partial {
                        socket.write_all(b"ISTS\r\n").unwrap();
                        partial = false;
                    }
                    socket
                        .write_all(format!("{idle_tag} OK idle finished\r\n").as_bytes())
                        .unwrap();
                } else {
                    panic!("Unexpected notification-connection command: {line}");
                }
            }
        });
        (socket, worker)
    }

    fn watch(socket: TcpStream) -> IdleWatch<TcpStream> {
        // A signed-in watch notes process-wide connectivity: hold the wake-set
        // test lock across the note and consume it here, so this account's wake
        // never leaks into the connectivity assertions of parallel tests.
        let _guard = crate::application::connectivity::test_guard();
        let account =
            super::super::mail::gmail_account(1, "", "sample@example.com", "synthetic", [0, 0, 0]);
        let watch = IdleWatch::new(socket, &account).unwrap();
        crate::application::connectivity::drain_wakes();
        watch
    }

    #[test]
    fn idle_receives_new_mail_flags_and_removals_on_the_same_connection() {
        let (socket, server) = server(&[
            "* 2 EXISTS\r\n",
            "* 1 FETCH (FLAGS (\\Seen))\r\n",
            "* 1 EXPUNGE\r\n",
        ]);
        let mut watch = watch(socket);
        for _ in 0..3 {
            assert!(watch.wait(Duration::from_secs(1)).unwrap());
        }
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn idle_timeout_does_not_report_mail_and_the_connection_can_be_reused() {
        let (socket, server) = server(&["", "* 2 EXISTS\r\n"]);
        let mut watch = watch(socket);
        assert!(!watch.wait(Duration::from_millis(25)).unwrap());
        assert!(watch.wait(Duration::from_secs(1)).unwrap());
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn dropped_idle_connection_returns_an_error_instead_of_hanging() {
        let (socket, server) = server(&["disconnect"]);
        let mut watch = watch(socket);
        let started = Instant::now();
        assert!(watch.wait(Duration::from_millis(25)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn changes_before_idle_continuation_do_not_panic_or_get_lost() {
        let (socket, server) = server(&["before-continuation"]);
        let mut watch = watch(socket);
        assert!(watch.wait(Duration::from_millis(25)).unwrap());
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn changes_during_done_are_not_lost_after_a_quiet_timeout() {
        let (socket, server) = server(&["during-done"]);
        let mut watch = watch(socket);
        assert!(watch.wait(Duration::from_millis(25)).unwrap());
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn a_notification_split_across_the_wait_timeout_is_preserved() {
        let (socket, server) = server(&["partial"]);
        let mut watch = watch(socket);
        assert!(watch.wait(Duration::from_millis(25)).unwrap());
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn session_expiration_requests_a_reconnect() {
        let (socket, server) = server(&["* BYE Session expired\r\n"]);
        let mut watch = watch(socket);
        assert!(watch.wait(Duration::from_millis(25)).is_err());
        drop(watch);
        server.join().unwrap();
    }

    #[test]
    fn servers_without_idle_are_reported_for_periodic_fallback() {
        let (socket, server) = server(&["no-idle"]);
        let account =
            super::super::mail::gmail_account(1, "", "sample@example.com", "synthetic", [0, 0, 0]);
        assert!(
            matches!(IdleWatch::new(socket, &account), Err(error) if error.code == "idle_unsupported")
        );
        server.join().unwrap();
    }
}
