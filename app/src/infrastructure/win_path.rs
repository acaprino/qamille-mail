//! Win32 paths for raw API calls. `std::fs` adds the `\\?\` prefix to long paths
//! itself; direct calls such as MoveFileExW or FILE_RENAME_INFO must do the same,
//! or they fail once a path exceeds MAX_PATH (260 characters).
use std::{os::windows::ffi::OsStrExt, path::Path};

/// Absolute, `\\?\`-prefixed UTF-16 form of `path`, without a terminating NUL.
pub(crate) fn verbatim(path: &Path) -> std::io::Result<Vec<u16>> {
    let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    // GetFullPathNameW also normalizes separators and dot segments, which the
    // verbatim form no longer does.
    let absolute: Vec<u16> = std::path::absolute(path)?.as_os_str().encode_wide().collect();
    if absolute.starts_with(&wide(r"\\?\")) || absolute.starts_with(&wide(r"\\.\")) {
        return Ok(absolute);
    }
    let mut verbatim;
    if let Some(share) = absolute.strip_prefix(wide(r"\\").as_slice()) {
        verbatim = wide(r"\\?\UNC\");
        verbatim.extend_from_slice(share);
    } else {
        verbatim = wide(r"\\?\");
        verbatim.extend_from_slice(&absolute);
    }
    Ok(verbatim)
}

/// NUL-terminated directory for GetDiskFreeSpaceExW, which requires a trailing
/// backslash on UNC names and accepts one on every directory.
pub(crate) fn verbatim_directory(path: &Path) -> std::io::Result<Vec<u16>> {
    let mut directory = verbatim(path)?;
    if directory.last() != Some(&u16::from(b'\\')) {
        directory.push(u16::from(b'\\'));
    }
    directory.push(0);
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &str) -> String {
        String::from_utf16(&verbatim(Path::new(path)).unwrap()).unwrap()
    }

    #[test]
    fn drive_unc_and_verbatim_paths_get_one_long_path_prefix() {
        assert_eq!(text(r"C:\a\b"), r"\\?\C:\a\b");
        assert_eq!(text("C:/a/../b"), r"\\?\C:\b");
        assert_eq!(text(r"\\server\share\x"), r"\\?\UNC\server\share\x");
        assert_eq!(text(r"\\?\C:\a"), r"\\?\C:\a");
    }

    #[test]
    fn free_space_directories_end_with_one_backslash_and_a_nul() {
        let directory = |path: &str| {
            let wide = verbatim_directory(Path::new(path)).unwrap();
            assert_eq!(wide.last(), Some(&0));
            String::from_utf16(&wide[..wide.len() - 1]).unwrap()
        };
        assert_eq!(directory(r"C:\a"), r"\\?\C:\a\");
        assert_eq!(directory(r"C:\"), r"\\?\C:\");
        assert_eq!(directory(r"\\server\share"), r"\\?\UNC\server\share\");
    }
}
