//! Bounded audio capture and durable WAV checkpoints. Streams never leave their owner thread.
//! CPAL's WASAPI input builder enables loopback when given an output (eRender) endpoint.
use crate::application::{AppError, AppResult};
use crate::domain::meeting::{
    AudioChunk, AudioDevice, AudioGap, CaptureRequest, CaptureSnapshot, CaptureState,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

#[path = "meeting_audio/durability.rs"]
mod durability;
pub use durability::{audio_bytes, discard_audio, load_capture, recover_capture};
use durability::{validate_chunk, write_chunk, write_snapshot};

const CHECKPOINT_SECONDS: usize = 1;
const MAX_MIX_MS: u64 = 120_000;
const FAULT_OVERFLOW: u32 = 1;
const FAULT_DEVICE: u32 = 2;

type Reply = mpsc::Sender<AppResult<CaptureSnapshot>>;
enum Command {
    Start(CaptureRequest, PathBuf, Reply),
    Pause(Reply),
    Resume(Reply),
    Stop(Reply),
}

/// Clones share one capture owner. Constructing this controller does not open any device.
#[derive(Clone)]
pub struct NativeAudioCapture {
    commands: mpsc::Sender<Command>,
    snapshot: Arc<Mutex<CaptureSnapshot>>,
}

impl NativeAudioCapture {
    pub fn new() -> AppResult<Self> {
        let (commands, receiver) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(CaptureSnapshot::default()));
        let shared = snapshot.clone();
        std::thread::Builder::new()
            .name("meeting-audio".into())
            .spawn(move || supervise_owner(shared, |state| owner_loop(receiver, state)))
            .map_err(AppError::from)?;
        Ok(Self { commands, snapshot })
    }
    pub fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        list_devices()
    }
    pub fn start(&self, request: CaptureRequest, folder: PathBuf) -> AppResult<CaptureSnapshot> {
        self.call(|reply| Command::Start(request, folder, reply))
    }
    pub fn pause(&self) -> AppResult<CaptureSnapshot> {
        self.call(Command::Pause)
    }
    pub fn resume(&self) -> AppResult<CaptureSnapshot> {
        self.call(Command::Resume)
    }
    pub fn stop(&self) -> AppResult<CaptureSnapshot> {
        self.call(Command::Stop)
    }
    pub fn snapshot(&self) -> AppResult<CaptureSnapshot> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| AppError::new("capture_unavailable", "Audio state is unavailable"))
    }
    fn call(&self, make: impl FnOnce(Reply) -> Command) -> AppResult<CaptureSnapshot> {
        let (tx, rx) = mpsc::channel();
        self.commands
            .send(make(tx))
            .map_err(|_| AppError::new("capture_unavailable", "Audio worker stopped"))?;
        // A timeout must not pretend an in-flight start was cancelled. The owner always replies.
        rx.recv()
            .map_err(|_| AppError::new("capture_unavailable", "Audio worker stopped"))?
    }
}

fn supervise_owner(
    shared: Arc<Mutex<CaptureSnapshot>>,
    run: impl FnOnce(Arc<Mutex<CaptureSnapshot>>),
) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(shared.clone()))).is_err() {
        let mut state = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.state = CaptureState::Interrupted;
        state.incomplete = true;
        state.microphone_level = 0.0;
        state.system_level = 0.0;
        let message = "Audio worker stopped unexpectedly. Confirmed audio blocks can be recovered; the final unsaved fraction may be missing.".to_string();
        state.warnings.push(message.clone());
        state.error = Some(message);
    }
}

fn list_devices() -> AppResult<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let default_mic = host.default_input_device().and_then(|d| d.name().ok());
    let default_out = host.default_output_device().and_then(|d| d.name().ok());
    let mut result = Vec::new();
    for (kind, devices, default_name) in [
        (
            "microphone",
            host.input_devices()
                .map_err(device_error)?
                .collect::<Vec<_>>(),
            default_mic,
        ),
        (
            "system",
            host.output_devices()
                .map_err(device_error)?
                .collect::<Vec<_>>(),
            default_out,
        ),
    ] {
        if kind == "system" && !cfg!(target_os = "windows") {
            continue;
        }
        let mut names = std::collections::HashMap::<String, usize>::new();
        for device in devices {
            let name = device.name().map_err(device_error)?;
            let occurrence = names.entry(name.clone()).or_default();
            let id = format!("{kind}:{}:{occurrence}", encode_name(&name));
            *occurrence += 1;
            result.push(AudioDevice {
                id,
                name: name.clone(),
                kind: kind.into(),
                is_default: default_name.as_ref() == Some(&name),
            });
        }
    }
    Ok(result)
}

fn encode_name(name: &str) -> String {
    name.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn select_device(kind: &str, requested: &str) -> AppResult<cpal::Device> {
    let host = cpal::default_host();
    if kind == "system" && !cfg!(target_os = "windows") {
        return Err(AppError::new(
            "unsupported_audio",
            "System audio capture is currently available on Windows",
        ));
    }
    if requested == "default" || requested.is_empty() {
        return (if kind == "microphone" {
            host.default_input_device()
        } else {
            host.default_output_device()
        })
        .ok_or_else(|| {
            AppError::new(
                "device_unavailable",
                "The selected audio source is unavailable",
            )
        });
    }
    let devices = if kind == "microphone" {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(device_error)?;
    let mut names = std::collections::HashMap::<String, usize>::new();
    for device in devices {
        let name = device.name().map_err(device_error)?;
        let occurrence = names.entry(name.clone()).or_default();
        let id = format!("{kind}:{}:{occurrence}", encode_name(&name));
        *occurrence += 1;
        if id == requested {
            return Ok(device);
        }
    }
    Err(AppError::new(
        "device_unavailable",
        "The selected audio source was disconnected; choose a device again",
    ))
}

fn device_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "device_unavailable",
        format!("Audio device unavailable: {error}"),
    )
}

struct Clock {
    started: Instant,
    paused_ns: AtomicU64,
    active: AtomicBool,
    in_flight: AtomicUsize,
}
impl Clock {
    fn elapsed_ns(&self) -> u64 {
        (self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64)
            .saturating_sub(self.paused_ns.load(Ordering::Acquire))
    }
}

struct Packet {
    source: usize,
    start_ms: u64,
    samples: Vec<f32>,
}
struct Track {
    source: String,
    sample_rate: u32,
    pending: Vec<f32>,
    start_ms: u64,
    sequence: u64,
    queued: Arc<AtomicUsize>,
    level: Arc<AtomicU32>,
    last_packet_ns: Arc<AtomicU64>,
    pending_since: Option<Instant>,
}
struct Session {
    folder: PathBuf,
    streams: Vec<cpal::Stream>,
    tracks: Vec<Track>,
    receiver: mpsc::Receiver<Packet>,
    clock: Arc<Clock>,
    fault: Arc<AtomicU32>,
    snapshot: CaptureSnapshot,
    paused_at: Option<Instant>,
    checked_space_at: Instant,
    manifest_saved_ms: u64,
    _folder_lock: File,
    _recording_lock: File,
}

