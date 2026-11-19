//! Disk commit and recovery. Only immutable, validated WAVs enter the playable manifest.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};

#[derive(Serialize, Deserialize)]
struct Receipt {
    schema: u32,
    chunk: AudioChunk,
    bytes: u64,
    sha256: String,
}

fn nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn sync_directory(folder: &Path) -> AppResult<()> {
    #[cfg(unix)]
    File::open(folder)?.sync_all()?;
    // Windows FlushFileBuffers is applied to each file before publication. Directory
    // handles cannot be flushed through std::fs; power-loss guarantees depend on NTFS/device.
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}

fn quarantine(path: &Path) -> AppResult<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| AppError::storage("Invalid recovery path"))?
        .to_string_lossy();
    let destination = path.with_file_name(format!("{name}.quarantine-{}", nonce()));
    // Keep a synchronized copy before removing the damaged active name. This also
    // works on exFAT and other filesystems without hard-link support.
    let mut original = File::open(path)?;
    let mut preserved = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    std::io::copy(&mut original, &mut preserved)?;
    preserved.sync_all()?;
    drop(original);
    drop(preserved);
    std::fs::remove_file(path)?;
    sync_directory(path.parent().unwrap())?;
    Ok(destination)
}

fn publish_new(temporary: &Path, destination: &Path) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    move_windows(temporary, destination, false)?;
    #[cfg(not(target_os = "windows"))]
    {
        if std::fs::hard_link(temporary, destination).is_err() {
            // create_new still never overwrites. If copying fails or the process
            // dies, the complete staging file survives and the receipt detects
            // the partial destination during recovery.
            let mut input = File::open(temporary)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
        }
    }
    sync_directory(destination.parent().unwrap())?;
    Ok(())
}

#[cfg(test)]
pub(super) fn test_publish_new(temporary: &Path, destination: &Path) -> AppResult<()> {
    publish_new(temporary, destination)
}

#[cfg(target_os = "windows")]
fn move_windows(source: &Path, destination: &Path, replace: bool) -> AppResult<()> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let mut source = crate::infrastructure::win_path::verbatim(source)?;
    source.push(0);
    let mut destination = crate::infrastructure::win_path::verbatim(destination)?;
    destination.push(0);
    // MOVEFILE_WRITE_THROUGH; REPLACE_EXISTING is deliberately absent for WAVs.
    // https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            8 | u32::from(replace),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn replace_manifest(source: &Path, destination: &Path) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    move_windows(source, destination, true)?;
    #[cfg(not(target_os = "windows"))]
    std::fs::rename(source, destination)?;
    Ok(())
}

fn remove_staging(path: &Path) -> AppResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn hash_file(path: &Path) -> AppResult<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}

fn receipt_path(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.sha256.json"))
}

