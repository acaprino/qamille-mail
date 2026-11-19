//! Speech recognition with whisper.cpp (`whisper-cli`), moved from the app's
//! `meeting_processing/{speech,models}.rs` (spec D3: the app keeps the 15-second windows, the
//! cache and the jobs; the server runs the recognizer).
//!
//! - **Models**: the multilingual ggml files Tiny, Base and Small of
//!   `huggingface.co/ggerganov/whisper.cpp`, pinned at [`MODEL_REVISION`] and checked against
//!   their length and SHA-256, in `<speech>/ggml-<size>.bin` (the folder the app used, so models
//!   installed before keep working). Protocol id: `whisper-<size>`.
//! - **Runtime**: on Windows x64 the official `whisper-bin-x64.zip` of whisper.cpp
//!   [`WHISPER_RELEASE`], downloaded at installation and hash-checked, as before. Elsewhere
//!   `whisper-cli` is looked up in the speech models folder, next to the server executable and on
//!   `PATH`; nothing is built or downloaded for it.
//! - **One window**: [`run_window`] runs `whisper-cli` on a WAV file with a JSON output, polls a
//!   cancellation flag every 100 ms (killing the process), and stops after 10 minutes.

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The whisper.cpp release whose runtime and output format this module expects.
pub const WHISPER_RELEASE: &str = "v1.9.2";
/// Official upstream LFS pointers, pinned at this revision (see the app's THIRD_PARTY.md).
pub const MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
/// Prefix of the protocol ids of the speech models (`whisper-base`).
pub const ID_PREFIX: &str = "whisper-";
/// Longest time one window may take.
pub const WINDOW_LIMIT: Duration = Duration::from_secs(600);
/// Largest JSON result read from `whisper-cli`.
pub const MAX_RESULT_BYTES: u64 = 4 * 1024 * 1024;

pub const RUNTIME_MISSING_MESSAGE: &str =
    "The speech recognizer whisper-cli is not installed on the compute server.";
pub const MODEL_MISSING_MESSAGE: &str = "Install the transcription model before transcribing.";

#[derive(Debug)]
pub struct SpeechModelSpec {
    /// `tiny`, `base` or `small`.
    pub size: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const CATALOG: &[SpeechModelSpec] = &[
    SpeechModelSpec {
        size: "tiny",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    SpeechModelSpec {
        size: "base",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    SpeechModelSpec {
        size: "small",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];

const RUNTIME_URL: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip";
/// Bytes of the Windows runtime archive (counted in the download of every model there).
pub const RUNTIME_BYTES: u64 = 8_194_445;
const RUNTIME_SHA: &str = "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a";
const RUNTIME_DIR: &str = "whisper-v1.9.2-win-x64";

#[cfg(windows)]
pub const PROGRAM: &str = "whisper-cli.exe";
#[cfg(not(windows))]
pub const PROGRAM: &str = "whisper-cli";

/// Whether installation downloads the official runtime (Windows x64 only).
pub fn bundled_runtime() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
}

/// `whisper-base` (or the bare `base`) → its catalog entry.
pub fn spec<'a>(catalog: &'a [SpeechModelSpec], id: &str) -> Option<&'a SpeechModelSpec> {
    let size = id.strip_prefix(ID_PREFIX).unwrap_or(id);
    catalog.iter().find(|model| model.size == size)
}

pub fn model_id(spec: &SpeechModelSpec) -> String {
    format!("{ID_PREFIX}{}", spec.size)
}

/// Where the model file lives in the speech models folder.
pub fn model_file(dir: &Path, spec: &SpeechModelSpec) -> PathBuf {
    dir.join(format!("ggml-{}.bin", spec.size))
}

/// Present with the expected length (the hash is checked by [`verify`]).
pub fn model_present(dir: &Path, spec: &SpeechModelSpec) -> bool {
    fs::metadata(model_file(dir, spec)).is_ok_and(|file| file.len() == spec.bytes)
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpeechError {
    Cancelled,
    /// The runtime or the model is missing or fails its check.
    Missing(String),
    /// The recognizer failed on this audio.
    Failed(String),
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpeechError::Cancelled => f.write_str("Cancelled."),
            SpeechError::Missing(message) | SpeechError::Failed(message) => f.write_str(message),
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), SpeechError> {
    if cancel.load(Ordering::Relaxed) { Err(SpeechError::Cancelled) } else { Ok(()) }
}

/// A name no other file of this process or another one uses.
fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

// --- Runtime ---------------------------------------------------------------------

/// The `whisper-cli` this server runs.
pub fn locate_runtime(dir: &Path) -> Result<PathBuf, SpeechError> {
    if bundled_runtime() {
        return verified_bundle(dir);
    }
    let mut candidates = vec![dir.join(PROGRAM)];
    if let Some(folder) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        candidates.push(folder.join(PROGRAM));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).filter(|folder| folder.is_absolute()).map(|folder| folder.join(PROGRAM)));
    }
    candidates
        .into_iter()
        .find(|candidate| executable(candidate))
        .ok_or_else(|| SpeechError::Missing(RUNTIME_MISSING_MESSAGE.to_owned()))
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