fn owner_loop(commands: mpsc::Receiver<Command>, shared: Arc<Mutex<CaptureSnapshot>>) {
    let mut active: Option<Session> = None;
    loop {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => {
                let (result, reply) = match command {
                    Command::Start(request, folder, reply) => {
                        let result = if let Some(session) = active.as_ref() {
                            if session.snapshot.meeting_id == Some(request.meeting_id) {
                                Ok(session.snapshot.clone())
                            } else {
                                Err(AppError::new(
                                    "recording_busy",
                                    "Another meeting is being recorded",
                                ))
                            }
                        } else {
                            match Session::start(request, folder) {
                                Ok(session) => {
                                    let snapshot = session.snapshot.clone();
                                    active = Some(session);
                                    Ok(snapshot)
                                }
                                Err(error) => Err(error),
                            }
                        };
                        (result, reply)
                    }
                    Command::Pause(reply) => (
                        active
                            .as_mut()
                            .ok_or_else(no_recording)
                            .and_then(Session::pause),
                        reply,
                    ),
                    Command::Resume(reply) => (
                        active
                            .as_mut()
                            .ok_or_else(no_recording)
                            .and_then(Session::resume),
                        reply,
                    ),
                    Command::Stop(reply) => {
                        let result = if let Some(mut session) = active.take() {
                            let result = session.stop();
                            publish(&shared, &session.snapshot);
                            result
                        } else {
                            shared.lock().map(|s| s.clone()).map_err(|_| no_recording())
                        };
                        (result, reply)
                    }
                };
                if let Ok(snapshot) = &result {
                    publish(&shared, snapshot);
                }
                if let Some(session) = active.as_ref() {
                    publish(&shared, &session.snapshot);
                }
                if active
                    .as_ref()
                    .is_some_and(|s| s.snapshot.state == CaptureState::Interrupted)
                {
                    active = None;
                }
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(mut session) = active.take() {
                    let _ = session.stop();
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(session) = active.as_mut() {
            if let Err(error) = session.tick() {
                let snapshot = session.interrupt(&error.message);
                publish(&shared, &snapshot);
                active = None;
            } else {
                publish(&shared, &session.snapshot);
            }
        }
    }
}
fn no_recording() -> AppError {
    AppError::new("not_recording", "No recording is active")
}
fn publish(shared: &Mutex<CaptureSnapshot>, snapshot: &CaptureSnapshot) {
    if let Ok(mut stored) = shared.lock() {
        if stored.meeting_id != snapshot.meeting_id || stored.chunks.len() > snapshot.chunks.len() {
            *stored = snapshot.clone();
            return;
        }
        // A live session only appends immutable chunk identities. Publish the
        // suffix, not thousands of paths every 20 ms while updating meters.
        let confirmed = stored.chunks.len();
        stored
            .chunks
            .extend_from_slice(&snapshot.chunks[confirmed..]);
        stored.state.clone_from(&snapshot.state);
        stored.elapsed_ms = snapshot.elapsed_ms;
        stored.saved_ms = snapshot.saved_ms;
        stored.microphone_level = snapshot.microphone_level;
        stored.system_level = snapshot.system_level;
        stored.error.clone_from(&snapshot.error);
        stored.warnings.clone_from(&snapshot.warnings);
        stored.missing_chunks.clone_from(&snapshot.missing_chunks);
        stored.incomplete = snapshot.incomplete;
        if stored.gaps.len() <= snapshot.gaps.len() {
            let reported = stored.gaps.len();
            stored.gaps.extend_from_slice(&snapshot.gaps[reported..]);
        } else {
            stored.gaps.clone_from(&snapshot.gaps);
        }
        stored.requires_receipts = snapshot.requires_receipts;
        stored
            .unverified_chunks
            .clone_from(&snapshot.unverified_chunks);
    }
}

impl Session {
    fn start(request: CaptureRequest, folder: PathBuf) -> AppResult<Self> {
        if request.microphone.is_none() && request.system.is_none() {
            return Err(AppError::validation(
                "Select a microphone or computer audio",
            ));
        }
        std::fs::create_dir_all(&folder)?;
        let folder = folder.canonicalize()?;
        if let Some(bytes) = available_disk_bytes(&folder)? {
            disk_notice(bytes)?;
        }
        let folder_lock = acquire_lock(&folder.join(".capture.lock"))?;
        let parent = folder
            .parent()
            .ok_or_else(|| AppError::validation("Invalid meeting folder"))?;
        let recording_lock = acquire_lock(&parent.join(".recording.lock"))?;
        if std::fs::read_dir(&folder)?
            .filter_map(Result::ok)
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with("capture.json")
                    || name.ends_with(".wav")
                    || name.ends_with(".wav.tmp")
            })
        {
            return Err(AppError::new(
                "recording_exists",
                "This meeting already has audio; recover it or start another meeting",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(1024);
        let clock = Arc::new(Clock {
            started: Instant::now(),
            paused_ns: AtomicU64::new(0),
            active: AtomicBool::new(false),
            in_flight: AtomicUsize::new(0),
        });
        let fault = Arc::new(AtomicU32::new(0));
        let mut tracks = Vec::new();
        let mut streams = Vec::new();
        for (kind, selection) in [
            ("microphone", request.microphone),
            ("system", request.system),
        ] {
            if let Some(selected) = selection {
                let device = select_device(kind, &selected)?;
                let config = if kind == "microphone" {
                    device.default_input_config()
                } else {
                    device.default_output_config()
                }
                .map_err(device_error)?;
                if config.channels() == 0
                    || config.channels() > 32
                    || !(8_000..=192_000).contains(&config.sample_rate().0)
                {
                    return Err(AppError::new(
                        "unsupported_audio",
                        "This audio device uses an unsupported format",
                    ));
                }
                let queued = Arc::new(AtomicUsize::new(0));
                let level = Arc::new(AtomicU32::new(0));
                let last_packet_ns = Arc::new(AtomicU64::new(0));
                let callback = Callback {
                    source: tracks.len(),
                    sample_rate: config.sample_rate().0,
                    channels: config.channels() as usize,
                    sender: sender.clone(),
                    queued: queued.clone(),
                    level: level.clone(),
                    last_packet_ns: last_packet_ns.clone(),
                    clock: clock.clone(),
                    fault: fault.clone(),
                };
                let stream = build_stream(&device, &config, callback)?;
                tracks.push(Track {
                    source: kind.into(),
                    sample_rate: config.sample_rate().0,
                    pending: Vec::new(),
                    start_ms: 0,
                    sequence: 0,
                    queued,
                    level,
                    last_packet_ns,
                    pending_since: None,
                });
                streams.push(stream);
            }
        }
        let snapshot = CaptureSnapshot {
            meeting_id: Some(request.meeting_id),
            state: CaptureState::Preparing,
            requires_receipts: true,
            ..Default::default()
        };
        write_snapshot(&folder, &snapshot)?;
        let mut session = Self {
            folder,
            streams,
            tracks,
            receiver,
            clock,
            fault,
            snapshot,
            paused_at: None,
            checked_space_at: Instant::now(),
            manifest_saved_ms: 0,
            _folder_lock: folder_lock,
            _recording_lock: recording_lock,
        };
        // Hardware opening time is not part of the captured timeline.
        session
            .clock
            .paused_ns
            .store(session.clock.elapsed_ns(), Ordering::Release);
        session.clock.active.store(true, Ordering::Release);
        for stream in &session.streams {
            if let Err(error) = stream.play() {
                session.interrupt(&format!("Could not start audio: {error}"));
                return Err(device_error(error));
            }
        }
        session.snapshot.state = CaptureState::Recording;
        if let Err(error) = write_snapshot(&session.folder, &session.snapshot) {
            session.interrupt(&error.message);
            return Err(error);
        }
        Ok(session)
    }
    fn tick(&mut self) -> AppResult<()> {
        if self.checked_space_at.elapsed() >= Duration::from_secs(1) {
            self.checked_space_at = Instant::now();
            if let Some(bytes) = available_disk_bytes(&self.folder)? {
                self.snapshot.error = disk_notice(bytes)?.map(String::from);
            }
        }
        self.drain()?;
        // Flush even a short packet after 900 ms; the 20 ms owner poll leaves
        // scheduling headroom below one second when the filesystem is responsive.
        for index in 0..self.tracks.len() {
            if self.tracks[index]
                .pending_since
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(900))
            {
                self.flush_track(index, true)?;
            }
        }
        match self.fault.load(Ordering::Acquire) {
            FAULT_OVERFLOW => {
                return Err(AppError::new(
                    "capture_overflow",
                    "Audio arrived faster than it could be saved. Recording stopped; saved audio can be recovered",
                ));
            }
            FAULT_DEVICE => {
                return Err(AppError::new(
                    "device_unavailable",
                    "An audio device disconnected. Recording stopped; saved audio can be recovered",
                ));
            }
            _ => {}
        }
        if self.snapshot.state == CaptureState::Recording {
            self.snapshot.elapsed_ms = self.clock.elapsed_ns() / 1_000_000;
            for track in &self.tracks {
                let value = if self
                    .clock
                    .elapsed_ns()
                    .saturating_sub(track.last_packet_ns.load(Ordering::Relaxed))
                    > 250_000_000
                {
                    0.0
                } else {
                    f32::from_bits(track.level.load(Ordering::Relaxed))
                };
                if track.source == "microphone" {
                    self.snapshot.microphone_level = value;
                } else {
                    self.snapshot.system_level = value;
                }
            }
        }
        Ok(())
    }
    fn drain(&mut self) -> AppResult<()> {
        while let Ok(packet) = self.receiver.try_recv() {
            let track = &mut self.tracks[packet.source];
            track
                .queued
                .fetch_sub(packet.samples.len(), Ordering::AcqRel);
            let expected =
                track.start_ms + track.pending.len() as u64 * 1000 / track.sample_rate as u64;
            let has_audio = track.sequence > 0 || !track.pending.is_empty();
            if has_audio && packet.start_ms > expected.saturating_add(40) {
                let source = track.source.clone();
                self.flush_track(packet.source, true)?;
                durability::record_gap(&mut self.snapshot, &source, expected, packet.start_ms);
            }
            let track = &mut self.tracks[packet.source];
            if track.pending.is_empty() {
                track.start_ms = track.start_ms.max(packet.start_ms);
                track.pending_since = Some(Instant::now());
            }
            track.pending.extend(packet.samples);
            self.flush_track(packet.source, false)?;
        }
        Ok(())
    }
    fn flush_track(&mut self, index: usize, all: bool) -> AppResult<()> {
        loop {
            let track = &mut self.tracks[index];
            let limit = track.sample_rate as usize * CHECKPOINT_SECONDS;
            if track.pending.is_empty() || (!all && track.pending.len() < limit) {
                return Ok(());
            }
            let count = track.pending.len().min(limit);
            let chunk = write_chunk(
                &self.folder,
                &track.source,
                track.sequence,
                track.start_ms,
                track.sample_rate,
                &track.pending[..count],
            )?;
            // Publish metadata only after the WAV is closed, synced, and renamed.
            self.snapshot.saved_ms = self.snapshot.saved_ms.max(chunk.end_ms);
            self.snapshot.elapsed_ms = self.snapshot.elapsed_ms.max(chunk.end_ms);
            self.snapshot.chunks.push(chunk.clone());
            track.sequence += 1;
            track.pending.drain(..count);
            track.start_ms = chunk.end_ms;
            track.pending_since = (!track.pending.is_empty()).then(Instant::now);
            // Immutable WAV+receipt pairs commit every second; the full index is
            // amortized so a long meeting does not rewrite all metadata every second.
            if self
                .snapshot
                .saved_ms
                .saturating_sub(self.manifest_saved_ms)
                >= 10_000
            {
                durability::crash_boundary("before_manifest");
                write_snapshot(&self.folder, &self.snapshot)?;
                self.manifest_saved_ms = self.snapshot.saved_ms;
            }
        }
    }
    fn halt_callbacks(&mut self) {
        self.clock.active.store(false, Ordering::Release);
        for stream in &self.streams {
            let _ = stream.pause();
        }
        // A callback already converting a bounded packet must finish before we drain.
        while self.clock.in_flight.load(Ordering::Acquire) != 0 {
            std::thread::yield_now();
        }
    }
    fn flush_all(&mut self) -> AppResult<()> {
        let mut failure = None;
        // Each failed drain has consumed a packet. Continue through the bounded
        // queue so a failed device/disk block cannot hide another source's tail.
        while let Err(error) = self.drain() {
            if failure.is_none() {
                failure = Some(error);
            }
        }
        for index in 0..self.tracks.len() {
            if let Err(error) = self.flush_track(index, true) {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn pause(&mut self) -> AppResult<CaptureSnapshot> {
        if self.snapshot.state == CaptureState::Paused {
            return Ok(self.snapshot.clone());
        }
        if self.snapshot.state != CaptureState::Recording {
            return Err(no_recording());
        }
        self.halt_callbacks();
        self.snapshot.elapsed_ms = self.clock.elapsed_ns() / 1_000_000;
        self.paused_at = Some(Instant::now());
        if let Err(error) = self.flush_all() {
            self.interrupt(&error.message);
            return Err(error);
        }
        self.snapshot.state = CaptureState::Paused;
        self.snapshot.microphone_level = 0.0;
        self.snapshot.system_level = 0.0;
        for track in &self.tracks {
            track.level.store(0, Ordering::Relaxed);
        }
        if let Err(error) = write_snapshot(&self.folder, &self.snapshot) {
            self.interrupt(&error.message);
            return Err(error);
        }
        Ok(self.snapshot.clone())
    }
    fn resume(&mut self) -> AppResult<CaptureSnapshot> {
        if self.snapshot.state == CaptureState::Recording {
            return Ok(self.snapshot.clone());
        }
        if self.snapshot.state != CaptureState::Paused {
            return Err(no_recording());
        }
        if let Some(since) = self.paused_at.take() {
            self.clock.paused_ns.fetch_add(
                since.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Ordering::AcqRel,
            );
        }
        self.clock.active.store(true, Ordering::Release);
        for stream in &self.streams {
            if let Err(error) = stream.play() {
                self.interrupt(&error.to_string());
                return Err(device_error(error));
            }
        }
        self.snapshot.state = CaptureState::Recording;
        if let Err(error) = write_snapshot(&self.folder, &self.snapshot) {
            self.interrupt(&error.message);
            return Err(error);
        }
        Ok(self.snapshot.clone())
    }
    fn stop(&mut self) -> AppResult<CaptureSnapshot> {
        self.halt_callbacks();
        if self.paused_at.is_none() {
            self.snapshot.elapsed_ms = self.clock.elapsed_ns() / 1_000_000;
        }
        self.snapshot.state = CaptureState::Saving;
        if let Err(error) = self.flush_all() {
            let snapshot = self.interrupt(&error.message);
            // Caller gets the error; owner publishes durable state on the following read.
            return Err(AppError::new(
                "storage",
                snapshot.error.unwrap_or(error.message),
            ));
        }
        if self.fault.load(Ordering::Acquire) != 0 {
            return Ok(self.interrupt("Recording was interrupted by an audio device or queue error; saved audio was preserved"));
        }
        self.snapshot.state = CaptureState::Saved;
        self.snapshot.elapsed_ms = self.snapshot.elapsed_ms.max(self.snapshot.saved_ms);
        self.snapshot.microphone_level = 0.0;
        self.snapshot.system_level = 0.0;
        if let Err(error) = write_snapshot(&self.folder, &self.snapshot) {
            self.interrupt(&error.message);
            return Err(error);
        }
        Ok(self.snapshot.clone())
    }
    fn interrupt(&mut self, message: &str) -> CaptureSnapshot {
        self.halt_callbacks();
        let _ = self.flush_all();
        self.snapshot.state = CaptureState::Interrupted;
        self.snapshot.incomplete = true;
        if !self
            .snapshot
            .warnings
            .iter()
            .any(|warning| warning == message)
        {
            self.snapshot.warnings.push(message.to_string());
        }
        self.snapshot.error = Some(message.to_string());
        self.snapshot.elapsed_ms = self.snapshot.elapsed_ms.max(self.snapshot.saved_ms);
        self.snapshot.microphone_level = 0.0;
        self.snapshot.system_level = 0.0;
        let _ = write_snapshot(&self.folder, &self.snapshot);
        self.snapshot.clone()
    }
}

struct Callback {
    source: usize,
    sample_rate: u32,
    channels: usize,
    sender: mpsc::SyncSender<Packet>,
    queued: Arc<AtomicUsize>,
    level: Arc<AtomicU32>,
    last_packet_ns: Arc<AtomicU64>,
    clock: Arc<Clock>,
    fault: Arc<AtomicU32>,
}
struct Flight<'a>(&'a AtomicUsize);
impl Drop for Flight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}
impl Callback {
    fn push<T: Copy>(
        &self,
        data: &[T],
        info: &cpal::InputCallbackInfo,
        convert: impl Fn(T) -> f32,
    ) {
        self.clock.in_flight.fetch_add(1, Ordering::AcqRel);
        let _flight = Flight(&self.clock.in_flight);
        if !self.clock.active.load(Ordering::Acquire) || self.fault.load(Ordering::Acquire) != 0 {
            return;
        }
        let stamp = info.timestamp();
        let delay = stamp
            .callback
            .duration_since(&stamp.capture)
            .unwrap_or_default();
        let start_ms = self
            .clock
            .elapsed_ns()
            .saturating_sub(delay.as_nanos().min(u64::MAX as u128) as u64)
            / 1_000_000;
        let count = data.len() / self.channels;
        if count == 0 {
            return;
        }
        if count > self.sample_rate as usize
            || self
                .queued
                .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current + count <= self.sample_rate as usize).then_some(current + count)
                })
                .is_err()
        {
            self.fault.store(FAULT_OVERFLOW, Ordering::Release);
            return;
        }
        let mut peak = 0.0_f32;
        let samples = data
            .chunks_exact(self.channels)
            .map(|frame| {
                let v = frame.iter().map(|s| convert(*s)).sum::<f32>() / self.channels as f32;
                let v = if v.is_finite() {
                    v.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                peak = peak.max(v.abs());
                v
            })
            .collect::<Vec<_>>();
        self.level.store(peak.to_bits(), Ordering::Relaxed);
        self.last_packet_ns
            .store(self.clock.elapsed_ns(), Ordering::Relaxed);
        if self
            .sender
            .try_send(Packet {
                source: self.source,
                start_ms,
                samples,
            })
            .is_err()
        {
            self.queued.fetch_sub(count, Ordering::AcqRel);
            self.fault.store(FAULT_OVERFLOW, Ordering::Release);
        }
    }
}
fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    callback: Callback,
) -> AppResult<cpal::Stream> {
    let fault = callback.fault.clone();
    let error = move |_: cpal::StreamError| {
        fault.store(FAULT_DEVICE, Ordering::Release);
    };
    let stream_config = config.config();
    match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], info| callback.push(data, info, |v| v),
            error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], info| callback.push(data, info, |v| v as f32 / 32768.0),
            error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], info| callback.push(data, info, |v| (v as f32 - 32768.0) / 32768.0),
            error,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            &stream_config,
            move |data: &[i32], info| callback.push(data, info, |v| v as f32 / 2147483648.0),
            error,
            None,
        ),
        other => {
            return Err(AppError::new(
                "unsupported_audio",
                format!("Unsupported audio sample format: {other:?}"),
            ));
        }
    }
    .map_err(device_error)
}

