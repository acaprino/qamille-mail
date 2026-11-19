//! The llama.cpp engine (`llama-server`) that runs the built-in LLM models (spec L8, INV-39,
//! INV-40): the pin of the version, the verified Windows x64 bundle and the resolution of the
//! program the supervisor starts.
//!
//! The program is never looked up in the `PATH` and never in the models folder (except the
//! Windows bundle, which carries its own manifest and hashes, all re-checked at every start).
//! The pins were found with `compute/tools/llama-lookup.mjs`.

use crate::download::{self, DownloadError, Options, Target};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The llama.cpp release this server pins (a `bNNNN` tag, older than two weeks when pinned).
pub const LLAMA_TAG: &str = "b12081";
/// The commit of [`LLAMA_TAG`] (`git ls-remote`; the tag is lightweight).
pub const LLAMA_COMMIT: &str = "8fc0376721e9662d498a5706d7ee8e7574ce99f4";
/// SHA-256 of `https://github.com/ggml-org/llama.cpp/archive/<LLAMA_COMMIT>.tar.gz` (the Docker build).
pub const SOURCE_SHA256: &str = "75f4a0e0663d172ee75744179689139a8ed933bc169d1d1264fa701f802ea66a";

const RUNTIME_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/b12081/llama-b12081-bin-win-cpu-x64.zip";
pub const RUNTIME_BYTES: u64 = 24_613_997;
const RUNTIME_SHA: &str = "563d4800833c364be0a0ffc99bb90597c57c9c1b67a85fee593538ea0be8b347";
/// The folder of the bundle inside the models folder.
const RUNTIME_DIR: &str = "llama-b12081-win-x64";

/// Exactly the files `llama-server.exe` needs, found with the import tables of the program and of
/// each DLL of the archive (`llama-server.exe` -> `llama-server-impl.dll` -> `llama-common.dll`,
/// `mtmd.dll`, `llama.dll`, `ggml.dll`, `ggml-base.dll` -> `libomp.dll`), plus the `ggml-cpu-*.dll`
/// backends that ggml loads from the program folder and `libomp`'s licence. Nothing else of the
/// archive is installed, and a folder with another file is invalid: ggml loads every backend it
/// finds next to the program.
pub const RUNTIME_FILES: &[&str] = &[
    "LICENSE-LLVM-OpenMP",
    "ggml-base.dll",
    "ggml-cpu-alderlake.dll",
    "ggml-cpu-cannonlake.dll",
    "ggml-cpu-cascadelake.dll",
    "ggml-cpu-cooperlake.dll",
    "ggml-cpu-haswell.dll",
    "ggml-cpu-icelake.dll",
    "ggml-cpu-ivybridge.dll",
    "ggml-cpu-piledriver.dll",
    "ggml-cpu-sandybridge.dll",
    "ggml-cpu-sapphirerapids.dll",
    "ggml-cpu-skylakex.dll",
    "ggml-cpu-sse42.dll",
    "ggml-cpu-x64.dll",
    "ggml-cpu-zen4.dll",
    "ggml.dll",
    "libomp.dll",
    "llama-common.dll",
    "llama-server-impl.dll",
    "llama-server.exe",
    "llama.dll",
    "mtmd.dll",
];

/// The listed files add up to about 40 MB; anything past this is not the engine.
const MAX_UNPACKED_BYTES: u64 = 128 * 1024 * 1024;

pub const RUNTIME_MISSING_MESSAGE: &str = "Install the assistant model: it includes the llama.cpp engine.";

/// Where the Linux image puts the engine.
pub const FIXED_PROGRAM: &str = "/usr/local/bin/llama-server";

#[cfg(windows)]
pub const PROGRAM: &str = "llama-server.exe";
#[cfg(not(windows))]
pub const PROGRAM: &str = "llama-server";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    WindowsX64,
    Linux,
    Other,
}

pub fn current_platform() -> Platform {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Platform::WindowsX64
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}

/// This platform runs the downloaded bundle.
pub fn bundled_runtime() -> bool {
    current_platform() == Platform::WindowsX64
}

#[derive(Debug, PartialEq, Eq)]
pub enum LlamaError {
    /// There is no engine to run.
    Missing(String),
    /// There is one, but it is not what the server accepts.
    Invalid(String),
}