/// The downloaded Windows runtime, each file matching the hash recorded at installation.
fn verified_bundle(dir: &Path) -> Result<PathBuf, SpeechError> {
    let directory = dir.join(RUNTIME_DIR);
    let path = directory.join("whisper-cli.exe");
    let invalid = || SpeechError::Missing("Install or reinstall the transcription model: it includes the Whisper engine.".to_owned());
    let manifest: serde_json::Value = fs::read(directory.join("upstream.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(invalid)?;
    let files = manifest["files"].as_array().ok_or_else(invalid)?;
    if files.is_empty() || !path.is_file() {
        return Err(invalid());
    }
    for file in files {
        let name = file["name"].as_str().ok_or_else(invalid)?;
        if Path::new(name).file_name().and_then(|s| s.to_str()) != Some(name) {
            return Err(invalid());
        }
        let bytes = fs::read(directory.join(name)).map_err(|_| invalid())?;
        if format!("{:x}", Sha256::digest(&bytes)) != file["sha256"].as_str().unwrap_or("") {
            return Err(invalid());
        }
    }
    Ok(path)
}

// --- Models ----------------------------------------------------------------------

/// The file has `size` bytes hashing to `expected`.
pub fn verify(path: &Path, size: u64, expected: &str, cancel: &AtomicBool) -> Result<(), SpeechError> {
    let integrity = |message: &str| SpeechError::Missing(message.to_owned());
    if fs::metadata(path).map_err(|_| integrity(MODEL_MISSING_MESSAGE))?.len() != size {
        return Err(integrity("The transcription model has the wrong size. Reinstall the model."));
    }
    let mut file = File::open(path).map_err(|_| integrity(MODEL_MISSING_MESSAGE))?;
    let mut hash = Sha256::new();
    let mut bytes = vec![0_u8; 65_536];
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut bytes).map_err(|_| integrity(MODEL_MISSING_MESSAGE))?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return Err(integrity("The transcription model failed its check. Reinstall the model."));
    }
    Ok(())
}

fn install_error(error: SpeechError) -> String {
    match error {
        SpeechError::Cancelled => "The installation was cancelled.".to_owned(),
        SpeechError::Missing(message) | SpeechError::Failed(message) => message,
    }
}

/// Bytes downloaded by [`install`] for `spec`: the model, plus the runtime where it is bundled.
pub fn install_bytes(spec: &SpeechModelSpec) -> u64 {
    spec.bytes + if bundled_runtime() { RUNTIME_BYTES } else { 0 }
}

/// Free disk for `bytes` plus a margin of 256 MiB, where the platform tells.
fn reserve_disk(dir: &Path, bytes: u64) -> Result<(), String> {
    match crate::memory::available_disk_bytes(dir) {
        Some(free) if free < bytes.saturating_add(256 * 1024 * 1024) => {
            Err("Not enough disk space for the transcription model. Free some space and try again.".to_owned())
        }
        _ => Ok(()),
    }
}

