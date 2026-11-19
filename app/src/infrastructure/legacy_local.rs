//! What the local compute mode left in `data/` (wave 2, compito 5): a server that may still run,
//! its record and lock, the model folders and `qemille-server` (now `qamille-server`) next to the app. Read-only copy
//! of the old record; the server is stopped only after it answers with the recorded token.
use crate::application::legacy_local::LegacyLocalPort;
use crate::application::{AppError, AppResult};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DISCOVERY_FILE: &str = "compute-server.json";
const LOCK_FILE: &str = "compute-server.lock";
const MODEL_FOLDERS: [&str; 2] = ["models", "speech-models"];
/// The program names, before and after the rebranding to Qamille.
const SERVER_NAMES: [&str; 2] = ["qamille-server", "qemille-server"];
#[cfg(windows)]
const SERVER_EXES: [&str; 2] = ["qamille-server.exe", "qemille-server.exe"];
#[cfg(not(windows))]
const SERVER_EXES: [&str; 2] = SERVER_NAMES;

/// The record the old supervisor wrote (read only).
#[derive(Deserialize)]
struct Discovery {
    pid: u32,
    port: u16,
    token: String,
}

pub struct FileSystemLegacyLocal {
    data_dir: PathBuf,
}
impl FileSystemLegacyLocal {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { data_dir: data_dir.into() }
    }
}

/// True while `pid` is a running process.
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else { return false };
        if pid <= 0 {
            return false;
        }
        // Signal 0 checks existence and permission only.
        unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE as u32
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// True when a process image name (file name or full path) is the server: its file name starts
/// with `qamille-server` (or `qemille-server`, its name before the rebranding), so a recycled pid
/// of another program is never terminated.
pub(crate) fn is_server_image(image: &str) -> bool {
    let name = image.rsplit(['\\', '/']).next().unwrap_or(image);
    let name = name.to_ascii_lowercase();
    SERVER_NAMES.iter().any(|server| name.starts_with(server))
}

/// The image path of `pid`, when it can be read.
fn process_image(pid: u32) -> Option<String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) != 0;
        CloseHandle(handle);
        ok.then(|| String::from_utf16_lossy(&buffer[..len as usize]))
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

fn terminate(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 0 {
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

/// `GET /health` with the recorded token: the server answers and knows the token (it lists
/// `models`; one that does not know the token answers without them).
fn answers(discovery: &Discovery) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    let Ok(mut response) = agent
        .get(&format!("http://127.0.0.1:{}/health", discovery.port))
        .header("Authorization", &format!("Bearer {}", discovery.token))
        .call()
    else {
        return false;
    };
    response.status().as_u16() == 200
        && response.body_mut().read_json::<serde_json::Value>().is_ok_and(|value| value["models"].is_array())
}

fn folder_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => folder_bytes(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

impl LegacyLocalPort for FileSystemLegacyLocal {
    fn stop_leftover(&self) -> bool {
        let record = self.data_dir.join(DISCOVERY_FILE);
        let had_record = record.exists();
        if let Some(discovery) = std::fs::read(&record).ok().and_then(|bytes| serde_json::from_slice::<Discovery>(&bytes).ok())
            && process_alive(discovery.pid)
            && process_image(discovery.pid).is_some_and(|image| is_server_image(&image))
            && answers(&discovery)
        {
            terminate(discovery.pid);
        }
        let _ = std::fs::remove_file(&record);
        let _ = std::fs::remove_file(self.data_dir.join(LOCK_FILE));
        had_record
    }
    fn models_bytes(&self) -> u64 {
        MODEL_FOLDERS.iter().map(|folder| folder_bytes(&self.data_dir.join(folder))).sum()
    }
    fn free_models(&self) -> AppResult<u64> {
        let freed = self.models_bytes();
        for folder in MODEL_FOLDERS {
            let path = self.data_dir.join(folder);
            if path.exists() {
                std::fs::remove_dir_all(&path).map_err(|error| AppError::new("io", format!("cannot remove {folder}: {error}")))?;
            }
        }
        Ok(freed)
    }
    fn remove_old_program(&self) {
        if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
            for program in SERVER_EXES {
                // Fails while it runs (Windows): then it stays, and the next start tries again.
                let _ = std::fs::remove_file(dir.join(program));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_server_image;

    #[test]
    fn only_a_qamille_server_image_is_a_server() {
        assert!(is_server_image("C:\\Program Files\\Qamille\\qamille-server.exe"));
        assert!(is_server_image("/usr/bin/qamille-server"));
        assert!(is_server_image("QAMILLE-SERVER.EXE"));
        assert!(is_server_image("C:\\Program Files\\Qemille\\qemille-server.exe"));
        assert!(!is_server_image("C:\\Windows\\notepad.exe"));
        assert!(!is_server_image("/opt/qamille-server/other"));
        assert!(!is_server_image(""));
    }
}
