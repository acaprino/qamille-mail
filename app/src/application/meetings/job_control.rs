use super::*;
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

/// Bridges a tiny shared cancellation marker to engines' AtomicBool contract.
/// No large meeting JSON is polled while inference or a network call is running.
pub(super) struct CancellationWatch {
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}
impl CancellationWatch {
    pub(super) fn start(
        lease: Arc<dyn MeetingJobLease>,
        job_id: String,
        cancel: Arc<AtomicBool>,
    ) -> AppResult<Self> {
        let (stop, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("meeting-cancellation".into())
            .spawn(move || {
                while !cancel.load(Ordering::Relaxed) {
                    match lease.cancellation_requested(&job_id) {
                        Ok(false) => {}
                        Ok(true) | Err(_) => {
                            cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    match receiver.recv_timeout(Duration::from_millis(250)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        _ => break,
                    }
                }
            })
            .map_err(AppError::from)?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for CancellationWatch {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