fn acquire_lock(path: &Path) -> AppResult<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|_| AppError::new("recording_busy", "Another process owns this recording"))?;
    Ok(file)
}

fn disk_notice(bytes: u64) -> AppResult<Option<&'static str>> {
    if bytes < 256 * 1024 * 1024 {
        Err(AppError::new(
            "storage_full",
            "Disk space is below 256 MiB. Recording stopped; free space before continuing",
        ))
    } else if bytes < 1024 * 1024 * 1024 {
        Ok(Some(
            "Less than 1 GiB of disk space remains. Free space before a long recording",
        ))
    } else {
        Ok(None)
    }
}

#[cfg(target_os = "windows")]
fn available_disk_bytes(folder: &Path) -> AppResult<Option<u64>> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let name = crate::infrastructure::win_path::verbatim_directory(folder)?;
    let mut available = 0_u64;
    // The UTF-16 string is null terminated; the available output lives for the call.
    let result = unsafe {
        GetDiskFreeSpaceExW(
            name.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(Some(available))
    }
}

#[cfg(not(target_os = "windows"))]
fn available_disk_bytes(_folder: &Path) -> AppResult<Option<u64>> {
    Ok(None)
}
fn wav_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        "unsupported_audio",
        format!("Could not read or write WAV audio: {error}"),
    )
}

