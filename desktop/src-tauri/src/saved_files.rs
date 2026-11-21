//! Files the app saved during this session (an attachment, an exported message) and what the person
//! can do with them afterwards: open them, or see them in File Explorer.
//!
//! The page never names a path. A save hands it a token, and only a token given by a save of this
//! session opens or reveals anything (docs/aspetto-nativo.md, rule 31): a command that took a path
//! would let any page launch any file on the disk.

use crate::runtime::blocking;
use qamille::application::{AppError, AppResult};
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How many saved files are remembered. The oldest goes first; its token is never given again.
const REMEMBERED: usize = 256;

struct SavedFiles {
    /// The last token given: tokens count up from 1 and are never reused, so 0 is never valid.
    last: u64,
    files: VecDeque<(u64, PathBuf)>,
}

static SAVED: Mutex<SavedFiles> = Mutex::new(SavedFiles::new());

impl SavedFiles {
    const fn new() -> Self {
        SavedFiles { last: 0, files: VecDeque::new() }
    }

    fn remember(&mut self, path: PathBuf) -> u64 {
        self.last += 1;
        self.files.push_back((self.last, path));
        while self.files.len() > REMEMBERED {
            self.files.pop_front();
        }
        self.last
    }

    fn path(&self, token: u64) -> Option<PathBuf> {
        self.files.iter().find(|(known, _)| *known == token).map(|(_, path)| path.clone())
    }
}

/// What a save tells the page: where the file went, to show it, and the token that stands for it.
#[derive(serde::Serialize)]
pub struct SavedFile {
    pub path: String,
    pub token: u64,
}

impl SavedFile {
    pub fn remember(path: PathBuf) -> Self {
        let shown = path.to_string_lossy().into_owned();
        SavedFile { path: shown, token: remember(path) }
    }
}

/// Remembers a file this session saved and returns the token that opens or reveals it.
pub fn remember(path: PathBuf) -> u64 {
    lock(&SAVED).remember(path)
}

fn lock(files: &Mutex<SavedFiles>) -> std::sync::MutexGuard<'_, SavedFiles> {
    files.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// A token this session never gave.
    Unknown,
    /// The file was moved or deleted after the save.
    Gone,
}

/// The words of a failure, in the language the page last reported (`window_shell::italian`).
fn failure_text(failure: Failure, italian: bool) -> &'static str {
    match (failure, italian) {
        (Failure::Unknown, true) => "Qamille non ha salvato questo file in questa sessione.",
        (Failure::Unknown, false) => "Qamille did not save this file in this session.",
        (Failure::Gone, true) => "Il file non c’è più: è stato spostato o eliminato.",
        (Failure::Gone, false) => "The file is gone: it was moved or deleted.",
    }
}

/// Looks the token up and runs `action` on its file. Nothing runs for a token that was not given.
fn act(
    files: &Mutex<SavedFiles>,
    token: u64,
    italian: bool,
    action: impl FnOnce(&Path) -> io::Result<()>,
) -> AppResult<()> {
    let known = lock(files).path(token);
    let path = known.ok_or_else(|| AppError::new("not_found", failure_text(Failure::Unknown, italian)))?;
    // Moved or deleted since the save: said here, not left to a Windows error box. A look that is
    // denied is not a removal, so only a file known to be gone stops here.
    if matches!(path.try_exists(), Ok(false)) {
        return Err(AppError::new("not_found", failure_text(Failure::Gone, italian)));
    }
    action(&path).map_err(|error| AppError::external(error.to_string()))
}

fn open_file(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

/// File Explorer with the file selected. The shell call takes the file itself: no command line is
/// built, so a file name can never become an Explorer switch.
#[cfg(windows)]
fn reveal_file(path: &Path) -> io::Result<()> {
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
    use windows::core::HSTRING;

    // COM on this pooled blocking thread for as long as the call lasts.
    let _apartment = crate::zone::Apartment::enter();
    unsafe {
        let item = ILCreateFromPathW(&HSTRING::from(path));
        if item.is_null() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "Windows cannot name the file"));
        }
        // No siblings given: the item is the one to select, in its own folder.
        let shown = SHOpenFolderAndSelectItems(item, None, 0);
        ILFree(Some(item));
        // Turned into text while COM is still initialised: the error may hold an IErrorInfo.
        shown.map_err(|error| io::Error::other(error.to_string()))
    }
}

