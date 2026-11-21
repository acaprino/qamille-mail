//! Saved attachments carry the Mark-of-the-Web, so Windows warns before a dangerous one is opened
//! (SmartScreen, Office Protected View, the "file came from another computer" prompt).

use crate::native_feel;
use std::io;
use std::path::Path;

/// Identifies Qamille to Attachment Services, which keeps per-client trust decisions under it.
#[cfg(windows)]
const QAMILLE_CLIENT: windows::core::GUID = windows::core::GUID::from_u128(0x54bc681d_1756_43ae_86c6_1a05d0f6ece4);

/// The Internet zone, for files without a web origin (a mail attachment): the URL Windows maps to zone 3.
#[cfg(windows)]
const INTERNET_SOURCE: &str = "about:internet";

/// Writes the file, then marks it as coming from the Internet zone. Only a failure to write the file, or
/// Windows security removing it while it is marked, fails the save: a zone that cannot be written is
/// reported to `native_feel::report` and the file stays.
pub fn save_downloaded(path: &Path, bytes: &[u8], source_url: Option<&str>) -> io::Result<()> {
    save_downloaded_with(path, bytes, |path| mark_from_internet(path, source_url))
}

fn save_downloaded_with(path: &Path, bytes: &[u8], zone: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    std::fs::write(path, bytes)?;
    if let Err(error) = zone(path) {
        native_feel::report(&format!("saved file left without its Internet zone: {error}"));
    }
    // Attachment Services hand the file to the antivirus and the attachment policy, which may delete it.
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Windows security removed the saved file"));
    }
    Ok(())
}

/// Attachment Services first (zone, antivirus scan, attachment policy), the bare `Zone.Identifier`
/// stream when COM cannot do it.
fn mark_from_internet(path: &Path, source_url: Option<&str>) -> io::Result<()> {
    #[cfg(windows)]
    {
        let Err(error) = attachment_services(path, source_url) else {
            return Ok(());
        };
        // Writing the stream of a file the antivirus removed would bring it back, empty.
        if !path.exists() {
            return Err(io::Error::other(format!("Attachment Services removed the file: {error}")));
        }
        native_feel::report(&format!("Attachment Services unavailable, writing Zone.Identifier: {error}"));
        write_zone_identifier(path)
    }
    #[cfg(not(windows))]
    {
        let _ = (path, source_url);
        Ok(())
    }
}

#[cfg(windows)]
fn attachment_services(path: &Path, source_url: Option<&str>) -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{AttachmentServices, IAttachmentExecute};
    use windows::core::HSTRING;

    let _apartment = Apartment::enter();
    // An empty or missing source makes Save fail or pick no zone: a mail attachment has no URL.
    let source = source_url.filter(|url| !url.trim().is_empty()).unwrap_or(INTERNET_SOURCE);
    unsafe {
        let execute: IAttachmentExecute = CoCreateInstance(&AttachmentServices, None, CLSCTX_INPROC_SERVER)?;
        execute.SetClientGuid(&QAMILLE_CLIENT)?;
        execute.SetLocalPath(&HSTRING::from(path))?;
        execute.SetSource(&HSTRING::from(source))?;
        execute.Save()
    }
}

/// COM on the calling thread (a pooled blocking thread) for as long as the guard lives.
#[cfg(windows)]
struct Apartment {
    owned: bool,
}

#[cfg(windows)]
impl Apartment {
    fn enter() -> Self {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
        // S_OK and S_FALSE each take one reference to give back. RPC_E_CHANGED_MODE: the thread already
        // has COM in the other apartment, usable as it is and not ours to close.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        Apartment { owned: result.is_ok() }
    }
}

#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.owned {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Marks the file as coming from the Internet zone without COM: the `Zone.Identifier` stream that
/// Attachment Services would write. Fails on a volume without alternate data streams (FAT32, exFAT).
pub fn write_zone_identifier(path: &Path) -> io::Result<()> {
    // Opening the stream of a missing file would create the file.
    std::fs::metadata(path)?;
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A folder owned by one test, under the ignored `desktop/.cache`, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.cache/zone-tests")
                .join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("test folder");
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    fn zone_stream(file: &Path) -> String {
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        std::fs::read_to_string(stream).expect("Zone.Identifier stream")
    }

    #[cfg(windows)]
    #[test]
    fn a_saved_file_carries_zone_3() {
        let scratch = Scratch::new("zone-3");
        let file = scratch.0.join("invoice.docx");
        std::fs::write(&file, b"PK").expect("file");
        write_zone_identifier(&file).expect("zone");
        assert!(zone_stream(&file).contains("ZoneId=3"));
        assert_eq!(std::fs::read(&file).expect("file"), b"PK");
    }

    #[cfg(windows)]
    #[test]
    fn attachment_services_mark_a_saved_attachment_as_internet() {
        let scratch = Scratch::new("attachment-services");
        let file = scratch.0.join("invoice.docx");
        std::fs::write(&file, b"PK").expect("file");
        // Called directly, not through `save_downloaded`, whose stream fallback would hide a COM failure.
        attachment_services(&file, None).expect("Attachment Services");
        assert!(zone_stream(&file).contains("ZoneId=3"));
        assert_eq!(std::fs::read(&file).expect("file"), b"PK");
    }

    #[cfg(windows)]
    #[test]
    fn the_zone_stream_never_recreates_a_removed_file() {
        let scratch = Scratch::new("missing");
        let file = scratch.0.join("invoice.exe");
        assert!(write_zone_identifier(&file).is_err());
        assert!(!file.exists());
    }

    #[test]
    fn a_volume_without_streams_still_keeps_the_file() {
        let scratch = Scratch::new("no-streams");
        let file = scratch.0.join("invoice.docx");
        let no_streams = |_: &Path| Err(io::Error::new(io::ErrorKind::Unsupported, "no alternate data streams"));
        save_downloaded_with(&file, b"PK", no_streams).expect("the save does not fail with the zone");
        assert_eq!(std::fs::read(&file).expect("file"), b"PK");
    }

    #[test]
    fn a_file_windows_security_removed_is_not_reported_as_saved() {
        let scratch = Scratch::new("removed");
        let file = scratch.0.join("invoice.exe");
        let antivirus = |path: &Path| {
            std::fs::remove_file(path)?;
            Err(io::Error::other("blocked"))
        };
        assert!(save_downloaded_with(&file, b"MZ", antivirus).is_err());
        assert!(!file.exists());
    }
}
