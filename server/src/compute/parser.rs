//! The sacrificial parser process of the server (INV-11): document readers and image decoding
//! run in `qamille-server --parser-worker`, never in the process that holds the model. The
//! frame protocol is the one of the app's former `--embedding-worker`.
//!
//! A process that dies on a file may have died on its own: one fresh process decides whether
//! the file is at fault (one repetition, spec D2). A second death on the same file, or a reader
//! that runs past its budget, makes the file `document_unreadable`; the server and the model
//! stay up.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use qamille_compute::frames::{self, Request, Response};

use super::{ComputeError, DOCUMENT_UNREADABLE};

/// The program of the parser process and its arguments.
#[derive(Clone, Debug)]
pub struct ParserCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl ParserCommand {
    /// This same executable with `--parser-worker`.
    /// The process watches this server's PID and ends with it, even inside a stuck reader.
    pub fn current_exe() -> std::io::Result<Self> {
        Ok(ParserCommand {
            program: std::env::current_exe()?,
            args: vec!["--parser-worker".to_owned(), "--parent".to_owned(), std::process::id().to_string()],
        })
    }
}

struct Process {
    /// Shared with [`ParserPool::shutdown`], which kills it without waiting for the slot.
    child: Arc<Mutex<Child>>,
    input: ChildStdin,
    replies: mpsc::Receiver<std::io::Result<(Response, Vec<u8>)>>,
}

fn kill(child: &Mutex<Child>) {
    let mut child = child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for Process {
    fn drop(&mut self) {
        kill(&self.child);
    }
}

struct Slot {
    process: Option<Process>,
    last_used: Option<Instant>,
}

pub struct ParserPool {
    command: ParserCommand,
    slot: Mutex<Slot>,
    /// Budget of one file, as the embedding worker had.
    timeout: Duration,
    idle: Duration,
    current: Mutex<Option<Arc<Mutex<Child>>>>,
    stopped: AtomicBool,
}

impl ParserPool {
    pub fn new(command: ParserCommand) -> Self {
        Self::with_timeout(command, Duration::from_secs(120))
    }

    pub fn with_timeout(command: ParserCommand, timeout: Duration) -> Self {
        ParserPool {
            command,
            slot: Mutex::new(Slot { process: None, last_used: None }),
            timeout,
            idle: Duration::from_secs(60),
            current: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }

    fn spawn(&self) -> std::io::Result<Process> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("the server is stopping"));
        }
        let mut command = Command::new(&self.command.program);
        command.args(&self.command.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        // Il lettore tratta file di altri: non eredita le chiavi della sincronizzazione.
        command.env_remove("QAMILLE_SYNC_KEY").env_remove("QAMILLE_SYNC_KEY_OLD");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS; the process inherits the job
            // (and its per-process memory limit) the desktop app applied to the server.
            command.creation_flags(0x0800_0000 | 0x0000_4000);
        }
        let mut child = command.spawn()?;
        let input = child.stdin.take().ok_or_else(|| std::io::Error::other("Embedding input unavailable"))?;
        let mut output = child.stdout.take().ok_or_else(|| std::io::Error::other("Embedding output unavailable"))?;
        let (sender, replies) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("parser-replies".to_owned()).spawn(move || loop {
            let reply = frames::read_frame(&mut output).and_then(|bytes| {
                let response: Response = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
                let payload = frames::read_payload(&mut output, response.bytes)?;
                Ok((response, payload))
            });
            let failed = reply.is_err();
            if sender.send(reply).is_err() || failed {
                break;
            }
        })?;
        let child = Arc::new(Mutex::new(child));
        *self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&child));
        Ok(Process { child, input, replies })
    }

    /// One request; the answer's own error (a reader that refused the file) is returned as
    /// `document_unreadable` with the reader's message.
    pub fn request(&self, kind: &str, name: &str, data: &[u8]) -> Result<(Response, Vec<u8>), ComputeError> {
        let request = Request { kind: kind.to_owned(), name: name.to_owned(), bytes: data.len() };
        let mut slot = self.slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut result: Result<(Response, Vec<u8>), String> = Err(String::new());
        for _ in 0..2 {
            if slot.process.is_none() {
                slot.process = Some(self.spawn().map_err(|error| ComputeError::Internal(format!("Cannot start the document reader: {error}")))?);
            }
            let started = Instant::now();
            let process = slot.process.as_mut().expect("parser process");
            result = (|| {
                frames::write_message(&mut process.input, &request, data).map_err(|e| e.to_string())?;
                process.input.flush().map_err(|e| e.to_string())?;
                match process.replies.recv_timeout(self.timeout) {
                    Ok(reply) => reply.map_err(|e| e.to_string()),
                    Err(_) => Err("Embedding worker stopped or exceeded its time budget".to_owned()),
                }
            })();
            slot.last_used = Some(Instant::now());
            if result.is_ok() {
                break;
            }
            slot.process = None;
            // One repetition on a fresh process (the old one may have died on its own); a
            // reader that ran out of time is not given a second budget.
            if started.elapsed() >= self.timeout {
                break;
            }
        }
        match result {
            Ok((response, payload)) => match response.error {
                Some(error) => {
                    // A failed reader may leave large allocations behind.
                    slot.process = None;
                    Err(ComputeError::DocumentUnreadable(error))
                }
                None => Ok((response, payload)),
            },
            Err(_) => Err(ComputeError::DocumentUnreadable(DOCUMENT_UNREADABLE.to_owned())),
        }
    }

    pub fn release_idle(&self) {
        if let Ok(mut slot) = self.slot.try_lock()
            && slot.last_used.is_some_and(|at| at.elapsed() >= self.idle)
        {
            slot.process = None;
        }
    }

    /// Kills the parser process now, even one stuck in a reader (the server is stopping).
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let current = self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(child) = current {
            kill(&child);
        }
    }

    /// Whether a parser process is running (for tests and diagnostics).
    pub fn running(&self) -> bool {
        self.pid().is_some()
    }

    /// The running parser process, if any (for tests and diagnostics).
    pub fn pid(&self) -> Option<u32> {
        let slot = self.slot.lock().ok()?;
        let child = slot.process.as_ref()?.child.lock().ok()?;
        Some(child.id())
    }
}