impl std::fmt::Display for LlamaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlamaError::Missing(text) | LlamaError::Invalid(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for LlamaError {}

/// The folder of the bundle inside the models folder.
pub fn runtime_dir(models_dir: &Path) -> PathBuf {
    models_dir.join(RUNTIME_DIR)
}

/// The program the supervisor starts. Windows: the verified bundle in `runtime_dir` (the setting is
/// ignored). Linux: `setting`, else [`FIXED_PROGRAM`]. Never the `PATH`.
pub fn resolve_program(platform: Platform, setting: Option<&Path>, runtime_dir: &Path) -> Result<PathBuf, LlamaError> {
    resolve_program_at(platform, setting, runtime_dir, Path::new(FIXED_PROGRAM))
}

fn resolve_program_at(platform: Platform, setting: Option<&Path>, runtime_dir: &Path, fixed: &Path) -> Result<PathBuf, LlamaError> {
    match platform {
        Platform::WindowsX64 => verified_bundle(runtime_dir),
        Platform::Linux => checked_program(setting.unwrap_or(fixed)),
        Platform::Other => Err(LlamaError::Missing("The built-in assistant is not supported on this platform.".to_owned())),
    }
}

/// An absolute regular file that is executable and that nobody but its owner can write.
fn checked_program(path: &Path) -> Result<PathBuf, LlamaError> {
    if !path.is_absolute() {
        return Err(LlamaError::Invalid("The llama.cpp program must be an absolute path.".to_owned()));
    }
    let meta = fs::metadata(path).map_err(|_| LlamaError::Missing(format!("The llama.cpp program was not found at {}.", path.display())))?;
    if !meta.is_file() {
        return Err(LlamaError::Invalid(format!("{} is not a program file.", path.display())));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(LlamaError::Invalid(format!("{} is not executable.", path.display())));
        }
        if mode & 0o022 != 0 {
            return Err(LlamaError::Invalid(format!("{} can be written by others; refusing to run it.", path.display())));
        }
    }
    Ok(path.to_owned())
}

// --- The Windows bundle -----------------------------------------------------------

fn bundle_invalid() -> LlamaError {
    LlamaError::Missing(RUNTIME_MISSING_MESSAGE.to_owned())
}

/// `llama-server.exe` of the bundle in `runtime_dir`, after the SHA-256 of **every** listed file
/// was recomputed against `upstream.json`. A file that is missing, changed or not in the manifest
/// (apart from `upstream.json` and the licence) makes the bundle invalid.
pub fn verified_bundle(runtime_dir: &Path) -> Result<PathBuf, LlamaError> {
    verified_bundle_with(runtime_dir, RUNTIME_FILES, "llama-server.exe")
}