/// Installs the runtime (Windows x64) and the model `spec` into `dir`. `progress` receives
/// bytes done and total. A validated file replaces nothing until it is complete.
pub fn install(dir: &Path, spec: &SpeechModelSpec, cancel: &AtomicBool, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<(), String> {
    install_with(dir, spec, &crate::download::Options::one_shot(), cancel, progress)
}

/// The Hugging Face address of the ggml model of `spec`, at the pinned revision.
fn model_url(spec: &SpeechModelSpec) -> String {
    format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/{MODEL_REVISION}/ggml-{}.bin", spec.size)
}

/// As [`install`], with the mirror of `options` (`models_mirror_url`); the download is always one-shot.
pub fn install_with(
    dir: &Path,
    spec: &SpeechModelSpec,
    options: &crate::download::Options,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    let options = &crate::download::Options { resume: false, ..options.clone() };
    let io = |error: std::io::Error| format!("Cannot write the transcription model: {}", error.kind());
    fs::create_dir_all(dir).map_err(io)?;
    fs::write(dir.join("OPENAI_WHISPER_LICENSE.txt"), include_str!("OPENAI_WHISPER_LICENSE.txt")).map_err(io)?;
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(dir.join("install.lock")).map_err(io)?;
    lock.try_lock().map_err(|_| "An installation is already in progress.".to_owned())?;
    let total = install_bytes(spec);
    let mut base = 0;
    if bundled_runtime() {
        if verified_bundle(dir).is_err() {
            reserve_disk(dir, RUNTIME_BYTES + 128 * 1024 * 1024)?;
            let archive = dir.join(format!("runtime-{}.part", unique_suffix()));
            let downloaded = download(RUNTIME_URL, &archive, RUNTIME_BYTES, RUNTIME_SHA, options, cancel, &|done| progress(done, total));
            if let Err(error) = downloaded {
                let _ = fs::remove_file(&archive);
                return Err(error);
            }
            let result = unpack_runtime(&archive, dir, cancel);
            let _ = fs::remove_file(&archive);
            result?;
        }
        base = RUNTIME_BYTES;
    }
    let path = model_file(dir, spec);
    if path.exists() && verify(&path, spec.bytes, spec.sha256, cancel).is_ok() {
        progress(total, total);
        return Ok(());
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(install_error(SpeechError::Cancelled));
    }
    reserve_disk(dir, spec.bytes)?;
    let partial = dir.join(format!("model-{}.part", unique_suffix()));
    let url = model_url(spec);
    let result = download(&url, &partial, spec.bytes, spec.sha256, options, cancel, &|done| progress(base + done, total));
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if cancel.load(Ordering::Relaxed) {
        let _ = fs::remove_file(&partial);
        return Err(install_error(SpeechError::Cancelled));
    }
    fs::rename(&partial, &path).map_err(io)?;
    progress(total, total);
    Ok(())
}

fn download(url: &str, destination: &Path, size: u64, hash: &str, options: &crate::download::Options, cancel: &AtomicBool, progress: &dyn Fn(u64)) -> Result<(), String> {
    use crate::download::{DownloadError, Target};
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1800)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        // `download::fetch` follows the redirects by hand (mirror and token rules).
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .build()
        .into();
    let dir = destination.parent().unwrap_or(Path::new("."));
    let local = destination.file_name().and_then(|name| name.to_str()).unwrap_or("download.part");
    let target = Target { name: local, size, sha256: hash };
    crate::download::fetch(&agent, url, dir, local, &target, options, cancel, progress).map_err(|error| match error {
        DownloadError::Cancelled => install_error(SpeechError::Cancelled),
        DownloadError::TooLarge(_) => "The download is larger than expected.".to_owned(),
        DownloadError::WrongHash(_) => "The transcription model failed its check. Reinstall the model.".to_owned(),
        DownloadError::Interrupted(_) => "Download interrupted. Try the installation again.".to_owned(),
        DownloadError::NoSpace => "Not enough disk space for the transcription model. Free some space and try again.".to_owned(),
        DownloadError::Failed(_) => "Download failed. Check the connection and try again.".to_owned(),
    })
}