/// Only regular files contained in the meeting folder can be opened through chunk metadata.
pub fn chunk_path(folder: &Path, relative: &str) -> AppResult<PathBuf> {
    let requested = Path::new(relative);
    if relative.is_empty()
        || requested
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(AppError::validation("Invalid meeting audio path"));
    }
    let base = folder.canonicalize()?;
    let original = base.join(requested);
    if !std::fs::symlink_metadata(&original)?.file_type().is_file() {
        return Err(AppError::validation("Audio chunk must be a regular file"));
    }
    let resolved = original.canonicalize()?;
    if !resolved.starts_with(&base) || !resolved.is_file() {
        return Err(AppError::validation("Audio file is outside this meeting"));
    }
    Ok(resolved)
}

/// Mixes a bounded timeline range. Uses a windowed-sinc low-pass kernel for downsampling.
/// Fixed per-source headroom avoids gain pumping when one speaker stops talking.
pub fn mix_audio(
    folder: &Path,
    chunks: &[AudioChunk],
    start_ms: u64,
    end_ms: u64,
    target_rate: u32,
) -> AppResult<Vec<f32>> {
    if end_ms <= start_ms
        || end_ms - start_ms > MAX_MIX_MS
        || !(8_000..=96_000).contains(&target_rate)
    {
        return Err(AppError::validation(
            "Audio requests must be 0–120 seconds at 8–96 kHz",
        ));
    }
    let frames = ((end_ms - start_ms) * target_rate as u64 / 1000) as usize;
    let mut output = vec![0.0_f32; frames];
    let source_count = chunks
        .iter()
        .map(|c| &c.source)
        .collect::<HashSet<_>>()
        .len()
        .max(1) as f32;
    for chunk in chunks
        .iter()
        .filter(|c| c.start_ms < end_ms && c.end_ms > start_ms)
    {
        validate_chunk(folder, chunk)?;
        let path = chunk_path(folder, &chunk.path)?;
        let mut reader = hound::WavReader::open(path).map_err(wav_error)?;
        let spec = reader.spec();
        if spec.sample_rate == 0
            || spec.channels == 0
            || spec.channels > 32
            || spec.sample_rate > 192_000
            || spec.sample_rate != chunk.sample_rate
            || spec.channels != chunk.channels
        {
            return Err(AppError::new(
                "unsupported_audio",
                "Invalid audio chunk format",
            ));
        }
        let from_ms = start_ms.max(chunk.start_ms);
        let to_ms = end_ms.min(chunk.end_ms);
        let radius = 12_i64;
        let first_frame = (((from_ms - chunk.start_ms) as f64 * spec.sample_rate as f64 / 1000.0)
            .floor() as i64
            - radius)
            .max(0) as u32;
        let last_frame = ((((to_ms - chunk.start_ms) as f64 * spec.sample_rate as f64 / 1000.0)
            .ceil() as u64)
            + radius as u64
            + 1)
        .min(reader.duration() as u64) as u32;
        if first_frame >= last_frame {
            continue;
        }
        reader.seek(first_frame).map_err(wav_error)?;
        let sample_count = (last_frame - first_frame) as usize * spec.channels as usize;
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .take(sample_count)
                .collect::<Result<Vec<_>, _>>()
                .map_err(wav_error)?,
            hound::SampleFormat::Int => {
                let scale = 2.0_f32.powi(spec.bits_per_sample as i32 - 1);
                reader
                    .samples::<i32>()
                    .take(sample_count)
                    .map(|s| s.map(|x| x as f32 / scale))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(wav_error)?
            }
        };
        if samples.len() != sample_count {
            return Err(AppError::storage("An audio block is incomplete"));
        }
        let mono: Vec<f32> = samples
            .chunks_exact(spec.channels as usize)
            .map(|s| s.iter().copied().sum::<f32>() / spec.channels as f32)
            .collect();
        let output_from = (((from_ms - start_ms) * target_rate as u64).div_ceil(1000)) as usize;
        let output_to =
            (((to_ms - start_ms) * target_rate as u64).div_ceil(1000)).min(frames as u64) as usize;
        let cutoff = (target_rate as f64 / spec.sample_rate as f64).min(1.0) * 0.92;
        for (index, value) in output
            .iter_mut()
            .enumerate()
            .take(output_to)
            .skip(output_from)
        {
            let at_ms = start_ms as f64 + index as f64 * 1000.0 / target_rate as f64;
            let position = (at_ms - chunk.start_ms as f64) * spec.sample_rate as f64 / 1000.0
                - first_frame as f64;
            let center = position.floor() as i64;
            let mut sum = 0.0_f64;
            let mut weight_sum = 0.0_f64;
            for tap in (center - radius + 1)..=(center + radius) {
                if tap < 0 || tap >= mono.len() as i64 {
                    continue;
                }
                let distance = position - tap as f64;
                if distance.abs() >= radius as f64 {
                    continue;
                }
                let x = std::f64::consts::PI * cutoff * distance;
                let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * distance / radius as f64).cos();
                let weight = cutoff * sinc * window;
                let sample = mono[tap as usize];
                sum += if sample.is_finite() {
                    sample as f64 * weight
                } else {
                    0.0
                };
                weight_sum += weight;
            }
            if weight_sum.abs() > 1e-12 {
                *value += (sum / weight_sum) as f32 / source_count;
            }
        }
    }
    for value in &mut output {
        *value = value.clamp(-1.0, 1.0);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let base = std::env::var_os("QAMILLE_TEST_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = base
                .join(format!(
                    "qamille-audio-{}-{}",
                    std::process::id(),
                    crate::model::now_ts()
                ))
                .join(format!("{:?}", std::thread::current().id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn synthetic_session(temp: &Temp) -> (Session, mpsc::SyncSender<Packet>) {
        let (sender, receiver) = mpsc::sync_channel(8);
        let clock = Arc::new(Clock {
            started: Instant::now(),
            paused_ns: AtomicU64::new(0),
            active: AtomicBool::new(true),
            in_flight: AtomicUsize::new(0),
        });
        let snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Recording,
            ..Default::default()
        };
        write_snapshot(&temp.0, &snapshot).unwrap();
        let session = Session {
            folder: temp.0.clone(),
            streams: vec![],
            receiver,
            clock,
            fault: Arc::new(AtomicU32::new(0)),
            snapshot,
            paused_at: None,
            checked_space_at: Instant::now(),
            manifest_saved_ms: 0,
            tracks: vec![Track {
                source: "microphone".into(),
                sample_rate: 8000,
                pending: vec![],
                start_ms: 0,
                sequence: 0,
                queued: Arc::new(AtomicUsize::new(0)),
                level: Arc::new(AtomicU32::new(0)),
                last_packet_ns: Arc::new(AtomicU64::new(0)),
                pending_since: None,
            }],
            _folder_lock: acquire_lock(&temp.0.join(".capture.lock")).unwrap(),
            _recording_lock: acquire_lock(&temp.0.join(".recording-test.lock")).unwrap(),
        };
        (session, sender)
    }
    #[test]
    fn publishing_progress_preserves_confirmed_history_and_resets_for_a_new_session() {
        let shared = Mutex::new(CaptureSnapshot::default());
        let mut snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Recording,
            ..Default::default()
        };
        for second in 0..100_u64 {
            snapshot.chunks.push(AudioChunk {
                path: format!("{second}.wav"),
                start_ms: second * 1000,
                end_ms: (second + 1) * 1000,
                ..Default::default()
            });
            snapshot.saved_ms = (second + 1) * 1000;
            snapshot.microphone_level = 0.75;
            publish(&shared, &snapshot);
        }
        snapshot.state = CaptureState::Interrupted;
        snapshot.incomplete = true;
        snapshot.error = Some("Synthetic interruption".into());
        snapshot.warnings.push("Audio preserved".into());
        snapshot.missing_chunks.push(AudioChunk::default());
        publish(&shared, &snapshot);
        assert_eq!(
            serde_json::to_value(&*shared.lock().unwrap()).unwrap(),
            serde_json::to_value(&snapshot).unwrap()
        );
        let fresh = CaptureSnapshot {
            meeting_id: Some(8),
            state: CaptureState::Preparing,
            ..Default::default()
        };
        publish(&shared, &fresh);
        assert_eq!(
            serde_json::to_value(&*shared.lock().unwrap()).unwrap(),
            serde_json::to_value(&fresh).unwrap()
        );
        // A restarted capture may reuse the ID but starts a shorter history.
        publish(&shared, &snapshot);
        snapshot.chunks.clear();
        publish(&shared, &snapshot);
        assert!(shared.lock().unwrap().chunks.is_empty());
    }
    #[test]
    #[ignore = "explicit two-hour synthetic durability/load test; writes about 350 MB"]
    fn two_hour_two_source_recording_recovers_every_confirmed_frame() {
        let temp = Temp::new();
        let started = Instant::now();
        let (mut session, sender) = synthetic_session(&temp);
        session.tracks.push(Track {
            source: "system".into(),
            sample_rate: 16000,
            pending: vec![],
            start_ms: 0,
            sequence: 0,
            queued: Arc::new(AtomicUsize::new(0)),
            level: Arc::new(AtomicU32::new(0)),
            last_packet_ns: Arc::new(AtomicU64::new(0)),
            pending_since: None,
        });
        for second in 0..7200_u64 {
            for (source, rate, sample) in [(0, 8000, 0.25), (1, 16000, -0.5)] {
                session.tracks[source].queued.store(rate, Ordering::Release);
                sender
                    .send(Packet {
                        source,
                        start_ms: second * 1000,
                        samples: vec![sample; rate],
                    })
                    .unwrap();
            }
            session.drain().unwrap();
            assert!(
                session
                    .tracks
                    .iter()
                    .all(|t| t.pending.is_empty() && t.queued.load(Ordering::Acquire) == 0)
            );
            if second % 1800 == 1799 {
                eprintln!(
                    "synthetic checkpoint: {} seconds, {} chunks",
                    second + 1,
                    session.snapshot.chunks.len()
                );
            }
        }
        assert_eq!(session.snapshot.chunks.len(), 14400);
        assert_eq!(session.snapshot.saved_ms, 7_200_000);
        let write_seconds = started.elapsed().as_secs_f64();
        // Drop without stop, then lose both indexes: reconstruct from WAVs + receipts.
        drop(session);
        for name in ["capture.json", "capture.json.prev", "capture.json.tmp"] {
            if temp.0.join(name).is_file() {
                std::fs::write(temp.0.join(name), b"broken-index").unwrap();
            }
        }
        let recovery_started = Instant::now();
        let recovered = recover_capture(&temp.0, 7).unwrap();
        assert_eq!(recovered.chunks.len(), 14400);
        assert!(recovered.missing_chunks.is_empty());
        assert_eq!(recovered.saved_ms, 7_200_000);
        let mut frames = [0_u64; 2];
        let mut bytes = 0_u64;
        for chunk in &recovered.chunks {
            let source = usize::from(chunk.source == "system");
            let reader = hound::WavReader::open(temp.0.join(&chunk.path)).unwrap();
            assert_eq!(reader.duration(), chunk.sample_rate);
            frames[source] += reader.duration() as u64;
            bytes += std::fs::metadata(temp.0.join(&chunk.path)).unwrap().len();
        }
        assert_eq!(frames, [57_600_000, 115_200_000]);
        for source in ["microphone", "system"] {
            let mut chunks: Vec<_> = recovered
                .chunks
                .iter()
                .filter(|c| c.source == source)
                .collect();
            chunks.sort_by_key(|c| c.start_ms);
            assert_eq!(chunks.first().unwrap().start_ms, 0);
            assert_eq!(chunks.last().unwrap().end_ms, 7_200_000);
            assert!(
                chunks
                    .windows(2)
                    .all(|pair| pair[0].end_ms == pair[1].start_ms)
            );
            for index in [0, 3599, 7199] {
                let mut wav = hound::WavReader::open(temp.0.join(&chunks[index].path)).unwrap();
                let expected = if source == "system" { -16383 } else { 8191 };
                assert!(
                    wav.samples::<i16>()
                        .all(|v| (i32::from(v.unwrap()) - expected).abs() <= 1)
                );
            }
        }
        let again = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(again.chunks.len(), 14400);
        eprintln!(
            "ENDURANCE_RESULT {}",
            serde_json::json!({
                "duration_seconds":7200,"sources":2,"chunks":14400,"frames":frames,
                "audio_bytes":bytes,"write_seconds":write_seconds,
                "recovery_and_verification_seconds":recovery_started.elapsed().as_secs_f64(),
                "devices_opened":false,"accelerated":true
            })
        );
    }
    #[test]
    fn writer_checkpoints_while_running_and_flushes_tail_on_stop() {
        let temp = Temp::new();
        let (mut session, sender) = synthetic_session(&temp);
        session.tracks[0].queued.store(52_000, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 0,
                samples: vec![0.25; 52_000],
            })
            .unwrap();
        session.drain().unwrap();
        assert_eq!(session.snapshot.chunks.len(), 6);
        assert_eq!(session.snapshot.saved_ms, 6000);
        let snapshot = session.stop().unwrap();
        assert_eq!(snapshot.state, CaptureState::Saved);
        assert_eq!(snapshot.chunks.len(), 7);
        assert_eq!(snapshot.chunks[6].start_ms, 6000);
        assert_eq!(snapshot.chunks[6].end_ms, 6500);
        assert_eq!(snapshot.saved_ms, 6500);
        assert!(
            snapshot
                .chunks
                .iter()
                .all(|chunk| chunk.end_ms - chunk.start_ms <= 1000)
        );
        assert!(session.tracks[0].pending.is_empty());
    }
    #[test]
    fn capture_detects_internal_gaps_with_pending_or_already_flushed_audio() {
        for first_frames in [800, 8000] {
            let temp = Temp::new();
            let (mut session, sender) = synthetic_session(&temp);
            let end = first_frames as u64 * 1000 / 8000;
            for (start_ms, frames) in [(0, first_frames), (end + 100, 800)] {
                session.tracks[0].queued.store(frames, Ordering::Release);
                sender
                    .send(Packet {
                        source: 0,
                        start_ms,
                        samples: vec![0.25; frames],
                    })
                    .unwrap();
                session.drain().unwrap();
            }
            let saved = session.stop().unwrap();
            assert!(saved.incomplete);
            assert_eq!(saved.gaps.len(), 1);
            assert_eq!(
                (saved.gaps[0].start_ms, saved.gaps[0].end_ms),
                (end, end + 100)
            );
            assert_eq!(saved.chunks.len(), 2);
            drop(session);
            assert_eq!(load_capture(&temp.0).unwrap().unwrap().gaps.len(), 1);
        }
    }
    #[test]
    fn capture_does_not_treat_device_start_offset_or_pause_as_missing_audio() {
        let temp = Temp::new();
        let (mut session, sender) = synthetic_session(&temp);
        session.tracks[0].queued.store(800, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 500,
                samples: vec![0.25; 800],
            })
            .unwrap();
        session.drain().unwrap();
        session.pause().unwrap();
        session.resume().unwrap();
        session.tracks[0].queued.store(800, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 600,
                samples: vec![0.25; 800],
            })
            .unwrap();
        let mut saved = session.stop().unwrap();
        // A second device starts at a different offset. Its timeline must be
        // checked independently, not against the microphone's first timestamp.
        saved
            .chunks
            .push(write_chunk(&temp.0, "system", 0, 200, 8000, &[0.25; 1600]).unwrap());
        write_snapshot(&temp.0, &saved).unwrap();
        assert!(saved.gaps.is_empty());
        assert!(!saved.incomplete);
        drop(session);
        assert!(load_capture(&temp.0).unwrap().unwrap().gaps.is_empty());
    }
    #[test]
    fn detected_capture_fault_cannot_be_reported_as_successful_stop() {
        let temp = Temp::new();
        let (mut session, sender) = synthetic_session(&temp);
        session.tracks[0].queued.store(800, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 0,
                samples: vec![0.25; 800],
            })
            .unwrap();
        session.fault.store(FAULT_DEVICE, Ordering::Release);
        let snapshot = session.stop().unwrap();
        assert_eq!(snapshot.state, CaptureState::Interrupted);
        assert_eq!(snapshot.chunks.len(), 1);
        assert!(snapshot.error.is_some());
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn checkpoint_survives_manifest_commit_failure_as_recoverable_orphan() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = Temp::new();
        let (mut session, sender) = synthetic_session(&temp);
        // Deny replacement of only the manifest to simulate the commit boundary failing.
        let denied = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(temp.0.join("capture.json"))
            .unwrap();
        session.tracks[0].queued.store(80_000, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 0,
                samples: vec![0.25; 80_000],
            })
            .unwrap();
        assert!(session.drain().is_err());
        assert_eq!(session.snapshot.chunks.len(), 10);
        assert!(session.tracks[0].pending.is_empty());
        drop(denied);
        drop(session);
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.state, CaptureState::Interrupted);
        assert_eq!(recovered.chunks.len(), 10);
        assert_eq!(recovered.saved_ms, 10_000);
    }
    #[test]
    fn two_sources_have_headroom_and_gaps_are_silent() {
        let temp = Temp::new();
        let a = write_chunk(&temp.0, "microphone", 0, 100, 8000, &vec![0.6; 800]).unwrap();
        let b = write_chunk(&temp.0, "system", 0, 100, 16000, &vec![0.2; 1600]).unwrap();
        let mixed = mix_audio(&temp.0, &[a, b], 0, 300, 8000).unwrap();
        assert_eq!(mixed.len(), 2400);
        assert!(mixed[..800].iter().all(|x| *x == 0.0));
        assert!((mixed[1200] - 0.4).abs() < 0.001);
        assert!(mixed[1600..].iter().all(|x| *x == 0.0));
    }
    #[test]
    fn recovers_confirmed_orphan_and_marks_crashed_session() {
        let temp = Temp::new();
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                meeting_id: Some(7),
                state: CaptureState::Recording,
                ..Default::default()
            },
        )
        .unwrap();
        let block = write_chunk(&temp.0, "microphone", 0, 0, 8000, &vec![0.1; 800]).unwrap();
        std::fs::write(temp.0.join("unconfirmed.wav.tmp"), b"incomplete").unwrap();
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.state, CaptureState::Interrupted);
        assert_eq!(recovered.chunks.len(), 1);
        assert_eq!(recovered.chunks[0].path, block.path);
        assert_eq!(recovered.saved_ms, 100);
        assert_eq!(load_capture(&temp.0).unwrap().unwrap().chunks.len(), 1);
    }
    #[test]
    fn bulk_recovery_keeps_every_block_without_a_warning_for_every_filename() {
        let temp = Temp::new();
        let chunks: Vec<_> = (0..512)
            .map(|sequence| {
                write_chunk(
                    &temp.0,
                    "microphone",
                    sequence,
                    sequence * 10,
                    8000,
                    &[0.25; 80],
                )
                .unwrap()
            })
            .collect();
        let mut snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Recording,
            chunks: chunks[..2].to_vec(),
            ..Default::default()
        };
        for chunk in &snapshot.chunks {
            snapshot.warnings.push(format!(
                "Recovered audio block {} not yet present in the manifest",
                chunk.path
            ));
        }
        snapshot.error = Some(snapshot.warnings.join("; "));
        write_snapshot(&temp.0, &snapshot).unwrap();
        let recovered = recover_capture(&temp.0, 7).unwrap();
        assert_eq!(recovered.chunks.len(), 512);
        assert!(recovered.missing_chunks.is_empty());
        assert!(recovered.warnings.len() <= 6);
        assert!(recovered.error.as_ref().unwrap().len() < 4096);
        assert!(
            recovered
                .warnings
                .iter()
                .any(|w| w == "Recovered 512 audio blocks omitted from the recording index")
        );
        let reopened = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(reopened.chunks.len(), 512);
        assert_eq!(reopened.state, CaptureState::Saved);
        assert!(reopened.warnings.len() <= 6);
    }
    #[test]
    fn active_capture_is_not_recovered_by_reader() {
        let temp = Temp::new();
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                state: CaptureState::Recording,
                ..Default::default()
            },
        )
        .unwrap();
        let _lock = acquire_lock(&temp.0.join(".capture.lock")).unwrap();
        assert_eq!(
            load_capture(&temp.0).unwrap().unwrap().state,
            CaptureState::Recording
        );
    }
    #[test]
    fn rejects_escape_and_unbounded_playback() {
        let temp = Temp::new();
        assert!(chunk_path(&temp.0, "../secret.wav").is_err());
        assert!(chunk_path(&temp.0, "C:\\secret.wav").is_err());
        assert!(mix_audio(&temp.0, &[], 0, 120001, 16000).is_err());
    }
    #[test]
    fn missing_checkpoint_is_visible_and_not_silently_removed() {
        let temp = Temp::new();
        let block = AudioChunk {
            path: "missing.wav".into(),
            sample_rate: 8000,
            channels: 1,
            end_ms: 100,
            ..Default::default()
        };
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                state: CaptureState::Saved,
                chunks: vec![block],
                ..Default::default()
            },
        )
        .unwrap();
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.state, CaptureState::Interrupted);
        assert!(recovered.chunks.is_empty());
        assert_eq!(recovered.missing_chunks.len(), 1);
        assert!(recovered.error.is_some());
    }
    #[test]
    fn low_pass_rejects_out_of_band_energy() {
        let temp = Temp::new();
        let tone: Vec<f32> = (0..4800)
            .map(|i| (2.0 * std::f32::consts::PI * 14000.0 * i as f32 / 48000.0).sin() * 0.5)
            .collect();
        let block = write_chunk(&temp.0, "microphone", 0, 0, 48000, &tone).unwrap();
        let mixed = mix_audio(&temp.0, &[block], 0, 100, 16000).unwrap();
        let rms = (mixed[100..1500].iter().map(|x| x * x).sum::<f32>() / 1400.0).sqrt();
        assert!(rms < 0.03, "out-of-band RMS {rms}");
    }
    #[test]
    fn disk_threshold_distinguishes_warning_from_stop() {
        assert_eq!(
            disk_notice(255 * 1024 * 1024).unwrap_err().code,
            "storage_full"
        );
        assert!(disk_notice(512 * 1024 * 1024).unwrap().is_some());
        assert!(disk_notice(2 * 1024 * 1024 * 1024).unwrap().is_none());
    }

    #[test]
    fn duplicate_checkpoint_cannot_replace_original_audio_or_receipt() {
        let temp = Temp::new();
        let chunk = write_chunk(&temp.0, "microphone", 0, 0, 8000, &[0.25; 800]).unwrap();
        let original = std::fs::read(temp.0.join(&chunk.path)).unwrap();
        let receipt = std::fs::read(temp.0.join(format!("{}.sha256.json", chunk.path))).unwrap();
        assert!(write_chunk(&temp.0, "microphone", 0, 0, 8000, &[-0.75; 800]).is_err());
        assert_eq!(std::fs::read(temp.0.join(&chunk.path)).unwrap(), original);
        assert_eq!(
            std::fs::read(temp.0.join(format!("{}.sha256.json", chunk.path))).unwrap(),
            receipt
        );
        // Exercise the publication primitive too: it cannot replace an existing name.
        let pending = temp.0.join("collision.tmp");
        std::fs::write(&pending, b"different bytes").unwrap();
        assert!(durability::test_publish_new(&pending, &temp.0.join(&chunk.path)).is_err());
        assert_eq!(std::fs::read(temp.0.join(&chunk.path)).unwrap(), original);
    }

    #[test]
    fn publication_works_beyond_the_classic_windows_path_limit() {
        // std::fs handles long paths itself; the raw rename must do the same.
        let temp = Temp::new();
        let deep = temp.0.join("a".repeat(120)).join("b".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let pending = deep.join("chunk.wav.tmp");
        let destination = deep.join("chunk.wav");
        assert!(destination.as_os_str().len() > 260);
        std::fs::write(&pending, b"complete").unwrap();
        durability::test_publish_new(&pending, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete");
        assert!(!pending.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn free_space_is_read_beyond_the_classic_windows_path_limit() {
        let temp = Temp::new();
        let deep = temp.0.join("a".repeat(120)).join("b".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        assert!(deep.as_os_str().len() > 260);
        assert!(available_disk_bytes(&deep).unwrap().is_some_and(|bytes| bytes > 0));
    }

    #[test]
    fn short_packet_is_committed_before_one_second_of_pending_time() {
        let temp = Temp::new();
        let (mut session, sender) = synthetic_session(&temp);
        session.tracks[0].queued.store(800, Ordering::Release);
        sender
            .send(Packet {
                source: 0,
                start_ms: 0,
                samples: vec![0.25; 800],
            })
            .unwrap();
        session.drain().unwrap();
        assert!(session.snapshot.chunks.is_empty());
        session.tracks[0].pending_since = Some(Instant::now() - Duration::from_millis(901));
        session.tick().unwrap();
        assert_eq!(session.snapshot.chunks.len(), 1);
        assert_eq!(session.snapshot.saved_ms, 100);
        validate_chunk(&temp.0, &session.snapshot.chunks[0]).unwrap();
    }

    #[test]
    fn a_failed_source_commit_still_flushes_the_other_sources_tail() {
        let temp = Temp::new();
        let (mut session, _sender) = synthetic_session(&temp);
        // A conflicting immutable filename prevents only microphone publication.
        write_chunk(&temp.0, "microphone", 0, 0, 8000, &[0.1; 800]).unwrap();
        session.tracks[0].pending = vec![0.2; 800];
        session.tracks.push(Track {
            source: "system".into(),
            sample_rate: 8000,
            pending: vec![0.3; 800],
            start_ms: 0,
            sequence: 0,
            queued: Arc::new(AtomicUsize::new(0)),
            level: Arc::new(AtomicU32::new(0)),
            last_packet_ns: Arc::new(AtomicU64::new(0)),
            pending_since: Some(Instant::now()),
        });
        assert!(session.flush_all().is_err());
        assert_eq!(session.snapshot.chunks.len(), 1);
        assert_eq!(session.snapshot.chunks[0].source, "system");
        validate_chunk(&temp.0, &session.snapshot.chunks[0]).unwrap();
        assert!(session.tracks[1].pending.is_empty());
        assert_eq!(session.tracks[0].pending.len(), 800);
    }

    #[test]
    fn corrupted_middle_block_does_not_prevent_healthy_audio_recovery() {
        use std::io::{Seek, SeekFrom};
        let temp = Temp::new();
        let chunks: Vec<_> = (0..3)
            .map(|sequence| {
                write_chunk(
                    &temp.0,
                    "microphone",
                    sequence,
                    sequence * 100,
                    8000,
                    &[0.25; 800],
                )
                .unwrap()
            })
            .collect();
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                meeting_id: Some(7),
                state: CaptureState::Saved,
                saved_ms: 300,
                elapsed_ms: 300,
                chunks: chunks.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut damaged = OpenOptions::new()
            .write(true)
            .open(temp.0.join(&chunks[1].path))
            .unwrap();
        damaged.seek(SeekFrom::Start(50)).unwrap();
        damaged.write_all(&[0x11, 0x22]).unwrap();
        damaged.sync_all().unwrap();
        drop(damaged);
        assert!(mix_audio(&temp.0, &chunks, 0, 300, 8000).is_err());
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.chunks.len(), 2);
        assert_eq!(recovered.missing_chunks.len(), 1);
        assert_eq!(recovered.missing_chunks[0].path, chunks[1].path);
        assert!(recovered.incomplete);
        let audio = mix_audio(&temp.0, &recovered.chunks, 0, 300, 8000).unwrap();
        assert!((audio[400] - 0.25).abs() < 0.001);
        assert!(audio[800..1600].iter().all(|sample| *sample == 0.0));
        assert!((audio[2000] - 0.25).abs() < 0.001);
        let saved = recover_capture(&temp.0, 7).unwrap();
        assert_eq!(saved.state, CaptureState::Saved);
        for _ in 0..2 {
            let reopened = load_capture(&temp.0).unwrap().unwrap();
            assert_eq!(reopened.state, CaptureState::Saved);
            assert_eq!(reopened.chunks.len(), 2);
            assert_eq!(reopened.missing_chunks.len(), 1);
            assert!(reopened.incomplete && !reopened.warnings.is_empty());
        }
        assert!(
            std::fs::read_dir(&temp.0)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{}.quarantine-", chunks[1].path)))
        );
    }

    #[test]
    fn damaged_manifest_offsets_do_not_quarantine_healthy_audio() {
        let temp = Temp::new();
        let original = write_chunk(&temp.0, "microphone", 0, 100, 8000, &[0.25; 800]).unwrap();
        let bytes = std::fs::read(temp.0.join(&original.path)).unwrap();
        let mut wrong = original.clone();
        wrong.start_ms = 99_000;
        wrong.sample_rate = 16_000;
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                meeting_id: Some(7),
                state: CaptureState::Saved,
                chunks: vec![wrong],
                ..Default::default()
            },
        )
        .unwrap();
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.chunks.len(), 1);
        assert_eq!(recovered.chunks[0].start_ms, 100);
        assert_eq!(recovered.chunks[0].sample_rate, 8000);
        assert!(recovered.missing_chunks.is_empty());
        assert_eq!(std::fs::read(temp.0.join(&original.path)).unwrap(), bytes);
        assert!(
            recovered
                .warnings
                .iter()
                .any(|message| message.contains("manifest metadata"))
        );
    }

    #[test]
    fn receipt_reports_a_lost_block_beyond_the_last_full_manifest() {
        let temp = Temp::new();
        write_snapshot(
            &temp.0,
            &CaptureSnapshot {
                meeting_id: Some(7),
                state: CaptureState::Recording,
                ..Default::default()
            },
        )
        .unwrap();
        let first = write_chunk(&temp.0, "microphone", 0, 0, 8000, &[0.25; 800]).unwrap();
        let lost = write_chunk(&temp.0, "microphone", 1, 100, 8000, &[0.25; 800]).unwrap();
        std::fs::remove_file(temp.0.join(&lost.path)).unwrap();
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.chunks.len(), 1);
        assert_eq!(recovered.chunks[0].path, first.path);
        assert_eq!(recovered.missing_chunks.len(), 1);
        assert_eq!(recovered.missing_chunks[0].path, lost.path);
        assert_eq!(recovered.missing_chunks[0].start_ms, 100);
        recover_capture(&temp.0, 7).unwrap();
        assert_eq!(
            load_capture(&temp.0).unwrap().unwrap().state,
            CaptureState::Saved
        );
    }

    #[test]
    fn missing_or_damaged_manifests_are_rebuilt_without_losing_audio() {
        let temp = Temp::new();
        let folder = temp.0.join("7");
        std::fs::create_dir(&folder).unwrap();
        let chunk = write_chunk(&folder, "microphone", 0, 0, 8000, &[0.25; 800]).unwrap();
        let recovered = load_capture(&folder).unwrap().unwrap();
        assert_eq!(recovered.meeting_id, Some(7));
        assert_eq!(recovered.chunks.len(), 1);
        assert!(recovered.incomplete);
        // Preserve a valid previous manifest, then simulate a damaged current one.
        write_snapshot(&folder, &recovered).unwrap();
        std::fs::write(folder.join("capture.json"), b"{ interrupted metadata").unwrap();
        let fallback = load_capture(&folder).unwrap().unwrap();
        assert_eq!(fallback.chunks[0].path, chunk.path);
        assert!(
            fallback
                .warnings
                .iter()
                .any(|message| message.contains("capture.json.prev"))
        );
        let preserved: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("capture.json.quarantine-")
            })
            .collect();
        assert_eq!(preserved.len(), 1);
        assert_eq!(
            std::fs::read(preserved[0].path()).unwrap(),
            b"{ interrupted metadata"
        );
    }

    #[test]
    fn synchronized_staging_manifest_is_used_if_main_and_previous_are_absent() {
        let temp = Temp::new();
        let chunk = write_chunk(&temp.0, "microphone", 0, 0, 8000, &[0.25; 800]).unwrap();
        let snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Recording,
            chunks: vec![chunk],
            ..Default::default()
        };
        std::fs::write(
            temp.0.join("capture.json.tmp"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        let recovered = load_capture(&temp.0).unwrap().unwrap();
        assert_eq!(recovered.chunks.len(), 1);
        assert_eq!(recovered.state, CaptureState::Interrupted);
        assert!(
            recovered
                .warnings
                .iter()
                .any(|message| message.contains("capture.json.tmp"))
        );
    }

    #[test]
    fn imported_audio_can_be_recovered_from_session_capture_metadata() {
        let temp = Temp::new();
        let folder = temp.0.join("7");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("import-000001.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..800 {
            wav.write_sample(1000_i16).unwrap();
        }
        wav.finalize().unwrap();
        let snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Saved,
            chunks: vec![AudioChunk {
                path: "import-000001.wav".into(),
                source: "import".into(),
                start_ms: 0,
                end_ms: 100,
                sample_rate: 8000,
                channels: 1,
                sequence: 0,
            }],
            ..Default::default()
        };
        std::fs::write(folder.join("session.json"), serde_json::to_vec(&serde_json::json!({"schema_version": 2, "meeting": {"id": 7}, "capture": snapshot})).unwrap()).unwrap();
        let recovered = load_capture(&folder).unwrap().unwrap();
        assert_eq!(recovered.chunks.len(), 1);
        assert_eq!(recovered.chunks[0].path, "import-000001.wav");
        assert!(folder.join("capture.json").exists());
    }

    #[test]
    fn audio_worker_panic_publishes_interruption_instead_of_stale_recording() {
        let shared = Arc::new(Mutex::new(CaptureSnapshot {
            state: CaptureState::Recording,
            ..Default::default()
        }));
        let worker_state = shared.clone();
        std::thread::spawn(move || {
            supervise_owner(worker_state, |_| panic!("synthetic worker failure"))
        })
        .join()
        .unwrap();
        let state = shared.lock().unwrap();
        assert_eq!(state.state, CaptureState::Interrupted);
        assert!(state.incomplete && state.error.is_some());
    }

    // Invoked only by the parent test below, with an owned folder and a selected
    // failpoint. This child never constructs CPAL or opens an audio device.
    #[test]
    fn crash_writer_child() {
        let Some(folder) = std::env::var_os("QAMILLE_AUDIO_CRASH_FOLDER") else {
            return;
        };
        let folder = PathBuf::from(folder);
        let _guard = acquire_lock(&folder.join(".capture.lock")).unwrap();
        let mut snapshot = CaptureSnapshot {
            meeting_id: Some(7),
            state: CaptureState::Recording,
            ..Default::default()
        };
        write_snapshot(&folder, &snapshot).unwrap();
        durability::TEST_CRASH_ARMED.store(true, Ordering::Release);
        let chunk = write_chunk(&folder, "microphone", 0, 0, 8000, &[0.25; 8000]).unwrap();
        snapshot.saved_ms = chunk.end_ms;
        snapshot.chunks.push(chunk);
        durability::crash_boundary("before_manifest");
        write_snapshot(&folder, &snapshot).unwrap();
        panic!("Selected crash boundary was not reached");
    }

    #[test]
    fn real_abort_and_kill_preserve_every_synced_complete_pcm_frame() {
        use std::process::{Command, Stdio};
        let temp = Temp::new();
        for mode in ["abort", "kill"] {
            for point in [
                "partial_data_synced",
                "data_synced",
                "before_publish",
                "after_publish",
                "before_manifest",
                "manifest_synced",
            ] {
                let case = temp.0.join(format!("{mode}-{point}"));
                let folder = case.join("7");
                std::fs::create_dir_all(&folder).unwrap();
                let marker = case.join("boundary-reached");
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args(["crash_writer_child", "--nocapture", "--test-threads=1"])
                    .env("QAMILLE_AUDIO_CRASH_FOLDER", &folder)
                    .env("QAMILLE_AUDIO_CRASH_POINT", point)
                    .env("QAMILLE_AUDIO_CRASH_MODE", mode)
                    .env("QAMILLE_AUDIO_CRASH_MARKER", &marker)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(15);
                while !marker.exists() && Instant::now() < deadline {
                    if child.try_wait().unwrap().is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !marker.exists() {
                    let _ = child.kill();
                    let status = child.wait().unwrap();
                    panic!(
                        "{mode}/{point}: child exited {status} before selected persistence boundary"
                    );
                }
                if mode == "kill" {
                    child.kill().unwrap();
                }
                let status = child.wait().unwrap();
                assert!(
                    !status.success(),
                    "child must really terminate at {mode}/{point}"
                );
                let recovered = load_capture(&folder).unwrap().unwrap();
                assert_eq!(recovered.chunks.len(), 1, "{mode}/{point}");
                let expected_frames = if point == "partial_data_synced" {
                    4000
                } else {
                    8000
                };
                let reader =
                    hound::WavReader::open(folder.join(&recovered.chunks[0].path)).unwrap();
                assert_eq!(
                    reader.duration(),
                    expected_frames,
                    "{mode}/{point}: no synced frame lost or invented"
                );
                assert!(
                    reader
                        .into_samples::<i16>()
                        .all(|sample| sample.unwrap() == 8191)
                );
                assert_eq!(recovered.saved_ms, expected_frames as u64 * 1000 / 8000);
                let saved = recover_capture(&folder, 7).unwrap();
                assert_eq!(saved.state, CaptureState::Saved);
                assert!(saved.incomplete && !saved.warnings.is_empty());
                for _ in 0..2 {
                    let reopened = load_capture(&folder).unwrap().unwrap();
                    assert_eq!(reopened.state, CaptureState::Saved, "{mode}/{point}");
                    assert_eq!(reopened.chunks.len(), 1);
                    assert!(reopened.incomplete);
                }
            }
        }
    }
}