fn verified_bundle_with(dir: &Path, files: &[&str], program: &str) -> Result<PathBuf, LlamaError> {
    let manifest: serde_json::Value = fs::read(dir.join("upstream.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(bundle_invalid)?;
    let listed = manifest["files"].as_array().ok_or_else(bundle_invalid)?;
    let mut seen = Vec::new();
    for file in listed {
        let name = file["name"].as_str().ok_or_else(bundle_invalid)?;
        // A bare name that the code itself lists: a manifest cannot add a file.
        if Path::new(name).file_name().and_then(|s| s.to_str()) != Some(name) || !files.contains(&name) || seen.contains(&name) {
            return Err(bundle_invalid());
        }
        let mut hasher = Sha256::new();
        let mut input = File::open(dir.join(name)).map_err(|_| bundle_invalid())?;
        let mut buffer = vec![0_u8; 1 << 20];
        loop {
            let n = input.read(&mut buffer).map_err(|_| bundle_invalid())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        if download::hash_hex(hasher) != file["sha256"].as_str().unwrap_or("") {
            return Err(bundle_invalid());
        }
        seen.push(name);
    }
    if files.iter().any(|name| !seen.contains(name)) {
        return Err(bundle_invalid());
    }
    // ggml loads every backend it finds next to the program: nothing may be added.
    for entry in fs::read_dir(dir).map_err(|_| bundle_invalid())? {
        let name = entry.map_err(|_| bundle_invalid())?.file_name();
        let name = name.to_str().ok_or_else(bundle_invalid)?;
        if !files.contains(&name) && name != "upstream.json" && name != "LLAMA_CPP_LICENSE.txt" {
            return Err(bundle_invalid());
        }
    }
    Ok(dir.join(program))
}

/// A name no other file of this process or another one uses.
fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos:x}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Downloads and unpacks the pinned Windows bundle into `runtime_dir`. Only on [`Platform::WindowsX64`].
/// A bundle that is already valid is kept.
pub fn install_runtime(runtime_dir: &Path, options: &Options, cancel: &AtomicBool, progress: &dyn Fn(u64)) -> Result<(), String> {
    if !bundled_runtime() {
        return Err("The llama.cpp engine is only downloaded on Windows x64.".to_owned());
    }
    install_runtime_from(RUNTIME_URL, RUNTIME_BYTES, RUNTIME_SHA, RUNTIME_FILES, "llama-server.exe", runtime_dir, options, cancel, progress)
}

/// As [`install_runtime`], with the archive and the file list given (tests).
#[allow(clippy::too_many_arguments)]
fn install_runtime_from(
    url: &str,
    bytes: u64,
    sha256: &str,
    files: &[&str],
    program: &str,
    runtime_dir: &Path,
    options: &Options,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(), String> {
    if verified_bundle_with(runtime_dir, files, program).is_ok() {
        return Ok(());
    }
    let io = |error: std::io::Error| format!("Cannot install the llama.cpp engine: {}", error.kind());
    let parent = runtime_dir.parent().ok_or_else(|| "Cannot install the llama.cpp engine: no folder.".to_owned())?;
    fs::create_dir_all(parent).map_err(io)?;
    download::ensure_space(parent, bytes + MAX_UNPACKED_BYTES).map_err(String::from)?;
    let local = format!("llama-runtime-{}.zip", unique_suffix());
    let archive = parent.join(&local);
    let options = Options { resume: false, ..options.clone() };
    let target = Target { name: "llama.cpp engine", size: bytes, sha256 };
    let fetched = download::fetch(&download::agent(), url, parent, &local, &target, &options, cancel, progress);
    let result = match fetched {
        Ok(()) => unpack_runtime(&archive, runtime_dir, files, program, MAX_UNPACKED_BYTES, cancel),
        Err(DownloadError::Cancelled) => Err("Download cancelled.".to_owned()),
        Err(error) => Err(error.to_string()),
    };
    let _ = fs::remove_file(&archive);
    result
}

fn unpack_runtime(archive_path: &Path, runtime_dir: &Path, files: &[&str], program: &str, limit: u64, cancel: &AtomicBool) -> Result<(), String> {
    let io = |error: std::io::Error| format!("Cannot install the llama.cpp engine: {}", error.kind());
    let parent = runtime_dir.parent().ok_or_else(|| "Cannot install the llama.cpp engine: no folder.".to_owned())?;
    let staging = parent.join(format!("llama-stage-{}", unique_suffix()));
    fs::create_dir(&staging).map_err(io)?;
    let _cleanup = StagingCleanup(staging.clone());
    let mut archive = zip::ZipArchive::new(File::open(archive_path).map_err(io)?).map_err(|_| "The llama.cpp engine archive is not valid.".to_owned())?;
    let mut written = 0_u64;
    let mut manifest = Vec::new();
    for index in 0..archive.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Download cancelled.".to_owned());
        }
        let mut entry = archive.by_index(index).map_err(|_| "The llama.cpp engine archive cannot be read.".to_owned())?;
        let raw = entry.name().to_owned();
        if raw.contains(':') || raw.contains('\\') || raw.starts_with('/') || raw.split('/').any(|part| part == "..") {
            return Err("The llama.cpp engine archive has an invalid path.".to_owned());
        }
        if entry.is_dir() {
            continue;
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| "The llama.cpp engine archive has an invalid path.".to_owned())?;
        let name = enclosed.file_name().and_then(|s| s.to_str()).ok_or_else(|| "The llama.cpp engine archive has an invalid file name.".to_owned())?.to_owned();
        if !files.contains(&name.as_str()) {
            continue;
        }
        if manifest.iter().any(|(seen, _): &(String, String)| *seen == name) {
            return Err("The llama.cpp engine archive lists a file twice.".to_owned());
        }
        written = written.saturating_add(entry.size());
        if written > limit {
            return Err("The llama.cpp engine archive exceeds the expected size.".to_owned());
        }
        let mut output = OpenOptions::new().create_new(true).write(true).open(staging.join(&name)).map_err(io)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1 << 20];
        let mut copied = 0_u64;
        loop {
            let n = entry.read(&mut buffer).map_err(io)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > entry.size() {
                return Err("The llama.cpp engine archive exceeds the expected size.".to_owned());
            }
            std::io::Write::write_all(&mut output, &buffer[..n]).map_err(io)?;
            hasher.update(&buffer[..n]);
        }
        output.sync_all().map_err(io)?;
        manifest.push((name, download::hash_hex(hasher)));
    }
    if let Some(missing) = files.iter().find(|name| !manifest.iter().any(|(seen, _)| seen == *name)) {
        return Err(if *missing == program {
            format!("The download does not contain {program}.")
        } else {
            format!("The download does not contain {missing}.")
        });
    }
    manifest.sort();
    let listing: Vec<_> = manifest.iter().map(|(name, sha256)| serde_json::json!({"name": name, "sha256": sha256})).collect();
    let upstream = serde_json::json!({
        "project": "https://github.com/ggml-org/llama.cpp", "release": LLAMA_TAG, "commit": LLAMA_COMMIT,
        "archive_sha256": RUNTIME_SHA, "license": "MIT", "files": listing
    });
    fs::write(staging.join("upstream.json"), serde_json::to_vec_pretty(&upstream).map_err(|e| e.to_string())?).map_err(io)?;
    fs::write(staging.join("LLAMA_CPP_LICENSE.txt"), include_str!("LLAMA_CPP_LICENSE.txt")).map_err(io)?;
    // The new bundle is complete: only now does it replace the previous one.
    let previous = parent.join(format!("llama-previous-{}", unique_suffix()));
    let had_previous = runtime_dir.exists();
    if had_previous {
        fs::rename(runtime_dir, &previous).map_err(io)?;
    }
    if let Err(error) = fs::rename(&staging, runtime_dir) {
        if had_previous {
            let _ = fs::rename(&previous, runtime_dir);
        }
        return Err(io(error));
    }
    if had_previous {
        let _ = fs::remove_dir_all(&previous);
    }
    Ok(())
}