fn unpack_runtime(archive_path: &Path, root: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let io = |error: std::io::Error| format!("Cannot install the Whisper engine: {}", error.kind());
    let staging = root.join(format!("runtime-stage-{}", unique_suffix()));
    fs::create_dir(&staging).map_err(io)?;
    let _cleanup = StagingCleanup(staging.clone());
    let mut archive = zip::ZipArchive::new(File::open(archive_path).map_err(io)?).map_err(|_| "The Whisper engine archive is not valid.".to_owned())?;
    let mut written = 0_u64;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(install_error(SpeechError::Cancelled));
        }
        let mut entry = archive.by_index(index).map_err(|_| "The Whisper engine archive cannot be read.".to_owned())?;
        if entry.is_dir() {
            continue;
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| "The Whisper engine archive has an invalid path.".to_owned())?;
        let name = enclosed.file_name().and_then(|s| s.to_str()).ok_or_else(|| "The Whisper engine archive has an invalid file name.".to_owned())?.to_owned();
        let lower = name.to_ascii_lowercase();
        if lower != "whisper-cli.exe" && lower != "whisper.dll" && !(lower.starts_with("ggml") && lower.ends_with(".dll")) {
            continue;
        }
        written = written.saturating_add(entry.size());
        if written > 128 * 1024 * 1024 {
            return Err("The Whisper engine archive exceeds the expected size.".to_owned());
        }
        let mut output = OpenOptions::new().create_new(true).write(true).open(staging.join(&name)).map_err(io)?;
        std::io::copy(&mut entry, &mut output).map_err(io)?;
        output.sync_all().map_err(io)?;
        drop(output);
        let digest = format!("{:x}", Sha256::digest(fs::read(staging.join(&name)).map_err(io)?));
        files.push(serde_json::json!({"name": name, "sha256": digest}));
    }
    if !staging.join("whisper-cli.exe").is_file() {
        return Err("The download does not contain whisper-cli.exe.".to_owned());
    }
    let manifest = serde_json::json!({
        "project": "https://github.com/ggml-org/whisper.cpp", "release": WHISPER_RELEASE, "archive_sha256": RUNTIME_SHA,
        "license": "MIT", "model_repository_revision": MODEL_REVISION, "files": files
    });
    fs::write(staging.join("upstream.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(io)?;
    fs::write(staging.join("WHISPER_CPP_LICENSE.txt"), include_str!("WHISPER_CPP_LICENSE.txt")).map_err(io)?;
    let target = root.join(RUNTIME_DIR);
    if target.exists() {
        fs::rename(&target, root.join(format!("runtime-previous-{}", unique_suffix()))).map_err(io)?;
    }
    fs::rename(&staging, target).map_err(io)?;
    Ok(())
}

struct StagingCleanup(PathBuf);
impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.0);
    }
}

// --- One window ------------------------------------------------------------------

/// A recognized segment, in milliseconds from the start of the audio sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// A language code (`it`, `pt-BR`) or `auto`: ASCII letters and `-` only.
pub fn valid_language(language: &str) -> bool {
    !language.is_empty() && language.len() <= 16 && language.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// Runs `program` on `wave` with `model` (`work` is a private folder of this request).
pub fn run_window(
    program: &Path,
    model: &Path,
    wave: &Path,
    work: &Path,
    language: &str,
    threads: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Segment>, SpeechError> {
    let failed = |message: &str| SpeechError::Failed(message.to_owned());
    let output_base = work.join("result");
    let output_json = work.join("result.json");
    if output_json.exists() {
        fs::remove_file(&output_json).map_err(|_| failed("Cannot prepare the speech recognizer."))?;
    }
    let mut command = Command::new(program);
    command
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wave)
        .arg("-l")
        .arg(language)
        .args(["-oj", "-np", "-nt", "-sns", "-nf", "-t"])
        .arg(threads.clamp(1, 64).to_string())
        .arg("-of")
        .arg(output_base)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // whisper.cpp lavora su audio di altri: non eredita le chiavi della sincronizzazione.
        .env_remove("QAMILLE_SYNC_KEY")
        .env_remove("QAMILLE_SYNC_KEY_OLD");
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW, and out of the job of the local server where it allows it: its
        // per-process memory cap is sized for the embedding model, and whisper.cpp ran outside
        // it before the server (the small model needs about 850 MB).
        command.creation_flags(0x0800_0000 | 0x0100_0000);
        command.spawn().or_else(|_| {
            command.creation_flags(0x0800_0000);
            command.spawn()
        })
    };
    #[cfg(not(windows))]
    let spawned = command.spawn();
    let mut process = spawned.map_err(|_| failed("Cannot start the speech recognizer. Reinstall the model."))?;
    let began = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = process.kill();
            let _ = process.wait();
            return Err(SpeechError::Cancelled);
        }
        match process.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(failed("The speech recognizer did not complete this audio."));
                }
                break;
            }
            Ok(None) => {}
            Err(_) => {
                let _ = process.kill();
                let _ = process.wait();
                return Err(failed("Cannot read the state of the speech recognizer."));
            }
        }
        if began.elapsed() > WINDOW_LIMIT {
            let _ = process.kill();
            let _ = process.wait();
            return Err(failed("The audio took longer than 10 minutes to transcribe. Try a smaller model."));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    cancelled(cancel)?;
    let size = fs::metadata(&output_json).map_err(|_| failed("The speech recognizer returned no result."))?.len();
    if size > MAX_RESULT_BYTES {
        return Err(failed("The speech recognizer returned a result that is too large."));
    }
    let bytes = fs::read(&output_json).map_err(|_| failed("The speech recognizer returned no result."))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| failed("The speech recognizer returned an invalid result."))?;
    parse_output(&value)
}

