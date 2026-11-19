//! Memory and disk probes of the machine that runs the model (spec D1: the "available memory
//! before loading the model" check lives in the server).

use std::path::Path;

/// The process limit the desktop app applies to the local server (Windows Job Object).
pub const MEMORY_LIMIT_MB: usize = 1024;

/// Available memory in MiB, or `None` where the platform gives no reliable figure.
pub fn available_memory_mb() -> Result<Option<u64>, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        memory.dwLength = std::mem::size_of_val(&memory) as u32;
        if unsafe { GlobalMemoryStatusEx(&mut memory) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // Available physical RAM alone is insufficient when the Windows commit
        // limit is nearly exhausted (for example a small paging file).
        Ok(Some(memory.ullAvailPhys.min(memory.ullAvailPageFile) / 1024 / 1024))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(std::fs::read_to_string("/proc/meminfo").ok().as_deref().and_then(mem_available_mb))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(None)
    }
}

/// `MemAvailable` of `/proc/meminfo`, in MiB.
pub fn mem_available_mb(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let rest = line.strip_prefix("MemAvailable:")?;
        let kib: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
        Some(kib / 1024)
    })
}

/// Free disk space for the folder (or its nearest existing parent), where the platform tells.
pub fn available_disk_bytes(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut existing = path;
        while !existing.is_dir() {
            existing = existing.parent()?;
        }
        let absolute = std::path::absolute(existing).ok()?;
        let mut wide: Vec<u16> = absolute.as_os_str().encode_wide().collect();
        // Verbatim form: free of the classic 260-character limit of portable folders.
        if !absolute.as_os_str().to_string_lossy().starts_with(r"\\") {
            let mut prefixed: Vec<u16> = r"\\?\".encode_utf16().collect();
            prefixed.extend(wide);
            wide = prefixed;
        }
        wide.push(0);
        let mut available = 0u64;
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (result != 0).then_some(available)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meminfo_available_is_read_in_mib() {
        let sample = "MemTotal:       16471956 kB\nMemFree:        11305912 kB\nMemAvailable:   15019616 kB\n";
        assert_eq!(mem_available_mb(sample), Some(14667));
        assert_eq!(mem_available_mb("MemTotal: 1 kB\n"), None);
    }
    #[test]
    fn the_probe_never_fails_on_this_host() {
        assert!(available_memory_mb().is_ok());
    }
}
