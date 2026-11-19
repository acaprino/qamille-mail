//! Disk admission shared by workers. It never removes files to make room.
use crate::application::{AppError, AppResult};
use std::{path::Path, sync::Mutex};

const PHYSICAL_RESERVE: u64 = 200_000_000;
static RESERVED: Mutex<u64> = Mutex::new(0);

pub struct DiskReservation(u64);
impl Drop for DiskReservation {
    fn drop(&mut self) {
        if let Ok(mut reserved) = RESERVED.lock() {
            *reserved = reserved.saturating_sub(self.0);
        }
    }
}

pub fn reserve(path: &Path, bytes: u64) -> AppResult<DiskReservation> {
    reserve_with_margin(path, bytes, PHYSICAL_RESERVE)
}

/// Essential state commits may use the margin, but still need space for the
/// complete atomic replacement. A failed reservation leaves the source intact.
pub fn reserve_commit(path: &Path, bytes: u64) -> AppResult<DiskReservation> {
    reserve_with_margin(path, bytes, 0)
}

fn reserve_with_margin(path: &Path, bytes: u64, margin: u64) -> AppResult<DiskReservation> {
    let mut reserved = RESERVED
        .lock()
        .map_err(|_| AppError::storage("Disk reservation unavailable"))?;
    let required = reserved
        .checked_add(bytes)
        .and_then(|n| n.checked_add(margin))
        .ok_or_else(|| AppError::new("disk_full", "Not enough disk space for this operation"))?;
    if let Some(available) = available_bytes(path)? {
        if available < required {
            return Err(AppError::new(
                "disk_full",
                "Not enough free disk space. The operation is paused; no mail was removed.",
            ));
        }
    }
    *reserved += bytes;
    Ok(DiskReservation(bytes))
}

#[cfg(windows)]
fn available_bytes(path: &Path) -> AppResult<Option<u64>> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let mut existing = path;
    while !existing.is_dir() {
        existing = existing
            .parent()
            .ok_or_else(|| AppError::storage("Storage directory is unavailable"))?;
    }
    let directory = super::win_path::verbatim_directory(existing)?;
    let mut available = 0u64;
    // The UTF-16 path is NUL terminated; output points to a live u64. Optional
    // outputs are null, as documented by GetDiskFreeSpaceExW.
    let result = unsafe {
        GetDiskFreeSpaceExW(
            directory.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Some(available))
}

#[cfg(not(windows))]
fn available_bytes(_path: &Path) -> AppResult<Option<u64>> {
    // The shipped desktop target is Windows. Other adapters still surface write
    // failures and preserve atomic replacements; they need a native space probe.
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_is_rejected_without_claiming_space() {
        let _hold = reserve_commit(&std::env::temp_dir(), 1).unwrap();
        assert!(reserve_commit(&std::env::temp_dir(), u64::MAX).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn free_space_is_read_beyond_the_classic_windows_path_limit() {
        let root = std::env::temp_dir().join(format!(
            "qamille-space-{}-{}",
            std::process::id(),
            crate::model::now_ts()
        ));
        let deep = root.join("a".repeat(120)).join("b".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        assert!(deep.as_os_str().len() > 260);
        let available = available_bytes(&deep);
        let _ = std::fs::remove_dir_all(&root);
        assert!(available.unwrap().is_some_and(|bytes| bytes > 0));
    }
    #[cfg(windows)]
    #[test]
    fn impossible_allocation_is_rejected_before_creating_files() {
        assert_eq!(
            reserve(&std::env::temp_dir(), u64::MAX / 2)
                .err()
                .unwrap()
                .code,
            "disk_full"
        );
    }
}