/// The `-oj` output of `whisper-cli`: `{"transcription":[{"text","offsets":{"from","to"}}]}`.
/// Blank markers (`[BLANK_AUDIO]`, `[_…]`) and empty texts are left out.
pub fn parse_output(value: &serde_json::Value) -> Result<Vec<Segment>, SpeechError> {
    let failed = |message: &str| SpeechError::Failed(message.to_owned());
    let rows = value["transcription"].as_array().ok_or_else(|| failed("The speech recognizer returned no transcription."))?;
    let mut out = Vec::new();
    for row in rows {
        let text = row["text"].as_str().unwrap_or("").trim();
        if text.is_empty() || text.starts_with("[_") || text == "[BLANK_AUDIO]" {
            continue;
        }
        let from = row["offsets"]["from"].as_u64().ok_or_else(|| failed("The speech recognizer returned invalid timestamps."))?;
        let to = row["offsets"]["to"].as_u64().ok_or_else(|| failed("The speech recognizer returned invalid timestamps."))?;
        if to <= from {
            continue;
        }
        out.push(Segment { start_ms: from, end_ms: to, text: text.to_owned() });
    }
    Ok(out)
}

/// A RIFF/WAVE header: the only container `whisper-cli` reads here.
pub fn looks_like_wav(bytes: &[u8]) -> bool {
    bytes.len() > 44 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

/// Duration in seconds of a PCM or float WAV, for the quota and `verbose_json`. It comes from
/// what the decoder will really read: the rate, channels and bits per sample of the `fmt ` chunk
/// and the bytes actually present in `data` (never the `byte_rate` or the sizes the client
/// declares). `None` when the header does not allow that: the caller refuses the audio.
pub fn wav_seconds(bytes: &[u8]) -> Option<f64> {
    if !looks_like_wav(bytes) {
        return None;
    }
    let mut position = 12usize;
    let mut format: Option<(u32, u64)> = None; // (sample rate, bytes of one frame)
    while position.checked_add(8)? <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes(bytes[position + 4..position + 8].try_into().ok()?) as usize;
        let body = position + 8;
        if id == b"fmt " {
            if size < 16 || body.checked_add(16)? > bytes.len() {
                return None;
            }
            let tag = u16::from_le_bytes(bytes[body..body + 2].try_into().ok()?);
            let channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().ok()?) as u64;
            let rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
            let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().ok()?) as u64;
            // PCM, IEEE float, extensible: the encodings whose frame size is channels x bits.
            // Compressed ones (ADPCM...) would decode to more samples than their size says.
            if !matches!(tag, 1 | 3 | 0xFFFE) || channels == 0 || rate == 0 || bits == 0 || bits % 8 != 0 || bits > 64 {
                return None;
            }
            format = Some((rate, channels * bits / 8));
        } else if id == b"data" {
            let (rate, frame) = format?;
            let length = size.min(bytes.len().saturating_sub(body)) as u64;
            if length < frame {
                return None;
            }
            return Some((length / frame) as f64 / rate as f64);
        }
        position = body.checked_add(size)?.checked_add(size % 2)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_name_the_catalog_and_cannot_escape_the_folder() {
        assert_eq!(spec(CATALOG, "whisper-base").unwrap().size, "base");
        assert_eq!(spec(CATALOG, "small").unwrap().size, "small");
        assert!(spec(CATALOG, "../secret").is_none());
        assert!(spec(CATALOG, "whisper-huge").is_none());
        assert!(model_file(Path::new("models"), spec(CATALOG, "tiny").unwrap()).ends_with("ggml-tiny.bin"));
        assert_eq!(model_id(&CATALOG[1]), "whisper-base");
    }

    #[test]
    fn cli_output_keeps_spoken_rows_relative_to_the_audio() {
        let value = serde_json::json!({"transcription": [
            {"text": " ciao ", "offsets": {"from": 50, "to": 3000}},
            {"text": "[BLANK_AUDIO]", "offsets": {"from": 3000, "to": 4000}},
            {"text": "[_TT_]", "offsets": {"from": 4000, "to": 5000}},
            {"text": "vuoto", "offsets": {"from": 6000, "to": 6000}},
        ]});
        assert_eq!(parse_output(&value).unwrap(), vec![Segment { start_ms: 50, end_ms: 3000, text: "ciao".into() }]);
        assert!(parse_output(&serde_json::json!({"x": 1})).is_err());
        let bad = serde_json::json!({"transcription": [{"text": "a", "offsets": {"from": "x", "to": 1}}]});
        assert!(parse_output(&bad).is_err());
    }

    #[test]
    fn a_corrupt_model_is_rejected() {
        let dir = std::env::temp_dir().join(format!("qamille-speech-test-{}", unique_suffix()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("bad.bin");
        fs::write(&path, b"123").unwrap();
        assert!(matches!(verify(&path, 3, "0000", &AtomicBool::new(false)), Err(SpeechError::Missing(_))));
        assert!(matches!(verify(&path, 4, "0000", &AtomicBool::new(false)), Err(SpeechError::Missing(_))));
        let good = format!("{:x}", Sha256::digest(b"123"));
        assert!(verify(&path, 3, &good, &AtomicBool::new(false)).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wav_headers_give_the_duration() {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + 32_000).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&32_000u32.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, 32_000));
        assert!(looks_like_wav(&wav));
        assert_eq!(wav_seconds(&wav), Some(1.0));
        assert!(!looks_like_wav(b"ID3 not a wave file at all, but long enough to pass the size"));
    }

    #[test]
    fn languages_are_codes_or_auto() {
        for good in ["it", "pt-BR", "auto"] {
            assert!(valid_language(good));
        }
        for bad in ["", "it;rm", "../x", "it it", &"a".repeat(17)] {
            assert!(!valid_language(bad), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_runtime_in_the_models_folder_is_found_first() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("qamille-speech-runtime-{}", unique_suffix()));
        fs::create_dir(&dir).unwrap();
        let program = dir.join(PROGRAM);
        fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(locate_runtime(&dir).ok(), Some(program.clone()), "not executable: not a runtime");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(locate_runtime(&dir).unwrap(), program);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::*;
    use crate::download::Mirror;

    #[test]
    fn the_whisper_runtime_and_model_urls_go_through_the_mirror() {
        let mirror = Mirror::parse("https://mirror.example.com/base").unwrap();
        let runtime = mirror.map(RUNTIME_URL).unwrap();
        assert_eq!(runtime, "https://mirror.example.com/base/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip");
        let spec = &CATALOG[0];
        let model = mirror.map(&model_url(spec)).unwrap();
        assert_eq!(model, format!("https://mirror.example.com/base/ggerganov/whisper.cpp/resolve/{MODEL_REVISION}/ggml-{}.bin", spec.size));
    }

    /// A 16 kHz mono 16-bit WAV of `data` bytes whose header declares `byte_rate`.
    fn wav_with(byte_rate: u32, declared_data: u32, data: usize) -> Vec<u8> {
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16_000u32.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&declared_data.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, data));
        wav
    }

    #[test]
    fn the_duration_ignores_the_declared_byte_rate_and_sizes() {
        // 10 s of real samples: a false byte_rate (or none) does not shrink the charge.
        for byte_rate in [0, 1, 32_000, u32::MAX] {
            assert_eq!(wav_seconds(&wav_with(byte_rate, 320_000, 320_000)), Some(10.0), "{byte_rate}");
        }
        // A data size larger than the body is capped to the bytes present; a smaller one is kept.
        assert_eq!(wav_seconds(&wav_with(32_000, u32::MAX, 320_000)), Some(10.0));
        assert_eq!(wav_seconds(&wav_with(32_000, 64_000, 320_000)), Some(2.0));
    }

    #[test]
    fn a_header_that_gives_no_duration_is_refused() {
        let good = wav_with(32_000, 32_000, 32_000);
        assert_eq!(wav_seconds(&good), Some(1.0));
        // No `fmt ` before `data`.
        let mut no_fmt = good.clone();
        no_fmt[12..16].copy_from_slice(b"junk");
        assert_eq!(wav_seconds(&no_fmt), None);
        // Zero rate, zero channels, compressed encoding, no samples.
        let mut zero_rate = good.clone();
        zero_rate[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(wav_seconds(&zero_rate), None);
        let mut zero_channels = good.clone();
        zero_channels[22..24].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(wav_seconds(&zero_channels), None);
        let mut adpcm = good.clone();
        adpcm[20..22].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(wav_seconds(&adpcm), None);
        assert_eq!(wav_seconds(&wav_with(32_000, 0, 100)), None);
    }
}