fn write_receipt(folder: &Path, chunk: &AudioChunk, wav: &Path) -> AppResult<()> {
    let (bytes, sha256) = hash_file(wav)?;
    let receipt = Receipt {
        schema: 1,
        chunk: chunk.clone(),
        bytes,
        sha256,
    };
    let destination = receipt_path(folder, &chunk.path);
    let temporary = destination.with_file_name(format!(
        "{}.tmp-{}",
        destination.file_name().unwrap().to_string_lossy(),
        nonce()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, &receipt).map_err(|e| AppError::storage(e.to_string()))?;
    file.sync_all()?;
    drop(file);
    publish_new(&temporary, &destination)?;
    remove_staging(&temporary)?;
    Ok(())
}

fn check_receipt(folder: &Path, chunk: &AudioChunk, wav: &Path) -> AppResult<()> {
    let sidecar = receipt_path(folder, &chunk.path);
    if !sidecar.exists() {
        return Ok(());
    } // Compatibility with original/imported recordings.
    let path = chunk_path(folder, &format!("{}.sha256.json", chunk.path))?;
    let receipt: Receipt = serde_json::from_reader(File::open(path)?)
        .map_err(|_| AppError::storage("Audio integrity receipt is damaged"))?;
    let (bytes, digest) = hash_file(wav)?;
    if receipt.schema != 1
        || receipt.bytes != bytes
        || receipt.sha256 != digest
        || receipt.chunk.path != chunk.path
        || receipt.chunk.source != chunk.source
        || receipt.chunk.sequence != chunk.sequence
        || receipt.chunk.start_ms != chunk.start_ms
        || receipt.chunk.end_ms != chunk.end_ms
        || receipt.chunk.sample_rate != chunk.sample_rate
        || receipt.chunk.channels != chunk.channels
    {
        return Err(AppError::storage(
            "Audio checksum or immutable metadata does not match",
        ));
    }
    Ok(())
}

pub(super) fn write_snapshot(folder: &Path, snapshot: &CaptureSnapshot) -> AppResult<()> {
    let temporary = folder.join("capture.json.tmp");
    if temporary.exists() {
        quarantine(&temporary)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    // Serializing thousands of chunk fields directly to File issues a syscall
    // for almost every token and can stall the recorder behind its bounded queue.
    let mut file = std::io::BufWriter::with_capacity(64 * 1024, file);
    serde_json::to_writer(&mut file, snapshot).map_err(|e| AppError::storage(e.to_string()))?;
    file.flush()?;
    file.get_ref().sync_all()?;
    drop(file);
    crash_boundary("manifest_synced");
    let current = folder.join("capture.json");
    if current.exists() {
        let bytes = std::fs::read(&current)?;
        if serde_json::from_slice::<CaptureSnapshot>(&bytes).is_ok() {
            let previous = folder.join("capture.json.prev");
            if previous.exists()
                && serde_json::from_slice::<CaptureSnapshot>(&std::fs::read(&previous)?).is_err()
            {
                quarantine(&previous)?;
            }
            let staging = folder.join(format!("capture.json.prev.tmp-{}", nonce()));
            let mut backup = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            backup.write_all(&bytes)?;
            backup.sync_all()?;
            drop(backup);
            replace_manifest(&staging, &previous)?;
        } else {
            quarantine(&current)?;
        }
    }
    replace_manifest(&temporary, &current)?;
    sync_directory(folder)?;
    Ok(())
}

fn pcm_header(rate: u32, channels: u16, bytes: u32) -> [u8; 44] {
    let mut header = [0_u8; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + bytes).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&rate.to_le_bytes());
    header[28..32].copy_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    header[32..34].copy_from_slice(&(channels * 2).to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&bytes.to_le_bytes());
    header
}

pub(super) fn write_chunk(
    folder: &Path,
    source: &str,
    sequence: u64,
    start_ms: u64,
    rate: u32,
    samples: &[f32],
) -> AppResult<AudioChunk> {
    if samples.is_empty()
        || !(8_000..=192_000).contains(&rate)
        || !matches!(source, "microphone" | "system")
        || samples.len() > u32::MAX as usize / 2 - 36
    {
        return Err(AppError::validation("Invalid audio block"));
    }
    let name = format!("{source}-{sequence:08}-{start_ms:016}.wav");
    let destination = folder.join(&name);
    let temporary = folder.join(format!("{name}.tmp"));
    if destination.exists() || receipt_path(folder, &name).exists() {
        return Err(AppError::storage("An audio checkpoint already exists"));
    }
    let chunk = AudioChunk {
        source: source.into(),
        sequence,
        path: name,
        start_ms,
        end_ms: start_ms
            .checked_add(samples.len() as u64 * 1000 / rate as u64)
            .ok_or_else(|| AppError::validation("Audio time overflow"))?,
        sample_rate: rate,
        channels: 1,
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    // An accurate format and intended data length are durable before PCM data.
    // Recovery only copies complete frames physically present; it never pads a tail.
    file.write_all(&pcm_header(rate, 1, samples.len() as u32 * 2))?;
    file.sync_all()?;
    let mut file = std::io::BufWriter::with_capacity(64 * 1024, file);
    let midpoint = samples.len() / 2;
    for (index, sample) in samples.iter().enumerate() {
        file.write_all(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
        #[cfg(test)]
        if index + 1 == midpoint
            && std::env::var_os("QAMILLE_AUDIO_CRASH_POINT").as_deref()
                == Some(std::ffi::OsStr::new("partial_data_synced"))
        {
            file.flush()?;
            file.get_ref().sync_all()?;
            crash_boundary("partial_data_synced");
        }
        #[cfg(not(test))]
        let _ = (index, midpoint);
    }
    file.flush()?;
    file.get_ref().sync_all()?;
    drop(file);
    crash_boundary("data_synced");
    write_receipt(folder, &chunk, &temporary)?;
    crash_boundary("before_publish");
    publish_new(&temporary, &destination)?;
    crash_boundary("after_publish");
    remove_staging(&temporary)?;
    Ok(chunk)
}

/// Failpoints exist only in test builds, and are armed exclusively in a subprocess.
#[cfg(test)]
pub(super) static TEST_CRASH_ARMED: AtomicBool = AtomicBool::new(false);

pub(super) fn crash_boundary(point: &str) {
    #[cfg(test)]
    if TEST_CRASH_ARMED.load(Ordering::Acquire)
        && std::env::var_os("QAMILLE_AUDIO_CRASH_POINT").as_deref()
            == Some(std::ffi::OsStr::new(point))
    {
        if let Some(marker) = std::env::var_os("QAMILLE_AUDIO_CRASH_MARKER") {
            let mut file = File::create(marker).unwrap();
            file.write_all(point.as_bytes()).unwrap();
            file.sync_all().unwrap();
        }
        if std::env::var_os("QAMILLE_AUDIO_CRASH_MODE").as_deref()
            == Some(std::ffi::OsStr::new("kill"))
        {
            loop {
                std::thread::park_timeout(Duration::from_millis(100));
            }
        }
        std::process::abort();
    }
    #[cfg(not(test))]
    let _ = point;
}

fn folder_id(folder: &Path) -> Option<u64> {
    folder.file_name()?.to_str()?.parse().ok()
}

fn warning(snapshot: &mut CaptureSnapshot, text: impl Into<String>) {
    let text = text.into();
    snapshot.incomplete = true;
    if !snapshot.warnings.contains(&text) {
        snapshot.warnings.push(text.clone());
    }
    snapshot.error = Some(snapshot.warnings.join("; "));
}

pub(super) fn record_gap(snapshot: &mut CaptureSnapshot, source: &str, start_ms: u64, end_ms: u64) {
    if end_ms <= start_ms.saturating_add(40) {
        return;
    }
    if !snapshot
        .gaps
        .iter()
        .any(|gap| gap.source == source && gap.start_ms == start_ms && gap.end_ms == end_ms)
    {
        snapshot.gaps.push(AudioGap {
            source: source.into(),
            start_ms,
            end_ms,
        });
    }
    // Intervals are retained separately; prose remains bounded for long captures.
    warning(
        snapshot,
        "Audio contains missing intervals; see the recording timeline",
    );
}

fn read_manifest(
    folder: &Path,
    expected: Option<u64>,
    mutate: bool,
) -> AppResult<Option<CaptureSnapshot>> {
    let expected = expected.or_else(|| folder_id(folder));
    let mut candidates = Vec::new();
    let mut problems = Vec::new();
    let mut evidence = false;
    for name in ["capture.json", "capture.json.tmp", "capture.json.prev"] {
        let path = folder.join(name);
        if !path.exists() {
            continue;
        }
        evidence = true;
        let parsed = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CaptureSnapshot>(&bytes).ok());
        if let Some(snapshot) = parsed.filter(|s| expected.is_none() || s.meeting_id == expected) {
            candidates.push((name, snapshot));
        } else {
            problems.push(format!("Recording manifest {name} is unreadable or belongs to another meeting; original retained"));
            if mutate {
                quarantine(&path)?;
            }
        }
    }
    // The current manifest is authoritative, including a completed partial recovery.
    // A previous index may reference damaged blocks deliberately excluded later.
    // Orphan scanning adds committed blocks beyond the selected index independently.
    let mut selected = candidates.into_iter().max_by_key(|(name, _)| match *name {
        "capture.json" => 3,
        "capture.json.tmp" => 2,
        _ => 1,
    });
    if selected.is_none() {
        for name in ["session.json", "session.json.tmp", "session.json.prev"] {
            let value: serde_json::Value = match std::fs::read(folder.join(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            {
                Some(value) => value,
                None => continue,
            };
            if value["schema_version"].as_u64() != Some(2)
                || (expected.is_some() && value["meeting"]["id"].as_u64() != expected)
            {
                continue;
            }
            if let Ok(mut snapshot) =
                serde_json::from_value::<CaptureSnapshot>(value["capture"].clone())
            {
                if snapshot.chunks.is_empty() {
                    continue;
                }
                snapshot.meeting_id = value["meeting"]["id"].as_u64();
                selected = Some((name, snapshot));
                break;
            }
        }
    }
    let mut snapshot = match selected {
        Some((name, mut snapshot)) => {
            // Imports publish their completed snapshot in session.json. It is
            // authoritative for an import, not evidence of a crashed recorder.
            // Backup sessions and native capture fallbacks still require recovery;
            // inspect_capture below independently validates every referenced WAV.
            let completed_import = name == "session.json"
                && !evidence
                && snapshot.state == CaptureState::Saved
                && !snapshot.incomplete
                && snapshot.chunks.iter().all(|chunk| chunk.source == "import");
            if name != "capture.json" && !completed_import {
                warning(
                    &mut snapshot,
                    format!(
                        "Recording metadata recovered from {name}; the final part may be incomplete"
                    ),
                );
                snapshot.state = CaptureState::Interrupted;
            }
            snapshot
        }
        None => {
            let audio = std::fs::read_dir(folder)?
                .filter_map(Result::ok)
                .any(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    parse_native(&name).is_some() || name.ends_with(".wav.sha256.json")
                });
            if !audio && !evidence {
                return Ok(None);
            }
            let id = expected.ok_or_else(|| {
                AppError::storage(
                    "Cannot identify recording without valid metadata or a numeric meeting folder",
                )
            })?;
            let mut snapshot = CaptureSnapshot {
                meeting_id: Some(id),
                state: CaptureState::Interrupted,
                requires_receipts: true,
                ..Default::default()
            };
            warning(
                &mut snapshot,
                "Recording manifest was unavailable; reconstructed from surviving audio files",
            );
            snapshot
        }
    };
    for problem in problems {
        warning(&mut snapshot, problem);
    }
    Ok(Some(snapshot))
}

/// Reads live captures without mutation. Stopped sessions are rebuilt from independently
/// verifiable files, even when manifests are missing or damaged.
pub fn load_capture(folder: &Path) -> AppResult<Option<CaptureSnapshot>> {
    if !folder.exists() {
        return Ok(None);
    }
    let _guard = match acquire_lock(&folder.join(".capture.lock")) {
        Ok(guard) => guard,
        Err(error) if error.code == "recording_busy" => return read_manifest(folder, None, false),
        Err(error) => return Err(error),
    };
    let Some(mut snapshot) = read_manifest(folder, None, true)? else {
        return Ok(None);
    };
    inspect_capture(folder, &mut snapshot)?;
    write_snapshot(folder, &snapshot)?;
    Ok(Some(snapshot))
}

/// Finalizes all recoverable blocks. Damage remains explicit and durable, but does not
/// prevent the healthy portion from being listened to or transcribed.
pub fn recover_capture(folder: &Path, meeting_id: u64) -> AppResult<CaptureSnapshot> {
    let _guard = acquire_lock(&folder.join(".capture.lock"))?;
    let mut snapshot = read_manifest(folder, Some(meeting_id), true)?
        .ok_or_else(|| AppError::missing("Recording"))?;
    inspect_capture(folder, &mut snapshot)?;
    if snapshot.chunks.is_empty() {
        snapshot.state = CaptureState::Interrupted;
        write_snapshot(folder, &snapshot)?;
        return Err(AppError::storage(
            "No valid audio frames were found; originals and the loss report were preserved",
        ));
    }
    warning(
        &mut snapshot,
        "Audio recuperato fino all'ultimo blocco disponibile. Registrazione incompleta: alcune parti possono mancare.",
    );
    snapshot.state = CaptureState::Saved;
    snapshot.elapsed_ms = snapshot.saved_ms;
    write_snapshot(folder, &snapshot)?;
    Ok(snapshot)
}

/// One audio-shaped entry at the top level of a meeting folder.
struct AudioEntry {
    path: PathBuf,
    /// A `.stt-*` scratch directory, removed without following links.
    tree: bool,
    /// Bytes of the files it holds (links count for nothing: they are never followed).
    bytes: u64,
}

fn tree_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() && !kind.is_symlink() => tree_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Everything in a meeting folder that is audio: blocks, receipts, staging, quarantined and
/// recovered copies and imported WAVs (any file whose name contains `.wav`), plus the
/// speech-to-text scratch space (`.stt-cache` and `.stt-*` directories). Only the top level
/// is considered. Metadata, manifests and locks never match.
fn audio_entries(folder: &Path) -> AppResult<Vec<AudioEntry>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        let scratch = name.starts_with(".stt-");
        let wav = name.contains(".wav") && !kind.is_dir();
        if !(scratch || wav) {
            continue;
        }
        let tree = kind.is_dir() && !kind.is_symlink();
        let bytes = if tree {
            tree_bytes(&entry.path())
        } else if kind.is_file() {
            entry.metadata().map_or(0, |m| m.len())
        } else {
            0
        };
        found.push(AudioEntry {
            path: entry.path(),
            tree,
            bytes,
        });
    }
    Ok(found)
}

/// What `discard_audio` would free, without touching anything.
pub fn audio_bytes(folder: &Path) -> AppResult<u64> {
    Ok(audio_entries(folder)?.iter().map(|entry| entry.bytes).sum())
}

fn remove_manifest_files(folder: &Path, names: &[&str]) -> AppResult<()> {
    for name in names {
        match std::fs::remove_file(folder.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// After the audio is gone the recording manifest must not describe it. A readable manifest
/// is rewritten with nothing left to report; one that is absent stays absent (nothing is
/// created), and one that cannot be trusted, or would still read as damaged, is deleted.
fn forget_discarded_audio(folder: &Path) -> AppResult<()> {
    const ALL: [&str; 3] = ["capture.json", "capture.json.tmp", "capture.json.prev"];
    let current = match std::fs::read(folder.join("capture.json")) {
        Ok(bytes) => serde_json::from_slice::<CaptureSnapshot>(&bytes).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let Some(mut snapshot) = current else {
        // Absent, or unreadable: either way the audio it described is gone.
        return remove_manifest_files(folder, &ALL);
    };
    snapshot.chunks.clear();
    snapshot.missing_chunks.clear();
    snapshot.unverified_chunks.clear();
    snapshot.gaps.clear();
    snapshot.warnings.clear();
    snapshot.incomplete = false;
    snapshot.error = None;
    // Exactly what the next read would conclude. A manifest that still comes out
    // damaged (a recording that never finished) is deleted rather than kept alarming.
    let mut reread = snapshot.clone();
    inspect_capture(folder, &mut reread)?;
    if reread.incomplete || !reread.warnings.is_empty() || !reread.missing_chunks.is_empty() {
        return remove_manifest_files(folder, &ALL);
    }
    // Stale staging and backup copies still list the removed blocks.
    remove_manifest_files(folder, &["capture.json.tmp", "capture.json.prev"])?;
    write_snapshot(folder, &snapshot)?;
    remove_manifest_files(folder, &["capture.json.prev"])
}

/// After a discard that could not remove everything, the manifest lists only the blocks that
/// are still on disk: a restart would otherwise read the removed ones as lost and mark the
/// recording damaged. `incomplete` and the rest stay as they were; best effort, the error of
/// the discard is what the caller sees.
fn keep_only_surviving_blocks(folder: &Path) {
    let Ok(bytes) = std::fs::read(folder.join("capture.json")) else {
        return;
    };
    let Ok(mut snapshot) = serde_json::from_slice::<CaptureSnapshot>(&bytes) else {
        return;
    };
    // Only a block that is plainly gone is dropped; one that cannot be judged stays listed.
    snapshot.chunks.retain(|chunk| {
        !matches!(
            std::fs::symlink_metadata(folder.join(&chunk.path)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        )
    });
    if write_snapshot(folder, &snapshot).is_ok() {
        // Stale staging and backup copies still list the removed blocks.
        let _ = remove_manifest_files(folder, &["capture.json.tmp", "capture.json.prev"]);
    }
}

/// Deletes the audio of a meeting folder and keeps the meeting: returns the bytes freed (the
/// files removed, including those inside scratch directories). Takes the capture lock like
/// `recover_capture`, so a live recorder makes it fail with `recording_busy` and nothing is
/// removed. Files already absent are not an error, so a repeat call frees 0.
pub fn discard_audio(folder: &Path) -> AppResult<u64> {
    if !folder.is_dir() {
        return Ok(0);
    }
    let _guard = acquire_lock(&folder.join(".capture.lock"))?;
    let mut freed = 0_u64;
    let mut first_error = None;
    for entry in audio_entries(folder)? {
        let removed: AppResult<()> = if entry.tree {
            // Do not follow junctions/symlinks out of the application's own scratch space.
            crate::infrastructure::meeting_repository::remove_owned_tree(&entry.path)
        } else {
            match crate::infrastructure::meeting_repository::remove_file_or_link(&entry.path) {
                Ok(()) => Ok(()),
                // Gone since it was listed: nothing left to free, nothing to report.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => Err(e.into()),
            }
        };
        match removed {
            Ok(()) => freed += entry.bytes,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    // Something could not be removed: a retry finishes the job.
    if let Some(error) = first_error {
        keep_only_surviving_blocks(folder);
        return Err(error);
    }
    forget_discarded_audio(folder)?;
    Ok(freed)
}

fn missing(snapshot: &mut CaptureSnapshot, chunk: AudioChunk, reason: &str) {
    warning(
        snapshot,
        format!(
            "{} [{}–{} ms]: {reason}",
            chunk.path, chunk.start_ms, chunk.end_ms
        ),
    );
    if !snapshot
        .missing_chunks
        .iter()
        .any(|lost| lost.path == chunk.path)
    {
        snapshot.missing_chunks.push(chunk);
    }
    snapshot.state = CaptureState::Interrupted;
}

fn inspect_capture(folder: &Path, snapshot: &mut CaptureSnapshot) -> AppResult<()> {
    let mut healthy = Vec::new();
    let mut known = HashSet::new();
    let mut repaired_manifest = false;
    // Older readers emitted one informational warning per orphan. Collapse those
    // too: paths and timing remain in chunks, not in megabytes of repeated prose.
    let mut recovered_blocks = 0_usize;
    snapshot.warnings.retain(|message| {
        let old = message.starts_with("Recovered audio block ")
            && message.ends_with(" not yet present in the manifest");
        recovered_blocks += usize::from(old);
        !old
    });
    for chunk in std::mem::take(&mut snapshot.chunks) {
        if !known.insert(chunk.path.clone()) {
            continue;
        }
        match validate_chunk(folder, &chunk) {
            Ok(()) => healthy.push(chunk),
            Err(error) => {
                // A readable JSON manifest can still contain wrong offsets/rates.
                // Verify independent immutable metadata before blaming the WAV.
                if let Ok(restored) = independent_chunk(folder, &chunk.path) {
                    warning(
                        snapshot,
                        format!(
                            "Repaired inconsistent manifest metadata for {} from its audio receipt or native filename",
                            chunk.path
                        ),
                    );
                    snapshot.state = CaptureState::Interrupted;
                    repaired_manifest = true;
                    healthy.push(restored);
                    continue;
                }
                if let Ok(path) = chunk_path(folder, &chunk.path) {
                    quarantine(&path)?;
                }
                missing(snapshot, chunk, &error.message);
            }
        }
    }
    if repaired_manifest && folder.join("capture.json").exists() {
        quarantine(&folder.join("capture.json"))?;
    }
    let entries: Vec<_> = std::fs::read_dir(folder)?.collect::<Result<_, _>>()?;
    for entry in entries {
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if known.contains(&name) {
            continue;
        }
        let Some((source, sequence, start_ms, temporary)) = parse_native(&name) else {
            if name.ends_with(".wav.tmp") {
                quarantine(&entry.path())?;
                warning(
                    snapshot,
                    format!(
                        "Unidentified audio staging file {name} was retained in quarantine; its timeline cannot be inferred"
                    ),
                );
            }
            continue;
        };
        if temporary {
            let final_name = name.strip_suffix(".tmp").unwrap();
            if folder.join(final_name).exists()
                && orphan_chunk(folder, final_name, source.clone(), sequence, start_ms).is_ok()
            {
                // A crash immediately after publish can leave a redundant staging name.
                quarantine(&entry.path())?;
                continue;
            }
            match salvage_temporary(folder, &name, &source, sequence, start_ms) {
                Ok(chunk) => {
                    warning(
                        snapshot,
                        format!(
                            "Recovered {} complete audio frames from interrupted staging file {name}",
                            (chunk.end_ms - chunk.start_ms) * chunk.sample_rate as u64 / 1000
                        ),
                    );
                    if !healthy.iter().any(|c| c.path == chunk.path) {
                        healthy.push(chunk);
                    }
                    quarantine(&entry.path())?;
                    snapshot.state = CaptureState::Interrupted;
                }
                Err(error) => {
                    quarantine(&entry.path())?;
                    missing(
                        snapshot,
                        AudioChunk {
                            source,
                            sequence,
                            path: name,
                            start_ms,
                            ..Default::default()
                        },
                        &error.message,
                    );
                }
            }
        } else {
            match orphan_chunk(folder, &name, source.clone(), sequence, start_ms) {
                Ok(chunk) => {
                    known.insert(chunk.path.clone());
                    healthy.push(chunk);
                    recovered_blocks += 1;
                    snapshot.state = CaptureState::Interrupted;
                }
                Err(error) => {
                    quarantine(&entry.path())?;
                    missing(
                        snapshot,
                        AudioChunk {
                            source,
                            sequence,
                            path: name,
                            start_ms,
                            ..Default::default()
                        },
                        &error.message,
                    );
                }
            }
        }
    }
    if recovered_blocks > 0 {
        warning(
            snapshot,
            format!("Recovered {recovered_blocks} audio blocks omitted from the recording index"),
        );
    }
    healthy.sort_by_key(|c| {
        (
            c.start_ms,
            c.source.clone(),
            c.sequence,
            std::cmp::Reverse(c.end_ms),
        )
    });
    // A .tmp and its newly committed counterpart can both be present in the
    // initial directory listing. Never play the same source/time range twice.
    healthy.dedup_by(|a, b| {
        a.source == b.source && a.sequence == b.sequence && a.start_ms == b.start_ms
    });
    // Receipts also prove that a block existed beyond an older full manifest.
    // A missing WAV must remain visible even if the last index preceded its commit.
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(audio_name) = name.strip_suffix(".sha256.json") else {
            continue;
        };
        if !audio_name.ends_with(".wav") || !entry.file_type()?.is_file() {
            continue;
        }
        let receipt: Receipt = match serde_json::from_reader(File::open(entry.path())?) {
            Ok(receipt) => receipt,
            Err(_) => continue, // Referenced corrupt receipts already fail validation above.
        };
        if receipt.schema != 1 || receipt.chunk.path != audio_name {
            continue;
        }
        let chunk = receipt.chunk;
        if healthy.iter().any(|found| {
            found.source == chunk.source
                && found.sequence == chunk.sequence
                && found.start_ms == chunk.start_ms
                && found.end_ms >= chunk.end_ms
        }) || snapshot
            .missing_chunks
            .iter()
            .any(|lost| lost.path == chunk.path)
        {
            continue;
        }
        missing(
            snapshot,
            chunk,
            "A synchronized audio receipt survived but its complete WAV is unavailable",
        );
    }
    // A healthy receipt in an older capture also identifies the hardened format.
    // Completely legacy/imported captures remain readable without requiring one.
    snapshot.requires_receipts |= healthy
        .iter()
        .any(|c| receipt_path(folder, &c.path).is_file());
    snapshot.unverified_chunks = if snapshot.requires_receipts {
        healthy
            .iter()
            .filter(|c| !receipt_path(folder, &c.path).is_file())
            .map(|c| c.path.clone())
            .collect()
    } else {
        vec![]
    };
    if !snapshot.unverified_chunks.is_empty() {
        warning(
            snapshot,
            "Some audio blocks have no integrity receipt; audio is kept but its integrity cannot be verified",
        );
    }
    // Leading offsets differ between devices and are not gaps. Pause time is
    // excluded by the capture clock. Only internal discontinuities count here.
    snapshot.gaps.clear();
    let mut source_ends = std::collections::HashMap::<String, u64>::new();
    for chunk in &healthy {
        if let Some(end) = source_ends.get(&chunk.source).copied() {
            record_gap(snapshot, &chunk.source, end, chunk.start_ms);
        }
        source_ends
            .entry(chunk.source.clone())
            .and_modify(|end| *end = (*end).max(chunk.end_ms))
            .or_insert(chunk.end_ms);
    }
    snapshot.chunks = healthy;
    snapshot.saved_ms = snapshot.chunks.iter().map(|c| c.end_ms).max().unwrap_or(0);
    snapshot.elapsed_ms = snapshot.elapsed_ms.max(snapshot.saved_ms);
    snapshot.microphone_level = 0.0;
    snapshot.system_level = 0.0;
    if matches!(
        snapshot.state,
        CaptureState::Preparing
            | CaptureState::Recording
            | CaptureState::Paused
            | CaptureState::Saving
    ) {
        snapshot.state = CaptureState::Interrupted;
        warning(
            snapshot,
            "Recording was interrupted; the final unsaved fraction may be missing",
        );
    }
    if snapshot.incomplete && snapshot.error.is_none() {
        snapshot.error = Some(snapshot.warnings.join("; "));
    }
    Ok(())
}

fn independent_chunk(folder: &Path, name: &str) -> AppResult<AudioChunk> {
    chunk_path(folder, name)?;
    if receipt_path(folder, name).exists() {
        let receipt: Receipt = serde_json::from_reader(File::open(receipt_path(folder, name))?)
            .map_err(|_| AppError::storage("Audio integrity receipt is damaged"))?;
        if receipt.chunk.path != name {
            return Err(AppError::storage(
                "Audio receipt identifies a different file",
            ));
        }
        validate_chunk(folder, &receipt.chunk)?;
        return Ok(receipt.chunk);
    }
    let (source, sequence, start_ms, temporary) = parse_native(name)
        .ok_or_else(|| AppError::storage("No independent audio timeline metadata is available"))?;
    if temporary {
        return Err(AppError::storage("Audio block is still staged"));
    }
    orphan_chunk(folder, name, source, sequence, start_ms)
}

pub(super) fn validate_chunk(folder: &Path, chunk: &AudioChunk) -> AppResult<()> {
    let path = chunk_path(folder, &chunk.path)?;
    let reader = hound::WavReader::open(&path).map_err(wav_error)?;
    let spec = reader.spec();
    if spec.channels == 0
        || spec.channels > 32
        || spec.channels != chunk.channels
        || spec.sample_rate != chunk.sample_rate
        || !(8_000..=192_000).contains(&spec.sample_rate)
        || chunk.end_ms < chunk.start_ms
    {
        return Err(AppError::storage(
            "Audio checkpoint metadata does not match its file",
        ));
    }
    let duration_ms = reader.duration() as u64 * 1000 / spec.sample_rate as u64;
    if (chunk.end_ms - chunk.start_ms).abs_diff(duration_ms) > 2 || reader.duration() == 0 {
        return Err(AppError::storage(
            "Audio checkpoint duration does not match its file",
        ));
    }
    let data_bytes = reader.len() as u64 * (spec.bits_per_sample as u64).div_ceil(8);
    if std::fs::metadata(&path)?.len() < data_bytes + 44 {
        return Err(AppError::storage("An audio checkpoint is truncated"));
    }
    check_receipt(folder, chunk, &path)?;
    Ok(())
}

fn parse_native(name: &str) -> Option<(String, u64, u64, bool)> {
    let temporary = name.ends_with(".wav.tmp");
    let name = if temporary {
        name.strip_suffix(".tmp")?
    } else {
        name
    };
    let stem = name.strip_suffix(".wav")?;
    let stem = stem.split(".recovered-").next()?;
    let parts: Vec<_> = stem.split('-').collect();
    if parts.len() != 3 || !matches!(parts[0], "microphone" | "system") {
        return None;
    }
    Some((
        parts[0].into(),
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
        temporary,
    ))
}

fn orphan_chunk(
    folder: &Path,
    name: &str,
    source: String,
    sequence: u64,
    start_ms: u64,
) -> AppResult<AudioChunk> {
    let path = chunk_path(folder, name)?;
    let reader = hound::WavReader::open(path).map_err(wav_error)?;
    let spec = reader.spec();
    if spec.sample_rate == 0 {
        return Err(AppError::storage("Invalid audio sample rate"));
    }
    let chunk = AudioChunk {
        source,
        sequence,
        path: name.into(),
        start_ms,
        end_ms: start_ms
            .checked_add(reader.duration() as u64 * 1000 / spec.sample_rate as u64)
            .ok_or_else(|| AppError::storage("Audio timestamp overflow"))?,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    };
    validate_chunk(folder, &chunk)?;
    Ok(chunk)
}

#[cfg(test)]
mod import_recovery_tests {
    use super::*;
    use crate::domain::meeting::MeetingDetail;

    #[test]
    fn completed_import_is_authoritative_but_damaged_and_backup_sessions_are_not() {
        let root = std::env::temp_dir().join(format!("qamille-import-recovery-{}", crate::infrastructure::meeting_processing::unique_id().unwrap()));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("original.wav");
        crate::infrastructure::meeting_processing::write_wav(&source, &vec![0.1; 16_000], 16_000).unwrap();
        for (index, mode) in ["healthy", "missing_audio", "backup_session", "damaged_manifest", "native_session"].into_iter().enumerate() {
            let id = 100 + index as u64;
            let folder = root.join(id.to_string());
            let chunks = crate::infrastructure::meeting_processing::import_audio(&source, &folder, &AtomicBool::new(false), &|_| {}).unwrap();
            let mut detail = MeetingDetail::legacy(crate::model::Meeting { id, ..Default::default() });
            detail.capture = CaptureSnapshot { meeting_id: Some(id), state: CaptureState::Saved, elapsed_ms: 1000, saved_ms: 1000, chunks, ..Default::default() };
            if mode == "native_session" { detail.capture.chunks[0].source = "microphone".into(); }
            let manifest = if mode == "backup_session" { "session.json.prev" } else { "session.json" };
            std::fs::write(folder.join(manifest), serde_json::to_vec(&detail).unwrap()).unwrap();
            if mode == "missing_audio" { std::fs::remove_file(folder.join(&detail.capture.chunks[0].path)).unwrap(); }
            if mode == "damaged_manifest" { std::fs::write(folder.join("capture.json"), b"broken").unwrap(); }
            let recovered = load_capture(&folder).unwrap().unwrap();
            if mode == "healthy" {
                assert_eq!(recovered.state, CaptureState::Saved);
                assert!(!recovered.incomplete);
                assert!(recovered.warnings.is_empty());
                let reopened = load_capture(&folder).unwrap().unwrap();
                assert_eq!(reopened.state, CaptureState::Saved);
                assert!(!reopened.incomplete);
            } else {
                assert_eq!(recovered.state, CaptureState::Interrupted, "{mode}");
                assert!(recovered.incomplete, "{mode}");
                assert!(!recovered.warnings.is_empty(), "{mode}");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn salvage_temporary(
    folder: &Path,
    name: &str,
    source: &str,
    sequence: u64,
    start_ms: u64,
) -> AppResult<AudioChunk> {
    let path = chunk_path(folder, name)?;
    let mut input = File::open(&path)?;
    let length = input.metadata()?.len();
    let mut header = [0_u8; 44];
    input.read_exact(&mut header)?;
    let u16_at = |at| u16::from_le_bytes(header[at..at + 2].try_into().unwrap());
    let u32_at = |at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let rate = u32_at(24);
    let channels = u16_at(22);
    // Recovery accepts the recorder's exact PCM16 header. Unknown containers and
    // unsupported layouts are retained for manual recovery, never guessed.
    if &header[..4] != b"RIFF"
        || &header[8..16] != b"WAVEfmt "
        || u32_at(16) != 16
        || u16_at(20) != 1
        || channels != 1
        || !(8_000..=192_000).contains(&rate)
        || u32_at(28) != rate * 2
        || u16_at(32) != 2
        || u16_at(34) != 16
        || &header[36..40] != b"data"
    {
        return Err(AppError::storage(
            "Staging WAV header is not trustworthy PCM16",
        ));
    }
    let declared = u32_at(40) as u64;
    let available = length.saturating_sub(44);
    let bytes = (if declared == 0 {
        available
    } else {
        available.min(declared)
    }) / 2
        * 2;
    // The original recorder stored at most five seconds; keep recovery bounded.
    if bytes == 0 || bytes > rate as u64 * 2 * 10 {
        return Err(AppError::storage(
            "Staging WAV has no bounded complete PCM frames",
        ));
    }
    let final_name = name.strip_suffix(".tmp").unwrap();
    if declared > 0 && available >= declared && receipt_path(folder, final_name).exists() {
        let original = AudioChunk {
            source: source.into(),
            sequence,
            path: final_name.into(),
            start_ms,
            end_ms: start_ms + bytes / 2 * 1000 / rate as u64,
            sample_rate: rate,
            channels,
        };
        check_receipt(folder, &original, &path)?;
    }
    let (_, digest) = hash_file(&path)?;
    let stem = final_name.strip_suffix(".wav").unwrap();
    let recovered_name = format!("{stem}.recovered-{}.wav", &digest[..16]);
    let chunk = AudioChunk {
        source: source.into(),
        sequence,
        path: recovered_name.clone(),
        start_ms,
        end_ms: start_ms + bytes / 2 * 1000 / rate as u64,
        sample_rate: rate,
        channels,
    };
    let destination = folder.join(&recovered_name);
    if destination.exists() {
        validate_chunk(folder, &chunk)?;
        return Ok(chunk);
    }
    let temporary = folder.join(format!("{recovered_name}.tmp"));
    if temporary.exists() {
        quarantine(&temporary)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    output.write_all(&pcm_header(rate, channels, bytes as u32))?;
    input.seek(SeekFrom::Start(44))?;
    let copied = std::io::copy(&mut input.take(bytes), &mut output)?;
    if copied != bytes {
        return Err(AppError::storage("Staging WAV changed during recovery"));
    }
    output.sync_all()?;
    drop(output);
    write_receipt(folder, &chunk, &temporary)?;
    publish_new(&temporary, &destination)?;
    remove_staging(&temporary)?;
    validate_chunk(folder, &chunk)?;
    Ok(chunk)
}