struct StagingCleanup(PathBuf);
impl Drop for StagingCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::sync::{Arc, Mutex};

    const NAMES: &[&str] = &["tool.exe", "a.dll", "b.dll"];

    fn temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qamille-llama-{label}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn good_zip() -> Vec<u8> {
        zip_of(&[("tool.exe", b"program"), ("a.dll", b"alpha"), ("b.dll", b"beta"), ("extra.dll", b"not listed"), ("README.md", b"x")])
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Unpacks `bytes` into a fresh folder with the synthetic file list.
    fn unpack(bytes: &[u8]) -> (PathBuf, Result<(), String>) {
        let root = temp("unpack");
        let archive = root.join("in.zip");
        fs::write(&archive, bytes).unwrap();
        let result = unpack_runtime(&archive, &root.join("bundle"), NAMES, "tool.exe", MAX_UNPACKED_BYTES, &AtomicBool::new(false));
        (root, result)
    }

    fn bundle() -> (PathBuf, PathBuf) {
        let (root, result) = unpack(&good_zip());
        result.unwrap();
        let dir = root.join("bundle");
        (root, dir)
    }

    // The tests that run the real bundle are `the_real_bundle_answers_help` (ignored).

    #[test]
    fn engine_pins_are_set() {
        assert!(LLAMA_TAG.len() > 1 && LLAMA_TAG.starts_with('b') && LLAMA_TAG[1..].chars().all(|c| c.is_ascii_digit()),
            "llama.cpp tag not pinned: owner decision on the llama.cpp release is missing");
        let hex = |value: &str, len: usize| value.len() == len && value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
        assert!(hex(LLAMA_COMMIT, 40), "llama.cpp commit not pinned: owner decision on the llama.cpp release is missing");
        assert!(hex(SOURCE_SHA256, 64), "llama.cpp source SHA-256 not pinned: owner decision on the llama.cpp release is missing");
        assert!(hex(RUNTIME_SHA, 64), "llama.cpp Windows SHA-256 not pinned: owner decision on the llama.cpp release is missing");
        assert!(RUNTIME_BYTES > 0 && RUNTIME_FILES.contains(&"llama-server.exe") && !RUNTIME_FILES.is_empty());
        assert!(RUNTIME_URL.contains(LLAMA_TAG) && RUNTIME_DIR.contains(LLAMA_TAG));
        assert!(RUNTIME_FILES.windows(2).all(|pair| pair[0] < pair[1]), "sorted and without duplicates");
    }

    #[cfg(unix)]
    fn script(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_program_is_never_searched_in_the_path_or_the_models_folder() {
        let models = temp("never");
        let on_path = temp("never-path");
        script(&models.join(PROGRAM), 0o755);
        script(&on_path.join(PROGRAM), 0o755);
        let old = std::env::var_os("PATH");
        // SAFETY: only this test of the crate touches PATH, and it restores it.
        unsafe { std::env::set_var("PATH", &on_path) };
        let found = resolve_program_at(Platform::Linux, None, &models, &models.join("absent-fixed"));
        if let Some(old) = old {
            unsafe { std::env::set_var("PATH", old) };
        }
        assert!(matches!(found, Err(LlamaError::Missing(_))), "{found:?}");
    }

    #[cfg(unix)]
    #[test]
    fn linux_uses_the_setting_when_present() {
        let dir = temp("setting");
        let program = dir.join("my-llama");
        script(&program, 0o755);
        let fixed = dir.join("fixed");
        script(&fixed, 0o755);
        assert_eq!(resolve_program_at(Platform::Linux, Some(&program), &dir, &fixed).unwrap(), program);
    }

    #[cfg(unix)]
    #[test]
    fn linux_uses_the_fixed_path_without_a_setting() {
        let dir = temp("fixed");
        let fixed = dir.join("fixed");
        script(&fixed, 0o755);
        assert_eq!(resolve_program_at(Platform::Linux, None, &dir, &fixed).unwrap(), fixed);
    }

    #[test]
    fn a_relative_setting_is_refused() {
        let result = resolve_program_at(Platform::Linux, Some(Path::new("bin/llama-server")), Path::new("x"), Path::new("/nonexistent/fixed"));
        assert!(matches!(result, Err(LlamaError::Invalid(_))), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_program_writable_by_others_is_refused() {
        let dir = temp("writable");
        for mode in [0o777, 0o775, 0o757] {
            let program = dir.join("p");
            script(&program, mode);
            let result = resolve_program_at(Platform::Linux, Some(&program), &dir, &dir.join("fixed"));
            assert!(matches!(result, Err(LlamaError::Invalid(_))), "mode {mode:o}: {result:?}");
        }
        let program = dir.join("plain");
        script(&program, 0o644);
        assert!(matches!(resolve_program_at(Platform::Linux, Some(&program), &dir, &dir.join("fixed")), Err(LlamaError::Invalid(_))), "not executable");
    }

    #[test]
    fn a_directory_or_a_missing_file_is_refused() {
        let dir = temp("dir");
        let as_dir = resolve_program_at(Platform::Linux, Some(&dir), &dir, &dir.join("fixed"));
        assert!(matches!(as_dir, Err(LlamaError::Invalid(_))), "{as_dir:?}");
        let missing = resolve_program_at(Platform::Linux, Some(&dir.join("nothing")), &dir, &dir.join("fixed"));
        assert!(matches!(missing, Err(LlamaError::Missing(_))), "{missing:?}");
    }

    #[test]
    fn windows_ignores_the_setting_and_needs_the_bundle() {
        let dir = temp("windows");
        let setting = dir.join("elsewhere.exe");
        fs::write(&setting, "x").unwrap();
        let result = resolve_program_at(Platform::WindowsX64, Some(&setting), &dir.join(RUNTIME_DIR), Path::new("/x"));
        assert_eq!(result, Err(LlamaError::Missing(RUNTIME_MISSING_MESSAGE.to_owned())));
    }

    #[test]
    fn other_platforms_are_not_supported() {
        let result = resolve_program_at(Platform::Other, Some(Path::new("/usr/bin/x")), Path::new("x"), Path::new("/usr/bin/x"));
        assert!(matches!(result, Err(LlamaError::Missing(_))));
    }

    #[test]
    fn the_bundle_needs_every_listed_file_with_its_recorded_hash() {
        let (_root, dir) = bundle();
        assert_eq!(verified_bundle_with(&dir, NAMES, "tool.exe").unwrap(), dir.join("tool.exe"));
        fs::remove_file(dir.join("b.dll")).unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err(), "a missing file");
        let (_root, dir) = bundle();
        assert!(verified_bundle_with(&dir, &["tool.exe", "a.dll", "b.dll", "c.dll"], "tool.exe").is_err(), "a file the code lists but the manifest does not");
        let manifest = fs::read_to_string(dir.join("upstream.json")).unwrap().replace("a.dll", "../a.dll");
        fs::write(dir.join("upstream.json"), manifest).unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err(), "a name with a path");
    }

    #[test]
    fn a_changed_dll_invalidates_the_bundle() {
        let (_root, dir) = bundle();
        verified_bundle_with(&dir, NAMES, "tool.exe").unwrap();
        fs::write(dir.join("a.dll"), b"alphA").unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err());
    }

    #[test]
    fn an_unlisted_file_in_the_bundle_folder_invalidates_it() {
        let (_root, dir) = bundle();
        fs::write(dir.join("ggml-cuda.dll"), b"a backend ggml would load").unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err());
        fs::remove_file(dir.join("ggml-cuda.dll")).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err(), "a sub-folder too");
    }

    #[test]
    fn the_bundle_is_rechecked_at_every_call() {
        let (_root, dir) = bundle();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_ok());
        fs::write(dir.join("b.dll"), b"changed after the first call").unwrap();
        assert!(verified_bundle_with(&dir, NAMES, "tool.exe").is_err());
    }

    #[test]
    fn unpack_keeps_only_the_listed_files() {
        let (_root, dir) = bundle();
        let mut names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["LLAMA_CPP_LICENSE.txt", "a.dll", "b.dll", "tool.exe", "upstream.json"]);
        assert_eq!(fs::read(dir.join("a.dll")).unwrap(), b"alpha");
    }

    #[test]
    fn unpack_rejects_paths_with_dotdot() {
        for name in ["../tool.exe", "x/../../tool.exe", "/tool.exe", "C:evil.dll", "..\\tool.exe"] {
            let archive = zip_of(&[("tool.exe", b"p"), ("a.dll", b"a"), ("b.dll", b"b"), (name, b"evil")]);
            let (root, result) = unpack(&archive);
            assert!(result.is_err(), "{name}");
            assert!(!root.join("bundle").exists(), "{name}: nothing installed");
            assert!(!root.join("tool.exe").exists() && !root.parent().unwrap().join("tool.exe").exists(), "{name}: nothing written outside");
        }
    }

    #[test]
    fn unpack_stops_over_the_size_limit() {
        let root = temp("limit");
        fs::write(root.join("in.zip"), good_zip()).unwrap();
        // tool.exe, a.dll and b.dll are 7 + 5 + 4 = 16 bytes.
        let run = |limit| unpack_runtime(&root.join("in.zip"), &root.join(format!("bundle{limit}")), NAMES, "tool.exe", limit, &AtomicBool::new(false));
        assert_eq!(run(15), Err("The llama.cpp engine archive exceeds the expected size.".to_owned()));
        assert!(!root.join("bundle15").exists());
        assert!(run(16).is_ok());
        const { assert!(MAX_UNPACKED_BYTES >= 3 * RUNTIME_BYTES && MAX_UNPACKED_BYTES <= 256 * 1024 * 1024) };
    }

    #[test]
    fn unpack_needs_the_program() {
        let archive = zip_of(&[("a.dll", b"a"), ("b.dll", b"b")]);
        let (root, result) = unpack(&archive);
        assert_eq!(result, Err("The download does not contain tool.exe.".to_owned()));
        assert!(!root.join("bundle").exists());
    }

    #[test]
    fn unpack_writes_a_manifest_with_tag_license_and_a_hash_per_file() {
        let (_root, dir) = bundle();
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(dir.join("upstream.json")).unwrap()).unwrap();
        assert_eq!(manifest["release"], LLAMA_TAG);
        assert_eq!(manifest["commit"], LLAMA_COMMIT);
        assert_eq!(manifest["license"], "MIT");
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        let alpha = files.iter().find(|f| f["name"] == "a.dll").unwrap();
        assert_eq!(alpha["sha256"], sha(b"alpha"));
        assert!(fs::read_to_string(dir.join("LLAMA_CPP_LICENSE.txt")).unwrap().starts_with("MIT License"));
    }

    /// Serves `payload` for any path and records each request path.
    fn serve(payload: Vec<u8>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let paths: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = paths.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                log.lock().unwrap().push(first.split_whitespace().nth(1).unwrap_or("").to_owned());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line.trim_end().is_empty() {
                        break;
                    }
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });
        (format!("http://{addr}"), paths)
    }

    const ARCHIVE_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/bX/llama.zip";

    fn install(root: &Path, payload: &[u8], url: &str, options: &Options) -> Result<(), String> {
        install_runtime_from(url, payload.len() as u64, &sha(payload), NAMES, "tool.exe", &root.join("bundle"), options, &AtomicBool::new(false), &|_| {})
    }

    #[test]
    fn install_runtime_downloads_through_the_mirror_and_checks_size_and_hash() {
        let payload = good_zip();
        let (base, paths) = serve(payload.clone());
        let mirror = download::Mirror::parse(&format!("{base}/mirror")).unwrap();
        let options = Options::one_shot().with_mirror(Some(mirror));
        let root = temp("install");
        install(&root, &payload, ARCHIVE_URL, &options).unwrap();
        assert_eq!(paths.lock().unwrap().as_slice(), ["/mirror/ggml-org/llama.cpp/releases/download/bX/llama.zip"]);
        verified_bundle_with(&root.join("bundle"), NAMES, "tool.exe").unwrap();
        assert!(fs::read_dir(&root).unwrap().all(|e| e.unwrap().file_name() == "bundle"), "no archive or staging left");
        // A valid bundle is not downloaded again.
        install(&root, &payload, ARCHIVE_URL, &options).unwrap();
        assert_eq!(paths.lock().unwrap().len(), 1);
        // A mirror that serves other bytes is refused, and nothing is installed.
        let other = temp("install-other");
        let wrong = Options::one_shot().with_mirror(download::Mirror::parse(&serve(vec![7_u8; payload.len()]).0).ok());
        let error = install(&other, &payload, ARCHIVE_URL, &wrong).unwrap_err();
        assert!(error.contains("integrity") || error.contains("size"), "{error}");
        assert!(fs::read_dir(&other).unwrap().next().is_none(), "no bundle, no archive");
    }

    #[test]
    fn a_failed_install_leaves_the_previous_bundle_in_place() {
        let (root, dir) = bundle();
        fs::write(dir.join("a.dll"), b"damaged").unwrap();
        // The old bundle is damaged but its files are still in place; the new archive lacks b.dll.
        let broken = zip_of(&[("tool.exe", b"p"), ("a.dll", b"a")]);
        let (base, _) = serve(broken.clone());
        let options = Options::one_shot().with_mirror(download::Mirror::parse(&base).ok());
        let error = install(&root, &broken, ARCHIVE_URL, &options).unwrap_err();
        assert!(error.contains("b.dll"), "{error}");
        assert_eq!(fs::read(dir.join("a.dll")).unwrap(), b"damaged", "the previous folder was not touched");
        assert!(fs::read_dir(&root).unwrap().all(|e| matches!(e.unwrap().file_name().to_str(), Some("bundle" | "in.zip"))), "no staging left");
        // A good archive replaces it.
        let good = good_zip();
        let (base, _) = serve(good.clone());
        let options = Options::one_shot().with_mirror(download::Mirror::parse(&base).ok());
        install(&root, &good, ARCHIVE_URL, &options).unwrap();
        verified_bundle_with(&dir, NAMES, "tool.exe").unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "downloads the real bundle (or uses QAMILLE_REAL_LLAMA_DIR, the folder of an installed one)"]
    fn the_real_bundle_answers_help() {
        let dir = match std::env::var_os("QAMILLE_REAL_LLAMA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => {
                let dir = runtime_dir(&temp("real"));
                install_runtime(&dir, &Options::one_shot(), &AtomicBool::new(false), &|_| {}).expect("the real bundle installs");
                dir
            }
        };
        let program = verified_bundle(&dir).expect("a verified bundle");
        let output = std::process::Command::new(program).arg("--help").env_clear().env("SystemRoot", std::env::var_os("SystemRoot").unwrap()).output().unwrap();
        let help = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
        for flag in ["--model", "--host", "--port", "--ctx-size", "--parallel", "--cache-ram", "--no-slots", "--jinja", "--no-webui", "--log-verbosity", "LLAMA_API_KEY"] {
            assert!(help.contains(flag), "{flag} missing from --help");
        }
    }
}