/// Off Windows there is no Explorer to select in: the folder that holds the file.
#[cfg(not(windows))]
fn reveal_file(path: &Path) -> io::Result<()> {
    open::that_detached(path.parent().unwrap_or(path))
}

/// Opens the saved file with the program Windows chooses for it. A file that came from the Internet
/// zone (a mail attachment) makes Windows ask first when its type can harm (`zone.rs`).
#[tauri::command]
pub async fn saved_file_open(token: u64) -> AppResult<()> {
    let italian = crate::window_shell::italian();
    blocking(move || act(&SAVED, token, italian, open_file)).await
}

/// Shows the saved file selected in a File Explorer window.
#[tauri::command]
pub async fn saved_file_reveal(token: u64) -> AppResult<()> {
    let italian = crate::window_shell::italian();
    blocking(move || act(&SAVED, token, italian, reveal_file)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A folder owned by one test, under the ignored `desktop/.cache`, removed when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.cache/saved-files-tests")
                .join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("test folder");
            Scratch(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            let file = self.0.join(name);
            std::fs::write(&file, b"PK").expect("file");
            file
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_unknown_token_opens_nothing() {
        let scratch = Scratch::new("unknown");
        let files = Mutex::new(SavedFiles::new());
        let given = lock(&files).remember(scratch.file("invoice.pdf"));
        let ran = Cell::new(0);
        for token in [0, given + 1, u64::MAX] {
            let outcome = act(&files, token, false, |_| {
                ran.set(ran.get() + 1);
                Ok(())
            });
            let error = outcome.expect_err("a token nobody was given is an error");
            assert_eq!(error.message, failure_text(Failure::Unknown, false));
        }
        assert_eq!(ran.get(), 0, "no action ran for a token that was not given");
    }

    #[test]
    fn tokens_are_not_reused() {
        let scratch = Scratch::new("reuse");
        let file = scratch.file("invoice.pdf");
        let mut files = SavedFiles::new();
        let first = files.remember(file.clone());
        let again = files.remember(file.clone());
        assert_ne!(first, again, "the same file saved twice gets two tokens");
        assert_ne!(first, 0, "0 is never a token");
        // Past the limit the oldest is forgotten, and the room it leaves is not given to a new token.
        let mut latest = again;
        for _ in 0..REMEMBERED + 3 {
            let next = files.remember(file.clone());
            assert!(next > latest, "a token only grows");
            latest = next;
        }
        assert_eq!(files.path(first), None, "the oldest file is forgotten once past the limit");
        assert_eq!(files.path(latest), Some(file));
        assert!(files.files.len() <= REMEMBERED);
    }

    #[test]
    fn a_remembered_file_is_acted_on_through_its_token() {
        let scratch = Scratch::new("known");
        let file = scratch.file("invoice.pdf");
        let files = Mutex::new(SavedFiles::new());
        let token = lock(&files).remember(file.clone());
        let seen = Cell::new(None);
        act(&files, token, true, |path| {
            seen.set(Some(path.to_path_buf()));
            Ok(())
        })
        .expect("a known token");
        assert_eq!(seen.take(), Some(file));
    }

    #[test]
    fn a_file_that_is_gone_is_said_not_launched() {
        let scratch = Scratch::new("gone");
        let file = scratch.file("invoice.pdf");
        let files = Mutex::new(SavedFiles::new());
        let token = lock(&files).remember(file.clone());
        std::fs::remove_file(&file).expect("remove");
        let ran = Cell::new(false);
        let error = act(&files, token, false, |_| {
            ran.set(true);
            Ok(())
        })
        .expect_err("a moved or deleted file");
        assert_eq!(error.message, failure_text(Failure::Gone, false));
        assert!(!ran.get(), "Windows is not asked to open what is not there");
    }

    #[test]
    fn a_failed_launch_reaches_the_page() {
        let scratch = Scratch::new("launch");
        let files = Mutex::new(SavedFiles::new());
        let token = lock(&files).remember(scratch.file("invoice.pdf"));
        let error = act(&files, token, false, |_| Err(io::Error::other("no program")))
            .expect_err("the launcher failed");
        assert!(error.message.contains("no program"));
    }

    #[test]
    fn both_languages_have_their_words() {
        for failure in [Failure::Unknown, Failure::Gone] {
            let (italian, english) = (failure_text(failure, true), failure_text(failure, false));
            assert!(!italian.is_empty() && !english.is_empty());
            assert_ne!(italian, english, "{failure:?} says the same in both languages");
        }
    }
}
